use super::Response;

use core::sync::atomic::{AtomicU32, Ordering};
use std::io;
use std::sync::Arc;

use tokio::task::JoinHandle;
use tokio_pipe::{PipeRead, PipeWrite};

mod awaitable;
mod read_end;
mod responses;
mod write_end;

use read_end::ReadEnd;
use responses::Responses;
use write_end::WriteEnd;

#[derive(Debug)]
pub struct Connection {
    write_end: WriteEnd,
    read_task: JoinHandle<io::Result<()>>,

    responses: Arc<Responses>,
    request_id: AtomicU32,
}
impl Connection {
    fn get_request_id(&self) -> u32 {
        self.request_id.fetch_add(1, Ordering::Relaxed)
    }

    pub(crate) async fn new(mut reader: PipeRead, mut writer: PipeWrite) -> Self {
        let responses = Arc::new(Responses::default());

        let mut read_end = ReadEnd::new(reader, responses.clone());

        Self {
            read_task: tokio::spawn(async move {
                loop {
                    read_end.read_one_response().await?;
                }
            }),
            write_end: WriteEnd::new(writer),
            responses,
            request_id: AtomicU32::new(0),
        }
    }
}
