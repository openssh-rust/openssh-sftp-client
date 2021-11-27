use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use std::io;

use tokio::sync::RwLock;

use tokio::io::AsyncWriteExt;
use tokio_pipe::{AtomicWriteBuffer, PipeWrite};

#[derive(Debug)]
pub(crate) struct WriteEnd(RwLock<PipeWrite>);

impl WriteEnd {
    pub(crate) fn new(writer: PipeWrite) -> Self {
        Self(RwLock::new(writer))
    }

    async fn write_atomic(&self, buf: AtomicWriteBuffer<'_>) -> io::Result<()> {
        struct AtomicWriteFuture<'a, 'b>(&'a PipeWrite, AtomicWriteBuffer<'b>);

        impl Future for AtomicWriteFuture<'_, '_> {
            type Output = io::Result<usize>;

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                Pin::new(self.0).poll_write_atomic(cx, self.1)
            }
        }

        let bytes = AtomicWriteFuture(&*self.0.read().await, buf).await?;
        if bytes != buf.into_inner().len() {
            panic!("tokio_pipe::PipeWrite::poll_write_atomic isn't atomic")
        }

        Ok(())
    }

    async fn write_locked(&self, buf: &[u8]) -> io::Result<()> {
        self.0.write().await.write_all(buf).await
    }

    async fn write(&self, buf: &[u8]) -> io::Result<()> {
        match AtomicWriteBuffer::new(buf) {
            Some(atomic_buf) => self.write_atomic(atomic_buf).await,
            None => self.write_locked(buf).await,
        }
    }
}
