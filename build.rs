use std::env;
use std::fs::{canonicalize, create_dir_all, metadata};
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn main() {
    // openssh-portable only exists in development.
    //
    // It is excluded from release.
    if metadata("openssh-portable").is_ok() {
        let mut build_dir: PathBuf = env::var("OUT_DIR").unwrap().into();
        build_dir.push("openssh-portable");

        let build_dir = build_dir;

        create_dir_all(&build_dir).unwrap();

        println!("cargo:rerun-if-changed=openssh-portable");

        let script = canonicalize("./compile-sftp-server.sh").unwrap();

        let status = Command::new(&script)
            .stdin(Stdio::null())
            .current_dir(&build_dir)
            .status()
            .unwrap();

        if !status.success() {
            panic!("{:#?} failed: {:#?}", script, status);
        }
    }
}
