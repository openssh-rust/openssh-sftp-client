use super::threadsafe_waker::ThreadSafeWaker;

use core::sync::atomic::{AtomicU32, Ordering};

use std::io;

use serde::Deserialize;
use ssh_format::from_bytes;

use dashmap::DashMap;

use tokio_pipe::PipeRead;

/// Prototype
#[derive(Debug)]
struct ResponseCallback {}

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

#[derive(Debug)]
pub(crate) struct ReadEnd {
    reader: PipeRead,
    buffer: Vec<u8>,
    response_callbacks: DashMap<u32, (ThreadSafeWaker, ResponseCallback)>,
    request_id: AtomicU32,
}
impl ReadEnd {
    fn get_request_id(&self) -> u32 {
        self.request_id.fetch_add(1, Ordering::Relaxed)
    }
}
