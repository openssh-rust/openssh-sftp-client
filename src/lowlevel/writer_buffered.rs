#![forbid(unsafe_code)]

use crate::{AtomicWriteIoSlicesTrait, Writer};

use std::cmp::max;
use std::convert::TryInto;
use std::future::Future;
use std::io;
use std::io::IoSlice;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::{BufMut, Bytes, BytesMut};
use openssh_sftp_protocol::ssh_format::SerBacker;

use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock as RwLockAsync;
use tokio_io_utility::queue::{Buffers, MpScBytesQueue, QueuePusher};

const MAX_ATOMIC_ATTEMPT: u16 = 50;

#[derive(Debug)]
pub(crate) struct WriterBuffered<W>(RwLockAsync<W>, MpScBytesQueue);

impl<W: Writer> WriterBuffered<W> {
    pub(crate) fn new(writer: W) -> Self {
        Self(
            RwLockAsync::new(writer),
            MpScBytesQueue::new(W::io_slices_buffer_len()),
        )
    }

    /// Return `Ok(true)` is atomic write succeeds, `Ok(false)` if non-atomic
    /// write is required.
    pub(crate) async fn atomic_write_vectored_all(
        &self,
        bufs: W::AtomicWriteIoSlices,
    ) -> Result<bool, io::Error> {
        #[must_use = "futures do nothing unless you `.await` or poll them"]
        struct AtomicWrite<'a, W: Writer>(&'a W, W::AtomicWriteIoSlices, u16, u16);

        impl<W: Writer> Future for AtomicWrite<'_, W> {
            type Output = Result<bool, io::Error>;

            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                if self.2 >= self.3 {
                    return Poll::Ready(Ok(false));
                }

                self.2 += 1;

                let writer = Pin::new(self.0);
                let input = self.1;

                writer.poll_write_vectored_atomic(cx, input).map(|res| {
                    res?;
                    Ok(true)
                })
            }
        }

        AtomicWrite(
            &*self.0.read().await,
            bufs,
            0,
            max(
                // PIPE_BUF is 4096, less than u16::MAX,
                // so the result of division must also be less than u16::MAX.
                (W::MAX_ATOMIC_WRITE_LEN / bufs.get_total_len()) as u16,
                MAX_ATOMIC_ATTEMPT,
            ),
        )
        .await
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

    async fn flush_impl(&self, buffers: Buffers<'_>) -> Result<(), io::Error> {
        #[must_use = "futures do nothing unless you `.await` or poll them"]
        struct FlushBufferFuture<'a, W: Writer>(&'a mut W, Buffers<'a>);

        impl<W: Writer> Future for FlushBufferFuture<'_, W> {
            type Output = Result<(), io::Error>;

            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                let writer = Pin::new(&mut *self.0);
                let buffers = &mut self.1;

                writer.poll_flush_buffers(cx, buffers)
            }
        }

        FlushBufferFuture(&mut *self.0.write().await, buffers).await
    }

    /// If another thread is flushing, then `Ok(false)` will be returned.
    ///
    /// # Cancel Safety
    ///
    /// This function is perfectly cancel safe.
    ///
    /// While it is true that it might only partially flushed out the data,
    /// it can be restarted by another thread.
    pub(crate) async fn try_flush(&self) -> Result<bool, io::Error> {
        // Every io_slice in the slice returned by buffers.get_io_slices() is guaranteed
        // to be non-empty
        match self.1.try_get_buffers() {
            Some(buffers) => self.flush_impl(buffers).await.map(|_| true),
            None => Ok(false),
        }
    }

    /// If another thread is flushing, then this function would wait until
    /// the other thread is done.
    ///
    /// # Cancel Safety
    ///
    /// This function is perfectly cancel safe.
    ///
    /// While it is true that it might only partially flushed out the data,
    /// it can be restarted by another thread.
    pub(crate) async fn flush(&self) -> Result<(), io::Error> {
        // Every io_slice in the slice returned by buffers.get_io_slices() is guaranteed
        // to be non-empty
        self.flush_impl(self.1.get_buffers_blocked().await).await
    }

    /// Push the bytes into buffer.
    #[inline(always)]
    pub(crate) fn push(&self, bytes: Bytes) {
        self.1.push(bytes);
    }

    #[inline(always)]
    pub(crate) fn get_pusher(&self) -> QueuePusher<'_> {
        self.1.get_pusher()
    }
}

#[repr(transparent)]
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
}

impl SerBacker for WriteBuffer {
    fn new() -> Self {
        // Since `BytesMut` v1.1.0 does not reuse the underlying `Vec` that is shared
        // with other `BytesMut`/`Bytes` if it is too small.
        let mut bytes = BytesMut::with_capacity(256);
        bytes.put([0_u8, 0_u8, 0_u8, 0_u8].as_ref());
        Self(bytes)
    }

    #[inline(always)]
    fn len(&self) -> usize {
        self.0.len()
    }

    fn get_first_4byte_slice(&mut self) -> &mut [u8; 4] {
        let slice = &mut self.0[..4];
        slice.try_into().unwrap()
    }

    #[inline(always)]
    fn extend_from_slice(&mut self, other: &[u8]) {
        self.0.extend_from_slice(other);
    }

    #[inline(always)]
    fn push(&mut self, byte: u8) {
        self.0.put_u8(byte);
    }

    #[inline(always)]
    fn reset(&mut self) {
        self.0.resize(4, 0);
    }

    #[inline(always)]
    fn reserve(&mut self, additional: usize) {
        self.0.reserve(additional);
    }
}
