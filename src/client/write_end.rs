use core::future::Future;
use core::pin::Pin;
use core::slice;
use core::task::{Context, Poll};

use std::io::{self, IoSlice};

use tokio::sync::RwLock;

use tokio::io::AsyncWriteExt;
use tokio_async_write_utility::AsyncWriteUtility;
use tokio_pipe::{AtomicWriteIoSlices, PipeWrite};

#[derive(Debug)]
pub(crate) struct WriteEnd(RwLock<PipeWrite>);

impl WriteEnd {
    pub(crate) fn new(writer: PipeWrite) -> Self {
        Self(RwLock::new(writer))
    }

    async fn writev_atomic(&self, bufs: AtomicWriteIoSlices<'_, '_>) -> io::Result<()> {
        struct AtomicWriteFuture<'a, 'b, 'c>(&'a PipeWrite, AtomicWriteIoSlices<'b, 'c>);

        impl Future for AtomicWriteFuture<'_, '_, '_> {
            type Output = io::Result<usize>;

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                //Pin::new(self.0).poll_write_vectored_atomic(cx, self.1)
                todo!()
            }
        }

        let _bytes = AtomicWriteFuture(&*self.0.read().await, bufs).await?;

        #[cfg(debug)]
        {
            let expected_bytes: usize = bufs.into_inner().iter().map(|slice| slice.len()).sum();
            if _bytes != expected_bytes {
                panic!(
                    "write_atomic(input len = {}), bytes writen {} which is not atomic",
                    expected_bytes, _bytes
                );
            }
        }

        Ok(())
    }

    async fn writev_locked(&self, bufs: &mut [IoSlice<'_>]) -> io::Result<()> {
        if bufs.is_empty() {
            return Ok(());
        }

        self.0.write().await.write_vectored_all(bufs).await
    }

    pub(crate) async fn writev(&self, bufs: &mut [IoSlice<'_>]) -> io::Result<()> {
        match AtomicWriteIoSlices::new(bufs) {
            Some(atomic_bufs) => self.writev_atomic(atomic_bufs).await,
            None => self.writev_locked(bufs).await,
        }
    }
}
