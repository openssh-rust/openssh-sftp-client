use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use std::io;

use dashmap::DashMap;
use parking_lot::RwLock;

use tokio::io::AsyncWriteExt;
use tokio_pipe::{AtomicWriteBuffer, PipeWrite};

#[derive(Debug)]
pub(crate) struct WriteEnd(RwLock<PipeWrite>);

impl WriteEnd {
    fn new(writer: PipeWrite) -> Self {
        Self(RwLock::new(writer))
    }

    async fn write_atomic(&self, buf: AtomicWriteBuffer<'_>, len: usize) -> io::Result<()> {
        struct AtomicWriteFuture<'pipe, 'a>(&'pipe PipeWrite, Option<AtomicWriteBuffer<'a>>);

        impl<'pipe, 'a> Future for AtomicWriteFuture<'pipe, 'a> {
            type Output = io::Result<usize>;

            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                Pin::new(self.0).poll_write_atomic(cx, self.1.take().unwrap())
            }
        }

        let bytes = AtomicWriteFuture(&*self.0.read(), Some(buf)).await?;
        if bytes != len {
            Err(io::Error::new(
                io::ErrorKind::Other,
                "tokio_pipe::PipeWrite::poll_write_atomic isn't atomic",
            ))
        } else {
            Ok(())
        }
    }

    async fn write_locked(&self, buf: &[u8]) -> io::Result<()> {
        self.0.write().write_all(buf).await
    }

    async fn write(&self, buf: &[u8]) -> io::Result<()> {
        match AtomicWriteBuffer::new(buf) {
            Some(atomic_buf) => self.write_atomic(atomic_buf, buf.len()).await,
            None => self.write_locked(buf).await,
        }
    }
}
