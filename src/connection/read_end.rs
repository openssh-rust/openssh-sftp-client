use std::io;

use tokio_pipe::PipeRead;

#[derive(Debug)]
pub(crate) struct ReadEnd {
    reader: PipeRead,
    buffer: Vec<u8>,
}
impl ReadEnd {
    pub(crate) fn new(reader: PipeRead) -> Self {
        Self {
            reader,
            buffer: Vec::new(),
        }
    }

    pub(crate) async fn read_one_response(&mut self) -> io::Result<()> {
        todo!()
    }
}
