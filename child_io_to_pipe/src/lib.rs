use std::convert::TryInto;
use std::io::Error;
use std::os::unix::io::{AsRawFd, RawFd};

use tokio::process::{ChildStdin, ChildStdout};
use tokio_pipe::{PipeRead, PipeWrite};

unsafe fn dup(raw_fd: RawFd) -> Result<RawFd, Error> {
    let res = libc::dup(raw_fd);
    if res == -1 {
        Err(Error::last_os_error())
    } else {
        Ok(res)
    }
}

pub fn child_stdin_to_pipewrite(stdin: ChildStdin) -> Result<PipeWrite, Error> {
    let fd = stdin.as_raw_fd();

    // safety: arg.as_raw_fd() is guaranteed to return a valid fd.
    let fd = unsafe { dup(fd) }?;
    // safety: under unix, ChildStdin/ChildStdout is implemented using pipe
    fd.try_into()
}

pub fn child_stdout_to_pipewrite(stdon: ChildStdout) -> Result<PipeRead, Error> {
    let fd = stdon.as_raw_fd();

    // safety: arg.as_raw_fd() is guaranteed to return a valid fd.
    let fd = unsafe { dup(fd) }?;
    // safety: under unix, ChildStdin/ChildStdout is implemented using pipe
    fd.try_into()
}
