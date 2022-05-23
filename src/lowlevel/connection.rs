#![forbid(unsafe_code)]

use super::awaitable_responses::AwaitableResponses;
use super::pin_util::PinnedArc;
use super::writer_buffered::WriterBuffered;
use super::*;

use std::fmt::Debug;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use openssh_sftp_protocol::constants::SSH2_FILEXFER_VERSION;
use pin_project::pin_project;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Notify;

// TODO:
//  - Support for zero copy syscalls

#[derive(Debug)]
#[pin_project]
struct SharedDataInner<W, Buffer, Auxiliary> {
    #[pin]
    writer: WriterBuffered<W>,
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
pub struct SharedData<W, Buffer, Auxiliary = ()>(PinnedArc<SharedDataInner<W, Buffer, Auxiliary>>);

impl<W, Buffer, Auxiliary> Clone for SharedData<W, Buffer, Auxiliary> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<W, Buffer, Auxiliary> Drop for SharedData<W, Buffer, Auxiliary> {
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

impl<W, Buffer, Auxiliary> SharedData<W, Buffer, Auxiliary> {
    pub(crate) fn writer(&self) -> Pin<&WriterBuffered<W>> {
        PinnedArc::deref_pinned(&self.0).project_ref().writer
    }

    pub(crate) fn responses(&self) -> &AwaitableResponses<Buffer> {
        &self.0.responses
    }

    /// `SharedData` is a newtype wrapper for `Arc<SharedDataInner>`,
    /// so this function returns how many `Arc` there are that referred
    /// to the shared data.
    #[inline(always)]
    pub fn strong_count(&self) -> usize {
        PinnedArc::strong_count(&self.0)
    }

    /// Returned the auxiliary data.
    pub fn get_auxiliary(&self) -> &Auxiliary {
        &self.0.auxiliary
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

impl<W, Buffer: Send + Sync, Auxiliary> SharedData<W, Buffer, Auxiliary> {
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
}

impl<W: AsyncWrite, Buffer: Send + Sync, Auxiliary> SharedData<W, Buffer, Auxiliary> {
    /// Flush the write buffer.
    ///
    /// If another thread is flushing, then `Ok(false)` will be returned.
    ///
    /// # Cancel Safety
    ///
    /// Upon cancel, it might only partially flushed out the data, which can be
    /// restarted by another thread.
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
    /// Upon cancel, it might only partially flushed out the data, which can be
    /// restarted by another thread.
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
pub async fn connect<
    R: AsyncRead + Unpin,
    W: AsyncWrite,
    Buffer: ToBuffer + Send + Sync + 'static,
>(
    reader: R,
    writer: W,
) -> Result<(WriteEnd<W, Buffer>, ReadEnd<R, W, Buffer>, Extensions), Error> {
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
pub async fn connect_with_auxiliary<
    R: AsyncRead + Unpin,
    W: AsyncWrite,
    Buffer: ToBuffer + Send + Sync + 'static,
    Auxiliary,
>(
    reader: R,
    writer: W,
    auxiliary: Auxiliary,
) -> Result<
    (
        WriteEnd<W, Buffer, Auxiliary>,
        ReadEnd<R, W, Buffer, Auxiliary>,
        Extensions,
    ),
    Error,
> {
    let shared_data = SharedData(PinnedArc::new(SharedDataInner {
        writer: WriterBuffered::new(writer),
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
