use core::sync::atomic::{AtomicU32, Ordering};
use std::io;

use dashmap::DashMap;

use tokio_pipe::{PipeRead, PipeWrite};

mod read_end;
mod threadsafe_waker;
mod write_end;

use read_end::ReadEnd;
use threadsafe_waker::ThreadSafeWaker;
use write_end::WriteEnd;

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
pub struct Client {
    write_end: WriteEnd,
    read_end: ReadEnd,

    response_callbacks: DashMap<u32, (ThreadSafeWaker, ResponseCallback)>,
    request_id: AtomicU32,
}
impl Client {
    async fn negotiate(reader: &mut PipeRead, writer: &mut PipeWrite) {}

    fn get_request_id(&self) -> u32 {
        self.request_id.fetch_add(1, Ordering::Relaxed)
    }

    pub async fn connect(mut reader: PipeRead, mut writer: PipeWrite) -> Self {
        Self::negotiate(&mut reader, &mut writer).await;

        Self {
            read_end: ReadEnd::new(reader),
            write_end: WriteEnd::new(writer),
            response_callbacks: DashMap::new(),
            request_id: AtomicU32::new(0),
        }
    }
}
