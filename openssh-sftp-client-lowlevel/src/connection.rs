#![forbid(unsafe_code)]

use super::awaitable_responses::AwaitableResponses;
use super::*;

use std::fmt::Debug;
use std::num::NonZeroUsize;
use std::sync::Arc;

use crate::openssh_sftp_protocol::constants::SSH2_FILEXFER_VERSION;
use tokio::io::AsyncRead;
use tokio_io_utility::queue::{Buffers, MpScBytesQueue};

// TODO:
//  - Support for zero copy syscalls

#[derive(Debug)]
struct SharedDataInner<Buffer, Auxiliary> {
    queue: MpScBytesQueue,
    responses: AwaitableResponses<Buffer>,

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

impl<Buffer: Send + Sync, Auxiliary> SharedData<Buffer, Auxiliary> {
    fn new(buffer_size: NonZeroUsize, auxiliary: Auxiliary) -> Self {
        SharedData(Arc::new(SharedDataInner {
            responses: AwaitableResponses::new(),
            queue: MpScBytesQueue::new(buffer_size),

            auxiliary,
        }))
    }
}

impl<Buffer, Auxiliary> SharedData<Buffer, Auxiliary> {
    pub(crate) fn queue(&self) -> &MpScBytesQueue {
        &self.0.queue
    }

    pub(crate) fn responses(&self) -> &AwaitableResponses<Buffer> {
        &self.0.responses
    }

    /// Returned the auxiliary data.
    pub fn get_auxiliary(&self) -> &Auxiliary {
        &self.0.auxiliary
    }

    /// Return the buffers that need to be flushed.
    pub async fn get_buffers(&self) -> Buffers<'_> {
        self.0.queue.get_buffers_blocked().await
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
}

/// Initialize connection to remote sftp server and
/// negotiate the sftp version.
///
/// User of this function must manually call [`ReadEnd::receive_server_hello`]
/// and manually flush the buffer.
///
/// # Cancel Safety
///
/// This function is not cancel safe.
///
/// After dropping the future, the connection would be in a undefined state.
pub async fn connect<R: AsyncRead, Buffer: ToBuffer + Send + Sync + 'static, Auxiliary>(
    reader: R,
    reader_buffer_len: NonZeroUsize,
    write_end_buffer_size: NonZeroUsize,
    auxiliary: Auxiliary,
) -> Result<(WriteEnd<Buffer, Auxiliary>, ReadEnd<R, Buffer, Auxiliary>), Error> {
    let shared_data = SharedData::new(write_end_buffer_size, auxiliary);

    // Send hello message
    let mut write_end = WriteEnd::new(shared_data);
    write_end.send_hello(SSH2_FILEXFER_VERSION).await?;

    let read_end = ReadEnd::new(reader, reader_buffer_len, (*write_end).clone());

    Ok((write_end, read_end))
}
