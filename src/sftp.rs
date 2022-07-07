use super::{
    auxiliary,
    file::{File, OpenOptions},
    fs::Fs,
    lowlevel, tasks, Error, SftpOptions, SharedData, WriteEnd, WriteEndWithCachedId,
};

use auxiliary::Auxiliary;
use lowlevel::{connect, Extensions};
use tasks::{create_flush_task, create_read_task};

use std::cmp::min;
use std::convert::TryInto;
use std::path::Path;
use std::sync::atomic::Ordering;

use derive_destructure2::destructure;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::oneshot::Receiver;
use tokio::task::JoinHandle;

/// A file-oriented channel to a remote host.
#[derive(Debug, destructure)]
pub struct Sftp {
    shared_data: SharedData,
    flush_task: JoinHandle<Result<(), Error>>,
    read_task: JoinHandle<Result<(), Error>>,
}

impl Sftp {
    /// Create [`Sftp`].
    pub async fn new<W: AsyncWrite + Send + 'static, R: AsyncRead + Send + 'static>(
        stdin: W,
        stdout: R,
        options: SftpOptions,
    ) -> Result<Self, Error> {
        let (write_end, read_end) = connect(
            stdout,
            options.get_write_end_buffer_size(),
            Auxiliary::new(options.get_max_pending_requests()),
        )
        .await?;

        let (rx, read_task) = create_read_task(read_end);

        let flush_task = create_flush_task(
            stdin,
            SharedData::clone(&write_end),
            options.get_flush_interval(),
        );

        Self::init(flush_task, read_task, write_end, rx, &options).await
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
            shared_data: SharedData::clone(&write_end),
            flush_task,
            read_task,
        };

        // Flush the hello message.
        //
        // Cannot use wakeup_flush_task as it would increment requests_to_read
        // by one, while the initial hello message is special and does not
        // count as regular response.
        let auxiliary = sftp.shared_data.get_auxiliary();

        auxiliary.pending_requests.fetch_add(1, Ordering::Relaxed);
        auxiliary.flush_end_notify.notify_one();
        auxiliary.flush_immediately.notify_one();

        let extensions = if let Ok(extensions) = rx.await {
            extensions
        } else {
            // Wait on flush_task and read_task to get a more detailed error message.
            sftp.close().await?;
            std::unreachable!("Error must have occurred in either read_task or flush_task")
        };

        match sftp.set_limits(write_end, options, extensions).await {
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
        &self,
        write_end: WriteEnd,
        options: &SftpOptions,
        extensions: Extensions,
    ) -> Result<(), Error> {
        let mut write_end = WriteEndWithCachedId::new(self, write_end);

        let default_download_buflen = lowlevel::OPENSSH_PORTABLE_DEFAULT_DOWNLOAD_BUFLEN as u64;
        let default_upload_buflen = lowlevel::OPENSSH_PORTABLE_DEFAULT_UPLOAD_BUFLEN as u64;

        // sftp can accept packet as large as u32::MAX, the header itself
        // is at least 9 bytes long.
        let default_max_packet_len = u32::MAX - 9;

        let (read_len, write_len, packet_len) = if extensions.limits {
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
            .map(|v| min(v, read_len))
            .unwrap_or(read_len);

        let write_len = write_len.try_into().unwrap_or(packet_len - 300);
        let write_len = options
            .get_max_write_len()
            .map(|v| min(v, write_len))
            .unwrap_or(write_len);

        let limits = auxiliary::Limits {
            read_len,
            write_len,
        };

        write_end
            .get_auxiliary()
            .conn_info
            .set(auxiliary::ConnInfo { limits, extensions })
            .expect("auxiliary.conn_info shall be empty");

        Ok(())
    }

    /// Close sftp connection
    pub async fn close(self) -> Result<(), Error> {
        let (shared_data, flush_task, read_task) = self.destructure();

        shared_data.get_auxiliary().order_shutdown();

        // Wait for responses for all requests buffered and sent.
        read_task.await??;

        // read_task would order the shutdown of read_task,
        // so we just need to wait for it here.
        flush_task.await??;

        Ok(())
    }

    /// Return a new [`OpenOptions`] object.
    pub fn options(&self) -> OpenOptions<'_> {
        OpenOptions::new(self)
    }

    /// Opens a file in write-only mode.
    ///
    /// This function will create a file if it does not exist, and will truncate
    /// it if it does.
    pub async fn create(&self, path: impl AsRef<Path>) -> Result<File<'_>, Error> {
        self.options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .await
    }

    /// Attempts to open a file in read-only mode.
    pub async fn open(&self, path: impl AsRef<Path>) -> Result<File<'_>, Error> {
        self.options().read(true).open(path).await
    }

    /// [`Fs`] defaults to the current working dir set by remote `sftp-server`,
    /// which usually is the home directory.
    pub fn fs(&self) -> Fs<'_> {
        Fs::new(self.write_end(), "".into())
    }
}

impl Sftp {
    pub(super) fn write_end(&self) -> WriteEndWithCachedId<'_> {
        WriteEndWithCachedId::new(self, WriteEnd::new(self.shared_data.clone()))
    }

    pub(super) fn auxiliary(&self) -> &Auxiliary {
        self.shared_data.get_auxiliary()
    }

    /// Triggers the flushing of the internal buffer in `flush_task`.
    ///
    /// If there are pending requests, then flushing would happen immediately.
    ///
    /// If not, then the next time a request is queued in the write buffer, it
    /// will be immediately flushed.
    pub(super) fn trigger_flushing(&self) {
        self.auxiliary().flush_immediately.notify_one();
    }

    /// Return number of pending requests in the write buffer.
    pub(super) fn get_pending_requests(&self) -> usize {
        self.auxiliary().pending_requests.load(Ordering::Relaxed)
    }
}

#[cfg(feature = "ci-tests")]
impl Sftp {
    /// The maximum amount of bytes that can be written in one request.
    /// Writing more than that, then your write will be split into multiple requests
    ///
    /// If [`Sftp::max_buffered_write`] is less than [`max_atomic_write_len`],
    /// then the direct write is enabled and [`Sftp::max_write_len`] must be
    /// less than [`max_atomic_write_len`].
    pub fn max_write_len(&self) -> u32 {
        self.shared_data.get_auxiliary().limits().write_len
    }

    /// The maximum amount of bytes that can be read in one request.
    /// Reading more than that, then your read will be split into multiple requests
    pub fn max_read_len(&self) -> u32 {
        self.shared_data.get_auxiliary().limits().read_len
    }
}

impl Drop for Sftp {
    fn drop(&mut self) {
        // This will terminate flush_task, otherwise read_task would not return.
        self.shared_data.get_auxiliary().order_shutdown();
    }
}
