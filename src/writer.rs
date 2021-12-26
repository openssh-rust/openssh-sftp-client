use std::io;

use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tokio_io_utility::write_vectored_all;
use tokio_pipe::PipeWrite;

#[derive(Debug)]
pub(crate) struct Writer(Mutex<PipeWrite>);

impl Writer {
    pub(crate) fn new(pipe_write: PipeWrite) -> Self {
        Self(Mutex::new(pipe_write))
    }

    pub(crate) async fn write_all(&self, buf: &[u8]) -> Result<(), io::Error> {
        self.0.lock().await.write_all(buf).await
    }

    pub(crate) async fn write_vectored_all(
        &self,
        bufs: &mut [io::IoSlice<'_>],
    ) -> Result<(), io::Error> {
        write_vectored_all(&mut *self.0.lock().await, bufs).await
    }
}
