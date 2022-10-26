use super::{
    lowlevel::{self, CreateFlags, Data, Extensions, FileAttrs, Handle},
    {
        metadata::{MetaData, MetaDataBuilder, Permissions},
        Auxiliary, Error, Id, OwnedHandle, Sftp, WriteEnd,
    },
};

use std::{
    borrow::Cow,
    cmp::min,
    convert::TryInto,
    future::Future,
    io::{self, IoSlice},
    num::NonZeroU64,
    path::Path,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::{Buf, Bytes, BytesMut};
use tokio::io::AsyncSeek;
use tokio_io_utility::IoSliceExt;

mod tokio_compat_file;
pub use tokio_compat_file::{TokioCompatFile, DEFAULT_BUFLEN};

mod utility;
use utility::{take_bytes, take_io_slices};

/// Options and flags which can be used to configure how a file is opened.
#[derive(Debug, Copy, Clone)]
pub struct OpenOptions<'s> {
    sftp: &'s Sftp,
    options: lowlevel::OpenOptions,
    truncate: bool,
    create: bool,
    create_new: bool,
}

impl<'s> OpenOptions<'s> {
    pub(super) fn new(sftp: &'s Sftp) -> Self {
        Self {
            sftp,
            options: lowlevel::OpenOptions::new(),
            truncate: false,
            create: false,
            create_new: false,
        }
    }

    /// Sets the option for read access.
    ///
    /// This option, when true, will indicate that the file
    /// should be read-able if opened.
    pub fn read(&mut self, read: bool) -> &mut Self {
        self.options = self.options.read(read);
        self
    }

    /// Sets the option for write access.
    ///
    /// This option, when true, will indicate that the file
    /// should be write-able if opened.
    ///
    /// If the file already exists, any write calls on it
    /// will overwrite its contents, without truncating it.
    pub fn write(&mut self, write: bool) -> &mut Self {
        self.options = self.options.write(write);
        self
    }

    /// Sets the option for the append mode.
    ///
    /// This option, when `true`, means that writes will append
    /// to a file instead of overwriting previous contents.
    ///
    /// Note that setting `.write(true).append(true)` has
    /// the same effect as setting only `.append(true)`.
    ///
    /// For most filesystems, the operating system guarantees that
    /// all writes are atomic: no writes get mangled because
    /// another process writes at the same time.
    ///
    /// Note that this function doesn’t create the file if it doesn’t exist.
    /// Use the [`OpenOptions::create`] method to do so.
    pub fn append(&mut self, append: bool) -> &mut Self {
        self.options = self.options.append(append);
        self
    }

    /// Sets the option for truncating a previous file.
    ///
    /// If a file is successfully opened with this option
    /// set it will truncate the file to `0` length if it already exists.
    ///
    /// Only take effect if [`OpenOptions::create`] is set to `true`.
    pub fn truncate(&mut self, truncate: bool) -> &mut Self {
        self.truncate = truncate;
        self
    }

    /// Sets the option to create a new file, or open it if it already exists.
    pub fn create(&mut self, create: bool) -> &mut Self {
        self.create = create;
        self
    }

    /// Sets the option to create a new file, failing if it already exists.
    ///
    /// No file is allowed to exist at the target location,
    /// also no (dangling) symlink.
    ///
    /// In this way, if the call succeeds, the file returned
    /// is guaranteed to be new.
    ///
    /// This option is useful because it is atomic.
    ///
    /// Otherwise between checking whether a file exists and
    /// creating a new one, the file may have been
    /// created by another process (a TOCTOU race condition / attack).
    ///
    /// If `.create_new(true)` is set, `.create()` and `.truncate()` are ignored.
    pub fn create_new(&mut self, create_new: bool) -> &mut Self {
        self.create_new = create_new;
        self
    }

    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn open(&self, path: impl AsRef<Path>) -> Result<File<'s>, Error> {
        async fn inner<'s>(this: &OpenOptions<'s>, path: &Path) -> Result<File<'s>, Error> {
            let filename = Cow::Borrowed(path);

            let params = if this.create || this.create_new {
                let flags = if this.create_new {
                    CreateFlags::Excl
                } else if this.truncate {
                    CreateFlags::Trunc
                } else {
                    CreateFlags::None
                };

                this.options.create(filename, flags, FileAttrs::new())
            } else {
                this.options.open(filename)
            };

            let mut write_end = this.sftp.write_end();

            let handle = write_end
                .send_request(|write_end, id| {
                    Ok(write_end.send_open_file_request(id, params)?.wait())
                })
                .await?;

            Ok(File {
                inner: OwnedHandle::new(write_end, handle),

                is_readable: this.options.get_read(),
                is_writable: this.options.get_write(),
                need_flush: false,
                offset: 0,
            })
        }

        inner(self, path.as_ref()).await
    }
}

/// A reference to the remote file.
///
/// Cloning [`File`] instance would return a new one that shares the same
/// underlying file handle as the existing File instance, while reads, writes
/// and seeks can be performed independently.
///
/// If you want a file that implements [`tokio::io::AsyncRead`] and
/// [`tokio::io::AsyncWrite`], checkout [`TokioCompatFile`].
#[derive(Debug)]
pub struct File<'s> {
    inner: OwnedHandle<'s>,

    is_readable: bool,
    is_writable: bool,
    need_flush: bool,
    offset: u64,
}

impl<'s> Clone for File<'s> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            is_writable: self.is_writable,
            is_readable: self.is_readable,
            need_flush: false,
            offset: self.offset,
        }
    }
}

impl<'s> File<'s> {
    fn max_write_len_impl(&self) -> u32 {
        self.get_auxiliary().limits().write_len
    }

    /// The maximum amount of bytes that can be read in one request.
    /// Reading more than that, then your read will be split into multiple requests
    pub(super) fn max_read_len_impl(&self) -> u32 {
        self.get_auxiliary().limits().read_len
    }
}

#[cfg(feature = "ci-tests")]
impl<'s> File<'s> {
    /// The maximum amount of bytes that can be written in one request.
    /// Writing more than that, then your write will be split into multiple requests
    pub fn max_write_len(&self) -> u32 {
        self.max_write_len_impl()
    }

    /// The maximum amount of bytes that can be read in one request.
    /// Reading more than that, then your read will be split into multiple requests
    pub fn max_read_len(&self) -> u32 {
        self.max_read_len_impl()
    }
}

impl<'s> File<'s> {
    fn get_auxiliary(&self) -> &'s Auxiliary {
        self.inner.get_auxiliary()
    }

    fn get_inner(&mut self) -> (&mut WriteEnd, Cow<'_, Handle>) {
        (&mut self.inner.write_end, Cow::Borrowed(&self.inner.handle))
    }

    fn check_for_writable_io_err(&self) -> Result<(), io::Error> {
        if !self.is_writable {
            Err(io::Error::new(
                io::ErrorKind::Other,
                "This file is not opened for writing",
            ))
        } else {
            Ok(())
        }
    }

    fn check_for_writable(&self) -> Result<(), Error> {
        self.check_for_writable_io_err()?;
        Ok(())
    }

    async fn send_writable_request<Func, F, R>(&mut self, f: Func) -> Result<R, Error>
    where
        Func: FnOnce(&mut WriteEnd, Cow<'_, Handle>, Id) -> Result<F, Error> + Send,
        F: Future<Output = Result<(Id, R), Error>> + Send + 'static,
        R: Send,
    {
        self.check_for_writable()?;

        self.inner.send_request(f).await
    }

    fn check_for_readable_io_err(&self) -> Result<(), io::Error> {
        if !self.is_readable {
            Err(io::Error::new(
                io::ErrorKind::Other,
                "This file is not opened for reading",
            ))
        } else {
            Ok(())
        }
    }

    fn check_for_readable(&self) -> Result<(), Error> {
        self.check_for_readable_io_err()?;
        Ok(())
    }

    async fn send_readable_request<Func, F, R>(&mut self, f: Func) -> Result<R, Error>
    where
        Func: FnOnce(&mut WriteEnd, Cow<'_, Handle>, Id) -> Result<F, Error> + Send,
        F: Future<Output = Result<(Id, R), Error>> + Send + 'static,
        R: Send,
    {
        self.check_for_readable()?;

        self.inner.send_request(f).await
    }

    /// Close the [`File`], send the close request
    /// if this is the last reference.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn close(self) -> Result<(), Error> {
        self.inner.close().await
    }

    /// Return the underlying sftp.
    pub fn sftp(&self) -> &'s Sftp {
        self.inner.write_end.sftp()
    }

    /// Change the metadata of a file or a directory.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn set_metadata(&mut self, metadata: MetaData) -> Result<(), Error> {
        let attrs = metadata.into_inner();

        self.send_writable_request(|write_end, handle, id| {
            Ok(write_end.send_fsetstat_request(id, handle, attrs)?.wait())
        })
        .await
    }

    /// Truncates or extends the underlying file, updating the size
    /// of this file to become size.
    ///
    /// If the size is less than the current file’s size, then the file
    /// will be shrunk.
    ///
    /// If it is greater than the current file’s size, then the file
    /// will be extended to size and have all of the intermediate data
    /// filled in with 0s.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn set_len(&mut self, size: u64) -> Result<(), Error> {
        let mut attrs = FileAttrs::new();
        attrs.set_size(size);

        self.set_metadata(MetaData::new(attrs)).await
    }

    /// Attempts to sync all OS-internal metadata to disk.
    ///
    /// This function will attempt to ensure that all in-core data
    /// reaches the filesystem before returning.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn sync_all(&mut self) -> Result<(), Error> {
        if !self
            .get_auxiliary()
            .extensions()
            .contains(Extensions::FSYNC)
        {
            return Err(Error::UnsupportedExtension(&"fsync"));
        }

        self.send_writable_request(|write_end, handle, id| {
            Ok(write_end.send_fsync_request(id, handle)?.wait())
        })
        .await
    }

    /// Changes the permissions on the underlying file.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn set_permissions(&mut self, perm: Permissions) -> Result<(), Error> {
        let metadata = MetaDataBuilder::new().permissions(perm).create();

        self.set_metadata(metadata).await
    }

    /// Queries metadata about the underlying file.
    pub async fn metadata(&mut self) -> Result<MetaData, Error> {
        self.send_readable_request(|write_end, handle, id| {
            Ok(write_end.send_fstat_request(id, handle)?.wait())
        })
        .await
        .map(MetaData::new)
    }

    /// * `n` - number of bytes to read in
    ///
    /// If the [`File`] has reached EOF or `n == 0`, then `None` is returned.
    pub async fn read(&mut self, n: u32, buffer: BytesMut) -> Result<Option<BytesMut>, Error> {
        if n == 0 {
            return Ok(None);
        }

        let offset = self.offset;
        let n: u32 = min(n, self.max_read_len_impl());

        let data = self
            .send_readable_request(|write_end, handle, id| {
                Ok(write_end
                    .send_read_request(id, handle, offset, n, Some(buffer))?
                    .wait())
            })
            .await?;

        let buffer = match data {
            Data::Buffer(buffer) => buffer,
            Data::Eof => return Ok(None),
            _ => std::unreachable!("Expect Data::Buffer"),
        };

        // Adjust offset
        Pin::new(self).start_seek(io::SeekFrom::Current(n as i64))?;

        Ok(Some(buffer))
    }

    /// Write data into the file.
    pub async fn write(&mut self, buf: &[u8]) -> Result<usize, Error> {
        if buf.is_empty() {
            return Ok(0);
        }

        let offset = self.offset;

        // sftp v3 cannot send more than self.max_write_len() data at once.
        let max_write_len = self.max_write_len_impl();

        let n: u32 = buf
            .len()
            .try_into()
            .map(|n| min(n, max_write_len))
            .unwrap_or(max_write_len);

        // sftp v3 cannot send more than self.max_write_len() data at once.
        let buf = &buf[..(n as usize)];

        self.send_writable_request(|write_end, handle, id| {
            Ok(write_end
                .send_write_request_buffered(id, handle, offset, Cow::Borrowed(buf))?
                .wait())
        })
        .await?;

        // Adjust offset
        Pin::new(self).start_seek(io::SeekFrom::Current(n as i64))?;

        Ok(n as usize)
    }

    /// Write from multiple buffer at once.
    pub async fn write_vectorized(&mut self, bufs: &[IoSlice<'_>]) -> Result<usize, Error> {
        if bufs.is_empty() {
            return Ok(0);
        }

        // sftp v3 cannot send more than self.max_write_len() data at once.
        let max_write_len = self.max_write_len_impl();

        let (n, bufs, buf) = if let Some(res) = take_io_slices(bufs, max_write_len as usize) {
            res
        } else {
            return Ok(0);
        };

        let n: u32 = n.try_into().unwrap();

        let buffers = [bufs, &buf];

        let offset = self.offset;

        self.send_writable_request(|write_end, handle, id| {
            Ok(write_end
                .send_write_request_buffered_vectored2(id, handle, offset, &buffers)?
                .wait())
        })
        .await?;

        // Adjust offset
        Pin::new(self).start_seek(io::SeekFrom::Current(n as i64))?;

        Ok(n as usize)
    }

    /// Zero copy write.
    pub async fn write_zero_copy(&mut self, bytes_slice: &[Bytes]) -> Result<usize, Error> {
        if bytes_slice.is_empty() {
            return Ok(0);
        }

        // sftp v3 cannot send more than self.max_write_len() data at once.
        let max_write_len = self.max_write_len_impl();

        let (n, bufs, buf) = if let Some(res) = take_bytes(bytes_slice, max_write_len as usize) {
            res
        } else {
            return Ok(0);
        };

        let buffers = [bufs, &buf];

        let offset = self.offset;

        self.send_writable_request(|write_end, handle, id| {
            Ok(write_end
                .send_write_request_zero_copy2(id, handle, offset, &buffers)?
                .wait())
        })
        .await?;

        // Adjust offset
        Pin::new(self).start_seek(io::SeekFrom::Current(n.try_into().unwrap()))?;

        Ok(n)
    }

    /// * `n` - number of bytes to read in.
    ///
    /// If `n == 0` or EOF is reached, then `buffer` is returned unchanged.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn read_all(
        &mut self,
        mut n: usize,
        mut buffer: BytesMut,
    ) -> Result<BytesMut, Error> {
        if n == 0 {
            return Ok(buffer);
        }

        buffer.reserve(n);

        while n > 0 {
            let len = buffer.len();
            if let Some(bytes) = self
                .read(n.try_into().unwrap_or(u32::MAX), buffer.split_off(len))
                .await?
            {
                n -= bytes.len();
                buffer.unsplit(bytes);
            } else {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "").into());
            }
        }

        Ok(buffer)
    }

    /// Write entire `buf`.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn write_all(&mut self, mut buf: &[u8]) -> Result<(), Error> {
        while !buf.is_empty() {
            let n = self.write(buf).await?;
            buf = &buf[n..];
        }

        Ok(())
    }

    /// Write entire `buf`.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn write_all_vectorized(
        &mut self,
        mut bufs: &mut [IoSlice<'_>],
    ) -> Result<(), Error> {
        if bufs.is_empty() {
            return Ok(());
        }

        loop {
            let mut n = self.write_vectorized(bufs).await?;

            // This loop would also skip all `IoSlice` that is empty
            // until the first non-empty `IoSlice` is met.
            while bufs[0].len() <= n {
                n -= bufs[0].len();
                bufs = &mut bufs[1..];

                if bufs.is_empty() {
                    debug_assert_eq!(n, 0);
                    return Ok(());
                }
            }

            bufs[0] = IoSlice::new(&bufs[0].into_inner()[n..]);
        }
    }

    /// Write entire `buf`.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn write_all_zero_copy(&mut self, mut bufs: &mut [Bytes]) -> Result<(), Error> {
        if bufs.is_empty() {
            return Ok(());
        }

        loop {
            let mut n = self.write_zero_copy(bufs).await?;

            // This loop would also skip all `IoSlice` that is empty
            // until the first non-empty `IoSlice` is met.
            while bufs[0].len() <= n {
                n -= bufs[0].len();
                bufs = &mut bufs[1..];

                if bufs.is_empty() {
                    debug_assert_eq!(n, 0);
                    return Ok(());
                }
            }

            bufs[0].advance(n);
        }
    }

    /// Return the offset of the file.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    async fn copy_to_impl(&mut self, dst: &mut Self, n: u64) -> Result<(), Error> {
        if !self
            .inner
            .get_auxiliary()
            .extensions()
            .contains(Extensions::COPY_DATA)
        {
            return Err(Error::UnsupportedExtension(&"copy_data"));
        }

        dst.check_for_writable()?;

        let offset = self.offset;

        self.send_readable_request(|write_end, handle, id| {
            Ok(write_end
                .send_copy_data_request(
                    id,
                    handle,
                    offset,
                    n,
                    Cow::Borrowed(&dst.inner.handle),
                    dst.offset,
                )?
                .wait())
        })
        .await?;

        // Adjust offset
        Pin::new(self).start_seek(io::SeekFrom::Current(n.try_into().unwrap()))?;
        Pin::new(dst).start_seek(io::SeekFrom::Current(n.try_into().unwrap()))?;

        Ok(())
    }

    /// Copy `n` bytes of data from `self` to `dst`.
    ///
    /// The server MUST copy the data exactly as if the data is copied
    /// using a series of read and write.
    ///
    /// There are no protocol restictions on this operation; however, the
    /// server MUST ensure that the user does not exceed quota, etc.  The
    /// server is, as always, free to complete this operation out of order if
    /// it is too large to complete immediately, or to refuse a request that
    /// is too large.
    ///
    /// After a successful function call, the offset of `self` and `dst`
    /// are increased by `n`.
    ///
    /// # Precondition
    ///
    /// Requires extension copy_data.
    /// For [openssh-portable], this is available from V_9_0_P1.
    ///
    /// If the extension is not supported by the server, this function
    /// would fail with [`Error::UnsupportedExtension`].
    ///
    /// [openssh-portable]: https://github.com/openssh/openssh-portable
    pub async fn copy_to(&mut self, dst: &mut Self, n: NonZeroU64) -> Result<(), Error> {
        self.copy_to_impl(dst, n.get()).await
    }

    /// Copy data from `self` to `dst` until EOF is encountered.
    ///
    /// The server MUST copy the data exactly as if the data is copied
    /// using a series of read and write.
    ///
    /// There are no protocol restictions on this operation; however, the
    /// server MUST ensure that the user does not exceed quota, etc.  The
    /// server is, as always, free to complete this operation out of order if
    /// it is too large to complete immediately, or to refuse a request that
    /// is too large.
    ///
    /// After a successful function call, the offset of `self` and `dst`
    /// are unchanged.
    ///
    /// # Precondition
    ///
    /// Requires extension copy_data.
    /// For [openssh-portable], this is available from V_9_0_P1.
    ///
    /// If the extension is not supported by the server, this function
    /// would fail with [`Error::UnsupportedExtension`].
    ///
    /// [openssh-portable]: https://github.com/openssh/openssh-portable
    pub async fn copy_all_to(&mut self, dst: &mut Self) -> Result<(), Error> {
        self.copy_to_impl(dst, 0).await
    }
}

impl AsyncSeek for File<'_> {
    /// start_seek only adjust local offset since sftp protocol
    /// does not provides a seek function.
    ///
    /// Instead, offset is provided when sending read/write requests,
    /// thus errors are reported at read/write.
    fn start_seek(mut self: Pin<&mut Self>, position: io::SeekFrom) -> io::Result<()> {
        use io::SeekFrom::*;

        match position {
            Start(pos) => self.offset = pos,
            End(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Seeking from the end is unsupported",
                ))
            }
            Current(n) => {
                if n >= 0 {
                    self.offset =
                        self.offset
                            .checked_add(n.try_into().unwrap())
                            .ok_or_else(|| {
                                io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    "Overflow occured during seeking",
                                )
                            })?;
                } else {
                    self.offset = self
                        .offset
                        .checked_sub((-n).try_into().unwrap())
                        .ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                "Underflow occured during seeking",
                            )
                        })?;
                }
            }
        }

        Ok(())
    }

    /// This function is a no-op, it simply return the offset.
    fn poll_complete(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
        Poll::Ready(Ok(self.offset))
    }
}
