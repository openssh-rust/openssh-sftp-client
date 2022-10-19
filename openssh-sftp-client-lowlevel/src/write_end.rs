#![forbid(unsafe_code)]

use super::*;

use awaitable_responses::ArenaArc;
use connection::SharedData;

use std::borrow::Cow;
use std::convert::TryInto;
use std::fmt::Debug;
use std::io::IoSlice;
use std::ops::{Deref, DerefMut};
use std::path::Path;

use bytes::{BufMut, Bytes, BytesMut};
use openssh_sftp_protocol::file_attrs::FileAttrs;
use openssh_sftp_protocol::request::*;
use openssh_sftp_protocol::serde::Serialize;
use openssh_sftp_protocol::ssh_format::Serializer;
use openssh_sftp_protocol::Handle;

/// It is recommended to create at most one `WriteEnd` per thread
/// using [`WriteEnd::clone`].
#[derive(Debug)]
pub struct WriteEnd<Buffer, Q, Auxiliary = ()> {
    serializer: Serializer<BytesMut>,
    shared_data: SharedData<Buffer, Q, Auxiliary>,
}

impl<Buffer, Q, Auxiliary> Clone for WriteEnd<Buffer, Q, Auxiliary> {
    fn clone(&self) -> Self {
        Self::new(self.shared_data.clone())
    }
}

impl<Buffer, Q, Auxiliary> WriteEnd<Buffer, Q, Auxiliary> {
    /// Create a [`WriteEnd`] from [`SharedData`].
    pub fn new(shared_data: SharedData<Buffer, Q, Auxiliary>) -> Self {
        Self {
            serializer: Serializer::default(),
            shared_data,
        }
    }

    /// Consume the [`WriteEnd`] and return the stored [`SharedData`].
    pub fn into_shared_data(self) -> SharedData<Buffer, Q, Auxiliary> {
        self.shared_data
    }
}

impl<Buffer, Q, Auxiliary> Deref for WriteEnd<Buffer, Q, Auxiliary> {
    type Target = SharedData<Buffer, Q, Auxiliary>;

    fn deref(&self) -> &Self::Target {
        &self.shared_data
    }
}

impl<Buffer, Q, Auxiliary> DerefMut for WriteEnd<Buffer, Q, Auxiliary> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.shared_data
    }
}

impl<Buffer, Q, Auxiliary> WriteEnd<Buffer, Q, Auxiliary>
where
    Buffer: Send + Sync,
    Q: Queue,
{
    pub(crate) async fn send_hello(&mut self, version: u32) -> Result<(), Error> {
        self.shared_data
            .queue()
            .push(Self::serialize(&mut self.serializer, Hello { version })?);

        Ok(())
    }

    fn reset(serializer: &mut Serializer<BytesMut>) {
        serializer.reset_counter();
        // Reserve for the header
        serializer.output.resize(4, 0);
    }

    fn serialize<T>(serializer: &mut Serializer<BytesMut>, value: T) -> Result<Bytes, Error>
    where
        T: Serialize,
    {
        Self::reset(serializer);

        value.serialize(&mut *serializer)?;

        let header = serializer.create_header(0)?;
        // Write the header
        serializer.output[..4].copy_from_slice(&header);

        Ok(serializer.output.split().freeze())
    }

    /// Send requests.
    ///
    /// NOTE that this merely add the request to the buffer, you need to call
    /// [`SharedData::flush`] to actually send the requests.
    fn send_request(
        &mut self,
        id: Id<Buffer>,
        request: RequestInner<'_>,
        buffer: Option<Buffer>,
    ) -> Result<ArenaArc<Buffer>, Error> {
        let serialized = Self::serialize(
            &mut self.serializer,
            Request {
                request_id: ArenaArc::slot(&id.0),
                inner: request,
            },
        )?;

        id.0.reset(buffer);
        self.shared_data.queue().push(serialized);

        Ok(id.into_inner())
    }

    pub fn send_open_file_request(
        &mut self,
        id: Id<Buffer>,
        params: OpenFileRequest<'_>,
    ) -> Result<AwaitableHandle<Buffer>, Error> {
        self.send_request(id, RequestInner::Open(params), None)
            .map(AwaitableHandle::new)
    }

    pub fn send_close_request(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_request(id, RequestInner::Close(handle), None)
            .map(AwaitableStatus::new)
    }

    /// - `buffer` - If set to `None` or the buffer is not long enough,
    ///   then [`crate::Data::AllocatedBox`] will be returned.
    ///
    /// Return [`crate::Data::Buffer`] or
    /// [`crate::Data::AllocatedBox`] if not EOF, otherwise returns
    /// [`crate::Data::Eof`].
    pub fn send_read_request(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
        offset: u64,
        len: u32,
        buffer: Option<Buffer>,
    ) -> Result<AwaitableData<Buffer>, Error> {
        self.send_request(
            id,
            RequestInner::Read {
                handle,
                offset,
                len,
            },
            buffer,
        )
        .map(AwaitableData::new)
    }

    pub fn send_remove_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_request(id, RequestInner::Remove(path), None)
            .map(AwaitableStatus::new)
    }

    pub fn send_rename_request(
        &mut self,
        id: Id<Buffer>,
        oldpath: Cow<'_, Path>,
        newpath: Cow<'_, Path>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_request(id, RequestInner::Rename { oldpath, newpath }, None)
            .map(AwaitableStatus::new)
    }

    /// * `attrs` - [`FileAttrs::get_size`] must be equal to `None`.
    pub fn send_mkdir_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
        attrs: FileAttrs,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_request(id, RequestInner::Mkdir { path, attrs }, None)
            .map(AwaitableStatus::new)
    }

    pub fn send_rmdir_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_request(id, RequestInner::Rmdir(path), None)
            .map(AwaitableStatus::new)
    }

    pub fn send_opendir_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableHandle<Buffer>, Error> {
        self.send_request(id, RequestInner::Opendir(path), None)
            .map(AwaitableHandle::new)
    }

    /// Return all entries in the directory specified by the `handle`, including
    /// `.` and `..`.
    ///
    /// The `filename` only contains the basename.
    pub fn send_readdir_request(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
    ) -> Result<AwaitableNameEntries<Buffer>, Error> {
        self.send_request(id, RequestInner::Readdir(handle), None)
            .map(AwaitableNameEntries::new)
    }

    pub fn send_stat_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableAttrs<Buffer>, Error> {
        self.send_request(id, RequestInner::Stat(path), None)
            .map(AwaitableAttrs::new)
    }

    /// Does not follow symlink
    pub fn send_lstat_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableAttrs<Buffer>, Error> {
        self.send_request(id, RequestInner::Lstat(path), None)
            .map(AwaitableAttrs::new)
    }

    /// * `handle` - Must be opened with `FileMode::READ`.
    pub fn send_fstat_request(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
    ) -> Result<AwaitableAttrs<Buffer>, Error> {
        self.send_request(id, RequestInner::Fstat(handle), None)
            .map(AwaitableAttrs::new)
    }

    pub fn send_setstat_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
        attrs: FileAttrs,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_request(id, RequestInner::Setstat { path, attrs }, None)
            .map(AwaitableStatus::new)
    }

    /// * `handle` - Must be opened with `OpenOptions::write` set.
    pub fn send_fsetstat_request(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
        attrs: FileAttrs,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_request(id, RequestInner::Fsetstat { handle, attrs }, None)
            .map(AwaitableStatus::new)
    }

    pub fn send_readlink_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableName<Buffer>, Error> {
        self.send_request(id, RequestInner::Readlink(path), None)
            .map(AwaitableName::new)
    }

    pub fn send_realpath_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableName<Buffer>, Error> {
        self.send_request(id, RequestInner::Realpath(path), None)
            .map(AwaitableName::new)
    }

    /// Create symlink
    pub fn send_symlink_request(
        &mut self,
        id: Id<Buffer>,
        targetpath: Cow<'_, Path>,
        linkpath: Cow<'_, Path>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_request(
            id,
            RequestInner::Symlink {
                linkpath,
                targetpath,
            },
            None,
        )
        .map(AwaitableStatus::new)
    }

    /// Return limits of the server
    ///
    /// # Precondition
    ///
    /// Requires `extensions::contains(Extensions::LIMITS)` to be true.
    pub fn send_limits_request(
        &mut self,
        id: Id<Buffer>,
    ) -> Result<AwaitableLimits<Buffer>, Error> {
        self.send_request(id, RequestInner::Limits, None)
            .map(AwaitableLimits::new)
    }

    /// This supports canonicalisation of relative paths and those that need
    /// tilde-expansion, i.e. "~", "~/..." and "~user/...".
    ///
    /// These paths are expanded using shell-like rules and the resultant path
    /// is canonicalised similarly to [`WriteEnd::send_realpath_request`].
    ///
    /// # Precondition
    ///
    /// Requires `extensions::contains(Extensions::EXPAND_PATH)` to be true.
    pub fn send_expand_path_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableName<Buffer>, Error> {
        self.send_request(id, RequestInner::ExpandPath(path), None)
            .map(AwaitableName::new)
    }

    /// # Precondition
    ///
    /// Requires `extensions::contains(Extensions::FSYNC)` to be true.
    pub fn send_fsync_request(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_request(id, RequestInner::Fsync(handle), None)
            .map(AwaitableStatus::new)
    }

    /// # Precondition
    ///
    /// Requires `extensions::contains(Extensions::HARDLINK)` to be true.
    pub fn send_hardlink_request(
        &mut self,
        id: Id<Buffer>,
        oldpath: Cow<'_, Path>,
        newpath: Cow<'_, Path>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_request(id, RequestInner::HardLink { oldpath, newpath }, None)
            .map(AwaitableStatus::new)
    }

    /// # Precondition
    ///
    /// Requires `extensions::contains(Extensions::POSIX_RENAME)` to be true.
    pub fn send_posix_rename_request(
        &mut self,
        id: Id<Buffer>,
        oldpath: Cow<'_, Path>,
        newpath: Cow<'_, Path>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_request(id, RequestInner::PosixRename { oldpath, newpath }, None)
            .map(AwaitableStatus::new)
    }

    /// The server MUST copy the data exactly as if the client had issued a
    /// series of [`RequestInner::Read`] requests on the `read_from_handle`
    /// starting at `read_from_offset` and totaling `read_data_length` bytes,
    /// and issued a series of [`RequestInner::Write`] packets on the
    /// `write_to_handle`, starting at the `write_from_offset`, and totaling
    /// the total number of bytes read by the [`RequestInner::Read`] packets.
    ///
    /// The server SHOULD allow `read_from_handle` and `write_to_handle` to
    /// be the same handle as long as the range of data is not overlapping.
    /// This allows data to efficiently be moved within a file.
    ///
    /// If `data_length` is `0`, this imples data should be read until EOF is
    /// encountered.
    ///
    /// There are no protocol restictions on this operation; however, the
    /// server MUST ensure that the user does not exceed quota, etc.  The
    /// server is, as always, free to complete this operation out of order if
    /// it is too large to complete immediately, or to refuse a request that
    /// is too large.
    ///
    /// # Precondition
    ///
    /// Requires `extensions::contains(Extensions::COPY_DATA)` to be true.
    ///
    /// For [openssh-portable], this is available from V_9_0_P1.
    ///
    /// [openssh-portable]: https://github.com/openssh/openssh-portable
    pub fn send_copy_data_request(
        &mut self,
        id: Id<Buffer>,

        read_from_handle: Cow<'_, Handle>,
        read_from_offset: u64,
        read_data_length: u64,

        write_to_handle: Cow<'_, Handle>,
        write_to_offset: u64,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_request(
            id,
            RequestInner::Cp {
                read_from_handle,
                read_from_offset,
                read_data_length,
                write_to_handle,
                write_to_offset,
            },
            None,
        )
        .map(AwaitableStatus::new)
    }
}

impl<Buffer, Q, Auxiliary> WriteEnd<Buffer, Q, Auxiliary>
where
    Buffer: ToBuffer + Send + Sync + 'static,
    Q: Queue,
{
    /// Write will extend the file if writing beyond the end of the file.
    ///
    /// It is legal to write way beyond the end of the file, the semantics
    /// are to write zeroes from the end of the file to the specified offset
    /// and then the data.
    ///
    /// On most operating systems, such writes do not allocate disk space but
    /// instead leave "holes" in the file.
    ///
    /// This function is only suitable for writing small data since it needs to copy the
    /// entire `data` into buffer.
    pub fn send_write_request_buffered(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
        offset: u64,
        data: Cow<'_, [u8]>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        let len: u32 = data.len().try_into()?;

        self.serializer.reserve(
            // 9 bytes for the 4-byte len of packet, 1-byte packet type and
            // 4-byte request id
            9 +
            handle.into_inner().len() +
            // 8 bytes for the offset
            8 +
            // 4 bytes for the lenght of the data to be sent
            4 +
            // len of the data
            len as usize,
        );

        self.send_request(
            id,
            RequestInner::Write {
                handle,
                offset,
                data,
            },
            None,
        )
        .map(AwaitableStatus::new)
    }

    /// Write will extend the file if writing beyond the end of the file.
    ///
    /// It is legal to write way beyond the end of the file, the semantics
    /// are to write zeroes from the end of the file to the specified offset
    /// and then the data.
    ///
    /// On most operating systems, such writes do not allocate disk space but
    /// instead leave "holes" in the file.
    ///
    /// This function is only suitable for writing small data since it needs to copy the
    /// entire `data` into buffer.
    pub fn send_write_request_buffered_vectored(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
        offset: u64,
        io_slices: &[IoSlice<'_>],
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_write_request_buffered_vectored2(id, handle, offset, &[io_slices])
    }

    fn serialize_write_request<'a>(
        serializer: &'a mut Serializer<BytesMut>,
        request_id: u32,
        handle: Cow<'_, Handle>,
        offset: u64,
        len: u32,
    ) -> Result<&'a mut BytesMut, Error> {
        Self::reset(serializer);

        let header = Request::serialize_write_request(serializer, request_id, handle, offset, len)?;

        let buffer = &mut serializer.output;
        // Write the header
        buffer[..4].copy_from_slice(&header);

        Ok(buffer)
    }

    /// Write will extend the file if writing beyond the end of the file.
    ///
    /// It is legal to write way beyond the end of the file, the semantics
    /// are to write zeroes from the end of the file to the specified offset
    /// and then the data.
    ///
    /// On most operating systems, such writes do not allocate disk space but
    /// instead leave "holes" in the file.
    ///
    /// This function is only suitable for writing small data since it needs to copy the
    /// entire `data` into buffer.
    pub fn send_write_request_buffered_vectored2(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
        offset: u64,
        bufs: &[&[IoSlice<'_>]],
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        let len: usize = bufs
            .iter()
            .flat_map(Deref::deref)
            .map(|io_slice| io_slice.len())
            .sum();
        let len: u32 = len.try_into()?;

        self.serializer.reserve(
            // 9 bytes for the 4-byte len of packet, 1-byte packet type and
            // 4-byte request id
            9 +
            handle.into_inner().len() +
            // 8 bytes for the offset
            8 +
            // 4 bytes for the lenght of the data to be sent
            4 +
            // len of the data
            len as usize,
        );

        let buffer = Self::serialize_write_request(
            &mut self.serializer,
            ArenaArc::slot(&id.0),
            handle,
            offset,
            len,
        )?;

        bufs.iter()
            .flat_map(|io_slices| io_slices.iter())
            .for_each(|io_slice| {
                buffer.put_slice(io_slice);
            });

        id.0.reset(None);
        self.shared_data.queue().push(buffer.split().freeze());

        Ok(AwaitableStatus::new(id.into_inner()))
    }

    /// Write will extend the file if writing beyond the end of the file.
    ///
    /// It is legal to write way beyond the end of the file, the semantics
    /// are to write zeroes from the end of the file to the specified offset
    /// and then the data.
    ///
    /// On most operating systems, such writes do not allocate disk space but
    /// instead leave "holes" in the file.
    ///
    /// This function is zero-copy.
    pub fn send_write_request_zero_copy(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
        offset: u64,
        data: &[Bytes],
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_write_request_zero_copy2(id, handle, offset, &[data])
    }

    /// Write will extend the file if writing beyond the end of the file.
    ///
    /// It is legal to write way beyond the end of the file, the semantics
    /// are to write zeroes from the end of the file to the specified offset
    /// and then the data.
    ///
    /// On most operating systems, such writes do not allocate disk space but
    /// instead leave "holes" in the file.
    ///
    /// This function is zero-copy.
    pub fn send_write_request_zero_copy2(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
        offset: u64,
        data_slice: &[&[Bytes]],
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        let len: usize = data_slice
            .iter()
            .flat_map(Deref::deref)
            .map(Bytes::len)
            .sum();
        let len: u32 = len.try_into()?;

        let header = Self::serialize_write_request(
            &mut self.serializer,
            ArenaArc::slot(&id.0),
            handle,
            offset,
            len,
        )?
        .split()
        .freeze();

        id.0.reset(None);
        self.shared_data.queue().extend(header, data_slice);

        Ok(AwaitableStatus::new(id.into_inner()))
    }
}
