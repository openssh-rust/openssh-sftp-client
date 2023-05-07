use crate::{
    auxiliary,
    file::{File, OpenOptions},
    fs::Fs,
    lowlevel, tasks, Error, MpscQueue, SftpOptions, SharedData, WriteEnd, WriteEndWithCachedId,
};

use auxiliary::Auxiliary;
use lowlevel::{connect, Extensions};
use tasks::{create_flush_task, create_read_task};

use std::{
    any::Any, convert::TryInto, fmt, future::Future, ops::Deref, path::Path, pin::Pin, sync::Arc,
};

use derive_destructure2::destructure;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::oneshot::Receiver,
    task::JoinHandle,
};
use tokio_io_utility::assert_send;

#[cfg(feature = "openssh")]
mod openssh_session;

#[cfg(feature = "openssh")]
pub use openssh_session::OpensshSession;

#[derive(Debug, destructure)]
pub(super) struct SftpHandle(SharedData);

impl Deref for SftpHandle {
    type Target = SharedData;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl SftpHandle {
    fn new(shared_data: &SharedData) -> Self {
        // Inc active_user_count for the same reason as Self::clone
        shared_data.get_auxiliary().inc_active_user_count();

        Self(shared_data.clone())
    }

    /// Takes `self` by value to ensure active_user_count get inc/dec properly.
    pub(super) fn write_end(self) -> WriteEndWithCachedId {
        // WriteEndWithCachedId also inc/dec active_user_count, so it's ok
        // to destructure self here.
        WriteEndWithCachedId::new(WriteEnd::new(self.destructure().0))
    }
}

impl Clone for SftpHandle {
    fn clone(&self) -> Self {
        self.0.get_auxiliary().inc_active_user_count();
        Self(self.0.clone())
    }
}

impl Drop for SftpHandle {
    fn drop(&mut self) {
        self.0.get_auxiliary().dec_active_user_count();
    }
}

/// A file-oriented channel to a remote host.
#[derive(Debug)]
pub struct Sftp {
    handle: SftpHandle,
    flush_task: JoinHandle<Result<(), Error>>,
    read_task: JoinHandle<Result<(), Error>>,
}

/// Auxiliary data for [`Sftp`].
#[non_exhaustive]
pub enum SftpAuxiliaryData {
    /// No auxiliary data.
    None,
    /// Store any `Box`ed value.
    Boxed(Box<dyn Any + Send + Sync + 'static>),
    /// Store any `Pin`ed `Future`.
    PinnedFuture(Pin<Box<dyn Future<Output = ()> + Send + Sync + 'static>>),
    /// Store any `Arc`ed value.
    Arced(Arc<dyn Any + Send + Sync + 'static>),
    /// Store [`OpensshSession`] with in an `Arc`.
    #[cfg(feature = "openssh")]
    ArcedOpensshSession(Arc<OpensshSession>),
}

impl fmt::Debug for SftpAuxiliaryData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use SftpAuxiliaryData::*;

        match self {
            None => f.write_str("None"),
            Boxed(_) => f.write_str("Boxed(boxed_any)"),
            PinnedFuture(_) => f.write_str("PinnedFuture"),
            Arced(_) => f.write_str("Arced(arced_any)"),
            #[cfg(feature = "openssh")]
            ArcedOpensshSession(session) => write!(f, "ArcedOpensshSession({session:?})"),
        }
    }
}

impl Sftp {
    /// Create [`Sftp`].
    pub async fn new<W: AsyncWrite + Send + 'static, R: AsyncRead + Send + 'static>(
        stdin: W,
        stdout: R,
        options: SftpOptions,
    ) -> Result<Self, Error> {
        Self::new_with_auxiliary(stdin, stdout, options, SftpAuxiliaryData::None).await
    }

    /// Create [`Sftp`] with some auxiliary data.
    ///
    /// The auxiliary data will be dropped after all sftp requests has been
    /// sent(flush_task), all responses processed (read_task) and [`Sftp`] has
    /// been dropped.
    ///
    /// If you want to get back the data, you can simply use
    /// [`SftpAuxiliaryData::Arced`] and then stores an [`Arc`] elsewhere.
    ///
    /// Once the sftp tasks is completed and [`Sftp`] is dropped, you can call
    /// [`Arc::try_unwrap`] to get back the exclusive ownership of it.
    pub async fn new_with_auxiliary<
        W: AsyncWrite + Send + 'static,
        R: AsyncRead + Send + 'static,
    >(
        stdin: W,
        stdout: R,
        options: SftpOptions,
        auxiliary: SftpAuxiliaryData,
    ) -> Result<Self, Error> {
        assert_send(async move {
            let write_end_buffer_size = options.get_write_end_buffer_size();

            let write_end = assert_send(Self::connect(
                write_end_buffer_size.get(),
                options.get_max_pending_requests(),
                auxiliary,
            ))
            .await?;

            let flush_task = create_flush_task(
                stdin,
                SharedData::clone(&write_end),
                write_end_buffer_size,
                options.get_flush_interval(),
            );

            let (rx, read_task) = create_read_task(
                stdout,
                options.get_read_end_buffer_size(),
                SharedData::clone(&write_end),
            );

            Self::init(flush_task, read_task, write_end, rx, &options).await
        })
        .await
    }

    async fn connect(
        write_end_buffer_size: usize,
        max_pending_requests: u16,
        auxiliary: SftpAuxiliaryData,
    ) -> Result<WriteEnd, Error> {
        connect(
            MpscQueue::with_capacity(write_end_buffer_size),
            Auxiliary::new(max_pending_requests, auxiliary),
        )
        .await
    }

    async fn init(
        flush_task: JoinHandle<Result<(), Error>>,
        read_task: JoinHandle<Result<(), Error>>,
        write_end: WriteEnd,
        rx: Receiver<Extensions>,
        options: &SftpOptions,
    ) -> Result<Self, Error> {
        // Create sftp here.
        //
        // It would also gracefully shutdown `flush_task` and `read_task` if
        // the future is cancelled or error is encounted.
        let sftp = Self {
            handle: SftpHandle::new(&write_end),
            flush_task,
            read_task,
        };

        let write_end = WriteEndWithCachedId::new(write_end);

        let extensions = if let Ok(extensions) = rx.await {
            extensions
        } else {
            drop(write_end);

            // Wait on flush_task and read_task to get a more detailed error message.
            sftp.close().await?;
            std::unreachable!("Error must have occurred in either read_task or flush_task")
        };

        match Self::set_limits(write_end, options, extensions).await {
            Err(Error::BackgroundTaskFailure(_)) => {
                // Wait on flush_task and read_task to get a more detailed error message.
                sftp.close().await?;
                std::unreachable!("Error must have occurred in either read_task or flush_task")
            }
            res => res?,
        }

        Ok(sftp)
    }

    async fn set_limits(
        mut write_end: WriteEndWithCachedId,
        options: &SftpOptions,
        extensions: Extensions,
    ) -> Result<(), Error> {
        let default_download_buflen = lowlevel::OPENSSH_PORTABLE_DEFAULT_DOWNLOAD_BUFLEN as u64;
        let default_upload_buflen = lowlevel::OPENSSH_PORTABLE_DEFAULT_UPLOAD_BUFLEN as u64;

        // sftp can accept packet as large as u32::MAX, the header itself
        // is at least 9 bytes long.
        let default_max_packet_len = u32::MAX - 9;

        let (read_len, write_len, packet_len) = if extensions.contains(Extensions::LIMITS) {
            let mut limits = write_end
                .send_request(|write_end, id| Ok(write_end.send_limits_request(id)?.wait()))
                .await?;

            if limits.read_len == 0 {
                limits.read_len = default_download_buflen;
            }

            if limits.write_len == 0 {
                limits.write_len = default_upload_buflen;
            }

            (
                limits.read_len,
                limits.write_len,
                limits
                    .packet_len
                    .try_into()
                    .unwrap_or(default_max_packet_len),
            )
        } else {
            (
                default_download_buflen,
                default_upload_buflen,
                default_max_packet_len,
            )
        };

        // Each read/write request also has a header and contains a handle,
        // which is 4-byte long for openssh but can be at most 256 bytes long
        // for other implementations.

        let read_len = read_len.try_into().unwrap_or(packet_len - 300);
        let read_len = options
            .get_max_read_len()
            .map(|v| v.min(read_len))
            .unwrap_or(read_len);

        let write_len = write_len.try_into().unwrap_or(packet_len - 300);
        let write_len = options
            .get_max_write_len()
            .map(|v| v.min(write_len))
            .unwrap_or(write_len);

        let limits = auxiliary::Limits {
            read_len,
            write_len,
        };

        write_end
            .get_auxiliary()
            .conn_info
            .set(auxiliary::ConnInfo { limits, extensions })
            .expect("auxiliary.conn_info shall be uninitialized");

        Ok(())
    }

    /// Close sftp connection
    ///
    /// If sftp is created using `Sftp::from_session`, then calling this
    /// function would also await on `openssh::RemoteChild::wait` and
    /// `openssh::Session::close` and propagate their error in
    /// [`Sftp::close`].
    pub async fn close(self) -> Result<(), Error> {
        let Self {
            handle,
            flush_task,
            read_task,
        } = self;

        let session = match &handle.get_auxiliary().auxiliary_data {
            #[cfg(feature = "openssh")]
            SftpAuxiliaryData::ArcedOpensshSession(session) => Some(Arc::clone(session)),
            _ => None,
        };

        // Drop handle.
        drop(handle);

        // Wait for responses for all requests buffered and sent.
        read_task.await??;

        // read_task would order the shutdown of read_task,
        // so we just need to wait for it here.
        flush_task.await??;

        #[cfg(feature = "openssh")]
        if let Some(session) = session {
            Arc::try_unwrap(session)
                .unwrap()
                .recover_session_err()
                .await?;
        }

        #[cfg(not(feature = "openssh"))]
        {
            // Help session infer generic T in Option<T>
            let _: Option<()> = session;
        }

        Ok(())
    }

    /// Return a new [`OpenOptions`] object.
    pub fn options(&self) -> OpenOptions {
        OpenOptions::new(self.handle.clone())
    }

    /// Opens a file in write-only mode.
    ///
    /// This function will create a file if it does not exist, and will truncate
    /// it if it does.
    pub async fn create(&self, path: impl AsRef<Path>) -> Result<File, Error> {
        async fn inner(this: &Sftp, path: &Path) -> Result<File, Error> {
            this.options()
                .write(true)
                .create(true)
                .truncate(true)
                .open(path)
                .await
        }

        inner(self, path.as_ref()).await
    }

    /// Attempts to open a file in read-only mode.
    pub async fn open(&self, path: impl AsRef<Path>) -> Result<File, Error> {
        async fn inner(this: &Sftp, path: &Path) -> Result<File, Error> {
            this.options().read(true).open(path).await
        }

        inner(self, path.as_ref()).await
    }

    /// [`Fs`] defaults to the current working dir set by remote `sftp-server`,
    /// which usually is the home directory.
    pub fn fs(&self) -> Fs {
        Fs::new(self.handle.clone().write_end(), "".into())
    }
}

#[cfg(feature = "__ci-tests")]
impl Sftp {
    /// The maximum amount of bytes that can be written in one request.
    /// Writing more than that, then your write will be split into multiple requests
    ///
    /// If [`Sftp::max_buffered_write`] is less than [`max_atomic_write_len`],
    /// then the direct write is enabled and [`Sftp::max_write_len`] must be
    /// less than [`max_atomic_write_len`].
    pub fn max_write_len(&self) -> u32 {
        self.handle.get_auxiliary().limits().write_len
    }

    /// The maximum amount of bytes that can be read in one request.
    /// Reading more than that, then your read will be split into multiple requests
    pub fn max_read_len(&self) -> u32 {
        self.handle.get_auxiliary().limits().read_len
    }
}
