#![forbid(unsafe_code)]

use super::awaitable_responses::AwaitableResponses;
use super::writer::Writer;
use super::*;

use core::fmt::Debug;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use tokio::sync::Notify;
use tokio_pipe::{PipeRead, PipeWrite};

use openssh_sftp_protocol::constants::SSH2_FILEXFER_VERSION;

// TODO:
//  - Support for zero copy API

#[derive(Debug)]
struct SharedDataInner<Buffer> {
    writer: Writer,
    responses: AwaitableResponses<Buffer>,

    notify: Notify,
    requests_sent: AtomicU32,

    is_conn_closed: AtomicBool,
}

/// SharedData contains both the writer and the responses because:
///  - The overhead of `Arc` and a separate allocation;
///  - If the write end of a connection is closed, then openssh implementation
///    of sftp-server would close the read end right away, discarding
///    any unsent but processed or unprocessed responses.
#[derive(Debug)]
pub struct SharedData<Buffer>(Arc<SharedDataInner<Buffer>>);

impl<Buffer> Clone for SharedData<Buffer> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<Buffer> Drop for SharedData<Buffer> {
    fn drop(&mut self) {
        // If this is the last reference, except for `ReadEnd`, to the SharedData,
        // then the connection is closed.
        if self.strong_count() == 2 {
            #[cfg(debug_assertions)]
            {
                assert!(!self.0.is_conn_closed.swap(true, Ordering::Relaxed));
            }
            #[cfg(not(debug_assertions))]
            {
                self.0.is_conn_closed.store(true, Ordering::Relaxed);
            }

            self.notify_read_end();
        }
    }
}

impl<Buffer> SharedData<Buffer> {
    pub(crate) fn writer(&self) -> &Writer {
        &self.0.writer
    }

    pub(crate) fn responses(&self) -> &AwaitableResponses<Buffer> {
        &self.0.responses
    }

    pub(crate) fn get_mut_writer(&mut self) -> Option<&mut Writer> {
        Arc::get_mut(&mut self.0).map(|shared_data| &mut shared_data.writer)
    }

    /// `SharedData` is a newtype wrapper for `Arc<SharedDataInner>`,
    /// so this function returns how many `Arc` there are that referred
    /// to the shared data.
    #[inline(always)]
    pub fn strong_count(&self) -> usize {
        Arc::strong_count(&self.0)
    }

    #[inline(always)]
    fn notify_read_end(&self) {
        // We only have one waiting task, that is `ReadEnd`.
        self.0.notify.notify_one();
    }

    pub(crate) fn notify_new_packet_event(&self) {
        let prev_requests_sent = self.0.requests_sent.fetch_add(1, Ordering::Relaxed);

        debug_assert_ne!(prev_requests_sent, u32::MAX);

        // Notify the `ReadEnd` after the requests_sent is incremented.
        self.notify_read_end();
    }

    /// Return number of requests and clear requests_sent.
    /// **Return 0 if the connection is closed.**
    pub(crate) async fn wait_for_new_request(&self) -> u32 {
        loop {
            let cnt = self.0.requests_sent.swap(0, Ordering::Relaxed);
            if cnt > 0 {
                break cnt;
            }

            if self.0.is_conn_closed.load(Ordering::Relaxed) {
                break 0;
            }

            self.0.notify.notified().await;
        }
    }
}

impl<Buffer: Send + Sync> SharedData<Buffer> {
    /// Create a useable response id.
    #[inline(always)]
    pub fn create_response_id(&self) -> Id<Buffer> {
        self.responses().insert()
    }

    /// Return true if reserve succeeds, false otherwise.
    #[inline(always)]
    pub fn try_reserve_id(&self, new_id_cnt: u32) -> bool {
        self.responses().try_reserve(new_id_cnt)
    }

    /// Return true if reserve succeeds, false otherwise.
    #[inline(always)]
    pub fn reserve_id(&self, new_id_cnt: u32) {
        self.responses().reserve(new_id_cnt);
    }

    /// Flush the write buffer.
    ///
    /// If another thread is flushing or there isn't any
    /// data to write, then `Ok(false)` will be returned.
    ///
    /// # Cancel Safety
    ///
    /// This function is only cancel safe if [`WriteEnd::send_write_request_direct`] or
    /// [`WriteEnd::send_write_request_direct_vectored`] is not called when this
    /// future is cancelled.
    ///
    /// Upon cancel, it might only partially flushed out the data, which can be
    /// restarted by another thread.
    ///
    /// However, if [`WriteEnd::send_write_request_direct`] or
    /// [`WriteEnd::send_write_request_direct_vectored`] is called, then the write data
    /// will be interleaved and thus produce undefined behavior.
    #[inline]
    pub async fn flush(&self) -> Result<bool, Error> {
        Ok(self.writer().flush().await?)
    }
}

/// Initialize connection to remote sftp server and
/// negotiate the sftp version.
///
/// # Cancel Safety
///
/// This function is not cancel safe.
///
/// After dropping the future, the connection would be in a undefined state.
pub async fn connect<Buffer: ToBuffer + Send + Sync + 'static>(
    reader: PipeRead,
    writer: PipeWrite,
) -> Result<(WriteEnd<Buffer>, ReadEnd<Buffer>, Extensions), Error> {
    let shared_data = SharedData(Arc::new(SharedDataInner {
        writer: Writer::new(writer),
        responses: AwaitableResponses::new(),
        notify: Notify::new(),
        requests_sent: AtomicU32::new(0),
        is_conn_closed: AtomicBool::new(false),
    }));

    // Send hello message
    let version = SSH2_FILEXFER_VERSION;

    let mut write_end = WriteEnd::new(shared_data);
    write_end.send_hello(version).await?;

    // Receive version and extensions
    let mut read_end = ReadEnd::new(reader, (*write_end).clone());
    let extensions = read_end.receive_server_version(version).await?;

    Ok((write_end, read_end, extensions))
}

#[cfg(test)]
mod tests {
    use crate::*;

    use child_io_to_pipe::*;

    use std::borrow::Cow;
    use std::env;
    use std::fs;
    use std::io;
    use std::io::IoSlice;
    use std::os::unix::fs::symlink;
    use std::path;
    use std::process::Stdio;

    use bytes::Bytes;
    use once_cell::sync::OnceCell;
    use tempfile::{Builder, TempDir};
    use tokio::process;

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

    async fn connect_with_extensions() -> (
        WriteEnd<Vec<u8>>,
        ReadEnd<Vec<u8>>,
        process::Child,
        Extensions,
    ) {
        let (child, stdin, stdout) = launch_sftp().await;

        let stdout = child_stdout_to_pipewrite(stdout).unwrap();
        let stdin = child_stdin_to_pipewrite(stdin).unwrap();

        let (write_end, read_end, extensions) = crate::connect(stdout, stdin).await.unwrap();
        (write_end, read_end, child, extensions)
    }

    async fn connect() -> (WriteEnd<Vec<u8>>, ReadEnd<Vec<u8>>, process::Child) {
        let (write_end, read_end, child, _extensions) = connect_with_extensions().await;
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

    async fn read_one_packet(write_end: &mut WriteEnd<Vec<u8>>, read_end: &mut ReadEnd<Vec<u8>>) {
        write_end.flush().await.unwrap();

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

        // Create file
        let mut file_attrs = FileAttrs::new();
        file_attrs.set_size(2000);
        file_attrs.set_permissions(Permissions::READ_BY_OWNER | Permissions::WRITE_BY_OWNER);
        let file_attrs = file_attrs;

        let awaitable = write_end
            .send_open_file_request(
                id,
                OpenOptions::new().read(true).write(true).create(
                    Cow::Borrowed(&filename),
                    CreateFlags::Excl,
                    file_attrs,
                ),
            )
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
        let (id, handle) = awaitable.wait().await.unwrap();

        eprintln!("handle = {:#?}", handle);

        // Write msg into it
        let msg = "Hello, world!".as_bytes();

        let awaitable = write_end
            .send_write_request_direct(id, Cow::Borrowed(&handle), 0, msg)
            .await
            .unwrap();

        eprintln!("Waiting for write response");

        read_one_packet(&mut write_end, &mut read_end).await;
        let id = awaitable.wait().await.unwrap().0;

        // Close it
        let awaitable = write_end
            .send_close_request(id, Cow::Borrowed(&handle))
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
        let id = awaitable.wait().await.unwrap().0;

        // Open it again
        let awaitable = write_end
            .send_open_file_request(id, OpenFileRequest::open(Cow::Borrowed(&filename)))
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
        let (id, handle) = awaitable.wait().await.unwrap();

        eprintln!("handle = {:#?}", handle);

        // Read from it
        let awaitable = write_end
            .send_read_request(id, Cow::Borrowed(&handle), 0, msg.len() as u32, None)
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
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

    async fn test_write_impl(
        write: impl FnOnce(
            &mut WriteEnd<Vec<u8>>,
            Id<Vec<u8>>,
            &Handle,
            &[u8],
        ) -> AwaitableStatus<Vec<u8>>,
    ) {
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
                OpenOptions::new().read(true).write(true).create(
                    Cow::Borrowed(&filename),
                    CreateFlags::Excl,
                    file_attrs,
                ),
            )
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
        let (id, handle) = awaitable.wait().await.unwrap();

        eprintln!("handle = {:#?}", handle);

        let msg = "Hello, world!".as_bytes();

        let awaitable = write(&mut write_end, id, &handle, &msg);

        eprintln!("Waiting for write response");

        read_one_packet(&mut write_end, &mut read_end).await;
        let id = awaitable.wait().await.unwrap().0;

        // Read from it
        let awaitable = write_end
            .send_read_request(id, Cow::Borrowed(&handle), 0, msg.len() as u32, None)
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
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
    async fn test_write_direct_vectored() {
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
                OpenOptions::new().read(true).write(true).create(
                    Cow::Borrowed(&filename),
                    CreateFlags::Excl,
                    file_attrs,
                ),
            )
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
        let (id, handle) = awaitable.wait().await.unwrap();

        eprintln!("handle = {:#?}", handle);

        let msg = "Hello, world!".as_bytes();

        let awaitable = write_end
            .send_write_request_direct_vectored(
                id,
                Cow::Borrowed(&handle),
                0,
                &[IoSlice::new(&msg[..3]), IoSlice::new(&msg[3..])],
            )
            .await
            .unwrap();

        eprintln!("Waiting for write response");

        read_one_packet(&mut write_end, &mut read_end).await;
        let id = awaitable.wait().await.unwrap().0;

        // Read from it
        let awaitable = write_end
            .send_read_request(id, Cow::Borrowed(&handle), 0, msg.len() as u32, None)
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
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
    async fn test_write_buffered() {
        test_write_impl(|write_end, id, handle, msg| {
            write_end
                .send_write_request_buffered(id, Cow::Borrowed(&handle), 0, Cow::Borrowed(msg))
                .unwrap()
        })
        .await;
    }

    #[tokio::test]
    async fn test_write_buffered_vectored() {
        test_write_impl(|write_end, id, handle, msg| {
            write_end
                .send_write_request_buffered_vectored(
                    id,
                    Cow::Borrowed(&handle),
                    0,
                    &[IoSlice::new(&msg[..3]), IoSlice::new(&msg[3..])],
                )
                .unwrap()
        })
        .await;
    }

    #[tokio::test]
    async fn test_write_zero_copy() {
        test_write_impl(|write_end, id, handle, msg| {
            write_end
                .send_write_request_zero_copy(
                    id,
                    Cow::Borrowed(&handle),
                    0,
                    &[
                        Bytes::copy_from_slice(&msg[..3]),
                        Bytes::copy_from_slice(&msg[3..]),
                    ],
                )
                .unwrap()
        })
        .await;
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
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
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
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
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
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
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
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
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
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
        let (id, handle) = awaitable.wait().await.unwrap();

        // read it
        let awaitable = write_end
            .send_readdir_request(id, Cow::Borrowed(&*handle))
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
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
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
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
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
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
            .send_open_file_request(id, OpenFileRequest::open(Cow::Borrowed(&filename)))
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
        let (id, handle) = awaitable.wait().await.unwrap();

        // fstat
        let awaitable = write_end
            .send_fstat_request(id, Cow::Borrowed(&handle))
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
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
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
        let id = awaitable.wait().await.unwrap().0;

        // stat
        let awaitable = write_end
            .send_stat_request(id, Cow::Borrowed(&filename))
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
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
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(Cow::Borrowed(&filename)),
            )
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
        let (id, handle) = awaitable.wait().await.unwrap();

        // fsetstat
        let mut fileattrs = FileAttrs::default();
        fileattrs.set_size(10000);

        let awaitable = write_end
            .send_fsetstat_request(id, Cow::Borrowed(&handle), fileattrs)
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
        let id = awaitable.wait().await.unwrap().0;

        // fstat
        let awaitable = write_end
            .send_fstat_request(id, Cow::Borrowed(&handle))
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
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
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
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
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
        let (id, path) = awaitable.wait().await.unwrap();

        assert_eq!(&*path, &*fs::canonicalize(&filename).unwrap());

        drop(id);
        drop(write_end);

        assert_eq!(read_end.wait_for_new_request().await, 0);

        drop(read_end);

        assert!(child.wait().await.unwrap().success());
    }

    #[tokio::test]
    async fn test_symlink() {
        let (mut write_end, mut read_end, mut child) = connect().await;

        let id = write_end.create_response_id();

        let tempdir = create_tmpdir();
        let filename = tempdir.path().join("file");

        fs::File::create(&filename).unwrap().set_len(2000).unwrap();

        let linkname = tempdir.path().join("symlink");

        // symlink
        let awaitable = write_end
            .send_symlink_request(id, Cow::Borrowed(&filename), Cow::Borrowed(&linkname))
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
        let id = awaitable.wait().await.unwrap().0;

        assert_eq!(
            &*fs::canonicalize(&linkname).unwrap(),
            &*fs::canonicalize(&filename).unwrap()
        );

        drop(id);
        drop(write_end);

        assert_eq!(read_end.wait_for_new_request().await, 0);

        drop(read_end);

        assert!(child.wait().await.unwrap().success());
    }

    #[tokio::test]
    async fn test_limits() {
        let (mut write_end, mut read_end, mut child, extensions) = connect_with_extensions().await;

        let id = write_end.create_response_id();

        assert!(extensions.limits);
        let awaitable = write_end.send_limits_request(id).unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
        let (id, limits) = awaitable.wait().await.unwrap();

        eprintln!("{:#?}", limits);

        drop(id);
        drop(write_end);

        assert_eq!(read_end.wait_for_new_request().await, 0);

        drop(read_end);

        assert!(child.wait().await.unwrap().success());
    }

    #[tokio::test]
    async fn test_expand_path() {
        let home: path::PathBuf = env::var("HOME").unwrap().into();

        env::set_current_dir(&home).unwrap();

        let (mut write_end, mut read_end, mut child, extensions) = connect_with_extensions().await;

        let id = write_end.create_response_id();

        assert!(extensions.expand_path);
        let awaitable = write_end
            .send_expand_path_request(id, Cow::Borrowed(path::Path::new("~")))
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
        let (id, expanded_path) = awaitable.wait().await.unwrap();

        assert_eq!(&*expanded_path, &*home);

        drop(id);
        drop(write_end);

        assert_eq!(read_end.wait_for_new_request().await, 0);

        drop(read_end);

        assert!(child.wait().await.unwrap().success());
    }

    #[tokio::test]
    async fn test_fsync() {
        let (mut write_end, mut read_end, mut child, extensions) = connect_with_extensions().await;
        assert!(extensions.fsync);

        let id = write_end.create_response_id();

        let tempdir = create_tmpdir();
        let filename = tempdir.path().join("file");

        fs::File::create(&filename).unwrap().set_len(2000).unwrap();

        // open
        let awaitable = write_end
            .send_open_file_request(
                id,
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(Cow::Borrowed(&filename)),
            )
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
        let (id, handle) = awaitable.wait().await.unwrap();

        // fsync
        let awaitable = write_end
            .send_fsync_request(id, Cow::Borrowed(&handle))
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
        let id = awaitable.wait().await.unwrap().0;

        drop(id);
        drop(write_end);

        assert_eq!(read_end.wait_for_new_request().await, 0);

        drop(read_end);

        assert!(child.wait().await.unwrap().success());
    }

    #[tokio::test]
    async fn test_hardlink() {
        let (mut write_end, mut read_end, mut child, extensions) = connect_with_extensions().await;
        assert!(extensions.hardlink);

        let id = write_end.create_response_id();

        let tempdir = create_tmpdir();
        let filename = tempdir.path().join("file");
        fs::File::create(&filename).unwrap().set_len(2000).unwrap();

        // hardlink it
        let new_filename = tempdir.path().join("file2");

        let awaitable = write_end
            .send_hardlink_requst(id, Cow::Borrowed(&filename), Cow::Borrowed(&new_filename))
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
        let id = awaitable.wait().await.unwrap().0;

        // Open the new name and old name again
        for name in [filename, new_filename] {
            let metadata = fs::File::open(name).unwrap().metadata().unwrap();

            assert!(metadata.is_file());
            assert_eq!(metadata.len(), 2000);
        }

        drop(id);
        drop(write_end);

        assert_eq!(read_end.wait_for_new_request().await, 0);

        drop(read_end);

        assert!(child.wait().await.unwrap().success());
    }

    #[tokio::test]
    async fn test_posix_rename() {
        let (mut write_end, mut read_end, mut child, extensions) = connect_with_extensions().await;
        assert!(extensions.posix_rename);

        let id = write_end.create_response_id();

        let tempdir = create_tmpdir();
        let filename = tempdir.path().join("file");
        fs::File::create(&filename).unwrap().set_len(2000).unwrap();

        // Posix rename it
        let new_filename = tempdir.path().join("file2");

        let awaitable = write_end
            .send_posix_rename_request(id, Cow::Borrowed(&filename), Cow::Borrowed(&new_filename))
            .unwrap();

        read_one_packet(&mut write_end, &mut read_end).await;
        let id = awaitable.wait().await.unwrap().0;

        // Open again
        let metadata = fs::File::open(new_filename).unwrap().metadata().unwrap();

        assert!(metadata.is_file());
        assert_eq!(metadata.len(), 2000);

        drop(id);
        drop(write_end);

        assert_eq!(read_end.wait_for_new_request().await, 0);

        drop(read_end);

        assert!(child.wait().await.unwrap().success());
    }
}
