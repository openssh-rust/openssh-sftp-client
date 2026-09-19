//! Regression test for issue #183.
//!
//! When the server side hits EOF before (or during) the version exchange,
//! `Sftp::new` must return an error on every runtime flavor.  On a
//! multi-thread runtime it used to hang forever: `read_task` set
//! `shutdown_stage = 2`, then the caller's `Sftp::init` error path dropped the
//! last handle, and `order_shutdown` overwrote the stage with `1` before
//! `flush_task` observed it, so `flush_task` never shut down and
//! `Sftp::close` waited on it forever.

use std::time::Duration;

use openssh_sftp_client::Sftp;
use tokio::{
    io::{empty, sink},
    time::timeout,
};

async fn assert_new_errors_on_eof() {
    // `empty()` is at EOF immediately; `sink()` accepts the client hello.
    let res = timeout(
        Duration::from_secs(10),
        Sftp::new(sink(), empty(), Default::default()),
    )
    .await;

    match res {
        Err(_) => panic!("Sftp::new hung on EOF before version exchange"),
        Ok(Ok(_)) => panic!("Sftp::new succeeded without a server hello"),
        Ok(Err(err)) => {
            let msg = err.to_string();
            assert!(
                msg.contains("unexpected end of file"),
                "unexpected error: {msg}"
            );
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn eof_before_hello_current_thread() {
    assert_new_errors_on_eof().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn eof_before_hello_multi_thread() {
    for _ in 0..20 {
        assert_new_errors_on_eof().await;
    }
}
