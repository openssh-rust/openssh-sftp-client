#![forbid(unsafe_code)]

use super::pin_util::PinnedMutexAsync;

use std::convert::TryInto;
use std::io::{self, IoSlice};
use std::num::NonZeroUsize;
use std::pin::Pin;

use crate::openssh_sftp_protocol::ssh_format::SerBacker;
use bytes::{BufMut, Bytes, BytesMut};

use pin_project::pin_project;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio_io_utility::queue::{Buffers, MpScBytesQueue, QueuePusher};

#[derive(Debug)]
#[pin_project]
pub(crate) struct WriterBuffered<W>(#[pin] PinnedMutexAsync<W>, MpScBytesQueue);

impl<W: AsyncWrite> WriterBuffered<W> {
    pub(crate) fn new(writer: W) -> Self {
        Self(
            PinnedMutexAsync::new(writer),
            MpScBytesQueue::new(NonZeroUsize::new(4096).unwrap()),
        )
    }

    /// * `buf` - Must not be empty
    ///
    /// Write to pipe without any buffering.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe, but it might cause the data to be partially written.
    pub(crate) async fn write_all(self: Pin<&Self>, buf: &[u8]) -> Result<(), io::Error> {
        self.project_ref()
            .0
            .lock()
            .await
            .deref_mut_pinned()
            .write_all(buf)
            .await
    }

    async fn flush_impl(self: Pin<&Self>, mut buffers: Buffers<'_>) -> Result<(), io::Error> {
        if buffers.is_empty() {
            return Ok(());
        }

        let mut guard = self.project_ref().0.lock().await;

        loop {
            let n = guard
                .deref_mut_pinned()
                .write_vectored(buffers.get_io_slices())
                .await?;

            // Since `MpScBytesQueue::get_buffers` guarantees that every `IoSlice`
            // returned must be non-empty, having `0` bytes written is an error
            // likely caused by the close of the read end.
            let n =
                NonZeroUsize::new(n).ok_or_else(|| io::Error::new(io::ErrorKind::WriteZero, ""))?;

            if !buffers.advance(n) {
                break Ok(());
            }
        }
    }

    /// If another thread is flushing, then `Ok(false)` will be returned.
    ///
    /// # Cancel Safety
    ///
    /// This function is perfectly cancel safe.
    ///
    /// While it is true that it might only partially flushed out the data,
    /// it can be restarted by another thread.
    pub(crate) async fn try_flush(self: Pin<&Self>) -> Result<bool, io::Error> {
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
    pub(crate) async fn flush(self: Pin<&Self>) -> Result<(), io::Error> {
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
