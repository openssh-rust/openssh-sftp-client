use super::awaitable_responses::AwaitableResponses;
use super::writer::Writer;
use super::*;

use core::fmt::Debug;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use tokio::sync::Notify;
use tokio_pipe::{PipeRead, PipeWrite};

use openssh_sftp_protocol::constants::SSH2_FILEXFER_VERSION;

/// TODO:
///  - Support for zero copy API

/// SharedData contains both the writer and the responses because:
///  - The overhead of `Arc` and a separate allocation;
///  - If the write end of a connection is closed, then openssh implementation
///    of sftp-server would close the read end right away, discarding
///    any unsent but processed or unprocessed responses.
#[derive(Debug)]
pub(crate) struct SharedData<Buffer: ToBuffer + 'static> {
    pub(crate) writer: Writer,
    pub(crate) responses: AwaitableResponses<Buffer>,

    notify: Notify,
    requests_sent: AtomicU32,

    is_conn_closed: AtomicBool,
}

impl<Buffer: ToBuffer + 'static> SharedData<Buffer> {
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

pub async fn connect<Buffer: ToBuffer + Debug + Send + Sync + 'static>(
    reader: PipeRead,
    writer: PipeWrite,
) -> Result<(WriteEnd<Buffer>, ReadEnd<Buffer>), Error> {
    let shared_data = Arc::new(SharedData {
        writer: Writer::new(writer),
        responses: AwaitableResponses::new(),
        notify: Notify::new(),
        requests_sent: AtomicU32::new(0),
        is_conn_closed: AtomicBool::new(false),
    });

    let mut read_end = ReadEnd::new(reader, shared_data.clone());
    let mut write_end = WriteEnd::new(shared_data);

    // negotiate
    let version = SSH2_FILEXFER_VERSION;

    write_end.send_hello(version).await?;
    read_end.receive_server_version(version).await?;

    Ok((write_end, read_end))
}

#[cfg(test)]
#[cfg(ci)]
mod tests {
    use crate::*;

    use child_io_to_pipe::*;

    use std::borrow::Cow;
    use std::env;
    use std::fs;
    use std::io;
    use std::os::unix::fs::symlink;
    use std::path;
    use std::process::Stdio;

    use once_cell::sync::OnceCell;

    use tokio::process;

    use tempfile::{Builder, TempDir};

    fn assert_not_found(err: io::Error) {
        assert!(matches!(err.kind(), io::ErrorKind::NotFound), "{:#?}", err);
    }

    fn get_sftp_path() -> &'static path::Path {
        static SFTP_PATH: OnceCell<path::PathBuf> = OnceCell::new();

        SFTP_PATH.get_or_init(|| {
            let mut sftp_path: path::PathBuf = env::var("OUT_DIR").unwrap().into();
            sftp_path.push("openssh-portable");
            sftp_path.push("sftp-server");

            eprintln!("sftp_path = {:#?}", sftp_path);

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

    async fn connect() -> (WriteEnd<Vec<u8>>, ReadEnd<Vec<u8>>, process::Child) {
        let (child, stdin, stdout) = launch_sftp().await;

        let stdout = child_stdout_to_pipewrite(stdout).unwrap();
        let stdin = child_stdin_to_pipewrite(stdin).unwrap();

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

    async fn read_one_packet(read_end: &mut ReadEnd<Vec<u8>>) {
        eprintln!("Wait for new request");
        assert_eq!(read_end.wait_for_new_request().await, 1);

        eprintln!("Read in one packet");
        read_end.read_in_one_packet().await.unwrap();
    }

    #[tokio::test]
    async fn test_file_desc() {
        let (mut write_end, mut read_end, mut child) = connect().await;

        let id = write_end.create_response_id();

        let tempdir = create_tmpdir();

        let filename = tempdir.path().join("file");

        // Create one file and write to it
        let mut file_attrs = FileAttrs::new();
        file_attrs.set_size(2000);
        file_attrs.set_permissions(Permissions::READ_BY_OWNER | Permissions::WRITE_BY_OWNER);
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

        read_one_packet(&mut read_end).await;
        let (id, handle) = awaitable.wait().await.unwrap();

        eprintln!("handle = {:#?}", handle);

        let msg = "Hello, world!".as_bytes();

        let awaitable = write_end
            .send_write_request(id, &handle, 0, msg)
            .await
            .unwrap();

        eprintln!("Waiting for write response");

        read_one_packet(&mut read_end).await;
        let id = awaitable.wait().await.unwrap().0;

        let awaitable = write_end
            .send_close_request(id, Cow::Borrowed(&handle))
            .await
            .unwrap();

        read_one_packet(&mut read_end).await;
        let id = awaitable.wait().await.unwrap().0;

        // Open it again and read from it
        let awaitable = write_end
            .send_open_file_request(id, OpenFile::open((&filename).into(), FileMode::READ))
            .await
            .unwrap();

        read_one_packet(&mut read_end).await;
        let (id, handle) = awaitable.wait().await.unwrap();

        eprintln!("handle = {:#?}", handle);

        let awaitable = write_end
            .send_read_request(id, Cow::Borrowed(&handle), 0, msg.len() as u32, None)
            .await
            .unwrap();

        read_one_packet(&mut read_end).await;
        let (id, data) = awaitable.wait().await.unwrap();

        match data {
            Data::AllocatedBox(data) => assert_eq!(&*data, msg),
            _ => panic!("Unexpected data"),
        };

        drop(id);
        drop(write_end);

        assert_eq!(read_end.wait_for_new_request().await, 0);

        drop(read_end);

        assert!(child.wait().await.unwrap().success());
    }

    #[tokio::test]
    async fn test_file_remove() {
        let (mut write_end, mut read_end, mut child) = connect().await;

        let id = write_end.create_response_id();

        let tempdir = create_tmpdir();
        let filename = tempdir.path().join("file");
        fs::File::create(&filename).unwrap().set_len(2000).unwrap();

        // remove it
        let awaitable = write_end
            .send_remove_request(id, Cow::Borrowed(&filename))
            .await
            .unwrap();

        read_one_packet(&mut read_end).await;
        let id = awaitable.wait().await.unwrap().0;

        // Try open it again
        let err = fs::File::open(&filename).unwrap_err();

        assert_not_found(err);

        drop(id);
        drop(write_end);

        assert_eq!(read_end.wait_for_new_request().await, 0);

        drop(read_end);

        assert!(child.wait().await.unwrap().success());
    }

    #[tokio::test]
    async fn test_file_rename() {
        let (mut write_end, mut read_end, mut child) = connect().await;

        let id = write_end.create_response_id();

        let tempdir = create_tmpdir();
        let filename = tempdir.path().join("file");
        fs::File::create(&filename).unwrap().set_len(2000).unwrap();

        // rename it
        let new_filename = tempdir.path().join("file2");

        let awaitable = write_end
            .send_rename_request(id, Cow::Borrowed(&filename), Cow::Borrowed(&new_filename))
            .await
            .unwrap();

        read_one_packet(&mut read_end).await;
        let id = awaitable.wait().await.unwrap().0;

        // Open it again
        let metadata = fs::File::open(&new_filename).unwrap().metadata().unwrap();

        assert!(metadata.is_file());
        assert_eq!(metadata.len(), 2000);

        drop(id);
        drop(write_end);

        assert_eq!(read_end.wait_for_new_request().await, 0);

        drop(read_end);

        assert!(child.wait().await.unwrap().success());
    }

    #[tokio::test]
    async fn test_mkdir() {
        let (mut write_end, mut read_end, mut child) = connect().await;

        let id = write_end.create_response_id();

        let tempdir = create_tmpdir();
        let dirname = tempdir.path().join("dir");

        // mkdir it
        let awaitable = write_end
            .send_mkdir_request(id, Cow::Borrowed(&dirname), FileAttrs::default())
            .await
            .unwrap();

        read_one_packet(&mut read_end).await;
        let id = awaitable.wait().await.unwrap().0;

        // Open it
        assert!(fs::read_dir(&dirname).unwrap().next().is_none());

        drop(id);
        drop(write_end);

        assert_eq!(read_end.wait_for_new_request().await, 0);

        drop(read_end);

        assert!(child.wait().await.unwrap().success());
    }

    #[tokio::test]
    async fn test_rmdir() {
        let (mut write_end, mut read_end, mut child) = connect().await;

        let id = write_end.create_response_id();

        let tempdir = create_tmpdir();
        let dirname = tempdir.path().join("dir");

        fs::DirBuilder::new().create(&dirname).unwrap();

        // rmdir it
        let awaitable = write_end
            .send_rmdir_request(id, Cow::Borrowed(&dirname))
            .await
            .unwrap();

        read_one_packet(&mut read_end).await;
        let id = awaitable.wait().await.unwrap().0;

        // Try open it
        let err = fs::read_dir(&dirname).unwrap_err();
        assert_not_found(err);

        drop(id);
        drop(write_end);

        assert_eq!(read_end.wait_for_new_request().await, 0);

        drop(read_end);

        assert!(child.wait().await.unwrap().success());
    }

    #[tokio::test]
    async fn test_dir_desc() {
        let (mut write_end, mut read_end, mut child) = connect().await;

        let id = write_end.create_response_id();

        let tempdir = create_tmpdir();
        let dirname = tempdir.path().join("dir");

        let subdir = dirname.join("subdir");
        fs::DirBuilder::new()
            .recursive(true)
            .create(&subdir)
            .unwrap();

        let file = dirname.join("file");
        fs::File::create(&file).unwrap().set_len(2000).unwrap();

        // open it
        let awaitable = write_end
            .send_opendir_request(id, Cow::Borrowed(&dirname))
            .await
            .unwrap();

        read_one_packet(&mut read_end).await;
        let (id, handle) = awaitable.wait().await.unwrap();

        // read it
        let awaitable = write_end
            .send_readdir_request(id, Cow::Borrowed(&*handle))
            .await
            .unwrap();

        read_one_packet(&mut read_end).await;
        let (id, entries) = awaitable.wait().await.unwrap();

        for entry in entries.iter() {
            let filename = &*entry.filename;

            if filename == path::Path::new(".") || filename == path::Path::new("..") {
                continue;
            }

            assert!(
                filename == path::Path::new("subdir") || filename == path::Path::new("file"),
                "{:#?}",
                filename
            );

            if filename == file {
                assert_eq!(entry.attrs.get_size().unwrap(), 2000);
            }
        }

        drop(id);
        drop(write_end);

        assert_eq!(read_end.wait_for_new_request().await, 0);

        drop(read_end);

        assert!(child.wait().await.unwrap().success());
    }

    #[tokio::test]
    async fn test_stat() {
        let (mut write_end, mut read_end, mut child) = connect().await;

        let id = write_end.create_response_id();

        let tempdir = create_tmpdir();
        let filename = tempdir.path().join("file");

        fs::File::create(&filename).unwrap().set_len(2000).unwrap();

        let linkname = tempdir.path().join("symlink");
        symlink(&filename, &linkname).unwrap();

        // stat
        let awaitable = write_end
            .send_stat_request(id, Cow::Borrowed(&linkname))
            .await
            .unwrap();

        read_one_packet(&mut read_end).await;
        let (id, attrs) = awaitable.wait().await.unwrap();

        assert_eq!(attrs.get_size().unwrap(), 2000);
        assert_eq!(attrs.get_filetype().unwrap(), FileType::RegularFile);

        drop(id);
        drop(write_end);

        assert_eq!(read_end.wait_for_new_request().await, 0);

        drop(read_end);

        assert!(child.wait().await.unwrap().success());
    }

    #[tokio::test]
    async fn test_lstat() {
        let (mut write_end, mut read_end, mut child) = connect().await;

        let id = write_end.create_response_id();

        let tempdir = create_tmpdir();
        let filename = tempdir.path().join("file");

        fs::File::create(&filename).unwrap().set_len(2000).unwrap();

        let linkname = tempdir.path().join("symlink");
        symlink(&filename, &linkname).unwrap();

        // lstat
        let awaitable = write_end
            .send_lstat_request(id, Cow::Borrowed(&linkname))
            .await
            .unwrap();

        read_one_packet(&mut read_end).await;
        let (id, attrs) = awaitable.wait().await.unwrap();

        assert_eq!(attrs.get_filetype().unwrap(), FileType::Symlink);

        drop(id);
        drop(write_end);

        assert_eq!(read_end.wait_for_new_request().await, 0);

        drop(read_end);

        assert!(child.wait().await.unwrap().success());
    }

    #[tokio::test]
    async fn test_fstat() {
        let (mut write_end, mut read_end, mut child) = connect().await;

        let id = write_end.create_response_id();

        let tempdir = create_tmpdir();
        let filename = tempdir.path().join("file");

        fs::File::create(&filename).unwrap().set_len(2000).unwrap();

        // open
        let awaitable = write_end
            .send_open_file_request(id, OpenFile::open(Cow::Borrowed(&filename), FileMode::READ))
            .await
            .unwrap();

        read_one_packet(&mut read_end).await;
        let (id, handle) = awaitable.wait().await.unwrap();

        // fstat
        let awaitable = write_end
            .send_fstat_request(id, Cow::Borrowed(&handle))
            .await
            .unwrap();

        read_one_packet(&mut read_end).await;
        let (id, attrs) = awaitable.wait().await.unwrap();

        assert_eq!(attrs.get_size().unwrap(), 2000);
        assert_eq!(attrs.get_filetype().unwrap(), FileType::RegularFile);

        drop(id);
        drop(write_end);

        assert_eq!(read_end.wait_for_new_request().await, 0);

        drop(read_end);

        assert!(child.wait().await.unwrap().success());
    }

    #[tokio::test]
    async fn test_setstat() {
        let (mut write_end, mut read_end, mut child) = connect().await;

        let id = write_end.create_response_id();

        let tempdir = create_tmpdir();
        let filename = tempdir.path().join("file");

        fs::File::create(&filename).unwrap().set_len(2000).unwrap();

        let mut fileattrs = FileAttrs::default();

        fileattrs.set_size(10000);

        // setstat
        let awaitable = write_end
            .send_setstat_request(id, Cow::Borrowed(&filename), fileattrs)
            .await
            .unwrap();

        read_one_packet(&mut read_end).await;
        let id = awaitable.wait().await.unwrap().0;

        // stat
        let awaitable = write_end
            .send_stat_request(id, Cow::Borrowed(&filename))
            .await
            .unwrap();

        read_one_packet(&mut read_end).await;
        let (id, attrs) = awaitable.wait().await.unwrap();

        assert_eq!(attrs.get_size().unwrap(), 10000);
        assert_eq!(attrs.get_filetype().unwrap(), FileType::RegularFile);

        drop(id);
        drop(write_end);

        assert_eq!(read_end.wait_for_new_request().await, 0);

        drop(read_end);

        assert!(child.wait().await.unwrap().success());
    }

    #[tokio::test]
    async fn test_fsetstat() {
        let (mut write_end, mut read_end, mut child) = connect().await;

        let id = write_end.create_response_id();

        let tempdir = create_tmpdir();
        let filename = tempdir.path().join("file");

        fs::File::create(&filename).unwrap().set_len(2000).unwrap();

        // open
        let awaitable = write_end
            .send_open_file_request(
                id,
                OpenFile::open(Cow::Borrowed(&filename), FileMode::READ | FileMode::WRITE),
            )
            .await
            .unwrap();

        read_one_packet(&mut read_end).await;
        let (id, handle) = awaitable.wait().await.unwrap();

        // fsetstat
        let mut fileattrs = FileAttrs::default();
        fileattrs.set_size(10000);

        let awaitable = write_end
            .send_fsetstat_request(id, Cow::Borrowed(&handle), fileattrs)
            .await
            .unwrap();

        // Error here
        read_one_packet(&mut read_end).await;
        let id = awaitable.wait().await.unwrap().0;

        // fstat
        let awaitable = write_end
            .send_fstat_request(id, Cow::Borrowed(&handle))
            .await
            .unwrap();

        read_one_packet(&mut read_end).await;
        let (id, attrs) = awaitable.wait().await.unwrap();

        assert_eq!(attrs.get_size().unwrap(), 10000);
        assert_eq!(attrs.get_filetype().unwrap(), FileType::RegularFile);

        drop(id);
        drop(write_end);

        assert_eq!(read_end.wait_for_new_request().await, 0);

        drop(read_end);

        assert!(child.wait().await.unwrap().success());
    }

    #[tokio::test]
    async fn test_readlink() {
        let (mut write_end, mut read_end, mut child) = connect().await;

        let id = write_end.create_response_id();

        let tempdir = create_tmpdir();
        let filename = tempdir.path().join("file");

        fs::File::create(&filename).unwrap().set_len(2000).unwrap();

        let linkname = tempdir.path().join("symlink");
        symlink(&filename, &linkname).unwrap();

        // readlink
        let awaitable = write_end
            .send_readlink_request(id, Cow::Borrowed(&linkname))
            .await
            .unwrap();

        read_one_packet(&mut read_end).await;
        let (id, path) = awaitable.wait().await.unwrap();

        assert_eq!(&*path, &*filename);

        drop(id);
        drop(write_end);

        assert_eq!(read_end.wait_for_new_request().await, 0);

        drop(read_end);

        assert!(child.wait().await.unwrap().success());
    }

    #[tokio::test]
    async fn test_readpath() {
        let (mut write_end, mut read_end, mut child) = connect().await;

        let id = write_end.create_response_id();

        let tempdir = create_tmpdir();
        let filename = tempdir.path().join("file");

        fs::File::create(&filename).unwrap().set_len(2000).unwrap();

        let linkname = tempdir.path().join("symlink");
        symlink(&filename, &linkname).unwrap();

        // readpath
        let awaitable = write_end
            .send_realpath_request(id, Cow::Borrowed(&linkname))
            .await
            .unwrap();

        read_one_packet(&mut read_end).await;
        let (id, path) = awaitable.wait().await.unwrap();

        assert_eq!(&*path, &*fs::canonicalize(&filename).unwrap());

        drop(id);
        drop(write_end);

        assert_eq!(read_end.wait_for_new_request().await, 0);

        drop(read_end);

        assert!(child.wait().await.unwrap().success());
    }
}
