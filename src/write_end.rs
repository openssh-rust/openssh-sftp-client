#![forbid(unsafe_code)]

use crate::*;
use awaitable_responses::ArenaArc;
use connection::SharedData;
use writer::WriteBuffer;

use std::borrow::Cow;
use std::fmt::Debug;
use std::io::IoSlice;
use std::path::Path;
use std::sync::Arc;

use openssh_sftp_protocol::file_attrs::FileAttrs;
use openssh_sftp_protocol::request::*;
use openssh_sftp_protocol::serde::Serialize;
use openssh_sftp_protocol::ssh_format::Serializer;
use openssh_sftp_protocol::Handle;

use bytes::Bytes;

// TODO:
//  - Support IoSlice for data in `send_write_request`

/// It is recommended to create at most one `WriteEnd` per thread
/// using [`WriteEnd::clone`].
#[derive(Debug)]
pub struct WriteEnd<Buffer: ToBuffer + 'static> {
    serializer: Serializer<WriteBuffer>,
    shared_data: Arc<SharedData<Buffer>>,
}

impl<Buffer: ToBuffer + 'static> Clone for WriteEnd<Buffer> {
    fn clone(&self) -> Self {
        Self::new(self.shared_data.clone())
    }
}

impl<Buffer: ToBuffer + 'static> Drop for WriteEnd<Buffer> {
    fn drop(&mut self) {
        let shared_data = &self.shared_data;

        // If this is the last reference, except for `ReadEnd`, to the SharedData,
        // then the connection is closed.
        if Arc::strong_count(shared_data) == 2 {
            shared_data.notify_conn_closed();
        }
    }
}

impl<Buffer: ToBuffer + 'static> WriteEnd<Buffer> {
    pub(crate) fn new(shared_data: Arc<SharedData<Buffer>>) -> Self {
        Self {
            serializer: Serializer::new(),
            shared_data,
        }
    }
}

impl<Buffer: ToBuffer + Debug + Send + Sync + 'static> WriteEnd<Buffer> {
    pub(crate) async fn send_hello(&mut self, version: u32) -> Result<(), Error> {
        self.shared_data
            .writer
            .write_all(&*Self::serialize(&mut self.serializer, Hello { version })?)
            .await?;

        Ok(())
    }

    fn serialize<T>(serializer: &mut Serializer<WriteBuffer>, value: T) -> Result<Bytes, Error>
    where
        T: Serialize,
    {
        serializer.reset();
        value.serialize(&mut *serializer)?;

        Ok(serializer.get_output()?.split())
    }

    /// Create a useable response id.
    pub fn create_response_id(&self) -> Id<Buffer> {
        self.shared_data.responses.insert()
    }

    /// Return true if reserve succeeds, false otherwise.
    pub fn try_reserve_id(&self, new_id_cnt: u32) -> bool {
        self.shared_data.responses.try_reserve(new_id_cnt)
    }

    /// Return true if reserve succeeds, false otherwise.
    pub fn reserve_id(&self, new_id_cnt: u32) {
        self.shared_data.responses.reserve(new_id_cnt);
    }

    /// Flush the write buffer.
    ///
    /// If another thread is flushing or there isn't any
    /// data to write, then `Ok(None)` will be returned.
    ///
    /// # Cancel Safety
    ///
    /// This function is perfectly cancel safe.
    ///
    /// While it is true that it might only partially flushed out the data,
    /// it can be restarted by another thread.
    pub async fn flush(&self) -> Result<Option<()>, Error> {
        Ok(self.shared_data.writer.flush().await?)
    }

    /// Send requests.
    ///
    /// NOTE that this merely add the request to the buffer, you need to call [`WriteEnd::flush`]
    /// to actually send the requests.
    ///
    /// # Cancel Safety
    ///
    /// This function is perfectly cancel safe.
    ///
    /// While it is true that it might only partially flushed out the data,
    /// it can be restarted by another thread.
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
        self.shared_data.writer.push(serialized);
        self.shared_data.notify_new_packet_event();

        Ok(id.into_inner())
    }

    /// NOTE that this merely add the request to the buffer, you need to call [`WriteEnd::flush`]
    /// to actually send the requests.
    pub fn send_open_file_request(
        &mut self,
        id: Id<Buffer>,
        params: OpenFileRequest<'_>,
    ) -> Result<AwaitableHandle<Buffer>, Error> {
        self.send_request(id, RequestInner::Open(params), None)
            .map(AwaitableHandle::new)
    }

    /// NOTE that this merely add the request to the buffer, you need to call [`WriteEnd::flush`]
    /// to actually send the requests.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub fn send_close_request(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_request(id, RequestInner::Close(handle), None)
            .map(AwaitableStatus::new)
    }

    /// - `buffer` - If set to `None` or the buffer is not long enough,
    ///   then `Data::AllocatedBox` will be returned.
    ///
    /// Return `Data::Buffer` if not EOF, otherwise returns `Data::EOF`.
    ///
    /// NOTE that this merely add the request to the buffer, you need to call [`WriteEnd::flush`]
    /// to actually send the requests.
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

    /// NOTE that this merely add the request to the buffer, you need to call [`WriteEnd::flush`]
    /// to actually send the requests.
    pub fn send_remove_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_request(id, RequestInner::Remove(path), None)
            .map(AwaitableStatus::new)
    }

    /// NOTE that this merely add the request to the buffer, you need to call [`WriteEnd::flush`]
    /// to actually send the requests.
    pub fn send_rename_request(
        &mut self,
        id: Id<Buffer>,
        oldpath: Cow<'_, Path>,
        newpath: Cow<'_, Path>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_request(id, RequestInner::Rename { oldpath, newpath }, None)
            .map(AwaitableStatus::new)
    }

    /// * `attrs` - `attrs.get_size()` must be equal to `None`.
    ///
    /// NOTE that this merely add the request to the buffer, you need to call [`WriteEnd::flush`]
    /// to actually send the requests.
    pub fn send_mkdir_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
        attrs: FileAttrs,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_request(id, RequestInner::Mkdir { path, attrs }, None)
            .map(AwaitableStatus::new)
    }

    /// NOTE that this merely add the request to the buffer, you need to call [`WriteEnd::flush`]
    /// to actually send the requests.
    pub fn send_rmdir_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_request(id, RequestInner::Rmdir(path), None)
            .map(AwaitableStatus::new)
    }

    /// NOTE that this merely add the request to the buffer, you need to call [`WriteEnd::flush`]
    /// to actually send the requests.
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
    ///
    /// NOTE that this merely add the request to the buffer, you need to call [`WriteEnd::flush`]
    /// to actually send the requests.
    pub fn send_readdir_request(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
    ) -> Result<AwaitableNameEntries<Buffer>, Error> {
        self.send_request(id, RequestInner::Readdir(handle), None)
            .map(AwaitableNameEntries::new)
    }

    /// NOTE that this merely add the request to the buffer, you need to call [`WriteEnd::flush`]
    /// to actually send the requests.
    pub fn send_stat_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableAttrs<Buffer>, Error> {
        self.send_request(id, RequestInner::Stat(path), None)
            .map(AwaitableAttrs::new)
    }

    /// Does not follow symlink
    ///
    /// NOTE that this merely add the request to the buffer, you need to call [`WriteEnd::flush`]
    /// to actually send the requests.
    pub fn send_lstat_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableAttrs<Buffer>, Error> {
        self.send_request(id, RequestInner::Lstat(path), None)
            .map(AwaitableAttrs::new)
    }

    /// * `handle` - Must be opened with `FileMode::READ`.
    ///
    /// NOTE that this merely add the request to the buffer, you need to call [`WriteEnd::flush`]
    /// to actually send the requests.
    pub fn send_fstat_request(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
    ) -> Result<AwaitableAttrs<Buffer>, Error> {
        self.send_request(id, RequestInner::Fstat(handle), None)
            .map(AwaitableAttrs::new)
    }

    /// NOTE that this merely add the request to the buffer, you need to call [`WriteEnd::flush`]
    /// to actually send the requests.
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
    ///
    /// NOTE that this merely add the request to the buffer, you need to call [`WriteEnd::flush`]
    /// to actually send the requests.
    pub fn send_fsetstat_request(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
        attrs: FileAttrs,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_request(id, RequestInner::Fsetstat { handle, attrs }, None)
            .map(AwaitableStatus::new)
    }

    /// NOTE that this merely add the request to the buffer, you need to call [`WriteEnd::flush`]
    /// to actually send the requests.
    pub fn send_readlink_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableName<Buffer>, Error> {
        self.send_request(id, RequestInner::Readlink(path), None)
            .map(AwaitableName::new)
    }

    /// NOTE that this merely add the request to the buffer, you need to call [`WriteEnd::flush`]
    /// to actually send the requests.
    pub fn send_realpath_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableName<Buffer>, Error> {
        self.send_request(id, RequestInner::Realpath(path), None)
            .map(AwaitableName::new)
    }

    /// Create symlink
    ///
    /// NOTE that this merely add the request to the buffer, you need to call [`WriteEnd::flush`]
    /// to actually send the requests.
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
    /// NOTE that this merely add the request to the buffer, you need to call [`WriteEnd::flush`]
    /// to actually send the requests.
    ///
    /// # Precondition
    ///
    /// Requires [`Extensions::limits`] to be true.
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
    /// These paths are expanded using shell-lilke rules and the resultant path
    /// is canonicalised similarly to [`WriteEnd::send_realpath_request`].
    ///
    /// NOTE that this merely add the request to the buffer, you need to call [`WriteEnd::flush`]
    /// to actually send the requests.
    ///
    /// # Precondition
    ///
    /// Requires [`Extensions::expand_path`] to be true.
    pub fn send_expand_path_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableName<Buffer>, Error> {
        self.send_request(id, RequestInner::ExpandPath(path), None)
            .map(AwaitableName::new)
    }

    /// NOTE that this merely add the request to the buffer, you need to call [`WriteEnd::flush`]
    /// to actually send the requests.
    ///
    /// # Precondition
    ///
    /// Requires [`Extensions::fsync`] to be true.
    pub fn send_fsync_request(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_request(id, RequestInner::Fsync(handle), None)
            .map(AwaitableStatus::new)
    }

    /// NOTE that this merely add the request to the buffer, you need to call [`WriteEnd::flush`]
    /// to actually send the requests.
    ///
    /// # Precondition
    ///
    /// Requires [`Extensions::hardlink`] to be true.
    pub fn send_hardlink_requst(
        &mut self,
        id: Id<Buffer>,
        oldpath: Cow<'_, Path>,
        newpath: Cow<'_, Path>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_request(id, RequestInner::HardLink { oldpath, newpath }, None)
            .map(AwaitableStatus::new)
    }

    /// NOTE that this merely add the request to the buffer, you need to call [`WriteEnd::flush`]
    /// to actually send the requests.
    ///
    /// # Precondition
    ///
    /// Requires [`Extensions::posix_rename`] to be true.
    pub fn send_posix_rename_request(
        &mut self,
        id: Id<Buffer>,
        oldpath: Cow<'_, Path>,
        newpath: Cow<'_, Path>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_request(id, RequestInner::PosixRename { oldpath, newpath }, None)
            .map(AwaitableStatus::new)
    }

    /// NOTE that this merely add the request to the buffer, you need to call
    /// [`WriteEnd::flush`] to actually send the requests.
    ///
    /// This function is only suitable for writing small data since it needs to copy the
    /// entire `data` into buffer.
    ///
    /// For writing large data, it is recommended to use [`WriteEnd::send_write_request_direct`].
    pub fn send_write_request_buffered(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
        offset: u64,
        data: Cow<'_, [u8]>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
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

    /// NOTE that this merely add the request to the buffer, you need to call
    /// [`WriteEnd::flush`] to actually send the requests.
    ///
    /// This function is only suitable for writing small data since it needs to copy the
    /// entire `data` into buffer.
    ///
    /// For writing large data, it is recommended to use [`WriteEnd::send_write_request_direct`].
    pub fn send_write_request_buffered_vectored(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
        offset: u64,
        io_slices: &[IoSlice<'_>],
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        let len: usize = io_slices.iter().map(|io_slice| io_slice.len()).sum();
        let len: u32 = len.try_into()?;

        let buffer = Request::serialize_write_request(
            &mut self.serializer,
            ArenaArc::slot(&id.0),
            handle,
            offset,
            len,
        )?;

        buffer.put_io_slices(io_slices);

        id.0.reset(None);
        self.shared_data.writer.push(buffer.split());
        self.shared_data.notify_new_packet_event();

        Ok(AwaitableStatus::new(id.into_inner()))
    }

    /// NOTE that this merely add the request to the buffer, you need to call
    /// [`WriteEnd::flush`] to actually send the requests.
    ///
    /// This function is zero-copy.
    pub fn send_write_request_zero_copy(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
        offset: u64,
        data: &[Bytes],
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        let len: usize = data.iter().map(Bytes::len).sum();
        let len: u32 = len.try_into()?;

        let header = Request::serialize_write_request(
            &mut self.serializer,
            ArenaArc::slot(&id.0),
            handle,
            offset,
            len,
        )?
        .split();

        // queue_pusher holds the mutex, so the `push` are atomic.
        let mut queue_pusher = self.shared_data.writer.get_pusher();
        queue_pusher.push(header);
        for bytes in data {
            queue_pusher.push(bytes.clone());
        }

        id.0.reset(None);
        self.shared_data.notify_new_packet_event();

        Ok(AwaitableStatus::new(id.into_inner()))
    }

    /// Send write requests directly, without any buffering.
    ///
    /// # Cancel Safety
    ///
    /// This function is only cancel safe if the `data` is no more than `1024` long.
    ///
    /// Otherwise it is not cancel safe, dropping the future returned
    /// might cause the data to paritaly written, and thus the sftp-server
    /// might demonstrate undefined behavior.
    pub async fn send_write_request_direct(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
        offset: u64,
        data: &[u8],
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        let header = Request::serialize_write_request(
            &mut self.serializer,
            ArenaArc::slot(&id.0),
            handle,
            offset,
            data.len().try_into()?,
        )?
        .split();

        id.0.reset(None);
        self.shared_data
            .writer
            .write_vectored_all(&mut [IoSlice::new(&*header), IoSlice::new(data)])
            .await?;

        self.shared_data.notify_new_packet_event();

        Ok(AwaitableStatus::new(id.into_inner()))
    }
}
