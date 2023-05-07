#![forbid(unsafe_code)]

use super::{awaitable_responses::AwaitableResponses, *};

use std::{fmt, sync::Arc};

use openssh_sftp_protocol::constants::SSH2_FILEXFER_VERSION;

// TODO:
//  - Support for zero copy syscalls

#[derive(Debug)]
struct SharedDataInner<Buffer, Q, Auxiliary> {
    queue: Q,
    responses: AwaitableResponses<Buffer>,

    auxiliary: Auxiliary,
}

/// SharedData contains both the writer and the responses because:
///  - The overhead of `Arc` and a separate allocation;
///  - If the write end of a connection is closed, then openssh implementation
///    of sftp-server would close the read end right away, discarding
///    any unsent but processed or unprocessed responses.
#[derive(Debug)]
pub struct SharedData<Buffer, Q, Auxiliary = ()>(Arc<SharedDataInner<Buffer, Q, Auxiliary>>);

impl<Buffer, Q, Auxiliary> fmt::Pointer for SharedData<Buffer, Q, Auxiliary> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Pointer::fmt(&self.0, f)
    }
}

impl<Buffer, Q, Auxiliary> Clone for SharedData<Buffer, Q, Auxiliary> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<Buffer: Send + Sync, Q, Auxiliary> SharedData<Buffer, Q, Auxiliary> {
    fn new(queue: Q, auxiliary: Auxiliary) -> Self {
        SharedData(Arc::new(SharedDataInner {
            responses: AwaitableResponses::new(),
            queue,

            auxiliary,
        }))
    }
}

impl<Buffer, Q, Auxiliary> SharedData<Buffer, Q, Auxiliary> {
    pub fn queue(&self) -> &Q {
        &self.0.queue
    }

    pub(crate) fn responses(&self) -> &AwaitableResponses<Buffer> {
        &self.0.responses
    }

    /// Returned the auxiliary data.
    pub fn get_auxiliary(&self) -> &Auxiliary {
        &self.0.auxiliary
    }
}

impl<Buffer: Send + Sync, Q, Auxiliary> SharedData<Buffer, Q, Auxiliary> {
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
/// User of this function must manually create [`ReadEnd`]
/// and manually flush the buffer.
///
/// # Cancel Safety
///
/// This function is not cancel safe.
///
/// After dropping the future, the connection would be in a undefined state.
pub fn connect<Buffer, Q, Auxiliary>(
    queue: Q,
    auxiliary: Auxiliary,
) -> Result<WriteEnd<Buffer, Q, Auxiliary>, Error>
where
    Buffer: ToBuffer + Send + Sync + 'static,
    Q: Queue,
{
    let shared_data = SharedData::new(queue, auxiliary);

    // Send hello message
    let mut write_end = WriteEnd::new(shared_data);
    write_end.send_hello(SSH2_FILEXFER_VERSION)?;

    Ok(write_end)
}
