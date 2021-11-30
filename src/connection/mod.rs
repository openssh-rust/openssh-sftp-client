mod awaitable;
mod read_end;
mod responses;
mod write_end;

use read_end::ReadEnd;
use responses::Responses;
use write_end::WriteEnd;

pub(crate) use responses::{SlotGuard, SlotGuardNoAwait};

use super::Response;

use std::io;
use std::sync::Arc;

use tokio::task::JoinHandle;
use tokio_pipe::{PipeRead, PipeWrite};

#[derive(Debug)]
pub struct Connection {
    write_end: WriteEnd,
    read_task: JoinHandle<io::Result<()>>,

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

    pub(crate) async fn new(mut reader: PipeRead, mut writer: PipeWrite) -> Self {
        let responses = Arc::new(Responses::default());

        let mut read_end = ReadEnd::new(reader);

        Self {
            read_task: tokio::spawn(async move {
                loop {
                    read_end.read_one_response().await?;
                }
            }),
            write_end: WriteEnd::new(writer),
            responses,
        }
    }
}
