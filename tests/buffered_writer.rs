/// Regression test for <https://github.com/openssh-rust/openssh-sftp-client/issues/154>
///
/// `tokio::io::stdout()` wraps `std::io::Stdout` (`LineWriter<StdoutRaw>`),
/// which buffers writes and only flushes automatically when it sees `'\n'`.
/// SFTP is a binary protocol — its packets never contain `'\n'` — so without
/// an explicit `flush()` call the SSH_FXP_INIT "hello" packet stays in the
/// buffer and never reaches the sftp-server.  The server never replies with
/// SSH_FXP_VERSION, and `Sftp::new` hangs forever.
///
/// This test reproduces the bug by wrapping the real sftp-server pipe with
/// `tokio::io::BufWriter`, which has the same buffering semantics as
/// `tokio::io::stdout()`: `poll_write` accumulates data internally and
/// `poll_flush` is the only way to push it to the peer.
use std::time::Duration;

use openssh_sftp_client::{Sftp, SftpOptions};
use sftp_test_common::launch_sftp;

#[tokio::test]
async fn sftp_new_works_with_buf_writer() {
    let (mut child, stdin, stdout) = launch_sftp().await;
    let buffered_stdin = tokio::io::BufWriter::new(stdin);

    let sftp = tokio::time::timeout(
        Duration::from_secs(5),
        Sftp::new(buffered_stdin, stdout, SftpOptions::default()),
    )
    .await
    .expect(
        "Sftp::new timed out — the SSH_FXP_INIT packet was never flushed \
         through BufWriter to sftp-server (missing flush() fix?)",
    )
    .expect("Sftp::new returned an error");

    sftp.close().await.expect("Sftp::close failed");
    assert!(child.wait().await.unwrap().success());
}
