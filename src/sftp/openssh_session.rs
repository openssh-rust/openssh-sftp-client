use std::{
    fmt,
    future::{poll_fn, Future},
    pin::Pin,
    sync::Arc,
    task::Poll,
};

use once_cell::sync::OnceCell;
use openssh::Stdio;
use pin_project::pin_project;
use tokio::sync::Notify;

use crate::{
    error::{Error, RecursiveError},
    Sftp, SftpAuxiliaryData, SftpOptions,
};

#[derive(Debug, Default)]
struct ErrChannel {
    closed_notify: Notify,
    once_cell: OnceCell<Error>,
}

/// The openssh session
#[pin_project]
pub struct OpensshSession {
    future: Pin<Box<dyn Future<Output = ()> + Send + Sync + 'static>>,
    err_channel: Arc<ErrChannel>,
}

impl fmt::Debug for OpensshSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let err_channel = &self.err_channel;
        write!(
            f,
            "OpensshSession {{\n    fut: Box<dyn Future<Output = ()>>,\n    err_channel: {err_channel:?}}}"
        )
    }
}

impl Sftp {
    /// Create [`Sftp`] from [`openssh::Session`].
    ///
    /// Calling [`Sftp::close`] on sftp instances created using this function
    /// would also await on [`openssh::RemoteChild::wait`] and
    /// [`openssh::Session::close`] and propagate their error in
    /// [`Sftp::close`].
    pub async fn from_session(
        session: openssh::Session,
        options: SftpOptions,
    ) -> Result<Self, Error> {
        let mut stdio = Arc::new(OnceCell::new());
        let stdio_cloned = Arc::clone(&stdio);

        let err_channel = Arc::new(ErrChannel::default());
        let err_channel_cloned = Arc::clone(&err_channel);

        let mut future = Box::pin(async move {
            let res = session
                .subsystem("sftp")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .await;

            let mut child = match res {
                Ok(child) => child,
                Err(err) => {
                    stdio_cloned.set(Err(err)).unwrap(); // Err
                    drop(stdio_cloned);
                    return;
                }
            };

            let stdin = child.stdin().take().unwrap();
            let stdout = child.stdout().take().unwrap();
            stdio_cloned.set(Ok((stdin, stdout))).unwrap(); // Ok
            drop(stdio_cloned);

            // Wait for sftp to be closed.
            err_channel_cloned.closed_notify.notified().await;

            let original_error = match child.wait().await {
                Ok(exit_status) => {
                    if !exit_status.success() {
                        Some(Error::SftpServerFailure(exit_status))
                    } else {
                        None
                    }
                }
                Err(err) => Some(err.into()),
            };

            let occuring_error = session.close().await.err().map(Error::from);

            let error = match (original_error, occuring_error) {
                (Some(original_error), Some(occuring_error)) => {
                    Some(Error::RecursiveErrors(Box::new(RecursiveError {
                        original_error,
                        occuring_error,
                    })))
                }
                (Some(err), None) | (None, Some(err)) => Some(err),
                (None, None) => None,
            };

            if let Some(err) = error {
                err_channel_cloned.once_cell.set(err).unwrap();
            }

            drop(err_channel_cloned);
        });

        let (stdin, stdout) = poll_fn(|cx| {
            let _ = future.as_mut().poll(cx);
            if let Some(once_cell) = Arc::get_mut(&mut stdio) {
                // future must have set some value before dropping stdio_cloned
                return Poll::Ready(once_cell.take().unwrap());
            }
            Poll::Pending
        })
        .await?;

        Sftp::new_with_auxiliary(
            stdin,
            stdout,
            options,
            SftpAuxiliaryData::ArcedOpensshSession(Arc::new(OpensshSession {
                future,
                err_channel,
            })),
        )
        .await
    }
}

impl OpensshSession {
    pub(super) async fn recover_session_err(self) -> Result<(), Error> {
        // Notify self.future to continue executing.
        self.err_channel.closed_notify.notify_one();
        self.future.await;
        // future is now completed and err_channel_cloned must be dropped
        if let Some(err) = Arc::try_unwrap(self.err_channel).unwrap().once_cell.take() {
            Err(err)
        } else {
            Ok(())
        }
    }
}
