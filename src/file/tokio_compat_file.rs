use crate::{
    cancel_error,
    file::{utility::take_io_slices, File},
    lowlevel::{AwaitableDataFuture, AwaitableStatusFuture, Handle},
    Buffer, Data, Error, Id, WriteEnd,
};

use std::{
    borrow::Cow,
    cmp::{max, min},
    collections::VecDeque,
    convert::TryInto,
    future::Future,
    io::{self, IoSlice},
    mem,
    num::{NonZeroU32, NonZeroUsize},
    ops::{Deref, DerefMut},
    pin::Pin,
    task::{Context, Poll},
};

use bytes::{Buf, Bytes, BytesMut};
use derive_destructure2::destructure;
use pin_project::{pin_project, pinned_drop};
use tokio::io::{AsyncBufRead, AsyncRead, AsyncSeek, AsyncWrite, ReadBuf};
use tokio_io_utility::ready;
use tokio_util::sync::WaitForCancellationFutureOwned;

/// The default length of the buffer used in [`TokioCompatFile`].
pub const DEFAULT_BUFLEN: NonZeroUsize = unsafe { NonZeroUsize::new_unchecked(4096) };

fn sftp_to_io_error(sftp_err: Error) -> io::Error {
    match sftp_err {
        Error::IOError(io_error) => io_error,
        sftp_err => io::Error::new(io::ErrorKind::Other, sftp_err),
    }
}

fn send_request<Func, R>(file: &mut File, f: Func) -> Result<R, Error>
where
    Func: FnOnce(&mut WriteEnd, Id, Cow<'_, Handle>, u64) -> Result<R, Error>,
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
#[pin_project(PinnedDrop)]
pub struct TokioCompatFile {
    inner: File,

    buffer_len: NonZeroUsize,
    buffer: BytesMut,

    write_len: usize,

    read_future: Option<AwaitableDataFuture<Buffer>>,
    write_futures: VecDeque<WriteFutureElement>,

    /// cancellation_fut is not only cancel-safe, but also can be polled after
    /// it is ready.
    ///
    /// Once it is ready, all polls after that immediately return Poll::Ready(())
    #[pin]
    cancellation_future: WaitForCancellationFutureOwned,
}

#[derive(Debug)]
struct WriteFutureElement {
    future: AwaitableStatusFuture<Buffer>,
    write_len: usize,
}

impl TokioCompatFile {
    /// Create a [`TokioCompatFile`] using [`DEFAULT_BUFLEN`].
    pub fn new(inner: File) -> Self {
        Self::with_capacity(inner, DEFAULT_BUFLEN)
    }

    /// Create a [`TokioCompatFile`].
    ///
    /// * `buffer_len` - buffer len to be used in [`AsyncBufRead`]
    ///   and the minimum length to read in [`AsyncRead`].
    pub fn with_capacity(inner: File, buffer_len: NonZeroUsize) -> Self {
        Self {
            cancellation_future: inner.get_auxiliary().cancel_token.clone().cancelled_owned(),

            inner,

            buffer: BytesMut::new(),
            buffer_len,

            write_len: 0,

            read_future: None,
            write_futures: VecDeque::new(),
        }
    }

    /// Return the inner [`File`].
    pub fn into_inner(self) -> File {
        self.destructure().0
    }

    /// Return capacity of the internal buffer
    ///
    /// Note that if there are pending requests, then the actual
    /// capacity might be more than the returned value.
    pub fn capacity(&self) -> usize {
        self.buffer.capacity()
    }

    /// Reserve the capacity of the internal buffer for at least `cap`
    /// bytes.
    pub fn reserve(&mut self, new_cap: usize) {
        let curr_cap = self.capacity();

        if curr_cap < new_cap {
            self.buffer.reserve(new_cap - curr_cap);
        }
    }

    /// Shrink the capacity of the internal buffer to at most `cap`
    /// bytes.
    pub fn shrink_to(&mut self, new_cap: usize) {
        let curr_cap = self.capacity();

        if curr_cap > new_cap {
            self.buffer = BytesMut::with_capacity(new_cap);
        }
    }

    /// This function is a low-level call.
    ///
    /// It needs to be paired with the `consume` method or
    /// [`TokioCompatFile::consume_and_return_buffer`] to function properly.
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
    pub async fn fill_buf(mut self: Pin<&mut Self>) -> Result<(), Error> {
        let this = self.as_mut().project();

        if this.buffer.is_empty() {
            let buffer_len = this.buffer_len.get().try_into().unwrap_or(u32::MAX);
            let buffer_len = NonZeroU32::new(buffer_len).unwrap();

            self.read_into_buffer(buffer_len).await?;
        }

        Ok(())
    }

    /// This can be used together with [`AsyncBufRead`] implementation for
    /// [`TokioCompatFile`] or [`TokioCompatFile::fill_buf`] or
    /// [`TokioCompatFile::read_into_buffer`] to avoid copying data.
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
    /// This function is a low-level call.
    ///
    /// It needs to be paired with the `consume` method or
    /// [`TokioCompatFile::consume_and_return_buffer`] to function properly.
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
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        amt: NonZeroU32,
    ) -> Poll<Result<(), Error>> {
        // Dereference it here once so that there will be only
        // one mutable borrow to self.
        let this = self.project();

        this.inner.check_for_readable()?;

        let max_read_len = this.inner.max_read_len_impl();
        let amt = min(amt.get(), max_read_len);

        let future = if let Some(future) = this.read_future {
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

            let future = send_request(this.inner, |write_end, id, handle, offset| {
                write_end.send_read_request(id, handle, offset, amt, Some(buffer))
            })?
            .wait();

            // Store it in this.read_future
            *this.read_future = Some(future);
            this.read_future
                .as_mut()
                .expect("FileFuture::Data is just assigned to self.future!")
        };

        if this.cancellation_future.poll(cx).is_ready() {
            return Poll::Ready(Err(cancel_error()));
        }

        // Wait for the future
        let res = ready!(Pin::new(future).poll(cx));
        *this.read_future = None;
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
    /// This function is a low-level call.
    ///
    /// It needs to be paired with the `consume` method or
    /// [`TokioCompatFile::consume_and_return_buffer`] to function properly.
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
    pub async fn read_into_buffer(self: Pin<&mut Self>, amt: NonZeroU32) -> Result<(), Error> {
        #[must_use]
        struct ReadIntoBuffer<'a>(Pin<&'a mut TokioCompatFile>, NonZeroU32);

        impl Future for ReadIntoBuffer<'_> {
            type Output = Result<(), Error>;

            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                let amt = self.1;
                self.0.as_mut().poll_read_into_buffer(cx, amt)
            }
        }

        ReadIntoBuffer(self, amt).await
    }

    /// Return the inner file
    pub fn as_mut_file(self: Pin<&mut Self>) -> &mut File {
        self.project().inner
    }

    fn flush_pending_requests(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Result<(), std::io::Error> {
        let this = self.project();

        // Flush only if there is pending awaitable writes
        if this.inner.need_flush {
            // Only flush if there are pending requests
            if this.inner.auxiliary().get_pending_requests() != 0 {
                this.inner.auxiliary().trigger_flushing();
            }
            this.inner.need_flush = false;
        }

        if this.cancellation_future.poll(cx).is_ready() {
            return Err(sftp_to_io_error(cancel_error()));
        }

        Ok(())
    }

    fn flush_one(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        self.as_mut().flush_pending_requests(cx)?;

        let this = self.project();

        let res = if let Some(element) = this.write_futures.front_mut() {
            let res = ready!(Pin::new(&mut element.future).poll(cx));
            *this.write_len -= element.write_len;
            res
        } else {
            // All futures consumed without error
            debug_assert_eq!(*this.write_len, 0);
            return Poll::Ready(Ok(()));
        };

        this.write_futures
            .pop_front()
            .expect("futures should have at least one elements in it");

        // propagate error and recycle id
        this.inner
            .inner
            .cache_id_mut(res.map_err(sftp_to_io_error)?.0);

        Poll::Ready(Ok(()))
    }
}

impl From<File> for TokioCompatFile {
    fn from(inner: File) -> Self {
        Self::new(inner)
    }
}

impl From<TokioCompatFile> for File {
    fn from(file: TokioCompatFile) -> Self {
        file.into_inner()
    }
}

/// Creates a new [`TokioCompatFile`] instance that shares the
/// same underlying file handle as the existing File instance.
///
/// Reads, writes, and seeks can be performed independently.
impl Clone for TokioCompatFile {
    fn clone(&self) -> Self {
        Self::with_capacity(self.inner.clone(), self.buffer_len)
    }
}

impl Deref for TokioCompatFile {
    type Target = File;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for TokioCompatFile {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl AsyncSeek for TokioCompatFile {
    fn start_seek(mut self: Pin<&mut Self>, position: io::SeekFrom) -> io::Result<()> {
        let this = self.as_mut().project();

        let prev_offset = this.inner.offset();
        Pin::new(&mut *this.inner).start_seek(position)?;
        let new_offset = this.inner.offset();

        if new_offset != prev_offset {
            // Reset future since they are invalidated by change of offset.
            *this.read_future = None;

            // Reset buffer or consume buffer if necessary.
            if new_offset < prev_offset {
                this.buffer.clear();
            } else if let Ok(offset) = (new_offset - prev_offset).try_into() {
                if offset > this.buffer.len() {
                    this.buffer.clear();
                } else {
                    this.buffer.advance(offset);
                }
            } else {
                this.buffer.clear();
            }
        }

        Ok(())
    }

    fn poll_complete(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
        Pin::new(self.project().inner).poll_complete(cx)
    }
}

impl AsyncBufRead for TokioCompatFile {
    fn poll_fill_buf(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<&[u8]>> {
        let this = self.as_mut().project();

        if this.buffer.is_empty() {
            let buffer_len = this.buffer_len.get().try_into().unwrap_or(u32::MAX);
            let buffer_len = NonZeroU32::new(buffer_len).unwrap();

            ready!(self.as_mut().poll_read_into_buffer(cx, buffer_len))
                .map_err(sftp_to_io_error)?;
        }

        Poll::Ready(Ok(self.project().buffer))
    }

    fn consume(self: Pin<&mut Self>, amt: usize) {
        let this = self.project();

        let buffer = this.buffer;

        buffer.advance(amt);
        this.inner.offset += amt as u64;
    }
}

impl AsyncRead for TokioCompatFile {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        read_buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.check_for_readable_io_err()?;

        let remaining = read_buf.remaining();
        if remaining == 0 {
            return Poll::Ready(Ok(()));
        }

        if self.buffer.is_empty() {
            let n = max(remaining, DEFAULT_BUFLEN.get());
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

/// [`TokioCompatFile::poll_write`] only writes data to the buffer.
///
/// [`TokioCompatFile::poll_write`] and
/// [`TokioCompatFile::poll_write_vectored`] would send at most one
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
/// calling [`AsyncWrite::poll_flush`] on [`TokioCompatFile`] when the message
/// is complete.
///
/// Calling [`AsyncWrite::poll_flush`] on [`TokioCompatFile`] would wait on
/// writes in the order they are sent.
impl AsyncWrite for TokioCompatFile {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.check_for_writable_io_err()?;

        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }

        // sftp v3 cannot send more than self.max_write_len() data at once.
        let max_write_len = self.max_write_len_impl();

        let mut n: u32 = buf
            .len()
            .try_into()
            .map(|n| min(n, max_write_len))
            .unwrap_or(max_write_len);

        let write_limit = self.get_auxiliary().tokio_compat_file_write_limit();
        let mut write_len = self.write_len;

        if write_len == write_limit {
            ready!(self.as_mut().flush_one(cx))?;
            write_len = self.write_len;
        }

        let new_write_len = match write_len.checked_add(n as usize) {
            Some(new_write_len) if new_write_len > write_limit => {
                n = (write_limit - write_len) as u32;
                write_limit
            }
            None => {
                // case overflow
                // This has to be a separate cases since
                // write_limit could be set to usize::MAX, in which case
                // saturating_add would never return anything larger than it.
                n = (write_limit - write_len) as u32;
                write_limit
            }
            Some(new_write_len) => new_write_len,
        };

        // sftp v3 cannot send more than self.max_write_len() data at once.
        let buf = &buf[..(n as usize)];

        let this = self.as_mut().project();

        let file = this.inner;

        let future = send_request(file, |write_end, id, handle, offset| {
            write_end.send_write_request_buffered(id, handle, offset, Cow::Borrowed(buf))
        })
        .map_err(sftp_to_io_error)?
        .wait();

        // Since a new request is buffered, flushing is required.
        file.need_flush = true;

        this.write_futures.push_back(WriteFutureElement {
            future,
            write_len: n as usize,
        });

        *self.as_mut().project().write_len = new_write_len;

        // Adjust offset and reset self.future
        Poll::Ready(
            self.start_seek(io::SeekFrom::Current(n as i64))
                .map(|_| n as usize),
        )
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.check_for_writable_io_err()?;

        if self.as_mut().project().write_futures.is_empty() {
            return Poll::Ready(Ok(()));
        }

        self.as_mut().flush_pending_requests(cx)?;

        let this = self.project();

        loop {
            let res = if let Some(element) = this.write_futures.front_mut() {
                let res = ready!(Pin::new(&mut element.future).poll(cx));
                *this.write_len -= element.write_len;
                res
            } else {
                // All futures consumed without error
                debug_assert_eq!(*this.write_len, 0);
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
        self.check_for_writable_io_err()?;

        if bufs.is_empty() {
            return Poll::Ready(Ok(0));
        }

        let max_write_len = self.max_write_len_impl();

        let n = if let Some(res) = take_io_slices(bufs, max_write_len as usize) {
            res.0
        } else {
            return Poll::Ready(Ok(0));
        };

        let mut n: u32 = n.try_into().unwrap();

        let write_limit = self.get_auxiliary().tokio_compat_file_write_limit();
        let mut write_len = self.write_len;

        if write_len == write_limit {
            ready!(self.as_mut().flush_one(cx))?;
            write_len = self.write_len;
        }

        let new_write_len = match write_len.checked_add(n as usize) {
            Some(new_write_len) if new_write_len > write_limit => {
                n = (write_limit - write_len) as u32;
                write_limit
            }
            None => {
                // case overflow
                // This has to be a separate cases since
                // write_limit could be set to usize::MAX, in which case
                // saturating_add would never return anything larger than it.
                n = (write_limit - write_len) as u32;
                write_limit
            }
            Some(new_write_len) => new_write_len,
        };

        let (_, bufs, buf) = take_io_slices(bufs, n as usize).unwrap();

        let buffers = [bufs, &buf];

        // Dereference it here once so that there will be only
        // one mutable borrow to self.
        let this = self.as_mut().project();

        let file = this.inner;

        let future = send_request(file, |write_end, id, handle, offset| {
            write_end.send_write_request_buffered_vectored2(id, handle, offset, &buffers)
        })
        .map_err(sftp_to_io_error)?
        .wait();

        // Since a new request is buffered, flushing is required.
        file.need_flush = true;

        this.write_futures.push_back(WriteFutureElement {
            future,
            write_len: n as usize,
        });

        *self.as_mut().project().write_len = new_write_len;

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

impl TokioCompatFile {
    async fn do_drop(
        mut file: File,
        read_future: Option<AwaitableDataFuture<Buffer>>,
        write_futures: VecDeque<WriteFutureElement>,
    ) {
        if let Some(read_future) = read_future {
            // read_future error is ignored since users are no longer interested
            // in this.
            if let Ok((id, _)) = read_future.await {
                file.inner.cache_id_mut(id);
            }
        }
        for write_element in write_futures {
            // There are some pending writes that aren't flushed.
            //
            // While users have dropped TokioCompatFile, presumably because
            // they assume the data has already been written and flushed, it
            // fails and we need to notify our users of the error.
            match write_element.future.await {
                Ok((id, _)) => file.inner.cache_id_mut(id),
                Err(_err) => {
                    #[cfg(feature = "tracing")]
                    tracing::error!(?_err, "failed to write to File")
                }
            }
        }
        if let Err(_err) = file.close().await {
            #[cfg(feature = "tracing")]
            tracing::error!(?_err, "failed to close handle");
        }
    }
}

/// We need to keep polling the read and write futures, otherwise it would drop
/// the internal request ids too early, causing read task to fail
/// when they should not fail.
#[pinned_drop]
impl PinnedDrop for TokioCompatFile {
    fn drop(self: Pin<&mut Self>) {
        let this = self.project();

        let file = this.inner.clone();
        let read_future = this.read_future.take();
        let write_futures = mem::take(this.write_futures);

        let cancellation_fut = file
            .inner
            .get_auxiliary()
            .cancel_token
            .clone()
            .cancelled_owned();

        let do_drop_fut = Self::do_drop(file, read_future, write_futures);

        tokio::spawn(async move {
            tokio::select! {
                biased;

                _ = cancellation_fut => (),
                _ = do_drop_fut => (),
            }
        });
    }
}
