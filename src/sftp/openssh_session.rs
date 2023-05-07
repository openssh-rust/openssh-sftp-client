use std::sync::Arc;

use openssh::Stdio;
use tokio::{sync::oneshot, task::JoinHandle};

use crate::{utils::ErrorExt, Error, Sftp, SftpAuxiliaryData, SftpOptions};

/// The openssh session
#[derive(Debug)]
pub struct OpensshSession(JoinHandle<Option<Error>>);

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
        let (tx, rx) = oneshot::channel();

        let handle = tokio::spawn(async move {
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
                    tx.send(Err(err)).unwrap(); // Err
                    return None;
                }
            };

            let stdin = child.stdin().take().unwrap();
            let stdout = child.stdout().take().unwrap();
            tx.send(Ok((stdin, stdout))).unwrap(); // Ok

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

            match (original_error, occuring_error) {
                (Some(original_error), Some(occuring_error)) => {
                    Some(original_error.error_on_cleanup(occuring_error))
                }
                (Some(err), None) | (None, Some(err)) => Some(err),
                (None, None) => None,
            }
        });

        let msg = "Task failed without sending anything, so it must have panicked";

        let (stdin, stdout) = match rx.await {
            Ok(res) => res?,
            Err(_) => return Err(handle.await.expect_err(msg).into()),
        };

        Sftp::new_with_auxiliary(
            stdin,
            stdout,
            options,
            SftpAuxiliaryData::ArcedOpensshSession(Arc::new(OpensshSession(handle))),
        )
        .await
    }
}

impl OpensshSession {
    pub(super) async fn recover_session_err(self) -> Result<(), Error> {
        if let Some(err) = self.0.await? {
            Err(err)
        } else {
            Ok(())
        }
    }
}
