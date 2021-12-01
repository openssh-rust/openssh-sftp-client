mod awaitable;
mod responses;

use responses::Responses;

pub(crate) use responses::{SlotGuard, SlotGuardNoAwait};

use openssh_sftp_protocol::response::Response;

use std::io;
use std::sync::Arc;

use tokio::task::JoinHandle;
use tokio_pipe::{PipeRead, PipeWrite};

#[derive(Debug)]
pub struct Connection {
    writer: PipeWrite,
    reader: PipeRead,

    responses: Arc<Responses>,
}
impl Connection {
    pub(crate) fn get_request_id(&self) -> SlotGuard<'_> {
        self.responses.insert()
    }

    /// Return a SlotGuardNoAwait that is automatically removed
    /// when the request is done.
    pub(crate) fn get_request_id_no_await(&self) -> SlotGuardNoAwait<'_> {
        self.responses.insert_no_await()
    }

    pub(crate) async fn new(reader: PipeRead, writer: PipeWrite) -> Self {
        let responses = Arc::new(Responses::default());

        Self {
            reader,
            writer,
            responses,
        }
    }
}
