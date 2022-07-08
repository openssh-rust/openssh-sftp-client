use lowlevel::*;
use openssh_sftp_client_lowlevel as lowlevel;

use std::borrow::Cow;
use std::env;
use std::fs;
use std::io;
use std::io::IoSlice;
use std::num::NonZeroUsize;
use std::os::unix::fs::symlink;
use std::path;

use sftp_test_common::*;

use bytes::Bytes;
use tempfile::{Builder, TempDir};
use tokio::sync::Mutex;
use tokio_io_utility::write_vectored_all;

type Id = lowlevel::Id<Vec<u8>>;
type WriteEnd = lowlevel::WriteEnd<Vec<u8>, Mutex<PipeWrite>>;
type ReadEnd = lowlevel::ReadEnd<PipeRead, Vec<u8>, Mutex<PipeWrite>>;

fn assert_not_found(err: io::Error) {
    assert!(matches!(err.kind(), io::ErrorKind::NotFound), "{:#?}", err);
}

async fn flush(read_end: &mut ReadEnd) {
    let shared_data = read_end.get_shared_data();
    let stdin = shared_data.get_auxiliary();
    let mut buffers = shared_data.get_buffers().await;

    let mut io_slices: Vec<IoSlice<'_>> = buffers.get_io_slices().to_vec();
    let n: usize = io_slices.iter().map(|slice| slice.len()).sum();

    write_vectored_all(&mut *stdin.lock().await, &mut io_slices)
        .await
        .unwrap();

    assert!(!buffers.advance(NonZeroUsize::new(n).unwrap()));
}

async fn connect_with_extensions() -> (WriteEnd, ReadEnd, process::Child, Extensions) {
    let (child, stdin, stdout) = launch_sftp().await;

    let buffer_size = NonZeroUsize::new(1000).unwrap();

    let (write_end, mut read_end) =
        lowlevel::connect(stdout, buffer_size, buffer_size, Mutex::new(stdin))
            .await
            .unwrap();
    flush(&mut read_end).await;
    let extensions = read_end.receive_server_hello().await.unwrap();
    (write_end, read_end, child, extensions)
}

async fn connect() -> (WriteEnd, ReadEnd, process::Child) {
    let (write_end, read_end, child, _extensions) = connect_with_extensions().await;
    (write_end, read_end, child)
}

#[tokio::test]
async fn test_connect() {
    let mut child = connect().await.2;
    assert!(child.wait().await.unwrap().success());
}

fn create_tmpdir() -> TempDir {
    let path = get_path_for_tmp_files();

    fs::create_dir_all(&path).unwrap();

    Builder::new()
        .prefix("lowlevel-test")
        .tempdir_in(path)
        .unwrap()
}

async fn read_one_packet(read_end: &mut ReadEnd) {
    flush(read_end).await;

    eprintln!("Wait for new request");

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

    read_one_packet(&mut read_end).await;
    let (id, handle) = awaitable.wait().await.unwrap();

    eprintln!("handle = {:#?}", handle);

    // Write msg into it
    let msg = "Hello, world!".as_bytes();

    let awaitable = write_end
        .send_write_request_buffered(id, Cow::Borrowed(&handle), 0, Cow::Borrowed(msg))
        .unwrap();

    eprintln!("Waiting for write response");

    read_one_packet(&mut read_end).await;
    let id = awaitable.wait().await.unwrap().0;

    // Close it
    let awaitable = write_end
        .send_close_request(id, Cow::Borrowed(&handle))
        .unwrap();

    read_one_packet(&mut read_end).await;
    let id = awaitable.wait().await.unwrap().0;

    // Open it again
    let awaitable = write_end
        .send_open_file_request(id, OpenFileRequest::open(Cow::Borrowed(&filename)))
        .unwrap();

    read_one_packet(&mut read_end).await;
    let (id, handle) = awaitable.wait().await.unwrap();

    eprintln!("handle = {:#?}", handle);

    // Read from it
    let awaitable = write_end
        .send_read_request(id, Cow::Borrowed(&handle), 0, msg.len() as u32, None)
        .unwrap();

    read_one_packet(&mut read_end).await;
    let (id, data) = awaitable.wait().await.unwrap();

    match data {
        Data::AllocatedBox(data) => assert_eq!(&*data, msg),
        _ => panic!("Unexpected data"),
    };

    drop(id);
    drop(write_end);
    drop(read_end);

    assert!(child.wait().await.unwrap().success());
}

async fn test_write_impl(
    write: impl FnOnce(&mut WriteEnd, Id, Cow<'_, Handle>, &[u8]) -> AwaitableStatus<Vec<u8>>,
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

    read_one_packet(&mut read_end).await;
    let (id, handle) = awaitable.wait().await.unwrap();

    eprintln!("handle = {:#?}", handle);

    let msg = "Hello, world!".as_bytes();

    let awaitable = write(&mut write_end, id, Cow::Borrowed(&handle), msg);

    eprintln!("Waiting for write response");

    read_one_packet(&mut read_end).await;
    let id = awaitable.wait().await.unwrap().0;

    // Read from it
    let awaitable = write_end
        .send_read_request(id, Cow::Borrowed(&handle), 0, msg.len() as u32, None)
        .unwrap();

    read_one_packet(&mut read_end).await;
    let (id, data) = awaitable.wait().await.unwrap();

    match data {
        Data::AllocatedBox(data) => assert_eq!(&*data, msg),
        _ => panic!("Unexpected data"),
    };

    drop(id);
    drop(write_end);
    drop(read_end);

    assert!(child.wait().await.unwrap().success());
}

#[tokio::test]
async fn test_write_buffered() {
    test_write_impl(|write_end, id, handle, msg| {
        write_end
            .send_write_request_buffered(id, handle, 0, Cow::Borrowed(msg))
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
                handle,
                0,
                &[IoSlice::new(&msg[..3]), IoSlice::new(&msg[3..])],
            )
            .unwrap()
    })
    .await;
}

#[tokio::test]
async fn test_write_buffered_vectored2() {
    test_write_impl(|write_end, id, handle, msg| {
        write_end
            .send_write_request_buffered_vectored2(
                id,
                handle,
                0,
                &[
                    [IoSlice::new(&msg[..3])].as_slice(),
                    [IoSlice::new(&msg[3..])].as_slice(),
                ],
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
                handle,
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
async fn test_write_zero_copy2() {
    test_write_impl(|write_end, id, handle, msg| {
        write_end
            .send_write_request_zero_copy2(
                id,
                handle,
                0,
                &[
                    [Bytes::copy_from_slice(&msg[..3])].as_slice(),
                    [Bytes::copy_from_slice(&msg[3..])].as_slice(),
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

    read_one_packet(&mut read_end).await;
    let id = awaitable.wait().await.unwrap().0;

    // Try open it again
    let err = fs::File::open(&filename).unwrap_err();

    assert_not_found(err);

    drop(id);
    drop(write_end);
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

    read_one_packet(&mut read_end).await;
    let id = awaitable.wait().await.unwrap().0;

    // Open it again
    let metadata = fs::File::open(&new_filename).unwrap().metadata().unwrap();

    assert!(metadata.is_file());
    assert_eq!(metadata.len(), 2000);

    drop(id);
    drop(write_end);
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

    read_one_packet(&mut read_end).await;
    let id = awaitable.wait().await.unwrap().0;

    // Open it
    assert!(fs::read_dir(&dirname).unwrap().next().is_none());

    drop(id);
    drop(write_end);
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

    read_one_packet(&mut read_end).await;
    let id = awaitable.wait().await.unwrap().0;

    // Try open it
    let err = fs::read_dir(&dirname).unwrap_err();
    assert_not_found(err);

    drop(id);
    drop(write_end);
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

    read_one_packet(&mut read_end).await;
    let (id, handle) = awaitable.wait().await.unwrap();

    // read it
    let awaitable = write_end
        .send_readdir_request(id, Cow::Borrowed(&*handle))
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

    read_one_packet(&mut read_end).await;
    let (id, attrs) = awaitable.wait().await.unwrap();

    assert_eq!(attrs.get_size().unwrap(), 2000);
    assert_eq!(attrs.get_filetype().unwrap(), FileType::RegularFile);

    drop(id);
    drop(write_end);
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

    read_one_packet(&mut read_end).await;
    let (id, attrs) = awaitable.wait().await.unwrap();

    assert_eq!(attrs.get_filetype().unwrap(), FileType::Symlink);

    drop(id);
    drop(write_end);
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

    read_one_packet(&mut read_end).await;
    let (id, handle) = awaitable.wait().await.unwrap();

    // fstat
    let awaitable = write_end
        .send_fstat_request(id, Cow::Borrowed(&handle))
        .unwrap();

    read_one_packet(&mut read_end).await;
    let (id, attrs) = awaitable.wait().await.unwrap();

    assert_eq!(attrs.get_size().unwrap(), 2000);
    assert_eq!(attrs.get_filetype().unwrap(), FileType::RegularFile);

    drop(id);
    drop(write_end);
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

    read_one_packet(&mut read_end).await;
    let id = awaitable.wait().await.unwrap().0;

    // stat
    let awaitable = write_end
        .send_stat_request(id, Cow::Borrowed(&filename))
        .unwrap();

    read_one_packet(&mut read_end).await;
    let (id, attrs) = awaitable.wait().await.unwrap();

    assert_eq!(attrs.get_size().unwrap(), 10000);
    assert_eq!(attrs.get_filetype().unwrap(), FileType::RegularFile);

    drop(id);
    drop(write_end);
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

    read_one_packet(&mut read_end).await;
    let (id, handle) = awaitable.wait().await.unwrap();

    // fsetstat
    let mut fileattrs = FileAttrs::default();
    fileattrs.set_size(10000);

    let awaitable = write_end
        .send_fsetstat_request(id, Cow::Borrowed(&handle), fileattrs)
        .unwrap();

    read_one_packet(&mut read_end).await;
    let id = awaitable.wait().await.unwrap().0;

    // fstat
    let awaitable = write_end
        .send_fstat_request(id, Cow::Borrowed(&handle))
        .unwrap();

    read_one_packet(&mut read_end).await;
    let (id, attrs) = awaitable.wait().await.unwrap();

    assert_eq!(attrs.get_size().unwrap(), 10000);
    assert_eq!(attrs.get_filetype().unwrap(), FileType::RegularFile);

    drop(id);
    drop(write_end);
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

    read_one_packet(&mut read_end).await;
    let (id, path) = awaitable.wait().await.unwrap();

    assert_eq!(&*path, &*filename);

    drop(id);
    drop(write_end);
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

    read_one_packet(&mut read_end).await;
    let (id, path) = awaitable.wait().await.unwrap();

    assert_eq!(&*path, &*fs::canonicalize(&filename).unwrap());

    drop(id);
    drop(write_end);
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

    read_one_packet(&mut read_end).await;
    let id = awaitable.wait().await.unwrap().0;

    assert_eq!(
        &*fs::canonicalize(&linkname).unwrap(),
        &*fs::canonicalize(&filename).unwrap()
    );

    drop(id);
    drop(write_end);
    drop(read_end);

    assert!(child.wait().await.unwrap().success());
}

#[tokio::test]
async fn test_limits() {
    let (mut write_end, mut read_end, mut child, extensions) = connect_with_extensions().await;

    let id = write_end.create_response_id();

    assert!(extensions.limits);
    let awaitable = write_end.send_limits_request(id).unwrap();

    read_one_packet(&mut read_end).await;
    let (id, limits) = awaitable.wait().await.unwrap();

    eprintln!("{:#?}", limits);

    drop(id);
    drop(write_end);
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

    read_one_packet(&mut read_end).await;
    let (id, expanded_path) = awaitable.wait().await.unwrap();

    assert_eq!(&*expanded_path, &*home);

    drop(id);
    drop(write_end);
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

    read_one_packet(&mut read_end).await;
    let (id, handle) = awaitable.wait().await.unwrap();

    // fsync
    let awaitable = write_end
        .send_fsync_request(id, Cow::Borrowed(&handle))
        .unwrap();

    read_one_packet(&mut read_end).await;
    let id = awaitable.wait().await.unwrap().0;

    drop(id);
    drop(write_end);
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
        .send_hardlink_request(id, Cow::Borrowed(&filename), Cow::Borrowed(&new_filename))
        .unwrap();

    read_one_packet(&mut read_end).await;
    let id = awaitable.wait().await.unwrap().0;

    // Open the new name and old name again
    for name in [filename, new_filename] {
        let metadata = fs::File::open(name).unwrap().metadata().unwrap();

        assert!(metadata.is_file());
        assert_eq!(metadata.len(), 2000);
    }

    drop(id);
    drop(write_end);
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

    read_one_packet(&mut read_end).await;
    let id = awaitable.wait().await.unwrap().0;

    // Open again
    let metadata = fs::File::open(new_filename).unwrap().metadata().unwrap();

    assert!(metadata.is_file());
    assert_eq!(metadata.len(), 2000);

    drop(id);
    drop(write_end);
    drop(read_end);

    assert!(child.wait().await.unwrap().success());
}

#[tokio::test]
async fn test_ext_copy_data() {
    let content = "Hello, World!\n";

    let (mut write_end, mut read_end, mut child, extensions) = connect_with_extensions().await;
    assert!(extensions.copy_data);

    let id = write_end.create_response_id();

    let tempdir = create_tmpdir();
    let filename = tempdir.path().join("test_ext_copy_data");
    fs::write(&filename, content).unwrap();

    let awaitable = write_end
        .send_open_file_request(
            id,
            OpenOptions::new().read(true).open(Cow::Borrowed(&filename)),
        )
        .unwrap();

    read_one_packet(&mut read_end).await;
    let (id, handle) = awaitable.wait().await.unwrap();

    // copy it
    let new_filename = tempdir.path().join("test_ext_copy_data_file2");

    let awaitable = write_end
        .send_open_file_request(
            id,
            OpenOptions::new().read(true).write(true).create(
                Cow::Borrowed(&new_filename),
                CreateFlags::None,
                FileAttrs::default(),
            ),
        )
        .unwrap();

    read_one_packet(&mut read_end).await;
    let (id, new_handle) = awaitable.wait().await.unwrap();

    let awaitable = write_end
        .send_copy_data_request(
            id,
            Cow::Borrowed(&handle),
            0,
            0,
            Cow::Borrowed(&new_handle),
            0,
        )
        .unwrap();

    read_one_packet(&mut read_end).await;
    let id = awaitable.wait().await.unwrap().0;

    // Open the new name and old name again
    assert_eq!(fs::read_to_string(&new_filename).unwrap(), content);

    drop(id);
    drop(write_end);
    drop(read_end);

    assert!(child.wait().await.unwrap().success());
}
