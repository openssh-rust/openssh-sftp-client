use super::awaitable_responses::ArenaArc;
use super::connection::SharedData;
use crate::*;

use core::fmt::Debug;

use std::borrow::Cow;
use std::path::Path;

use std::sync::Arc;

use openssh_sftp_protocol::file_attrs::FileAttrs;
use openssh_sftp_protocol::request::*;
use openssh_sftp_protocol::serde::Serialize;
use openssh_sftp_protocol::ssh_format::Serializer;
use openssh_sftp_protocol::Handle;

use std::io::IoSlice;

// TODO:
//  - Support IoSlice for data in `send_write_request`

/// It is recommended to create at most one `WriteEnd` per thread
/// using [`WriteEnd::clone`].
#[derive(Debug)]
pub struct WriteEnd<Buffer: ToBuffer + 'static> {
    serializer: Serializer,
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
            .write_all(Self::serialize(&mut self.serializer, Hello { version })?)
            .await?;

        Ok(())
    }

    fn serialize<T>(serializer: &mut Serializer, value: T) -> Result<&[u8], Error>
    where
        T: Serialize,
    {
        serializer.reset();
        value.serialize(&mut *serializer)?;

        Ok(serializer.get_output()?)
    }

    async fn write<T>(&mut self, value: T) -> Result<(), Error>
    where
        T: Serialize,
    {
        self.shared_data
            .writer
            .write_all(Self::serialize(&mut self.serializer, value)?)
            .await?;

        Ok(())
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

    /// Send requests.
    ///
    /// **Please use `Self::send_write_request` for sending write requests.**
    async fn send_request(
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
        self.shared_data.writer.write_all(serialized).await?;
        self.shared_data.notify_new_packet_event();

        Ok(id.into_inner())
    }

    /// # Cancel Safety
    ///
    /// This function is not cancel safe
    pub async fn send_open_file_request(
        &mut self,
        id: Id<Buffer>,
        params: OpenFileRequest<'_>,
    ) -> Result<AwaitableHandle<Buffer>, Error> {
        Ok(AwaitableHandle::new(
            self.send_request(id, RequestInner::Open(params), None)
                .await?,
        ))
    }

    /// # Cancel Safety
    ///
    /// This function is not cancel safe
    pub async fn send_close_request(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        Ok(AwaitableStatus::new(
            self.send_request(id, RequestInner::Close(handle), None)
                .await?,
        ))
    }

    /// - `buffer` - If set to `None` or the buffer is not long enough,
    ///   then `Data::AllocatedBox` will be returned.
    ///   Return `Data::Buffer` if not EOF, otherwise returns `Data::EOF`.
    ///
    /// # Cancel Safety
    ///
    /// This function is not cancel safe
    pub async fn send_read_request(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
        offset: u64,
        len: u32,
        buffer: Option<Buffer>,
    ) -> Result<AwaitableData<Buffer>, Error> {
        Ok(AwaitableData::new(
            self.send_request(
                id,
                RequestInner::Read {
                    handle,
                    offset,
                    len,
                },
                buffer,
            )
            .await?,
        ))
    }

    /// # Cancel Safety
    ///
    /// This function is not cancel safe
    pub async fn send_remove_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        Ok(AwaitableStatus::new(
            self.send_request(id, RequestInner::Remove(path), None)
                .await?,
        ))
    }

    /// # Cancel Safety
    ///
    /// This function is not cancel safe
    pub async fn send_rename_request(
        &mut self,
        id: Id<Buffer>,
        oldpath: Cow<'_, Path>,
        newpath: Cow<'_, Path>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        Ok(AwaitableStatus::new(
            self.send_request(id, RequestInner::Rename { oldpath, newpath }, None)
                .await?,
        ))
    }

    /// * `attrs` - `attrs.get_size()` must be equal to `None`.
    ///
    /// # Cancel Safety
    ///
    /// This function is not cancel safe
    pub async fn send_mkdir_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
        attrs: FileAttrs,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        Ok(AwaitableStatus::new(
            self.send_request(id, RequestInner::Mkdir { path, attrs }, None)
                .await?,
        ))
    }

    /// # Cancel Safety
    ///
    /// This function is not cancel safe
    pub async fn send_rmdir_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        Ok(AwaitableStatus::new(
            self.send_request(id, RequestInner::Rmdir(path), None)
                .await?,
        ))
    }

    /// # Cancel Safety
    ///
    /// This function is not cancel safe
    pub async fn send_opendir_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableHandle<Buffer>, Error> {
        Ok(AwaitableHandle::new(
            self.send_request(id, RequestInner::Opendir(path), None)
                .await?,
        ))
    }

    /// Return all entries in the directory specified by the `handle`, including
    /// `.` and `..`.
    ///
    /// The `filename` only contains the basename.
    ///
    /// # Cancel Safety
    ///
    /// This function is not cancel safe
    pub async fn send_readdir_request(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
    ) -> Result<AwaitableNameEntries<Buffer>, Error> {
        Ok(AwaitableNameEntries::new(
            self.send_request(id, RequestInner::Readdir(handle), None)
                .await?,
        ))
    }

    /// # Cancel Safety
    ///
    /// This function is not cancel safe
    pub async fn send_stat_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableAttrs<Buffer>, Error> {
        Ok(AwaitableAttrs::new(
            self.send_request(id, RequestInner::Stat(path), None)
                .await?,
        ))
    }

    /// Does not follow symlink
    ///
    /// # Cancel Safety
    ///
    /// This function is not cancel safe
    pub async fn send_lstat_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableAttrs<Buffer>, Error> {
        Ok(AwaitableAttrs::new(
            self.send_request(id, RequestInner::Lstat(path), None)
                .await?,
        ))
    }

    /// * `handle` - Must be opened with `FileMode::READ`.
    ///
    /// Does not follow symlink
    ///
    /// # Cancel Safety
    ///
    /// This function is not cancel safe
    pub async fn send_fstat_request(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
    ) -> Result<AwaitableAttrs<Buffer>, Error> {
        Ok(AwaitableAttrs::new(
            self.send_request(id, RequestInner::Fstat(handle), None)
                .await?,
        ))
    }

    /// # Cancel Safety
    ///
    /// This function is not cancel safe
    pub async fn send_setstat_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
        attrs: FileAttrs,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        Ok(AwaitableStatus::new(
            self.send_request(id, RequestInner::Setstat { path, attrs }, None)
                .await?,
        ))
    }

    /// * `handle` - Must be opened with `OpenOptions::write` set.
    ///
    /// # Cancel Safety
    ///
    /// This function is not cancel safe
    pub async fn send_fsetstat_request(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
        attrs: FileAttrs,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        Ok(AwaitableStatus::new(
            self.send_request(id, RequestInner::Fsetstat { handle, attrs }, None)
                .await?,
        ))
    }

    /// # Cancel Safety
    ///
    /// This function is not cancel safe
    pub async fn send_readlink_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableName<Buffer>, Error> {
        Ok(AwaitableName::new(
            self.send_request(id, RequestInner::Readlink(path), None)
                .await?,
        ))
    }

    /// # Cancel Safety
    ///
    /// This function is not cancel safe
    pub async fn send_realpath_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableName<Buffer>, Error> {
        Ok(AwaitableName::new(
            self.send_request(id, RequestInner::Realpath(path), None)
                .await?,
        ))
    }

    /// Create symlink
    ///
    /// # Cancel Safety
    ///
    /// This function is not cancel safe
    pub async fn send_symlink_request(
        &mut self,
        id: Id<Buffer>,
        targetpath: Cow<'_, Path>,
        linkpath: Cow<'_, Path>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        Ok(AwaitableStatus::new(
            self.send_request(
                id,
                RequestInner::Symlink {
                    linkpath,
                    targetpath,
                },
                None,
            )
            .await?,
        ))
    }

    /// Send write requests
    ///
    /// # Cancel Safety
    ///
    /// This function is not cancel safe
    pub async fn send_write_request(
        &mut self,
        id: Id<Buffer>,
        handle: &[u8],
        offset: u64,
        data: &[u8],
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        let header = Request::serialize_write_request(
            &mut self.serializer,
            ArenaArc::slot(&id.0),
            handle,
            offset,
            data.len().try_into()?,
        )?;

        id.0.reset(None);
        self.shared_data
            .writer
            .write_vectored_all(&mut [IoSlice::new(header), IoSlice::new(data)])
            .await?;

        self.shared_data.notify_new_packet_event();

        Ok(AwaitableStatus::new(id.into_inner()))
    }

    /// Return limits of the server
    ///
    /// # Precondition
    ///
    /// Requires [`Extensions::limits`] to be true.
    ///
    /// # Cancel safety
    ///
    /// This function is not cancel safe
    pub async fn send_limits_request(
        &mut self,
        id: Id<Buffer>,
    ) -> Result<AwaitableLimits<Buffer>, Error> {
        Ok(AwaitableLimits::new(
            self.send_request(id, RequestInner::Limits, None).await?,
        ))
    }

    /// This supports canonicalisation of relative paths and those that need
    /// tilde-expansion, i.e. "~", "~/..." and "~user/...".
    ///
    /// These paths are expanded using shell-lilke rules and the resultant path
    /// is canonicalised similarly to [`WriteEnd::send_realpath_request`].
    ///
    /// # Precondition
    ///
    /// Requires [`Extensions::expand_path`] to be true.
    ///
    /// # Cancel safety
    ///
    /// This function is not cancel safe
    pub async fn send_expand_path_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableName<Buffer>, Error> {
        Ok(AwaitableName::new(
            self.send_request(id, RequestInner::ExpandPath(path), None)
                .await?,
        ))
    }

    /// # Precondition
    ///
    /// Requires [`Extensions::fsync`] to be true.
    ///
    /// # Cancel safety
    ///
    /// This function is not cancel safe
    pub async fn send_fsync_request(
        &mut self,
        id: Id<Buffer>,
        handle: &[u8],
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        Ok(AwaitableStatus::new(
            self.send_request(id, RequestInner::Fsync(Cow::Borrowed(handle)), None)
                .await?,
        ))
    }

    /// # Precondition
    ///
    /// Requires [`Extensions::hardlink`] to be true.
    ///
    /// # Cancel safety
    ///
    /// This function is not cancel safe
    pub async fn send_hardlink_requst(
        &mut self,
        id: Id<Buffer>,
        oldpath: Cow<'_, Path>,
        newpath: Cow<'_, Path>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        Ok(AwaitableStatus::new(
            self.send_request(id, RequestInner::HardLink { oldpath, newpath }, None)
                .await?,
        ))
    }

    /// # Precondition
    ///
    /// Requires [`Extensions::posix_rename`] to be true.
    ///
    /// # Cancel safety
    ///
    /// This function is not cancel safe
    pub async fn send_posix_rename_request(
        &mut self,
        id: Id<Buffer>,
        oldpath: Cow<'_, Path>,
        newpath: Cow<'_, Path>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        Ok(AwaitableStatus::new(
            self.send_request(id, RequestInner::PosixRename { oldpath, newpath }, None)
                .await?,
        ))
    }
}
