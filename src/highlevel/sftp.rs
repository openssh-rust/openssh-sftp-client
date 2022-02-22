use super::{
    auxiliary, lowlevel, tasks, Error, File, Fs, OpenOptions, SftpOptions, SharedData, WriteEnd,
    WriteEndWithCachedId, MAX_ATOMIC_WRITE_LEN,
};

use auxiliary::Auxiliary;
use lowlevel::{connect_with_auxiliary, Extensions};
use tasks::{create_flush_task, create_read_task};

use std::cmp::min;
use std::convert::TryInto;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use derive_destructure2::destructure;
use tokio::task::JoinHandle;
use tokio_pipe::{PipeRead, PipeWrite};
use tokio_util::sync::CancellationToken;

/// A file-oriented channel to a remote host.
#[derive(Debug, destructure)]
pub struct Sftp {
    shared_data: SharedData,
    flush_task: JoinHandle<Result<(), Error>>,
    read_task: JoinHandle<Result<(), Error>>,
}

impl Sftp {
    async fn set_limits(
        &self,
        write_end: WriteEnd,
        options: SftpOptions,
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
        let mut write_len = options
            .get_max_write_len()
            .map(|v| min(v, write_len))
            .unwrap_or(write_len);

        // If max_buffered_write is less than MAX_ATOMIC_WRITE_LEN,
        // then the direct write is enabled and MAX_ATOMIC_WRITE_LEN
        // applies.
        if write_end.get_auxiliary().max_buffered_write < MAX_ATOMIC_WRITE_LEN {
            write_len = min(write_len, MAX_ATOMIC_WRITE_LEN);
        }

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

    /// Create [`Sftp`].
    pub async fn new(
        stdin: PipeWrite,
        stdout: PipeRead,
        options: SftpOptions,
    ) -> Result<Sftp, Error> {
        let (write_end, read_end, extensions) = connect_with_auxiliary(
            stdout,
            stdin,
            Auxiliary::new(
                options.get_max_pending_requests(),
                options.get_max_buffered_write(),
            ),
        )
        .await?;

        // Create sftp here.
        //
        // It would also gracefully shutdown `flush_task` and `read_task` if
        // the future is cancelled or error is encounted.
        let sftp = Self {
            shared_data: SharedData::clone(&write_end),

            flush_task: create_flush_task(
                SharedData::clone(&write_end),
                options.get_flush_interval(),
            ),
            read_task: create_read_task(read_end),
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

    /// Close sftp connection
    pub async fn close(self) -> Result<(), Error> {
        let (shared_data, flush_task, read_task) = self.destructure();

        // This will terminate flush_task, otherwise read_task would not return.
        shared_data.get_auxiliary().requests_shutdown();

        flush_task.await??;

        // Drop the shared_data, otherwise read_task would not return.
        debug_assert_eq!(shared_data.strong_count(), 2);
        drop(shared_data);

        // Wait for responses for all requests buffered and sent.
        read_task.await??;

        Ok(())
    }

    pub(super) fn write_end(&self) -> WriteEndWithCachedId<'_> {
        WriteEndWithCachedId::new(self, WriteEnd::new(self.shared_data.clone()))
    }

    /// Get maximum amount of bytes that one single write requests
    /// can write.
    ///
    /// If [`Sftp::max_buffered_write`] is less than [`MAX_ATOMIC_WRITE_LEN`],
    /// then the direct write is enabled and [`Sftp::max_write_len`] must be
    /// less than [`MAX_ATOMIC_WRITE_LEN`].
    pub fn max_write_len(&self) -> u32 {
        self.shared_data.get_auxiliary().limits().write_len
    }

    /// Get maximum amount of bytes that one single read requests
    /// can read.
    pub fn max_read_len(&self) -> u32 {
        self.shared_data.get_auxiliary().limits().read_len
    }

    /// Get maximum amount of bytes that [`File`] and [`TokioCompactFile`]
    /// would write in a buffered manner.
    pub fn max_buffered_write(&self) -> u32 {
        self.shared_data.get_auxiliary().max_buffered_write
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

    /// * `cwd` - The current working dir for the [`Fs`].
    ///           If `cwd` is empty, then it is set to use
    ///           the default directory set by the remote
    ///           `sftp-server`.
    pub fn fs(&self, cwd: impl Into<PathBuf>) -> Fs<'_> {
        Fs::new(self.write_end(), cwd.into())
    }

    pub(super) fn auxiliary(&self) -> &Auxiliary {
        self.shared_data.get_auxiliary()
    }

    /// without doing anything and return `false`.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn try_flush(&self) -> Result<bool, io::Error> {
        let auxiliary = self.auxiliary();

        let prev_pending_requests = auxiliary.pending_requests.load(Ordering::Relaxed);

        if self.shared_data.try_flush().await? {
            auxiliary.consume_pending_requests(prev_pending_requests);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Forcibly flush the write buffer.
    ///
    /// If another thread is doing flushing, then this function would
    /// wait until it completes or cancelled the future.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub async fn flush(&self) -> Result<(), io::Error> {
        let auxiliary = self.auxiliary();

        let prev_pending_requests = auxiliary.pending_requests.load(Ordering::Relaxed);
        self.shared_data.flush().await?;
        auxiliary.consume_pending_requests(prev_pending_requests);

        Ok(())
    }

    /// Trigger flushing in the `flush_task`.
    ///
    /// If there are pending requests, then flushing would happen immediately.
    ///
    /// If not, then the next time a request is queued in the write buffer, it
    /// will be immediately flushed.
    pub fn trigger_flushing(&self) {
        self.auxiliary().flush_immediately.notify_one();
    }

    /// Return number of pending requests in the write buffer.
    pub fn get_pending_requests(&self) -> u32 {
        self.auxiliary().pending_requests.load(Ordering::Relaxed)
    }

    /// Return a cancellation token that will be cancelled if the `flush_task`
    /// or `read_task` failed is called.
    ///
    /// Cancelling this returned token has no effect on any function in this
    /// module.
    pub fn get_cancellation_token(&self) -> CancellationToken {
        self.auxiliary().cancel_token.child_token()
    }
}

impl Drop for Sftp {
    fn drop(&mut self) {
        // This will terminate flush_task, otherwise read_task would not return.
        self.shared_data.get_auxiliary().requests_shutdown();
    }
}
