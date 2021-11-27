use super::Response;

use core::sync::atomic::{AtomicU32, Ordering};
use std::io;
use std::sync::Arc;

use tokio::task::JoinHandle;
use tokio_pipe::{PipeRead, PipeWrite};

mod counted_reader;
mod read_end;
mod response_callback;
mod response_callbacks;
mod threadsafe_waker;
mod write_end;

use counted_reader::CountedReader;
use read_end::ReadEnd;
use response_callback::ResponseCallback;
use response_callbacks::ResponseCallbacks;
use threadsafe_waker::ThreadSafeWaker;
use write_end::WriteEnd;

#[derive(Debug)]
pub struct Client {
    write_end: WriteEnd,
    read_task: JoinHandle<io::Result<()>>,

    response_callbacks: Arc<ResponseCallbacks>,
    request_id: AtomicU32,
}
impl Client {
    async fn negotiate(reader: &mut PipeRead, writer: &mut PipeWrite) {}

    fn get_request_id(&self) -> u32 {
        self.request_id.fetch_add(1, Ordering::Relaxed)
    }

    pub async fn connect(mut reader: PipeRead, mut writer: PipeWrite) -> Self {
        Self::negotiate(&mut reader, &mut writer).await;

        let response_callbacks = Arc::new(ResponseCallbacks::default());

        let mut read_end = ReadEnd::new(reader, response_callbacks.clone());

        Self {
            read_task: tokio::spawn(async move {
                loop {
                    read_end.read_one_response().await?;
                }
            }),
            write_end: WriteEnd::new(writer),
            response_callbacks,
            request_id: AtomicU32::new(0),
        }
    }
}
