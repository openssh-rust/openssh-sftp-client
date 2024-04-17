use std::{future::Future, pin::Pin, sync::Arc};

use openssh::{ChildStdin, ChildStdout, Error as OpensshError, Session, Stdio};
use tokio::{sync::oneshot, task::JoinHandle};

use crate::{utils::ErrorExt, Error, Sftp, SftpAuxiliaryData, SftpOptions};

/// The openssh session
#[derive(Debug)]
pub struct OpensshSession(JoinHandle<Option<Error>>);

/// Check for openssh connection to be alive
pub trait CheckOpensshConnection {
    /// This function should only return on `Err()`.
    /// Once the sftp session is closed, the future will be cancelled (dropped).
    fn check_connection<'session>(
        self: Box<Self>,
        session: &'session Session,
    ) -> Pin<Box<dyn Future<Output = Result<(), OpensshError>> + Send + Sync + 'session>>;
}

impl Drop for OpensshSession {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg_attr(
    feature = "tracing",
    tracing::instrument(name = "session_task", skip(tx, check_openssh_connection))
)]
async fn create_session_task(
    session: Session,
    tx: oneshot::Sender<Result<(ChildStdin, ChildStdout), OpensshError>>,
    check_openssh_connection: Option<Box<dyn CheckOpensshConnection + Send + Sync>>,
) -> Option<Error> {
    #[cfg(feature = "tracing")]
    tracing::info!("Connecting to sftp subsystem, session = {session:?}");

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
            #[cfg(feature = "tracing")]
            tracing::error!(
                "Failed to connect to remote sftp subsystem: {err}, session = {session:?}"
            );

            tx.send(Err(err)).unwrap(); // Err
            return None;
        }
    };

    #[cfg(feature = "tracing")]
    tracing::info!("Connection to sftp subsystem established, session = {session:?}");

    let stdin = child.stdin().take().unwrap();
    let stdout = child.stdout().take().unwrap();
    tx.send(Ok((stdin, stdout))).unwrap(); // Ok

    let original_error = {
        let check_conn_future = async {
            if let Some(checker) = check_openssh_connection {
                checker
                    .check_connection(&session)
                    .await
                    .err()
                    .map(Error::from)
            } else {
                None
            }
        };

        let wait_on_child_future = async {
            match child.wait().await {
                Ok(exit_status) => {
                    if !exit_status.success() {
                        Some(Error::SftpServerFailure(exit_status))
                    } else {
                        None
                    }
                }
                Err(err) => Some(err.into()),
            }
        };
        tokio::pin!(wait_on_child_future);

        tokio::select! {
            biased;

            original_error = check_conn_future => {
                let occuring_error = wait_on_child_future.await;
                match (original_error, occuring_error) {
                    (Some(original_error), Some(occuring_error)) => {
                        Some(original_error.error_on_cleanup(occuring_error))
                    }
                    (Some(err), None) | (None, Some(err)) => Some(err),
                    (None, None) => None,
                }
            }
            original_error = &mut wait_on_child_future => original_error,
        }
    };

    #[cfg(feature = "tracing")]
    if let Some(err) = &original_error {
        tracing::error!(
            "Waiting on remote sftp subsystem to exit failed: {err}, session = {session:?}"
        );
    }

    let _session_str = format!("{session:?}");
    let occuring_error = session.close().await.err().map(Error::from);

    #[cfg(feature = "tracing")]
    if let Some(err) = &occuring_error {
        tracing::error!("Closing session failed: {err}, session = {_session_str}");
    }

    match (original_error, occuring_error) {
        (Some(original_error), Some(occuring_error)) => {
            Some(original_error.error_on_cleanup(occuring_error))
        }
        (Some(err), None) | (None, Some(err)) => Some(err),
        (None, None) => None,
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
        Self::from_session_with_check_connection(session, options, None).await
    }

    /// Similar to [`Sftp::from_session`], but takes an additional parameter
    /// for checking if the connection is still alive.
    pub async fn from_session_with_check_connection(
        session: openssh::Session,
        options: SftpOptions,
        check_openssh_connection: Option<Box<dyn CheckOpensshConnection + Send + Sync>>,
    ) -> Result<Self, Error> {
        let (tx, rx) = oneshot::channel();

        let handle = tokio::spawn(create_session_task(session, tx, check_openssh_connection));

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
    pub(super) async fn recover_session_err(mut self) -> Result<(), Error> {
        if let Some(err) = (&mut self.0).await? {
            Err(err)
        } else {
            Ok(())
        }
    }
}
