use std::convert::TryInto;
use std::env;
use std::io::Error;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use once_cell::sync::OnceCell;

pub use tokio::process;
use tokio::process::{ChildStdin, ChildStdout};

pub use tokio_pipe::{PipeRead, PipeWrite};

unsafe fn dup(raw_fd: RawFd) -> Result<RawFd, Error> {
    let res = libc::dup(raw_fd);
    if res == -1 {
        Err(Error::last_os_error())
    } else {
        Ok(res)
    }
}

fn child_stdin_to_pipewrite(stdin: ChildStdin) -> Result<PipeWrite, Error> {
    let fd = stdin.as_raw_fd();

    // safety: arg.as_raw_fd() is guaranteed to return a valid fd.
    let fd = unsafe { dup(fd) }?;
    // safety: under unix, ChildStdin/ChildStdout is implemented using pipe
    fd.try_into()
}

fn child_stdout_to_pipewrite(stdon: ChildStdout) -> Result<PipeRead, Error> {
    let fd = stdon.as_raw_fd();

    // safety: arg.as_raw_fd() is guaranteed to return a valid fd.
    let fd = unsafe { dup(fd) }?;
    // safety: under unix, ChildStdin/ChildStdout is implemented using pipe
    fd.try_into()
}

pub fn get_sftp_path() -> &'static Path {
    static SFTP_PATH: OnceCell<PathBuf> = OnceCell::new();

    SFTP_PATH.get_or_init(|| {
        let mut sftp_path: PathBuf = env!("OUT_DIR").into();
        sftp_path.push("openssh-portable");
        sftp_path.push("sftp-server");

        eprintln!("sftp_path = {sftp_path:#?}");

        sftp_path
    })
}

pub async fn launch_sftp() -> (process::Child, PipeWrite, PipeRead) {
    let mut child = process::Command::new(get_sftp_path())
        .args(["-e", "-l", "DEBUG"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();

    let stdin = child_stdin_to_pipewrite(child.stdin.take().unwrap()).unwrap();
    let stdout = child_stdout_to_pipewrite(child.stdout.take().unwrap()).unwrap();

    (child, stdin, stdout)
}

fn get_tmp_path() -> &'static Path {
    Path::new(if cfg!(target_os = "linux") {
        "/tmp"
    } else if cfg!(target_os = "macos") {
        "/private/tmp"
    } else {
        panic!("Unsupported target")
    })
}

pub fn get_path_for_tmp_files() -> PathBuf {
    static XDG_RUNTIME_DIR: OnceCell<Option<Box<Path>>> = OnceCell::new();

    XDG_RUNTIME_DIR
        .get_or_init(|| {
            env::var_os("RUNTIME_DIR").map(|os_str| {
                let pathbuf: PathBuf = os_str.into();
                pathbuf
                    .canonicalize()
                    .expect("Failed to canonicalize $RUNTIME_DIR")
                    .into_boxed_path()
            })
        })
        .as_deref()
        .unwrap_or_else(get_tmp_path)
        .join("openssh_sftp_client")
}
