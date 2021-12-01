mod awaitable;
mod awaitable_responses;

use awaitable_responses::*;

use openssh_sftp_protocol::response::Response;

use tokio_pipe::{PipeRead, PipeWrite};

#[derive(Debug)]
pub struct Connection {
    writer: PipeWrite,
    reader: PipeRead,
    responses: AwaitableResponses,
}
impl Connection {
    pub(crate) async fn new(reader: PipeRead, writer: PipeWrite) -> Self {
        Self {
            reader,
            writer,
            responses: AwaitableResponses::default(),
        }
    }
}
