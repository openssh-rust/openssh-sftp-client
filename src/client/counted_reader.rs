use std::io;

use tokio_pipe::PipeRead;

#[derive(Debug)]
pub(crate) struct CountedReader<'a>(&'a PipeRead, usize);
impl<'a> CountedReader<'a> {
    pub(crate) fn new(pipe: &'a PipeRead, len: usize) -> Self {
        Self(pipe, len)
    }

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
