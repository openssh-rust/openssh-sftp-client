#![forbid(unsafe_code)]

use super::awaitable_responses::AwaitableResponses;
use super::writer::Writer;
use super::*;

use std::fmt::Debug;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use openssh_sftp_protocol::constants::SSH2_FILEXFER_VERSION;
use tokio::sync::Notify;
use tokio_pipe::{PipeRead, PipeWrite};

// TODO:
//  - Support for zero copy syscalls

#[derive(Debug)]
struct SharedDataInner<Buffer, Auxiliary> {
    writer: Writer,
    responses: AwaitableResponses<Buffer>,

    notify: Notify,
    requests_sent: AtomicU32,

    is_conn_closed: AtomicBool,

    auxiliary: Auxiliary,
}

/// SharedData contains both the writer and the responses because:
///  - The overhead of `Arc` and a separate allocation;
///  - If the write end of a connection is closed, then openssh implementation
///    of sftp-server would close the read end right away, discarding
///    any unsent but processed or unprocessed responses.
#[derive(Debug)]
pub struct SharedData<Buffer, Auxiliary = ()>(Arc<SharedDataInner<Buffer, Auxiliary>>);

impl<Buffer, Auxiliary> Clone for SharedData<Buffer, Auxiliary> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<Buffer, Auxiliary> Drop for SharedData<Buffer, Auxiliary> {
    fn drop(&mut self) {
        // If this is the last reference, except for `ReadEnd`, to the SharedData,
        // then the connection is closed.
        if self.strong_count() == 2 {
            #[cfg(debug_assertions)]
            {
                assert!(!self.0.is_conn_closed.swap(true, Ordering::Relaxed));
            }
            #[cfg(not(debug_assertions))]
            {
                self.0.is_conn_closed.store(true, Ordering::Relaxed);
            }

            self.notify_read_end();
        }
    }
}

impl<Buffer, Auxiliary> SharedData<Buffer, Auxiliary> {
    pub(crate) fn writer(&self) -> &Writer {
        &self.0.writer
    }

    pub(crate) fn responses(&self) -> &AwaitableResponses<Buffer> {
        &self.0.responses
    }

    pub(crate) fn get_mut_writer(&mut self) -> Option<&mut Writer> {
        Arc::get_mut(&mut self.0).map(|shared_data| &mut shared_data.writer)
    }

    /// `SharedData` is a newtype wrapper for `Arc<SharedDataInner>`,
    /// so this function returns how many `Arc` there are that referred
    /// to the shared data.
    #[inline(always)]
    pub fn strong_count(&self) -> usize {
        Arc::strong_count(&self.0)
    }

    /// Returned the auxiliary data.
    pub fn get_auxiliary(&self) -> &Auxiliary {
        &self.0.auxiliary
    }

    /// Return the auxiliary data.
    pub fn get_auxiliary_mut(&mut self) -> Option<&mut Auxiliary> {
        Arc::get_mut(&mut self.0).map(|shared_data| &mut shared_data.auxiliary)
    }

    #[inline(always)]
    fn notify_read_end(&self) {
        // We only have one waiting task, that is `ReadEnd`.
        self.0.notify.notify_one();
    }

    pub(crate) fn notify_new_packet_event(&self) {
        let prev_requests_sent = self.0.requests_sent.fetch_add(1, Ordering::Relaxed);

        debug_assert_ne!(prev_requests_sent, u32::MAX);

        // Notify the `ReadEnd` after the requests_sent is incremented.
        self.notify_read_end();
    }

    /// Return number of requests and clear requests_sent.
    /// **Return 0 if the connection is closed.**
    pub(crate) async fn wait_for_new_request(&self) -> u32 {
        loop {
            let cnt = self.0.requests_sent.swap(0, Ordering::Relaxed);
            if cnt > 0 {
                break cnt;
            }

            if self.0.is_conn_closed.load(Ordering::Relaxed) {
                break 0;
            }

            self.0.notify.notified().await;
        }
    }
}

impl<Buffer: Send + Sync, Auxiliary> SharedData<Buffer, Auxiliary> {
    /// Create a useable response id.
    #[inline(always)]
    pub fn create_response_id(&self) -> Id<Buffer> {
        self.responses().insert()
    }

    /// Return true if reserve succeeds, false otherwise.
    #[inline(always)]
    pub fn try_reserve_id(&self, new_id_cnt: u32) -> bool {
        self.responses().try_reserve(new_id_cnt)
    }

    /// Return true if reserve succeeds, false otherwise.
    #[inline(always)]
    pub fn reserve_id(&self, new_id_cnt: u32) {
        self.responses().reserve(new_id_cnt);
    }

    /// Flush the write buffer.
    ///
    /// If another thread is flushing, then `Ok(false)` will be returned.
    ///
    /// # Cancel Safety
    ///
    /// This function is only cancel safe if [`WriteEnd::send_write_request_direct`] or
    /// [`WriteEnd::send_write_request_direct_vectored`] is not called when this
    /// future is cancelled.
    ///
    /// Upon cancel, it might only partially flushed out the data, which can be
    /// restarted by another thread.
    ///
    /// However, if [`WriteEnd::send_write_request_direct`] or
    /// [`WriteEnd::send_write_request_direct_vectored`] is called, then the write data
    /// will be interleaved and thus produce undefined behavior.
    #[inline(always)]
    pub async fn try_flush(&self) -> Result<bool, io::Error> {
        self.writer().try_flush().await
    }

    /// Flush the write buffer.
    ///
    /// If another thread is flushing, then this function would wait until
    /// the other thread is done.
    ///
    /// # Cancel Safety
    ///
    /// This function is only cancel safe if [`WriteEnd::send_write_request_direct`] or
    /// [`WriteEnd::send_write_request_direct_vectored`] is not called when this
    /// future is cancelled.
    ///
    /// Upon cancel, it might only partially flushed out the data, which can be
    /// restarted by another thread.
    ///
    /// However, if [`WriteEnd::send_write_request_direct`] or
    /// [`WriteEnd::send_write_request_direct_vectored`] is called, then the write data
    /// will be interleaved and thus produce undefined behavior.
    #[inline(always)]
    pub async fn flush(&self) -> Result<(), io::Error> {
        self.writer().flush().await
    }
}

/// Initialize connection to remote sftp server and
/// negotiate the sftp version.
///
/// # Cancel Safety
///
/// This function is not cancel safe.
///
/// After dropping the future, the connection would be in a undefined state.
pub async fn connect<Buffer: ToBuffer + Send + Sync + 'static>(
    reader: PipeRead,
    writer: PipeWrite,
) -> Result<(WriteEnd<Buffer>, ReadEnd<Buffer>, Extensions), Error> {
    connect_with_auxiliary(reader, writer, ()).await
}

/// Initialize connection to remote sftp server and
/// negotiate the sftp version.
///
/// # Cancel Safety
///
/// This function is not cancel safe.
///
/// After dropping the future, the connection would be in a undefined state.
pub async fn connect_with_auxiliary<Buffer: ToBuffer + Send + Sync + 'static, Auxiliary>(
    reader: PipeRead,
    writer: PipeWrite,
    auxiliary: Auxiliary,
) -> Result<
    (
        WriteEnd<Buffer, Auxiliary>,
        ReadEnd<Buffer, Auxiliary>,
        Extensions,
    ),
    Error,
> {
    let shared_data = SharedData(Arc::new(SharedDataInner {
        writer: Writer::new(writer),
        responses: AwaitableResponses::new(),
        notify: Notify::new(),
        requests_sent: AtomicU32::new(0),
        is_conn_closed: AtomicBool::new(false),

        auxiliary,
    }));

    // Send hello message
    let version = SSH2_FILEXFER_VERSION;

    let mut write_end = WriteEnd::new(shared_data);
    write_end.send_hello(version).await?;

    // Receive version and extensions
    let mut read_end = ReadEnd::new(reader, (*write_end).clone());
    let extensions = read_end.receive_server_version(version).await?;

    Ok((write_end, read_end, extensions))
}
