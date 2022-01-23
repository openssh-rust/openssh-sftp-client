#![forbid(unsafe_code)]

use std::cmp::max;
use std::future::Future;
use std::io;
use std::io::IoSlice;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::{BufMut, Bytes, BytesMut};
use openssh_sftp_protocol::ssh_format::SerBacker;

use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock as RwLockAsync;
use tokio_io_utility::queue::{MpScBytesQueue, QueuePusher};
use tokio_io_utility::write_vectored_all;
use tokio_pipe::{AtomicWriteBuffer, AtomicWriteIoSlices, PipeWrite, PIPE_BUF};

use arrayvec::ArrayVec;

const MAX_ATOMIC_ATTEMPT: u16 = 50;

#[derive(Debug)]
pub(crate) struct Writer(RwLockAsync<PipeWrite>, MpScBytesQueue);

type PollFn<T> = fn(Pin<&PipeWrite>, cx: &mut Context<'_>, T) -> Poll<Result<usize, io::Error>>;

impl Writer {
    pub(crate) fn new(pipe_write: PipeWrite) -> Self {
        Self(
            RwLockAsync::new(pipe_write),
            MpScBytesQueue::new(NonZeroUsize::new(100).unwrap()),
        )
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
    ///
    /// Write to pipe without any buffering.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe, but it might cause the data to be partially written.
    pub(crate) async fn write_all(&self, buf: &[u8]) -> Result<(), io::Error> {
        if let Some(buf) = AtomicWriteBuffer::new(buf) {
            if self.atomic_write_all(buf).await?.is_some() {
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
    ///
    /// Write to pipe without any buffering.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe, but it might cause the data to be partially written.
    pub(crate) async fn write_vectored_all(
        &self,
        bufs: &mut [io::IoSlice<'_>],
    ) -> Result<(), io::Error> {
        if let Some(bufs) = AtomicWriteIoSlices::new(bufs) {
            let len: usize = bufs.into_inner().iter().map(|slice| slice.len()).sum();

            if self.atomic_write_vectored_all(bufs, len).await?.is_some() {
                return Ok(());
            }
        }

        write_vectored_all(&mut *self.0.write().await, bufs).await
    }

    /// * `bufs` - Accmulated len of all buffers must not be `0`.
    ///
    /// Write to pipe without any buffering.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe, but it might cause the data to be partially written.
    pub(crate) async fn write_vectored_all_with_header(
        &self,
        header: &io::IoSlice<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Result<(), io::Error> {
        if bufs.len() <= 29 {
            let mut vec = ArrayVec::<_, 30>::new();
            vec.push(*header);
            vec.try_extend_from_slice(bufs).unwrap();

            self.write_vectored_all(&mut vec).await
        } else {
            let mut vec = Vec::with_capacity(1 + bufs.len());
            vec.push(*header);
            vec.extend_from_slice(bufs);

            self.write_vectored_all(&mut vec).await
        }
    }

    async fn write_vectored(&self, bufs: &[io::IoSlice<'_>]) -> Result<usize, io::Error> {
        if let Some(bufs) = AtomicWriteIoSlices::new(bufs) {
            let len: usize = bufs.into_inner().iter().map(|slice| slice.len()).sum();

            if let Some(n) = self.atomic_write_vectored_all(bufs, len).await? {
                debug_assert_eq!(n, len);
                return Ok(n);
            }
        }

        self.0.write().await.write_vectored(bufs).await
    }

    /// If another thread is flushing or there isn't any
    /// data to write, then `Ok(None)` will be returned.
    ///
    /// # Cancel Safety
    ///
    /// This function is perfectly cancel safe.
    ///
    /// While it is true that it might only partially flushed out the data,
    /// it can be restarted by another thread.
    pub(crate) async fn flush(&self) -> Result<Option<()>, io::Error> {
        let mut buffers = match self.1.get_buffers() {
            Some(buffers) => buffers,
            None => return Ok(None),
        };
        loop {
            let n = self.write_vectored(buffers.get_io_slices()).await?;
            let n = if let Some(n) = NonZeroUsize::new(n) {
                n
            } else {
                return Err(io::Error::new(io::ErrorKind::WriteZero, ""));
            };

            if !buffers.advance(n) {
                break Ok(Some(()));
            }
        }
    }

    /// Push the bytes into buffer.
    pub(crate) fn push(&self, bytes: Bytes) {
        self.1.push(bytes);
    }

    pub(crate) fn get_pusher(&self) -> QueuePusher<'_> {
        self.1.get_pusher()
    }
}

#[derive(Debug)]
pub(crate) struct WriteBuffer(BytesMut);

impl WriteBuffer {
    /// split out one buffer
    pub(crate) fn split(&mut self) -> Bytes {
        self.0.split().freeze()
    }

    pub(crate) fn put_io_slices(&mut self, io_slices: &[IoSlice<'_>]) {
        for io_slice in io_slices {
            self.0.put_slice(&*io_slice);
        }
    }

    pub(crate) fn reserve(&mut self, len: usize) {
        self.0.reserve(len);
    }
}

impl SerBacker for WriteBuffer {
    fn new() -> Self {
        let mut bytes = BytesMut::with_capacity(4);
        bytes.put([0_u8, 0_u8, 0_u8, 0_u8].as_ref());
        Self(bytes)
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    fn get_first_4byte_slice(&mut self) -> &mut [u8; 4] {
        (&mut (*self.0)[..4]).try_into().unwrap()
    }

    fn extend_from_slice(&mut self, other: &[u8]) {
        self.0.extend_from_slice(other);
    }

    fn push(&mut self, byte: u8) {
        self.0.put_u8(byte);
    }

    fn reset(&mut self) {
        self.0.resize(4, 0);
    }
}
