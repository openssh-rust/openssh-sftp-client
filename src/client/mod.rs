use core::sync::atomic::{AtomicU32, Ordering};
use std::io;

use dashmap::DashMap;
use parking_lot::Mutex;

use tokio_pipe::{PipeRead, PipeWrite};

mod read_end;
mod response_callback;
mod threadsafe_waker;
mod write_end;

use read_end::{CountedReader, ReadEnd};
use response_callback::ResponseCallback;
use threadsafe_waker::ThreadSafeWaker;
use write_end::WriteEnd;

#[derive(Debug)]
pub struct Client {
    write_end: WriteEnd,
    read_end: Mutex<ReadEnd>,

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
            read_end: Mutex::new(ReadEnd::new(reader)),
            write_end: WriteEnd::new(writer),
            response_callbacks: DashMap::new(),
            request_id: AtomicU32::new(0),
        }
    }
}
