use std::cmp::max;

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use std::io;

use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;
use tokio_io_utility::write_vectored_all;
use tokio_pipe::{AtomicWriteBuffer, AtomicWriteIoSlices, PipeWrite, PIPE_BUF};

const MAX_ATOMIC_SIZE: usize = PIPE_BUF / 2 + PIPE_BUF / 3;
const MAX_ATOMIC_ATTEMPT: u16 = 50;

#[derive(Debug)]
pub(crate) struct Writer(RwLock<PipeWrite>);

type PollFn<T> = fn(Pin<&PipeWrite>, cx: &mut Context<'_>, T) -> Poll<Result<usize, io::Error>>;

impl Writer {
    pub(crate) fn new(pipe_write: PipeWrite) -> Self {
        Self(RwLock::new(pipe_write))
    }

    async fn do_atomic_write_all<T: Copy + Unpin>(
        &self,
        input: T,
        len: usize,
        f: PollFn<T>,
    ) -> Result<Option<usize>, io::Error> {
        #[must_use = "futures do nothing unless you `.await` or poll them"]
        struct AtomicWrite<'a, T>(&'a PipeWrite, T, u16, u16, PollFn<T>);

        impl<T: Copy + Unpin> Future for AtomicWrite<'_, T> {
            type Output = Option<Result<usize, io::Error>>;

            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                if self.2 >= self.3 {
                    return Poll::Ready(None);
                }

                self.2 += 1;

                let writer = Pin::new(self.0);
                let input = self.1;

                self.4(writer, cx, input).map(Some)
            }
        }

        AtomicWrite(
            &*self.0.read().await,
            input,
            0,
            max(
                // PIPE_BUF is 4096, less than u16::MAX,
                // so the result of division must also be less than u16::MAX.
                (PIPE_BUF / len) as u16,
                MAX_ATOMIC_ATTEMPT,
            ),
            f,
        )
        .await
        .transpose()
    }

    async fn atomic_write_all(
        &self,
        buf: AtomicWriteBuffer<'_>,
    ) -> Result<Option<usize>, io::Error> {
        self.do_atomic_write_all(buf, buf.into_inner().len(), PipeWrite::poll_write_atomic)
            .await
    }

    /// * `buf` - Must not be empty
    pub(crate) async fn write_all(&self, buf: &[u8]) -> Result<(), io::Error> {
        if let Some(buf) = AtomicWriteBuffer::new(buf) {
            if buf.into_inner().len() <= MAX_ATOMIC_SIZE
                && self.atomic_write_all(buf).await?.is_some()
            {
                return Ok(());
            }
        }

        self.0.write().await.write_all(buf).await
    }

    async fn atomic_write_vectored_all(
        &self,
        bufs: AtomicWriteIoSlices<'_, '_>,
        len: usize,
    ) -> Result<Option<usize>, io::Error> {
        self.do_atomic_write_all(bufs, len, PipeWrite::poll_write_vectored_atomic)
            .await
    }

    /// * `bufs` - Accmulated len of all buffers must not be `0`.
    pub(crate) async fn write_vectored_all(
        &self,
        bufs: &mut [io::IoSlice<'_>],
    ) -> Result<(), io::Error> {
        if let Some(bufs) = AtomicWriteIoSlices::new(bufs) {
            let len: usize = bufs.into_inner().iter().map(|slice| slice.len()).sum();

            if len <= MAX_ATOMIC_SIZE && self.atomic_write_vectored_all(bufs, len).await?.is_some()
            {
                return Ok(());
            }
        }

        write_vectored_all(&mut *self.0.write().await, bufs).await
    }
}