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
use tokio_pipe::{AtomicWriteIoSlices, PipeWrite, PIPE_BUF};

use arrayvec::ArrayVec;

const MAX_ATOMIC_ATTEMPT: u16 = 50;

#[derive(Debug)]
pub(crate) struct Writer(RwLockAsync<PipeWrite>, MpScBytesQueue);

impl Writer {
    pub(crate) fn new(pipe_write: PipeWrite) -> Self {
        Self(
            RwLockAsync::new(pipe_write),
            MpScBytesQueue::new(NonZeroUsize::new(128).unwrap()),
        )
    }

    /// Return `Ok(true)` is atomic write succeeds, `Ok(false)` if non-atomic
    /// write is required.
    async fn atomic_write_vectored_all(
        &self,
        bufs: AtomicWriteIoSlices<'_, '_>,
        len: usize,
    ) -> Result<bool, io::Error> {
        #[must_use = "futures do nothing unless you `.await` or poll them"]
        struct AtomicWrite<'a>(&'a PipeWrite, AtomicWriteIoSlices<'a, 'a>, u16, u16);

        impl Future for AtomicWrite<'_> {
            type Output = Option<Result<usize, io::Error>>;

            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                if self.2 >= self.3 {
                    return Poll::Ready(None);
                }

                self.2 += 1;

                let writer = Pin::new(self.0);
                let input = self.1;

                writer.poll_write_vectored_atomic(cx, input).map(Some)
            }
        }

        if len == 0 {
            return Ok(true);
        }

        let ret = AtomicWrite(
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
        .transpose()?;

        if let Some(n) = ret {
            if n == 0 {
                Err(io::Error::new(io::ErrorKind::WriteZero, ""))
            } else {
                debug_assert_eq!(n, len);
                Ok(true)
            }
        } else {
            Ok(false)
        }
    }

    /// * `buf` - Must not be empty
    ///
    /// Write to pipe without any buffering.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe, but it might cause the data to be partially written.
    pub(crate) async fn write_all(&mut self, buf: &[u8]) -> Result<(), io::Error> {
        self.0.get_mut().write_all(buf).await
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

            if self.atomic_write_vectored_all(bufs, len).await? {
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

    /// If another thread is flushing or there isn't any
    /// data to write, then `Ok(false)` will be returned.
    ///
    /// # Cancel Safety
    ///
    /// This function is perfectly cancel safe.
    ///
    /// While it is true that it might only partially flushed out the data,
    /// it can be restarted by another thread.
    pub(crate) async fn flush(&self) -> Result<bool, io::Error> {
        // Every io_slice in the slice returned by buffers.get_io_slices() is guaranteed
        // to be non-empty
        let mut buffers = match self.1.get_buffers() {
            Some(buffers) => buffers,
            None => return Ok(false),
        };

        if let Some(bufs) = AtomicWriteIoSlices::new(buffers.get_io_slices()) {
            let len: usize = bufs.into_inner().iter().map(|slice| slice.len()).sum();

            if self.atomic_write_vectored_all(bufs, len).await? {
                let res = !buffers.advance(NonZeroUsize::new(len).unwrap());
                debug_assert!(res);

                return Ok(true);
            }
        }

        // Acquire the mutex to ensure no interleave write
        let mut guard = self.0.write().await;

        loop {
            let n = guard.write_vectored(buffers.get_io_slices()).await?;

            // Since `MpScBytesQueue::get_buffers` guarantees that every `IoSlice`
            // returned must be non-empty, having `0` bytes written is an error
            // likely caused by the close of the read end.
            let n = if let Some(n) = NonZeroUsize::new(n) {
                n
            } else {
                return Err(io::Error::new(io::ErrorKind::WriteZero, ""));
            };

            if !buffers.advance(n) {
                break Ok(true);
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
        // Since `BytesMut` v1.1.0 does not reuse the underlying `Vec` that is shared
        // with other `BytesMut`/`Bytes` if it is too small.
        let mut bytes = BytesMut::with_capacity(256);
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

    fn reserve(&mut self, additional: usize) {
        self.0.reserve(additional);
    }
}
