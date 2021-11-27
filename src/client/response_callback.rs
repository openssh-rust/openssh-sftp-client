use std::io;

use tokio_pipe::PipeRead;

/// Prototype
#[derive(Debug)]
pub(crate) struct ResponseCallback {}

impl ResponseCallback {
    /// reader is used to read additional variable length data, especially
    /// one that can be very long (response body of read request).
    ///
    /// Return true if the callback is already called and should be removed.
    async fn call(&mut self, response: u8, reader: CountedReader<'_>) -> io::Result<bool> {
        todo!()
    }
}

#[derive(Debug)]
struct CountedReader<'a>(&'a PipeRead, usize);
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
