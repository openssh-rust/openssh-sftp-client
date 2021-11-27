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

#[derive(Debug)]
pub(crate) struct CountedReader<'a>(&'a PipeRead, usize);
impl CountedReader<'_> {
    fn get_bytes_left(&self) -> usize {
        self.1
    }

    /// Read at most get_bytes_left()
    fn read(&mut self, len: usize) -> io::Result<usize> {
        todo!()
    }
}
impl Drop for CountedReader<'_> {
    fn drop(&mut self) {
        // consume all bytes left readable
    }
}
