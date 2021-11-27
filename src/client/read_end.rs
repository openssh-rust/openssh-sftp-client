use super::ResponseCallbacks;

use std::io;
use std::sync::Arc;

use serde::Deserialize;
use ssh_format::from_bytes;

use tokio_pipe::PipeRead;

#[derive(Debug)]
pub(crate) struct ReadEnd {
    reader: PipeRead,
    buffer: Vec<u8>,
    response_callbacks: Arc<ResponseCallbacks>,
}
impl ReadEnd {
    pub(crate) fn new(reader: PipeRead, response_callbacks: Arc<ResponseCallbacks>) -> Self {
        Self {
            response_callbacks,
            reader,
            buffer: Vec::new(),
        }
    }

    pub(crate) async fn read_one_response(&mut self) -> io::Result<()> {
        todo!()
    }
}
