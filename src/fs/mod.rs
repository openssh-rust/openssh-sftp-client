use crate::{
    file::OpenOptions,
    lowlevel::{self, Extensions},
    metadata::{MetaData, MetaDataBuilder, Permissions},
    Auxiliary, Buffer, Error, Id, OwnedHandle, WriteEnd, WriteEndWithCachedId,
};

use std::{
    borrow::Cow,
    cmp::min,
    convert::TryInto,
    path::{Path, PathBuf},
};

use bytes::BytesMut;

mod dir;
pub use dir::{DirEntry, ReadDir};

type AwaitableStatus = lowlevel::AwaitableStatus<Buffer>;
type AwaitableAttrs = lowlevel::AwaitableAttrs<Buffer>;
type SendLinkingRequest =
    fn(&mut WriteEnd, Id, Cow<'_, Path>, Cow<'_, Path>) -> Result<AwaitableStatus, Error>;

type SendRmRequest = fn(&mut WriteEnd, Id, Cow<'_, Path>) -> Result<AwaitableStatus, Error>;
type SendMetadataRequest = fn(&mut WriteEnd, Id, Cow<'_, Path>) -> Result<AwaitableAttrs, Error>;

/// A struct used to perform operations on remote filesystem.
#[derive(Debug, Clone)]
pub struct Fs {
    write_end: WriteEndWithCachedId,
    cwd: Box<Path>,
}

impl Fs {
    pub(super) fn new(write_end: WriteEndWithCachedId, cwd: PathBuf) -> Self {
        Self {
            write_end,
            cwd: cwd.into_boxed_path(),
        }
    }

    fn get_auxiliary(&self) -> &Auxiliary {
        self.write_end.get_auxiliary()
    }

    /// Return current working dir.
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Set current working dir.
    ///
    /// * `cwd` - Can include `~`.
    ///   If it is empty, then it is set to use the default
    ///   directory set by the remote `sftp-server`.
    pub fn set_cwd(&mut self, cwd: impl Into<PathBuf>) {
        self.cwd = cwd.into().into_boxed_path();
    }

    fn concat_path_if_needed<'path>(&self, path: &'path Path) -> Cow<'path, Path> {
        if path.is_absolute() || self.cwd.as_os_str().is_empty() {
            Cow::Borrowed(path)
        } else {
            Cow::Owned(self.cwd.join(path))
        }
    }
}

impl Fs {
    /// Open a remote dir
    pub async fn open_dir(&mut self, path: impl AsRef<Path>) -> Result<Dir, Error> {
        async fn inner(this: &mut Fs, path: &Path) -> Result<Dir, Error> {
            let path = this.concat_path_if_needed(path);

            this.write_end
                .send_request(|write_end, id| Ok(write_end.send_opendir_request(id, path)?.wait()))
                .await
                .map(|handle| Dir(OwnedHandle::new(this.write_end.clone(), handle)))
        }

        inner(self, path.as_ref()).await
    }

    /// Create a directory builder.
    pub fn dir_builder(&mut self) -> DirBuilder<'_> {
        DirBuilder {
            fs: self,
            metadata_builder: MetaDataBuilder::new(),
        }
    }

    /// Creates a new, empty directory at the provided path.
    pub async fn create_dir(&mut self, path: impl AsRef<Path>) -> Result<(), Error> {
        async fn inner(this: &mut Fs, path: &Path) -> Result<(), Error> {
            this.dir_builder().create(path).await
        }

        inner(self, path.as_ref()).await
    }

    async fn remove_impl(&mut self, path: &Path, f: SendRmRequest) -> Result<(), Error> {
        let path = self.concat_path_if_needed(path);

        self.write_end
            .send_request(|write_end, id| Ok(f(write_end, id, path)?.wait()))
            .await
    }

    /// Removes an existing, empty directory.
    pub async fn remove_dir(&mut self, path: impl AsRef<Path>) -> Result<(), Error> {
        self.remove_impl(path.as_ref(), WriteEnd::send_rmdir_request)
            .await
    }

    /// Removes a file from remote filesystem.
    pub async fn remove_file(&mut self, path: impl AsRef<Path>) -> Result<(), Error> {
        self.remove_impl(path.as_ref(), WriteEnd::send_remove_request)
            .await
    }

    /// Returns the canonical, absolute form of a path with all intermediate
    /// components normalized and symbolic links resolved.
    ///
    /// If the remote server supports the `expand-path` extension, then this
    /// method will also expand tilde characters (“~”) in the path. You can
    /// check it with [`Sftp::support_expand_path`](crate::sftp::Sftp::support_expand_path).
    pub async fn canonicalize(&mut self, path: impl AsRef<Path>) -> Result<PathBuf, Error> {
        async fn inner(this: &mut Fs, path: &Path) -> Result<PathBuf, Error> {
            let path = this.concat_path_if_needed(path);

            let f = if this
                .get_auxiliary()
                .extensions()
                .contains(Extensions::EXPAND_PATH)
            {
                // This supports canonicalisation of relative paths and those that
                // need tilde-expansion, i.e. “~”, “~/…” and “~user/…”.
                //
                // These paths are expanded using shell-like rules and the resultant
                // path is canonicalised similarly to WriteEnd::send_realpath_request.
                WriteEnd::send_expand_path_request
            } else {
                WriteEnd::send_realpath_request
            };

            this.write_end
                .send_request(|write_end, id| Ok(f(write_end, id, path)?.wait()))
                .await
                .map(Into::into)
        }

        inner(self, path.as_ref()).await
    }

    async fn linking_impl(
        &mut self,
        src: &Path,
        dst: &Path,
        f: SendLinkingRequest,
    ) -> Result<(), Error> {
        let src = self.concat_path_if_needed(src);
        let dst = self.concat_path_if_needed(dst);

        self.write_end
            .send_request(|write_end, id| Ok(f(write_end, id, src, dst)?.wait()))
            .await
    }

    /// Creates a new hard link on the remote filesystem.
    ///
    /// # Precondition
    ///
    /// Require extension `hardlink`
    ///
    /// You can check it with [`Sftp::support_hardlink`](crate::sftp::Sftp::support_hardlink).
    pub async fn hard_link(
        &mut self,
        src: impl AsRef<Path>,
        dst: impl AsRef<Path>,
    ) -> Result<(), Error> {
        async fn inner(this: &mut Fs, src: &Path, dst: &Path) -> Result<(), Error> {
            if !this
                .get_auxiliary()
                .extensions()
                .contains(Extensions::HARDLINK)
            {
                return Err(Error::UnsupportedExtension(&"hardlink"));
            }

            this.linking_impl(src, dst, WriteEnd::send_hardlink_request)
                .await
        }

        inner(self, src.as_ref(), dst.as_ref()).await
    }

    /// Creates a new symlink on the remote filesystem.
    pub async fn symlink(
        &mut self,
        src: impl AsRef<Path>,
        dst: impl AsRef<Path>,
    ) -> Result<(), Error> {
        self.linking_impl(src.as_ref(), dst.as_ref(), WriteEnd::send_symlink_request)
            .await
    }

    /// Renames a file or directory to a new name, replacing the original file if to already exists.
    ///
    /// If the server supports the `posix-rename` extension, it will be used.
    /// You can check it with [`Sftp::support_posix_rename`](crate::sftp::Sftp::support_posix_rename).
    ///
    /// This will not work if the new name is on a different mount point.
    pub async fn rename(
        &mut self,
        from: impl AsRef<Path>,
        to: impl AsRef<Path>,
    ) -> Result<(), Error> {
        async fn inner(this: &mut Fs, from: &Path, to: &Path) -> Result<(), Error> {
            let f = if this
                .get_auxiliary()
                .extensions()
                .contains(Extensions::POSIX_RENAME)
            {
                // posix rename is guaranteed to be atomic
                WriteEnd::send_posix_rename_request
            } else {
                WriteEnd::send_rename_request
            };

            this.linking_impl(from, to, f).await
        }

        inner(self, from.as_ref(), to.as_ref()).await
    }

    /// Reads a symbolic link, returning the file that the link points to.
    pub async fn read_link(&mut self, path: impl AsRef<Path>) -> Result<PathBuf, Error> {
        async fn inner(this: &mut Fs, path: &Path) -> Result<PathBuf, Error> {
            let path = this.concat_path_if_needed(path);

            this.write_end
                .send_request(|write_end, id| Ok(write_end.send_readlink_request(id, path)?.wait()))
                .await
                .map(Into::into)
        }

        inner(self, path.as_ref()).await
    }

    async fn set_metadata_impl(&mut self, path: &Path, metadata: MetaData) -> Result<(), Error> {
        let path = self.concat_path_if_needed(path);

        self.write_end
            .send_request(|write_end, id| {
                Ok(write_end
                    .send_setstat_request(id, path, metadata.into_inner())?
                    .wait())
            })
            .await
    }

    /// Change the metadata of a file or a directory.
    pub async fn set_metadata(
        &mut self,
        path: impl AsRef<Path>,
        metadata: MetaData,
    ) -> Result<(), Error> {
        self.set_metadata_impl(path.as_ref(), metadata).await
    }

    /// Changes the permissions found on a file or a directory.
    pub async fn set_permissions(
        &mut self,
        path: impl AsRef<Path>,
        perm: Permissions,
    ) -> Result<(), Error> {
        async fn inner(this: &mut Fs, path: &Path, perm: Permissions) -> Result<(), Error> {
            this.set_metadata_impl(path, MetaDataBuilder::new().permissions(perm).create())
                .await
        }

        inner(self, path.as_ref(), perm).await
    }

    async fn metadata_impl(
        &mut self,
        path: &Path,
        f: SendMetadataRequest,
    ) -> Result<MetaData, Error> {
        let path = self.concat_path_if_needed(path);

        self.write_end
            .send_request(|write_end, id| Ok(f(write_end, id, path)?.wait()))
            .await
            .map(MetaData::new)
    }

    /// Given a path, queries the file system to get information about a file,
    /// directory, etc.
    pub async fn metadata(&mut self, path: impl AsRef<Path>) -> Result<MetaData, Error> {
        self.metadata_impl(path.as_ref(), WriteEnd::send_stat_request)
            .await
    }

    /// Queries the file system metadata for a path.
    pub async fn symlink_metadata(&mut self, path: impl AsRef<Path>) -> Result<MetaData, Error> {
        self.metadata_impl(path.as_ref(), WriteEnd::send_lstat_request)
            .await
    }

    /// Reads the entire contents of a file into a bytes.
    pub async fn read(&mut self, path: impl AsRef<Path>) -> Result<BytesMut, Error> {
        async fn inner(this: &mut Fs, path: &Path) -> Result<BytesMut, Error> {
            let path = this.concat_path_if_needed(path);

            let mut file = OpenOptions::open_inner(
                lowlevel::OpenOptions::new().read(true),
                false,
                false,
                false,
                path.as_ref(),
                this.write_end.clone(),
            )
            .await?;
            let max_read_len = file.max_read_len_impl();

            let cap_to_reserve: usize = if let Some(len) = file.metadata().await?.len() {
                // To detect EOF, we need to a little bit more then the length
                // of the file.
                len.saturating_add(300)
                    .try_into()
                    .unwrap_or(max_read_len as usize)
            } else {
                max_read_len as usize
            };

            let mut buffer = BytesMut::with_capacity(cap_to_reserve);

            loop {
                let cnt = buffer.len();

                let n: u32 = if cnt <= cap_to_reserve {
                    // To detect EOF, we need to a little bit more then the
                    // length of the file.
                    (cap_to_reserve - cnt)
                        .saturating_add(300)
                        .try_into()
                        .map(|n| min(n, max_read_len))
                        .unwrap_or(max_read_len)
                } else {
                    max_read_len
                };
                buffer.reserve(n.try_into().unwrap_or(usize::MAX));

                if let Some(bytes) = file.read(n, buffer.split_off(cnt)).await? {
                    buffer.unsplit(bytes);
                } else {
                    // Eof
                    break Ok(buffer);
                }
            }
        }

        inner(self, path.as_ref()).await
    }

    /// Open/Create a file for writing and write the entire `contents` into it.
    pub async fn write(
        &mut self,
        path: impl AsRef<Path>,
        content: impl AsRef<[u8]>,
    ) -> Result<(), Error> {
        async fn inner(this: &mut Fs, path: &Path, content: &[u8]) -> Result<(), Error> {
            let path = this.concat_path_if_needed(path);

            OpenOptions::open_inner(
                lowlevel::OpenOptions::new().write(true),
                true,
                true,
                false,
                path.as_ref(),
                this.write_end.clone(),
            )
            .await?
            .write_all(content)
            .await
        }

        inner(self, path.as_ref(), content.as_ref()).await
    }
}

/// Remote Directory
#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct Dir(OwnedHandle);

impl Dir {
    /// Read dir.
    pub fn read_dir(self) -> ReadDir {
        ReadDir::new(self)
    }

    /// Close dir.
    pub async fn close(self) -> Result<(), Error> {
        self.0.close().await
    }
}

/// Builder for new directory to create.
#[derive(Debug)]
pub struct DirBuilder<'a> {
    fs: &'a mut Fs,
    metadata_builder: MetaDataBuilder,
}

impl DirBuilder<'_> {
    /// Reset builder back to default.
    pub fn reset(&mut self) -> &mut Self {
        self.metadata_builder = MetaDataBuilder::new();
        self
    }

    /// Set id of the dir to be built.
    pub fn id(&mut self, (uid, gid): (u32, u32)) -> &mut Self {
        self.metadata_builder.id((uid, gid));
        self
    }

    /// Set permissions of the dir to be built.
    pub fn permissions(&mut self, perm: Permissions) -> &mut Self {
        self.metadata_builder.permissions(perm);
        self
    }
}

impl DirBuilder<'_> {
    /// Creates the specified directory with the configured options.
    pub async fn create(&mut self, path: impl AsRef<Path>) -> Result<(), Error> {
        async fn inner(this: &mut DirBuilder<'_>, path: &Path) -> Result<(), Error> {
            let fs = &mut this.fs;

            let path = fs.concat_path_if_needed(path);
            let attrs = this.metadata_builder.create().into_inner();

            fs.write_end
                .send_request(|write_end, id| {
                    Ok(write_end.send_mkdir_request(id, path, attrs)?.wait())
                })
                .await
        }

        inner(self, path.as_ref()).await
    }
}
