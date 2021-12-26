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

impl Writer {
    pub(crate) fn new(pipe_write: PipeWrite) -> Self {
        Self(RwLock::new(pipe_write))
    }

    async fn atomic_write_all(
        &self,
        buf: AtomicWriteBuffer<'_>,
    ) -> Result<Option<usize>, io::Error> {
        #[must_use = "futures do nothing unless you `.await` or poll them"]
        struct AtomicWrite<'a, 'b>(&'a PipeWrite, AtomicWriteBuffer<'b>, u16, u16);

        impl Future for AtomicWrite<'_, '_> {
            type Output = Option<Result<usize, io::Error>>;

            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                if self.2 >= self.3 {
                    return Poll::Ready(None);
                }

                self.2 += 1;

                let writer = Pin::new(self.0);
                let buf = self.1;

                writer.poll_write_atomic(cx, buf).map(Some)
            }
        }

        AtomicWrite(
            &*self.0.read().await,
            buf,
            0,
            max(
                // PIPE_BUF is 4096, less than u16::MAX,
                // so the result of division must also be less than u16::MAX.
                (PIPE_BUF / buf.into_inner().len()) as u16,
                MAX_ATOMIC_ATTEMPT,
            ),
        )
        .await
        .transpose()
    }

    /// * `buf` - Must not be empty
    pub(crate) async fn write_all(&self, buf: &[u8]) -> Result<(), io::Error> {
        if let Some(buf) = AtomicWriteBuffer::new(buf) {
            if buf.into_inner().len() <= MAX_ATOMIC_SIZE {
                if let Some(_len) = self.atomic_write_all(buf).await? {
                    return Ok(());
                }
            }
        }

        self.0.write().await.write_all(buf).await
    }

    async fn atomic_write_vectored_all(
        &self,
        bufs: AtomicWriteIoSlices<'_, '_>,
        len: usize,
    ) -> Result<Option<usize>, io::Error> {
        #[must_use = "futures do nothing unless you `.await` or poll them"]
        struct AtomicVectoredWrite<'a, 'b, 'c>(
            &'a PipeWrite,
            AtomicWriteIoSlices<'b, 'c>,
            u16,
            u16,
        );

        impl Future for AtomicVectoredWrite<'_, '_, '_> {
            type Output = Option<Result<usize, io::Error>>;

            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                if self.2 >= self.3 {
                    return Poll::Ready(None);
                }

                self.2 += 1;

                let writer = Pin::new(self.0);
                let bufs = self.1;

                writer.poll_write_vectored_atomic(cx, bufs).map(Some)
            }
        }

        AtomicVectoredWrite(
            &*self.0.read().await,
            bufs,
            0,
            max(
                // PIPE_BUF is 4096, less than u16::MAX,
                // so the result of division must also be less than u16::MAX.
                (PIPE_BUF / len) as u16,
                MAX_ATOMIC_ATTEMPT,
            ),
        )
        .await
        .transpose()
    }

    /// * `bufs` - Accmulated len of all buffers must not be `0`.
    pub(crate) async fn write_vectored_all(
        &self,
        bufs: &mut [io::IoSlice<'_>],
    ) -> Result<(), io::Error> {
        if let Some(bufs) = AtomicWriteIoSlices::new(bufs) {
            let len: usize = bufs.into_inner().iter().map(|slice| slice.len()).sum();

            if len <= MAX_ATOMIC_SIZE {
                if let Some(_len) = self.atomic_write_vectored_all(bufs, len).await? {
                    return Ok(());
                }
            }
        }

        write_vectored_all(&mut *self.0.write().await, bufs).await
    }
}
