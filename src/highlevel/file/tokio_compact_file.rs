use super::super::{BoxedWaitForCancellationFuture, Buffer, Data};
use super::lowlevel::{AwaitableDataFuture, AwaitableStatusFuture, Handle};
use super::utility::take_io_slices;
use super::{max_atomic_write_len, Error, File, Id, WriteEnd, Writer};

use std::borrow::Cow;
use std::cmp::{max, min};
use std::collections::VecDeque;
use std::convert::TryInto;
use std::future::Future;
use std::io::{self, IoSlice};
use std::num::{NonZeroU32, NonZeroUsize};
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::{Buf, Bytes, BytesMut};
use tokio::io::{AsyncBufRead, AsyncRead, AsyncSeek, AsyncWrite, ReadBuf};
use tokio_io_utility::ready;

use derive_destructure2::destructure;

/// The default length of the buffer used in [`TokioCompactFile`].
pub const DEFAULT_BUFLEN: NonZeroUsize = unsafe { NonZeroUsize::new_unchecked(4096) };

/// The default maximum length of the buffer that can be created in
/// [`AsyncRead`] implementation of [`TokioCompactFile`].
pub const DEFAULT_MAX_BUFLEN: NonZeroUsize = unsafe { NonZeroUsize::new_unchecked(4096 * 10) };

fn sftp_to_io_error(sftp_err: Error) -> io::Error {
    match sftp_err {
        Error::IOError(io_error) => io_error,
        sftp_err => io::Error::new(io::ErrorKind::Other, sftp_err),
    }
}

fn send_request<Func, R, W: Writer>(file: &mut File<'_, W>, f: Func) -> Result<R, Error>
where
    Func: FnOnce(&mut WriteEnd<W>, Id, Cow<'_, Handle>, u64) -> Result<R, Error>,
{
    // Get id and offset to avoid reference to file.
    let id = file.inner.get_id_mut();
    let offset = file.offset;

    let (write_end, handle) = file.get_inner();

    // Add request to write buffer
    let awaitable = f(write_end, id, handle, offset)?;

    // Requests is already added to write buffer, so wakeup
    // the `flush_task`.
    write_end.get_auxiliary().wakeup_flush_task();

    Ok(awaitable)
}

/// File that implements [`AsyncRead`], [`AsyncBufRead`], [`AsyncSeek`] and
/// [`AsyncWrite`], which is compatible with
/// [`tokio::fs::File`](https://docs.rs/tokio/latest/tokio/fs/struct.File.html).
#[derive(Debug, destructure)]
pub struct TokioCompactFile<'s, W: Writer> {
    inner: File<'s, W>,

    buffer_len: NonZeroUsize,
    max_buffer_len: NonZeroUsize,
    buffer: BytesMut,

    read_future: Option<AwaitableDataFuture<Buffer>>,
    read_cancellation_future: BoxedWaitForCancellationFuture<'s>,

    write_futures: VecDeque<AwaitableStatusFuture<Buffer>>,
    write_cancellation_future: BoxedWaitForCancellationFuture<'s>,
}

impl<'s, W: Writer> TokioCompactFile<'s, W> {
    /// Create a [`TokioCompactFile`].
    pub fn new(inner: File<'s, W>) -> Self {
        Self::with_capacity(inner, DEFAULT_BUFLEN, DEFAULT_MAX_BUFLEN)
    }

    /// Create a [`TokioCompactFile`].
    ///
    /// * `buffer_len` - buffer len to be used in [`AsyncBufRead`]
    ///   and the minimum length to read in [`AsyncRead`].
    /// * `max_buffer_len` - maximum len to be used in [`AsyncRead`].
    ///
    /// If `max_buffer_len` is less than `buffer_len`, then it will be set to
    /// `buffer_len`.
    pub fn with_capacity(
        inner: File<'s, W>,
        buffer_len: NonZeroUsize,
        mut max_buffer_len: NonZeroUsize,
    ) -> Self {
        if max_buffer_len < buffer_len {
            max_buffer_len = buffer_len;
        }

        Self {
            inner,

            buffer: BytesMut::new(),
            buffer_len,
            max_buffer_len,

            read_future: None,
            read_cancellation_future: BoxedWaitForCancellationFuture::new(),

            write_futures: VecDeque::new(),
            write_cancellation_future: BoxedWaitForCancellationFuture::new(),
        }
    }

    /// Return the inner [`File`].
    pub fn into_inner(self) -> File<'s, W> {
        self.destructure().0
    }

    /// Flush the write buffer, wait for the status report and send
    /// the close request if this is the last reference.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn close(mut self) -> Result<(), Error> {
        let need_flush = self.need_flush;
        let write_end = &mut self.inner.inner.write_end;

        // Only flush if there are pending requests
        if need_flush && write_end.sftp().get_pending_requests() != 0 {
            write_end.sftp().trigger_flushing();
        }

        while let Some(future) = self.write_futures.pop_front() {
            let id = write_end.cancel_if_task_failed(future).await?.0;
            write_end.cache_id_mut(id);
        }

        self.into_inner().close().await
    }

    /// This function is a lower-level call.
    ///
    /// It needs to be paired with the `consume` method or
    /// [`TokioCompactFile::consume_and_return_buffer`] to function properly.
    ///
    /// When calling this method, none of the contents will be "read" in the
    /// sense that later calling read may return the same contents.
    ///
    /// As such, you must consume the corresponding bytes using the methods
    /// listed above.
    ///
    /// An empty buffer returned indicates that the stream has reached EOF.
    ///
    /// This function does not change the offset into the file.
    pub async fn fill_buf(&mut self) -> Result<(), Error> {
        if self.buffer.is_empty() {
            let buffer_len = self.buffer_len.get().try_into().unwrap_or(u32::MAX);
            let buffer_len = NonZeroU32::new(buffer_len).unwrap();

            self.read_into_buffer(buffer_len).await?;
        }

        Ok(())
    }

    /// This can be used together with [`AsyncBufRead`] implementation for
    /// [`TokioCompactFile`] or [`TokioCompactFile::fill_buf`] or
    /// [`TokioCompactFile::read_into_buffer`] to avoid copying data.
    ///
    /// Return empty [`Bytes`] on EOF.
    ///
    /// This function does change the offset into the file.
    pub fn consume_and_return_buffer(&mut self, amt: usize) -> Bytes {
        let buffer = &mut self.buffer;
        let amt = min(amt, buffer.len());
        let bytes = self.buffer.split_to(amt).freeze();

        self.offset += amt as u64;

        bytes
    }

    /// * `amt` - Amount of data to read into the buffer.
    ///
    /// This function is a lower-level call.
    ///
    /// It needs to be paired with the `consume` method or
    /// [`TokioCompactFile::consume_and_return_buffer`] to function properly.
    ///
    /// When calling this method, none of the contents will be "read" in the
    /// sense that later calling read may return the same contents.
    ///
    /// As such, you must consume the corresponding bytes using the methods
    /// listed above.
    ///
    /// An empty buffer returned indicates that the stream has reached EOF.
    ///
    /// This function does not change the offset into the file.
    pub fn poll_read_into_buffer(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        amt: NonZeroU32,
    ) -> Poll<Result<(), Error>> {
        // Dereference it here once so that there will be only
        // one mutable borrow to self.
        let this = &mut *self;

        if !this.is_readable {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::Other,
                "This file is not opened for reading",
            )
            .into()));
        }

        let max_read_len = this.max_read_len();
        let amt = min(amt.get(), max_read_len);

        let future = if let Some(future) = &mut this.read_future {
            // Get the active future.
            //
            // The future might read more/less than remaining,
            // but the offset must be equal to this.offset,
            // since AsyncSeek::start_seek would reset this.future
            // if this.offset is changed.
            future
        } else {
            this.buffer.reserve(amt as usize);
            let cap = this.buffer.capacity();
            let buffer = this.buffer.split_off(cap - (amt as usize));

            let future = send_request(&mut this.inner, |write_end, id, handle, offset| {
                write_end.send_read_request(id, handle, offset, amt, Some(buffer))
            })?
            .wait();

            // Store it in this.read_future
            this.read_future = Some(future);
            this.read_future
                .as_mut()
                .expect("FileFuture::Data is just assigned to self.future!")
        };

        this.read_cancellation_future
            .poll_for_task_failure(cx, this.inner.get_auxiliary())?;

        // Wait for the future
        let res = ready!(Pin::new(future).poll(cx));
        this.read_future = None;
        let (id, data) = res?;

        this.inner.inner.cache_id_mut(id);
        match data {
            Data::Buffer(buffer) => {
                // Since amt != 0, all AwaitableDataFuture created
                // must at least read in one byte.
                debug_assert!(!buffer.is_empty());

                // sftp v3 can at most read in max_read_len bytes.
                debug_assert!(buffer.len() <= max_read_len as usize);

                this.buffer.unsplit(buffer);
            }
            Data::Eof => return Poll::Ready(Ok(())),
            _ => std::unreachable!("Expect Data::Buffer"),
        };

        Poll::Ready(Ok(()))
    }

    /// * `amt` - Amount of data to read into the buffer.
    ///
    /// This function is a lower-level call.
    ///
    /// It needs to be paired with the `consume` method or
    /// [`TokioCompactFile::consume_and_return_buffer`] to function properly.
    ///
    /// When calling this method, none of the contents will be "read" in the
    /// sense that later calling read may return the same contents.
    ///
    /// As such, you must consume the corresponding bytes using the methods
    /// listed above.
    ///
    /// An empty buffer returned indicates that the stream has reached EOF.
    ///
    /// This function does not change the offset into the file.
    pub async fn read_into_buffer(&mut self, amt: NonZeroU32) -> Result<(), Error> {
        #[must_use]
        struct ReadIntoBuffer<'a, 's, W: Writer>(&'a mut TokioCompactFile<'s, W>, NonZeroU32);

        impl<W: Writer> Future for ReadIntoBuffer<'_, '_, W> {
            type Output = Result<(), Error>;

            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                let amt = self.1;
                Pin::new(&mut *self.0).poll_read_into_buffer(cx, amt)
            }
        }

        ReadIntoBuffer(self, amt).await
    }
}

impl<'s, W: Writer> From<File<'s, W>> for TokioCompactFile<'s, W> {
    fn from(inner: File<'s, W>) -> Self {
        Self::new(inner)
    }
}

impl<'s, W: Writer> From<TokioCompactFile<'s, W>> for File<'s, W> {
    fn from(file: TokioCompactFile<'s, W>) -> Self {
        file.into_inner()
    }
}

/// Creates a new [`TokioCompactFile`] instance that shares the
/// same underlying file handle as the existing File instance.
///
/// Reads, writes, and seeks can be performed independently.
impl<W: Writer> Clone for TokioCompactFile<'_, W> {
    fn clone(&self) -> Self {
        Self::with_capacity(self.inner.clone(), self.buffer_len, self.max_buffer_len)
    }
}

impl<'s, W: Writer> Deref for TokioCompactFile<'s, W> {
    type Target = File<'s, W>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<W: Writer> DerefMut for TokioCompactFile<'_, W> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<W: Writer> AsyncSeek for TokioCompactFile<'_, W> {
    fn start_seek(mut self: Pin<&mut Self>, position: io::SeekFrom) -> io::Result<()> {
        let prev_offset = self.offset();
        Pin::new(&mut self.inner).start_seek(position)?;
        let new_offset = self.offset();

        if new_offset != prev_offset {
            // Reset future since they are invalidated by change of offset.
            self.read_future = None;

            // Reset buffer or consume buffer if necessary.
            if new_offset < prev_offset {
                self.buffer.clear();
            } else if let Ok(offset) = (new_offset - prev_offset).try_into() {
                self.consume(offset);
            } else {
                self.buffer.clear();
            }
        }

        Ok(())
    }

    fn poll_complete(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
        Pin::new(&mut self.inner).poll_complete(cx)
    }
}

impl<W: Writer> AsyncBufRead for TokioCompactFile<'_, W> {
    fn poll_fill_buf(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<&[u8]>> {
        if self.buffer.is_empty() {
            let buffer_len = self.buffer_len.get().try_into().unwrap_or(u32::MAX);
            let buffer_len = NonZeroU32::new(buffer_len).unwrap();

            ready!(self.as_mut().poll_read_into_buffer(cx, buffer_len))
                .map_err(sftp_to_io_error)?;
        }

        Poll::Ready(Ok(&Pin::into_inner(self).buffer))
    }

    fn consume(mut self: Pin<&mut Self>, amt: usize) {
        let buffer = &mut self.buffer;
        let amt = min(buffer.len(), amt);

        buffer.advance(amt);
        self.offset += amt as u64;
    }
}

/// [`TokioCompactFile`] can read in at most [`File::max_read_len`] bytes
/// at a time.
impl<W: Writer> AsyncRead for TokioCompactFile<'_, W> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        read_buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if !self.is_readable {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::Other,
                "This file is not opened for reading",
            )));
        }

        let remaining = read_buf.remaining();
        if remaining == 0 {
            return Poll::Ready(Ok(()));
        }

        if self.buffer.is_empty() {
            let n = max(remaining, DEFAULT_BUFLEN.get());
            let n = min(n, self.max_buffer_len.get());
            let n = n.try_into().unwrap_or(u32::MAX);
            let n = NonZeroU32::new(n).unwrap();

            ready!(self.as_mut().poll_read_into_buffer(cx, n)).map_err(sftp_to_io_error)?;
        }

        let n = min(remaining, self.buffer.len());
        read_buf.put_slice(&self.buffer[..n]);
        self.consume(n);

        Poll::Ready(Ok(()))
    }
}

/// [`TokioCompactFile::poll_write`] only writes data to the buffer.
///
/// [`TokioCompactFile::poll_write`] and
/// [`TokioCompactFile::poll_write_vectored`] would send at most one
/// sftp request.
///
/// It is perfectly safe to buffer requests and send them in one go,
/// since sftp v3 guarantees that requests on the same file handler
/// is processed sequentially.
///
/// NOTE that these writes cannot be cancelled.
///
/// One maybe obvious note when using append-mode:
///
/// make sure that all data that belongs together is written
/// to the file in one operation.
///
/// This can be done by concatenating strings before passing them to
/// [`AsyncWrite::poll_write`] or [`AsyncWrite::poll_write_vectored`] and
/// calling [`AsyncWrite::poll_flush`] on [`TokioCompactFile`] when the message
/// is complete.
///
/// Calling [`AsyncWrite::poll_flush`] on [`TokioCompactFile`] would wait on
/// writes in the order they are sent.
///
/// [`TokioCompactFile`] can write at most [`File::max_write_len`] bytes
/// at a time.
impl<W: Writer> AsyncWrite for TokioCompactFile<'_, W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if !self.is_writable {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::Other,
                "This file is not opened for writing",
            )));
        }

        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }

        // sftp v3 cannot send more than self.max_write_len() data at once.
        let max_write_len = self.max_write_len();

        let max_buffered_write = self.max_buffered_write();

        // If max_buffered_write is larger than or equal to max_atomic_write_len,
        // then the direct write is disabled.
        let is_direct_write_enabled = max_buffered_write < max_atomic_write_len::<W>();

        let n: u32 = buf
            .len()
            .try_into()
            .map(|n| min(n, max_write_len))
            .unwrap_or(max_write_len);

        // sftp v3 cannot send more than self.max_write_len() data at once.
        let buf = &buf[..(n as usize)];

        // Dereference it here once so that there will be only
        // one mutable borrow to self.
        let this = &mut *self;

        let file = &mut this.inner;

        let future = if n <= max_buffered_write || !is_direct_write_enabled {
            let future = send_request(file, |write_end, id, handle, offset| {
                write_end.send_write_request_buffered(id, handle, offset, Cow::Borrowed(buf))
            })
            .map_err(sftp_to_io_error)?
            .wait();

            // Since a new request is buffered, flushing is required.
            self.need_flush = true;

            future
        } else {
            let id = file.inner.get_id_mut();
            let offset = file.offset;

            let (write_end, handle) = file.get_inner();

            let future = write_end.send_write_request_direct_atomic(id, handle, offset, buf);
            tokio::pin!(future);

            ready!(future.poll(cx)).map_err(sftp_to_io_error)?.wait()
        };

        self.write_futures.push_back(future);

        // Adjust offset and reset self.future
        Poll::Ready(
            self.start_seek(io::SeekFrom::Current(n as i64))
                .map(|_| n as usize),
        )
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if !self.is_writable {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::Other,
                "This file does not support writing",
            )));
        }

        let this = &mut *self;

        if this.write_futures.is_empty() {
            return Poll::Ready(Ok(()));
        }

        // Flush only if there is pending awaitable writes
        if this.need_flush {
            // Only flush if there are pending requests
            if this.inner.sftp().get_pending_requests() != 0 {
                this.inner.sftp().trigger_flushing();
            }
            this.inner.need_flush = false;
        }

        this.write_cancellation_future
            .poll_for_task_failure(cx, this.inner.get_auxiliary())
            .map_err(sftp_to_io_error)?;

        loop {
            let res = if let Some(future) = this.write_futures.front_mut() {
                ready!(Pin::new(future).poll(cx))
            } else {
                // All futures consumed without error
                break Poll::Ready(Ok(()));
            };

            this.write_futures
                .pop_front()
                .expect("futures should have at least one elements in it");

            // propagate error and recycle id
            this.inner
                .inner
                .cache_id_mut(res.map_err(sftp_to_io_error)?.0);
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll_flush(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        if !self.is_writable {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::Other,
                "This file does not support writing",
            )));
        }

        if bufs.is_empty() {
            return Poll::Ready(Ok(0));
        }

        let max_write_len = self.max_write_len();

        let max_buffered_write = self.max_buffered_write();

        // If max_buffered_write is larger than or equal to max_atomic_write_len,
        // then the direct write is disabled.
        let is_direct_write_enabled = max_buffered_write < max_atomic_write_len::<W>();

        let (n, bufs, buf) = if let Some(res) = take_io_slices(bufs, max_write_len as usize) {
            res
        } else {
            return Poll::Ready(Ok(0));
        };

        let n: u32 = n.try_into().unwrap();

        let buffers = [bufs, &buf];

        // Dereference it here once so that there will be only
        // one mutable borrow to self.
        let this = &mut *self;

        let file = &mut this.inner;

        let future = if n <= max_buffered_write || !is_direct_write_enabled {
            let future = send_request(file, |write_end, id, handle, offset| {
                write_end.send_write_request_buffered_vectored2(id, handle, offset, &buffers)
            })
            .map_err(sftp_to_io_error)?
            .wait();

            // Since a new request is buffered, flushing is required.
            self.need_flush = true;

            future
        } else {
            let id = file.inner.get_id_mut();
            let offset = file.offset;

            let (write_end, handle) = file.get_inner();

            let future =
                write_end.send_write_request_direct_atomic_vectored2(id, handle, offset, &buffers);
            tokio::pin!(future);

            ready!(future.poll(cx)).map_err(sftp_to_io_error)?.wait()
        };

        self.write_futures.push_back(future);

        // Adjust offset and reset self.future
        Poll::Ready(
            self.start_seek(io::SeekFrom::Current(n as i64))
                .map(|_| n as usize),
        )
    }

    fn is_write_vectored(&self) -> bool {
        true
    }
}
