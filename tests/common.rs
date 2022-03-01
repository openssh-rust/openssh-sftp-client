use once_cell::sync::OnceCell;
use std::convert::TryInto;
use std::env;
use std::io::Error;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path;
use std::process::Stdio;
use tokio::process::{self, ChildStdin, ChildStdout};
use tokio_pipe::{PipeRead, PipeWrite};

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

pub fn get_sftp_path() -> &'static path::Path {
    static SFTP_PATH: OnceCell<path::PathBuf> = OnceCell::new();

    SFTP_PATH.get_or_init(|| {
        let mut sftp_path: path::PathBuf = env::var("OUT_DIR").unwrap().into();
        sftp_path.push("openssh-portable");
        sftp_path.push("sftp-server");

        eprintln!("sftp_path = {:#?}", sftp_path);

        sftp_path
    })
}

pub async fn launch_sftp() -> (process::Child, PipeWrite, PipeRead) {
    let mut child = process::Command::new(get_sftp_path())
        .args(&["-e", "-l", "DEBUG"])
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
