use super::awaitable_responses::AwaitableResponses;
use super::*;

use openssh_sftp_protocol::constants::SSH2_FILEXFER_VERSION;

use core::fmt::Debug;
use core::marker::Unpin;

use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite};

/// TODO:
///  - Support for zero copy API

pub async fn connect<
    Buffer: ToBuffer + Debug + Send + Sync + 'static,
    Writer: AsyncWrite + Unpin,
    Reader: AsyncRead + Unpin,
>(
    reader: Reader,
    writer: Writer,
) -> Result<(WriteEnd<Writer, Buffer>, ReadEnd<Reader, Buffer>), Error> {
    let responses = Arc::new(AwaitableResponses::new());

    let mut read_end = ReadEnd::new(reader, responses.clone());
    let mut write_end = WriteEnd::new(writer, responses);

    // negotiate
    let version = SSH2_FILEXFER_VERSION;

    write_end.send_hello(version, Default::default()).await?;
    read_end.receive_server_version(version).await?;

    Ok((write_end, read_end))
}

#[cfg(test)]
mod tests {
    use crate::*;

    use std::path;
    use std::process::Stdio;

    use once_cell::sync::OnceCell;

    use tokio::process;

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
        let (write_end, read_end) = crate::connect(stdout, stdin).await.unwrap();
        (write_end, read_end, child)
    }

    #[tokio::test]
    async fn test_connect() {
        let mut child = connect().await.2;
        assert!(child.wait().await.unwrap().success());
    }
}
