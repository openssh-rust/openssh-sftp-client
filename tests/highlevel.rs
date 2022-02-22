mod common;
use common::*;

use openssh_sftp_client::highlevel::*;

use std::cmp::{max, min};
use std::convert::TryInto;
use std::env;
use std::io::IoSlice;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use bytes::BytesMut;
use once_cell::sync::OnceCell;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::process;
use tokio_io_utility::write_vectored_all;

use pretty_assertions::assert_eq;

async fn connect(options: SftpOptions) -> (process::Child, Sftp) {
    let (child, stdin, stdout) = launch_sftp().await;

    (child, Sftp::new(stdin, stdout, options).await.unwrap())
}

fn gen_path(func: &str) -> PathBuf {
    static XDG_RUNTIME_DIR: OnceCell<Option<Box<Path>>> = OnceCell::new();

    XDG_RUNTIME_DIR
        .get_or_init(|| {
            env::var_os("XDG_RUNTIME_DIR")
                .map(From::from)
                .map(PathBuf::into_boxed_path)
        })
        .as_deref()
        .unwrap_or_else(|| Path::new("/tmp"))
        .join(func)
}

#[tokio::test]
async fn sftp_init() {
    let (mut child, sftp) = connect(Default::default()).await;

    sftp.close().await.unwrap();
    assert!(child.wait().await.unwrap().success());
}

#[tokio::test]
/// Test creating new file, truncating and opening existing file,
/// basic read, write and removal.
async fn sftp_file_basics() {
    let path = gen_path("sftp_file_basics");

    let content = b"HELLO, WORLD!\n".repeat(200);

    let (mut child, sftp) = connect(Default::default()).await;

    let content = &content[..min(sftp.max_write_len() as usize, content.len())];

    {
        let mut fs = sftp.fs("");

        // Create new file with Excl and write to it.
        debug_assert_eq!(
            sftp.options()
                .write(true)
                .create_new(true)
                .open(&path)
                .await
                .unwrap()
                .write(content)
                .await
                .unwrap(),
            content.len()
        );

        debug_assert_eq!(&*fs.read(&path).await.unwrap(), &*content);

        // Create new file with Trunc and write to it.
        //
        // Sftp::Create opens the file truncated.
        debug_assert_eq!(
            sftp.create(&path)
                .await
                .unwrap()
                .write(content)
                .await
                .unwrap(),
            content.len()
        );

        debug_assert_eq!(&*fs.read(&path).await.unwrap(), &*content);

        // remove the file
        fs.remove_file(path).await.unwrap();
    }

    // close sftp and child
    sftp.close().await.unwrap();
    assert!(child.wait().await.unwrap().success());
}

struct SftpFileWriteAllTester<'s> {
    path: &'s Path,
    file: File<'s>,
    fs: Fs<'s>,
    content: Vec<u8>,
}

impl<'s> SftpFileWriteAllTester<'s> {
    async fn new(sftp: &'s Sftp, path: &'s Path) -> SftpFileWriteAllTester<'s> {
        let max_len = max(sftp.max_write_len(), sftp.max_read_len()) as usize;
        let content = b"HELLO, WORLD!\n".repeat(max_len / 8);

        let file = sftp
            .options()
            .write(true)
            .read(true)
            .create(true)
            .open(path)
            .await
            .unwrap();
        let fs = sftp.fs("");

        Self {
            path,
            file,
            fs,
            content,
        }
    }

    async fn assert_content(mut self) {
        let len = self.content.len();

        self.file.rewind().await.unwrap();

        let buffer = self
            .file
            .read_all(len, BytesMut::with_capacity(len))
            .await
            .unwrap();

        assert_eq!(&*buffer, &*self.content);

        // remove the file
        self.fs.remove_file(self.path).await.unwrap();
    }
}

#[tokio::test]
/// Test File::write_all, File::read_all and AsyncSeek implementation
async fn sftp_file_write_all() {
    let path = gen_path("sftp_file_write_all");

    for (msg, (mut child, sftp)) in vec![
        (
            "Test direct write",
            connect(SftpOptions::new().max_buffered_write(NonZeroU32::new(1).unwrap())).await,
        ),
        (
            "Test buffered write",
            connect(SftpOptions::new().max_buffered_write(NonZeroU32::new(u32::MAX).unwrap()))
                .await,
        ),
    ] {
        let mut tester = SftpFileWriteAllTester::new(&sftp, &path).await;

        eprintln!("{}", msg);
        tester.file.write_all(&tester.content).await.unwrap();

        eprintln!("Verifing the write");
        tester.assert_content().await;

        eprintln!("Closing sftp and child");
        sftp.close().await.unwrap();
        // TODO: somehow sftp-server hangs here
        child.kill().await.unwrap();
    }
}

#[tokio::test]
/// Test File::write_all_vectorized, File::read_all and AsyncSeek implementation
async fn sftp_file_write_all_vectored() {
    let path = gen_path("sftp_file_write_all_vectored");
    let max_rw_len = NonZeroU32::new(200).unwrap();

    let sftp_options = SftpOptions::new()
        .max_write_len(max_rw_len)
        .max_read_len(max_rw_len);

    for (msg, (mut child, sftp)) in vec![
        (
            "Test direct write",
            connect(sftp_options.max_buffered_write(NonZeroU32::new(1).unwrap())).await,
        ),
        (
            "Test buffered write",
            connect(sftp_options.max_buffered_write(NonZeroU32::new(u32::MAX).unwrap())).await,
        ),
    ] {
        let mut tester = SftpFileWriteAllTester::new(&sftp, &path).await;

        let content = &tester.content;
        let len = content.len();

        eprintln!("{}", msg);

        tester
            .file
            .write_all_vectorized(
                [
                    IoSlice::new(&content[..len / 2]),
                    IoSlice::new(&content[len / 2..]),
                ]
                .as_mut_slice(),
            )
            .await
            .unwrap();

        eprintln!("Verifing the write");
        tester.assert_content().await;

        eprintln!("Closing sftp and child");
        sftp.close().await.unwrap();
        // TODO: somehow sftp-server hangs here
        child.kill().await.unwrap();
    }
}

#[tokio::test]
/// Test File::write_all_vectorized, File::read_all and AsyncSeek implementation
async fn sftp_file_write_all_zero_copy() {
    let path = gen_path("sftp_file_write_all_zero_copy");
    let max_rw_len = NonZeroU32::new(200).unwrap();

    {
        let (mut child, sftp) = connect(
            SftpOptions::new()
                .max_write_len(max_rw_len)
                .max_read_len(max_rw_len),
        )
        .await;

        let mut tester = SftpFileWriteAllTester::new(&sftp, &path).await;

        let content = &tester.content;
        let len = content.len();

        tester
            .file
            .write_all_zero_copy(
                [
                    BytesMut::from(&content[..len / 2]).freeze(),
                    BytesMut::from(&content[len / 2..]).freeze(),
                ]
                .as_mut_slice(),
            )
            .await
            .unwrap();

        eprintln!("Verifing the write");
        tester.assert_content().await;

        // close sftp and child
        sftp.close().await.unwrap();
        assert!(child.wait().await.unwrap().success());
    }
}

#[tokio::test]
/// Test creating new TokioCompactFile, truncating and opening existing file,
/// basic read, write and removal.
async fn sftp_tokio_compact_file_basics() {
    let path = gen_path("sftp_tokio_compact_file_basics");
    let content = b"HELLO, WORLD!\n".repeat(200);

    let (mut child, sftp) = connect(Default::default()).await;

    let content = &content[..min(sftp.max_write_len() as usize, content.len())];

    let read_entire_file = || async {
        let mut buffer = Vec::with_capacity(content.len());

        let mut file: TokioCompactFile = sftp.open(&path).await.unwrap().into();
        file.read_to_end(&mut buffer).await.unwrap();
        file.close().await.unwrap();

        buffer
    };

    {
        let mut fs = sftp.fs("");

        let mut file = sftp
            .options()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
            .map(TokioCompactFile::new)
            .unwrap();

        // Create new file with Excl and write to it.
        debug_assert_eq!(file.write(content).await.unwrap(), content.len());

        file.flush().await.unwrap();
        file.close().await.unwrap();

        debug_assert_eq!(&*read_entire_file().await, &*content);

        // Create new file with Trunc and write to it.
        //
        // Sftp::Create opens the file truncated.
        let mut file = sftp.create(&path).await.map(TokioCompactFile::new).unwrap();
        debug_assert_eq!(file.write(content).await.unwrap(), content.len());

        // close also flush the internal future buffers, but using a
        // different implementation from `TokioCompactFile::poll_flush`
        // since it is executed in async context.
        //
        // Call `close` without calling `flush` first would force
        // `close` to do all the flush work, thus testing its implementation
        file.close().await.unwrap();

        debug_assert_eq!(&*read_entire_file().await, &*content);

        // remove the file
        fs.remove_file(&path).await.unwrap();
    }

    // close sftp and child
    sftp.close().await.unwrap();
    assert!(child.wait().await.unwrap().success());
}

struct SftpTokioCompactFileWriteAllTester<'s> {
    path: &'s Path,
    file: TokioCompactFile<'s>,
    fs: Fs<'s>,
    content: Vec<u8>,
}

impl<'s> SftpTokioCompactFileWriteAllTester<'s> {
    async fn new(sftp: &'s Sftp, path: &'s Path) -> SftpTokioCompactFileWriteAllTester<'s> {
        let max_len = max(sftp.max_write_len(), sftp.max_read_len()) as usize;
        let content = b"HELLO, WORLD!\n".repeat(max_len / 8);

        let file = sftp
            .options()
            .write(true)
            .read(true)
            .create(true)
            .open(path)
            .await
            .map(TokioCompactFile::new)
            .unwrap();
        let fs = sftp.fs("");

        Self {
            path,
            file,
            fs,
            content,
        }
    }

    async fn assert_content(mut self) {
        let len = self.content.len();

        self.file.rewind().await.unwrap();

        let buffer = self
            .file
            .read_all(len, BytesMut::with_capacity(len))
            .await
            .unwrap();

        assert_eq!(&*buffer, &*self.content);

        // remove the file
        self.fs.remove_file(self.path).await.unwrap();
    }
}

#[tokio::test]
/// Test File::write_all_vectorized, File::read_all and AsyncSeek implementation
async fn sftp_tokio_compact_file_write_all() {
    let path = gen_path("sftp_tokio_compact_file_write_all");
    let max_rw_len = NonZeroU32::new(200).unwrap();

    {
        let (mut child, sftp) = connect(
            SftpOptions::new()
                .max_write_len(max_rw_len)
                .max_read_len(max_rw_len),
        )
        .await;

        let mut tester = SftpTokioCompactFileWriteAllTester::new(&sftp, &path).await;

        let content = &tester.content;

        tester.file.write_all(content).await.unwrap();
        tester.assert_content().await;

        // close sftp and child
        sftp.close().await.unwrap();
        assert!(child.wait().await.unwrap().success());
    }
}

#[tokio::test]
/// Test File::write_all_vectorized, File::read_all and AsyncSeek implementation
async fn sftp_tokio_compact_file_write_vectored_all() {
    let path = gen_path("sftp_tokio_compact_file_write_vectored_all");
    let max_rw_len = NonZeroU32::new(200).unwrap();

    {
        let (mut child, sftp) = connect(
            SftpOptions::new()
                .max_write_len(max_rw_len)
                .max_read_len(max_rw_len),
        )
        .await;

        let mut tester = SftpTokioCompactFileWriteAllTester::new(&sftp, &path).await;

        let content = &tester.content;
        let len = content.len();

        write_vectored_all(
            &mut tester.file,
            [
                IoSlice::new(&content[..len / 2]),
                IoSlice::new(&content[len / 2..]),
            ]
            .as_mut_slice(),
        )
        .await
        .unwrap();
        tester.assert_content().await;

        // close sftp and child
        sftp.close().await.unwrap();
        assert!(child.wait().await.unwrap().success());
    }
}

#[tokio::test]
/// Test File::{set_len, set_permissions, metadata}.
async fn sftp_file_metadata() {
    let path = gen_path("sftp_file_metadata");
    let max_rw_len = NonZeroU32::new(200).unwrap();

    let (mut child, sftp) = connect(
        SftpOptions::new()
            .max_write_len(max_rw_len)
            .max_read_len(max_rw_len),
    )
    .await;

    {
        let mut file = sftp
            .options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .await
            .unwrap();
        assert_eq!(file.metadata().await.unwrap().len().unwrap(), 0);

        file.set_len(28802).await.unwrap();
        assert_eq!(file.metadata().await.unwrap().len().unwrap(), 28802);
    }

    // close sftp and child
    sftp.close().await.unwrap();
    assert!(child.wait().await.unwrap().success());
}

#[tokio::test]
/// Test File::sync_all.
async fn sftp_file_sync_all() {
    let path = gen_path("sftp_file_sync_all");
    let max_rw_len = NonZeroU32::new(200).unwrap();

    let (mut child, sftp) = connect(
        SftpOptions::new()
            .max_write_len(max_rw_len)
            .max_read_len(max_rw_len),
    )
    .await;

    sftp.create(path).await.unwrap().sync_all().await.unwrap();

    // close sftp and child
    sftp.close().await.unwrap();
    assert!(child.wait().await.unwrap().success());
}

#[tokio::test]
/// Test creating, removing and iterating over dir, as well
/// as removing file.
async fn sftp_dir_basics() {
    let path = gen_path("sftp_dir_basics");

    let (mut child, sftp) = connect(Default::default()).await;

    {
        let mut fs = sftp.fs("");

        fs.create_dir(&path).await.unwrap();

        fs.create_dir(&path.join("dir")).await.unwrap();
        sftp.create(&path.join("file")).await.unwrap();

        for entry in fs.open_dir(&path).await.unwrap().read_dir().await.unwrap() {
            let filename = entry.filename().as_os_str();

            if filename == "." || filename == ".." {
                continue;
            } else if filename == "dir" {
                assert!(entry.file_type().unwrap().is_dir());
            } else if filename == "file" {
                assert!(entry.file_type().unwrap().is_file());
            } else {
                unreachable!("Unreachable!");
            }
        }

        fs.remove_file(&path.join("file")).await.unwrap();
        fs.remove_dir(&path.join("dir")).await.unwrap();
        fs.remove_dir(&path).await.unwrap();
    }

    // close sftp and child
    sftp.close().await.unwrap();
    assert!(child.wait().await.unwrap().success());
}

#[tokio::test]
/// Test creation of symlink and canonicalize/read_link
async fn sftp_fs_symlink() {
    let filename = gen_path("sftp_fs_symlink_file");
    let symlink = gen_path("sftp_fs_symlink_symlink");

    let content = b"hello, world!\n";

    let (mut child, sftp) = connect(Default::default()).await;

    {
        let mut fs = sftp.fs("");

        fs.write(&filename, content).await.unwrap();
        fs.symlink(&filename, &symlink).await.unwrap();

        assert_eq!(&*fs.read(&symlink).await.unwrap(), content);

        assert_eq!(fs.canonicalize(&filename).await.unwrap(), filename);
        assert_eq!(fs.canonicalize(&symlink).await.unwrap(), filename);
        assert_eq!(fs.read_link(&symlink).await.unwrap(), filename);

        fs.remove_file(&symlink).await.unwrap();
    }

    // close sftp and child
    sftp.close().await.unwrap();
    assert!(child.wait().await.unwrap().success());
}

#[tokio::test]
/// Test creation of hard_link and canonicalize
async fn sftp_fs_hardlink() {
    let filename = gen_path("sftp_fs_hard_link_file");
    let hardlink = gen_path("sftp_fs_hard_link_hardlink");

    let content = b"hello, world!\n";

    let (mut child, sftp) = connect(Default::default()).await;

    {
        let mut fs = sftp.fs("");

        fs.write(&filename, content).await.unwrap();
        fs.hard_link(&filename, &hardlink).await.unwrap();

        assert_eq!(&*fs.read(&hardlink).await.unwrap(), content);

        assert_eq!(fs.canonicalize(&filename).await.unwrap(), filename);
        assert_eq!(fs.canonicalize(&hardlink).await.unwrap(), hardlink);

        fs.remove_file(&hardlink).await.unwrap();
    }

    // close sftp and child
    sftp.close().await.unwrap();
    assert!(child.wait().await.unwrap().success());
}

#[tokio::test]
/// Test creation of rename and canonicalize
async fn sftp_fs_rename() {
    let filename = gen_path("sftp_fs_rename_file");
    let renamed = gen_path("sftp_fs_rename_renamed");

    let content = b"hello, world!\n";

    let (mut child, sftp) = connect(Default::default()).await;

    {
        let mut fs = sftp.fs("");

        fs.write(&filename, content).await.unwrap();
        fs.rename(&filename, &renamed).await.unwrap();

        fs.read(&filename).await.unwrap_err();

        assert_eq!(&*fs.read(&renamed).await.unwrap(), content);
        assert_eq!(fs.canonicalize(&renamed).await.unwrap(), renamed);

        fs.remove_file(&renamed).await.unwrap();
    }

    // close sftp and child
    sftp.close().await.unwrap();
    assert!(child.wait().await.unwrap().success());
}

#[tokio::test]
/// Test Fs::{metadata, set_metadata}.
async fn sftp_fs_metadata() {
    let path = gen_path("sftp_fs_metadata");
    let max_rw_len = NonZeroU32::new(200).unwrap();
    let content = b"hello, world!\n";

    let (mut child, sftp) = connect(
        SftpOptions::new()
            .max_write_len(max_rw_len)
            .max_read_len(max_rw_len),
    )
    .await;

    {
        let mut fs = sftp.fs("");

        fs.write(&path, content).await.unwrap();
        assert_eq!(
            fs.metadata(&path).await.unwrap().len().unwrap(),
            content.len().try_into().unwrap()
        );

        fs.set_metadata(&path, MetaDataBuilder::new().len(2834).create())
            .await
            .unwrap();
        assert_eq!(fs.metadata(&path).await.unwrap().len().unwrap(), 2834);
    }

    // close sftp and child
    sftp.close().await.unwrap();
    assert!(child.wait().await.unwrap().success());
}
