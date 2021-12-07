use super::*;

use awaitable_responses::AwaitableResponseFactory;

use openssh_sftp_protocol::constants::SSH2_FILEXFER_VERSION;

use core::fmt::Debug;
use core::marker::Unpin;

use parking_lot::RwLock;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite};

/// TODO:
///  - Support for zero copy API

#[derive(Debug)]
pub struct ConnectionFactory<Buffer: ToBuffer + 'static>(AwaitableResponseFactory<Buffer>);

impl<Buffer: ToBuffer + Debug + 'static> Default for ConnectionFactory<Buffer> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Buffer: ToBuffer + Debug + 'static> ConnectionFactory<Buffer> {
    pub fn new() -> Self {
        Self(AwaitableResponseFactory::new())
    }

    pub async fn create<Writer, Reader>(
        &self,
        reader: Reader,
        writer: Writer,
    ) -> Result<(WriteEnd<Writer, Buffer>, ReadEnd<Reader, Buffer>), Error>
    where
        Writer: AsyncWrite + Unpin,
        Reader: AsyncRead + Unpin,
    {
        let responses = Arc::new(RwLock::new(self.0.create()));

        let mut read_end = ReadEnd::new(reader, responses.clone());
        let mut write_end = WriteEnd::new(writer, responses);

        // negotiate
        let version = SSH2_FILEXFER_VERSION;

        write_end.send_hello(version, Default::default()).await?;
        read_end.receive_server_version(version).await?;

        Ok((write_end, read_end))
    }
}

impl<Buffer: ToBuffer + 'static> Clone for ConnectionFactory<Buffer> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    use std::path;
    use std::process::Stdio;

    use once_cell::sync::OnceCell;

    use tokio::process;

    fn get_conn_factory() -> &'static ConnectionFactory<Vec<u8>> {
        static CONN_FACTORY: OnceCell<ConnectionFactory<Vec<u8>>> = OnceCell::new();
        CONN_FACTORY.get_or_init(ConnectionFactory::new)
    }

    fn get_sftp_path() -> &'static path::Path {
        static SFTP_PATH: OnceCell<path::PathBuf> = OnceCell::new();

        SFTP_PATH.get_or_init(|| {
            let mut sftp_path = path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            sftp_path.push("static-openssh-sftp-server");
            sftp_path.push("sftp-server");

            sftp_path
        })
    }

    async fn launch_sftp() -> (process::Child, process::ChildStdin, process::ChildStdout) {
        let mut child = process::Command::new(get_sftp_path())
            .args(&["-e", "-l", "DEBUG"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .unwrap();

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        (child, stdin, stdout)
    }

    async fn connect() -> (
        WriteEnd<process::ChildStdin, Vec<u8>>,
        ReadEnd<process::ChildStdout, Vec<u8>>,
        process::Child,
    ) {
        let (child, stdin, stdout) = launch_sftp().await;
        let (write_end, read_end) = get_conn_factory().create(stdout, stdin).await.unwrap();
        (write_end, read_end, child)
    }

    #[tokio::test]
    async fn test_connect() {
        let mut child = connect().await.2;
        assert!(child.wait().await.unwrap().success());
    }
}
