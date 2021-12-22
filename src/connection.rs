use super::awaitable_responses::AwaitableResponses;
use super::*;

use core::fmt::Debug;
use core::marker::Unpin;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::sync::Notify;

use tokio::io::{AsyncRead, AsyncWrite};

use openssh_sftp_protocol::constants::SSH2_FILEXFER_VERSION;

/// TODO:
///  - Support for zero copy API

/// SharedData contains both the writer and the responses because:
///  - The overhead of `Arc` and a separate allocation;
///  - If the write end of a connection is closed, then openssh implementation
///    of sftp-server would close the read end right away, discarding
///    any unsent but processed or unprocessed responses.
#[derive(Debug)]
pub(crate) struct SharedData<Writer, Buffer: ToBuffer + 'static> {
    pub(crate) writer: Mutex<Writer>,
    pub(crate) responses: AwaitableResponses<Buffer>,

    notify: Notify,
    requests_sent: AtomicU32,

    is_conn_closed: AtomicBool,
}

impl<Writer, Buffer: ToBuffer + 'static> SharedData<Writer, Buffer> {
    fn notify_read_end(&self) {
        // We only have one waiting task, that is `ReadEnd`.
        self.notify.notify_one();
    }

    pub(crate) fn notify_new_packet_event(&self) {
        let prev_requests_sent = self.requests_sent.fetch_add(1, Ordering::Relaxed);

        debug_assert_ne!(prev_requests_sent, u32::MAX);

        // Notify the `ReadEnd` after the requests_sent is incremented.
        self.notify_read_end();
    }

    /// Return number of requests and clear requests_sent.
    /// **Return 0 if the connection is closed.**
    pub(crate) async fn wait_for_new_request(&self) -> u32 {
        loop {
            let cnt = self.requests_sent.swap(0, Ordering::Relaxed);
            if cnt > 0 {
                break cnt;
            }

            if self.is_conn_closed.load(Ordering::Relaxed) {
                break 0;
            }

            self.notify.notified().await;
        }
    }

    /// Notify conn closed should only be called once.
    pub(crate) fn notify_conn_closed(&self) {
        #[cfg(debug_assertions)]
        {
            assert!(!self.is_conn_closed.swap(true, Ordering::Relaxed));
        }
        #[cfg(not(debug_assertions))]
        {
            self.is_conn_closed.store(true, Ordering::Relaxed);
        }

        self.notify_read_end();
    }
}

pub async fn connect<
    Buffer: ToBuffer + Debug + Send + Sync + 'static,
    Writer: AsyncWrite + Unpin,
    Reader: AsyncRead + Unpin,
>(
    reader: Reader,
    writer: Writer,
) -> Result<(WriteEnd<Writer, Buffer>, ReadEnd<Writer, Reader, Buffer>), Error> {
    let shared_data = Arc::new(SharedData {
        writer: Mutex::new(writer),
        responses: AwaitableResponses::new(),
        notify: Notify::new(),
        requests_sent: AtomicU32::new(0),
        is_conn_closed: AtomicBool::new(false),
    });

    let mut read_end = ReadEnd::new(reader, shared_data.clone());
    let mut write_end = WriteEnd::new(shared_data);

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

    use tempfile::{Builder, TempDir};

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
        ReadEnd<process::ChildStdin, process::ChildStdout, Vec<u8>>,
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

    fn create_tmpdir() -> TempDir {
        Builder::new()
            .prefix(".openssh-sftp-client-test")
            .tempdir_in("/tmp")
            .unwrap()
    }

    #[tokio::test]
    async fn test_file_desc() {
        let (mut write_end, mut read_end, mut child) = connect().await;

        let id = write_end.create_response_id();

        let tempdir = create_tmpdir();

        let filename = tempdir.path().join("file");

        let mut file_attrs = FileAttrs::new();
        file_attrs.set_size(2000);
        //file_attrs.set_permissions(libc::S_IRUSR | libc::S_IWUSR);
        let file_attrs = file_attrs;

        let awaitable = write_end
            .send_open_file_request(
                id,
                OpenFile::create(
                    (&filename).into(),
                    FileMode::READ | FileMode::WRITE,
                    CreateFlags::EXCL,
                    file_attrs,
                ),
            )
            .await
            .unwrap();

        assert_eq!(read_end.wait_for_new_request().await, 1);
        read_end.read_in_one_packet().await.unwrap();

        let handle = awaitable.wait().await.unwrap();

        drop(write_end);
        drop(read_end);

        assert!(child.wait().await.unwrap().success());
    }
}
