use openssh_sftp_client::*;

use std::cmp::{max, min};
use std::convert::identity;
use std::convert::TryInto;
use std::fs;
use std::io::IoSlice;
use std::num::{NonZeroU32, NonZeroU64};
use std::path::PathBuf;
use std::stringify;

use sftp_test_common::*;

use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_io_utility::write_vectored_all;

use pretty_assertions::assert_eq;

async fn connect(options: SftpOptions) -> (process::Child, Sftp) {
    let (child, stdin, stdout) = launch_sftp().await;

    (child, Sftp::new(stdin, stdout, options).await.unwrap())
}

fn gen_path(func: &str) -> PathBuf {
    let mut path = get_path_for_tmp_files().join("highlevel");
    fs::create_dir_all(&path).unwrap();

    path.push(func);

    fs::remove_dir_all(&path).ok();
    fs::remove_file(&path).ok();

    path
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
        let mut fs = sftp.fs();

        // Create new file (fail if already exists) and write to it.
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

/// Return `SftpOptions` that has `max_rw_len` set to `200`.
fn sftp_options_with_max_rw_len() -> SftpOptions {
    let max_rw_len = NonZeroU32::new(200).unwrap();

    SftpOptions::new()
        .max_write_len(max_rw_len)
        .max_read_len(max_rw_len)
}

macro_rules! def_write_all_test {
    ($fname:ident, $sftp_options:expr, $file_converter:expr, $file_var:ident, $content_var:ident , $test_block:block) => {
        #[tokio::test]
        async fn $fname() {
            let path = gen_path(stringify!($fname));

            for (msg, (mut child, sftp)) in
                vec![("Test direct write", connect($sftp_options).await)]
            {
                let max_len = max(sftp.max_write_len(), sftp.max_read_len()) as usize;
                let content = b"HELLO, WORLD!\n".repeat(max_len / 8);

                let mut file = sftp
                    .options()
                    .write(true)
                    .read(true)
                    .create(true)
                    .open(&path)
                    .await
                    .map($file_converter)
                    .unwrap();

                let len = content.len();

                eprintln!("{}", msg);

                {
                    let $file_var = &mut file;
                    let $content_var = &content;

                    $test_block;
                }

                eprintln!("Verifing the write");

                file.rewind().await.unwrap();

                let buffer = file
                    .read_all(len, BytesMut::with_capacity(len))
                    .await
                    .unwrap();

                assert_eq!(&*buffer, &content);

                // remove the file
                drop(file);
                sftp.fs().remove_file(&path).await.unwrap();

                eprintln!("Closing sftp and child");
                sftp.close().await.unwrap();
                // TODO: somehow sftp-server hangs here
                child.kill().await.unwrap();
            }
        }
    };
}

def_write_all_test!(
    sftp_file_write_all,
    SftpOptions::new(),
    identity,
    file,
    content,
    {
        file.write_all(content).await.unwrap();
    }
);

def_write_all_test!(
    sftp_file_write_all_vectored,
    sftp_options_with_max_rw_len(),
    identity,
    file,
    content,
    {
        let len = content.len();

        file.write_all_vectorized(
            [
                IoSlice::new(&content[..len / 2]),
                IoSlice::new(&content[len / 2..]),
            ]
            .as_mut_slice(),
        )
        .await
        .unwrap();
    }
);

def_write_all_test!(
    sftp_file_write_all_zero_copy,
    sftp_options_with_max_rw_len(),
    identity,
    file,
    content,
    {
        let len = content.len();

        file.write_all_zero_copy(
            [
                BytesMut::from(&content[..len / 2]).freeze(),
                BytesMut::from(&content[len / 2..]).freeze(),
            ]
            .as_mut_slice(),
        )
        .await
        .unwrap();
    }
);

#[tokio::test]
/// Test creating new TokioCompatFile, truncating and opening existing file,
/// basic read, write and removal.
async fn sftp_tokio_compact_file_basics() {
    let path = gen_path("sftp_tokio_compact_file_basics");
    let content = b"HELLO, WORLD!\n".repeat(200);

    let (mut child, sftp) = connect(Default::default()).await;

    let content = &content[..min(sftp.max_write_len() as usize, content.len())];

    let read_entire_file = || async {
        let mut buffer = Vec::with_capacity(content.len());

        let mut file: file::TokioCompatFile = sftp.open(&path).await.unwrap().into();
        file.read_to_end(&mut buffer).await.unwrap();
        file.close().await.unwrap();

        buffer
    };

    {
        let mut fs = sftp.fs();

        let mut file = sftp
            .options()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
            .map(file::TokioCompatFile::new)
            .unwrap();

        // Create new file (fail if already exists) and write to it.
        debug_assert_eq!(file.write(content).await.unwrap(), content.len());

        file.flush().await.unwrap();
        file.close().await.unwrap();

        debug_assert_eq!(&*read_entire_file().await, &*content);

        // Create new file with Trunc and write to it.
        //
        // Sftp::Create opens the file truncated.
        let mut file = sftp
            .create(&path)
            .await
            .map(file::TokioCompatFile::new)
            .unwrap();
        debug_assert_eq!(file.write(content).await.unwrap(), content.len());

        // close also flush the internal future buffers, but using a
        // different implementation from `TokioCompatFile::poll_flush`
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

def_write_all_test!(
    sftp_tokio_compact_file_write_all,
    sftp_options_with_max_rw_len(),
    file::TokioCompatFile::new,
    file,
    content,
    {
        file.write_all(content).await.unwrap();
    }
);

def_write_all_test!(
    sftp_tokio_compact_file_write_vectored_all,
    sftp_options_with_max_rw_len(),
    file::TokioCompatFile::new,
    file,
    content,
    {
        let len = content.len();

        write_vectored_all(
            file,
            [
                IoSlice::new(&content[..len / 2]),
                IoSlice::new(&content[len / 2..]),
            ]
            .as_mut_slice(),
        )
        .await
        .unwrap();
    }
);

#[tokio::test]
/// Test File::{set_len, set_permissions, metadata}.
async fn sftp_file_metadata() {
    let path = gen_path("sftp_file_metadata");

    let (mut child, sftp) = connect(sftp_options_with_max_rw_len()).await;

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

    let (mut child, sftp) = connect(sftp_options_with_max_rw_len()).await;

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
        let mut fs = sftp.fs();

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
        let mut fs = sftp.fs();

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
        let mut fs = sftp.fs();

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
        let mut fs = sftp.fs();

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
    let content = b"hello, world!\n";

    let (mut child, sftp) = connect(sftp_options_with_max_rw_len()).await;

    {
        let mut fs = sftp.fs();

        fs.write(&path, content).await.unwrap();
        assert_eq!(
            fs.metadata(&path).await.unwrap().len().unwrap(),
            content.len().try_into().unwrap()
        );

        fs.set_metadata(&path, metadata::MetaDataBuilder::new().len(2834).create())
            .await
            .unwrap();
        assert_eq!(fs.metadata(&path).await.unwrap().len().unwrap(), 2834);
    }

    // close sftp and child
    sftp.close().await.unwrap();
    assert!(child.wait().await.unwrap().success());
}

#[tokio::test]
/// Test File::copy_to
async fn sftp_file_copy_to() {
    let path = gen_path("sftp_file_copy_to");
    let content = b"hello, world!\n";

    let (mut child, sftp) = connect(sftp_options_with_max_rw_len()).await;

    sftp.fs().create_dir(&path).await.unwrap();

    {
        let mut file0 = sftp
            .options()
            .read(true)
            .write(true)
            .create(true)
            .open(path.join("file0"))
            .await
            .unwrap();

        file0.write_all(content).await.unwrap();
        file0.rewind().await.unwrap();

        let mut file1 = sftp
            .options()
            .read(true)
            .write(true)
            .create(true)
            .open(path.join("file1"))
            .await
            .unwrap();

        file0
            .copy_to(
                &mut file1,
                NonZeroU64::new(content.len().try_into().unwrap()).unwrap(),
            )
            .await
            .unwrap();

        file1.rewind().await.unwrap();
        assert_eq!(
            &*file1
                .read_all(content.len(), BytesMut::new())
                .await
                .unwrap(),
            content
        );
    }

    // close sftp and child
    sftp.close().await.unwrap();
    assert!(child.wait().await.unwrap().success());
}
