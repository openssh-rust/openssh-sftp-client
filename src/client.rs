use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicU32, Ordering};
use core::task::{Context, Poll, Waker};

use std::io;

use serde::Deserialize;
use ssh_format::from_bytes;

use dashmap::DashMap;
use parking_lot::RwLock;

use tokio::io::AsyncWriteExt;
use tokio_pipe::{AtomicWriteBuffer, PipeRead, PipeWrite};

#[derive(Debug)]
struct WriteEnd(RwLock<PipeWrite>);

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

/// Prototype
#[derive(Debug)]
struct ResponseCallback {}

impl ResponseCallback {
    async fn call(&mut self, response: u8) -> io::Result<()> {
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
struct ReadEnd {
    reader: PipeRead,
    buffer: Vec<u8>,
    response_callbacks: DashMap<u32, (Waker, ResponseCallback)>,
    request_id: AtomicU32,
}
impl ReadEnd {
    fn get_request_id(&self) -> u32 {
        self.request_id.fetch_add(1, Ordering::Relaxed)
    }
}

#[derive(Debug)]
pub struct Client {
    write_end: WriteEnd,
    read_end: ReadEnd,
}
