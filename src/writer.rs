#![forbid(unsafe_code)]

use std::io::{self, IoSlice};
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::AsyncWrite;
use tokio_io_utility::{queue::Buffers, ready};

/// trait for Writer that can be used as output for sftp client.
pub trait Writer: AsyncWrite + Unpin {
    /// The maximum length of which `poll_write_vectored_atomic` can accept.
    const MAX_ATOMIC_WRITE_LEN: usize;

    /// Return `0` if and only if the the `bufs` cannot be written atomically.
    ///
    /// For write zero error, use `io::ErrorKind::WriteZero`.
    ///
    /// The write is atomic, either if fails, or it writes all `bufs` at once.
    fn poll_write_vectored_atomic(
        &self,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<()>>;

    /// Flush the `buffers`.
    ///
    /// It is cancellable and can be restarted with another thread.
    fn poll_flush_buffers(
        &mut self,
        cx: &mut Context<'_>,
        buffers: &mut Buffers<'_>,
    ) -> Poll<io::Result<()>> {
        if buffers.is_empty() {
            return Poll::Ready(Ok(()));
        }

        loop {
            let n = ready!(Pin::new(&mut *self).poll_write_vectored(cx, buffers.get_io_slices()))?;

            // Since `MpScBytesQueue::get_buffers` guarantees that every `IoSlice`
            // returned must be non-empty, having `0` bytes written is an error
            // likely caused by the close of the read end.
            let n =
                NonZeroUsize::new(n).ok_or_else(|| io::Error::new(io::ErrorKind::WriteZero, ""))?;

            if !buffers.advance(n) {
                break Poll::Ready(Ok(()));
            }
        }
    }

    /// Return the length of [`IoSlice`] buffer.
    ///
    /// Return `0` if your `poll_flush_buffers` is completely zero-copy
    /// by using [`Buffers::drain_bytes`] API.
    fn io_slices_buffer_len() -> NonZeroUsize {
        NonZeroUsize::new(128).unwrap()
    }
}

#[cfg(unix)]
mod pipe_write_implementation {
    use super::*;
    use tokio_pipe::{AtomicWriteIoSlices, PipeWrite, PIPE_BUF};

    impl Writer for PipeWrite {
        const MAX_ATOMIC_WRITE_LEN: usize = PIPE_BUF;

        fn poll_write_vectored_atomic(
            &self,
            cx: &mut Context<'_>,
            bufs: &[IoSlice<'_>],
        ) -> Poll<io::Result<()>> {
            match AtomicWriteIoSlices::new(bufs) {
                Some(bufs) if bufs.get_total_len() != 0 => Pin::new(self)
                    .poll_write_vectored_atomic(cx, bufs)
                    .map(|res| {
                        let n = res?;
                        if n == 0 {
                            Err(io::Error::new(io::ErrorKind::WriteZero, ""))
                        } else {
                            debug_assert_eq!(n, bufs.get_total_len());
                            Ok(())
                        }
                    }),
                _ => Poll::Ready(Ok(())),
            }
        }
    }
}
