#![forbid(unsafe_code)]

use super::awaitable_responses::AwaitableResponses;
use super::writer_buffered::WriterBuffered;
use super::*;

use std::fmt::Debug;
use std::io;
use std::pin::Pin;
use std::sync::Arc;

use crate::openssh_sftp_protocol::constants::SSH2_FILEXFER_VERSION;
use pin_project::pin_project;
use tokio::io::{AsyncRead, AsyncWrite};

// TODO:
//  - Support for zero copy syscalls

#[derive(Debug)]
#[pin_project]
struct SharedDataInner<W, Buffer, Auxiliary> {
    #[pin]
    writer: WriterBuffered<W>,
    responses: AwaitableResponses<Buffer>,

    auxiliary: Auxiliary,
}

/// SharedData contains both the writer and the responses because:
///  - The overhead of `Arc` and a separate allocation;
///  - If the write end of a connection is closed, then openssh implementation
///    of sftp-server would close the read end right away, discarding
///    any unsent but processed or unprocessed responses.
#[derive(Debug)]
pub struct SharedData<W, Buffer, Auxiliary = ()>(Pin<Arc<SharedDataInner<W, Buffer, Auxiliary>>>);

impl<W, Buffer, Auxiliary> Clone for SharedData<W, Buffer, Auxiliary> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<W: AsyncWrite, Buffer: Send + Sync, Auxiliary> SharedData<W, Buffer, Auxiliary> {
    fn new(writer: W, auxiliary: Auxiliary) -> Self {
        SharedData(Arc::pin(SharedDataInner {
            writer: WriterBuffered::new(writer),
            responses: AwaitableResponses::new(),

            auxiliary,
        }))
    }
}

impl<W, Buffer, Auxiliary> SharedData<W, Buffer, Auxiliary> {
    pub(crate) fn writer(&self) -> Pin<&WriterBuffered<W>> {
        self.0.as_ref().project_ref().writer
    }

    pub(crate) fn responses(&self) -> &AwaitableResponses<Buffer> {
        &self.0.responses
    }

    /// Returned the auxiliary data.
    pub fn get_auxiliary(&self) -> &Auxiliary {
        &self.0.auxiliary
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
    let (write_end, mut read_end) =
        connect_with_auxiliary_relaxed_unpin(reader, writer, auxiliary).await?;

    // Receive version and extensions
    let extensions = read_end.receive_server_hello().await?;

    Ok((write_end, read_end, extensions))
}

/// Initialize connection to remote sftp server and
/// negotiate the sftp version.
///
/// User of this function must manually call [`ReadEnd::receive_server_hello`].
///
/// # Cancel Safety
///
/// This function is not cancel safe.
///
/// After dropping the future, the connection would be in a undefined state.
pub async fn connect_with_auxiliary_relaxed_unpin<
    R: AsyncRead,
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
    ),
    Error,
> {
    let shared_data = SharedData::new(writer, auxiliary);

    // Send hello message

    let mut write_end = WriteEnd::new(shared_data);
    write_end.send_hello(SSH2_FILEXFER_VERSION).await?;

    // Receive version and extensions
    let read_end = ReadEnd::new(reader, (*write_end).clone());

    Ok((write_end, read_end))
}
