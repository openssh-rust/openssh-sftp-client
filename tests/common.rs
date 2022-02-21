use once_cell::sync::OnceCell;

use std::env;
use std::path;
use std::process::Stdio;
use tokio::process;

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

pub async fn launch_sftp() -> (process::Child, process::ChildStdin, process::ChildStdout) {
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
