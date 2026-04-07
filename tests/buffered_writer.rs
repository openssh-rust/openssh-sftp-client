/// Regression test for <https://github.com/openssh-rust/openssh-sftp-client/issues/154>
///
/// `tokio::io::stdout()` wraps `std::io::Stdout` (`LineWriter<StdoutRaw>`),
/// which buffers writes and only flushes automatically when it sees `'\n'`.
/// SFTP is a binary protocol — its packets never contain `'\n'` — so without
/// an explicit `flush()` call the SSH_FXP_INIT "hello" packet stays in the
/// buffer and never reaches the sftp-server.  The server never replies with
/// SSH_FXP_VERSION, and `Sftp::new` hangs forever.
///
/// This test reproduces the bug with a `HoldWriter` that buffers all writes
/// indefinitely and only forwards data to the underlying writer on an explicit
/// `flush()`.  An `Arc<AtomicBool>` records whether `flush` was ever called,
/// letting the test assert both that `Sftp::new` completes *and* that it did
/// so by issuing a flush rather than by luck.
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use openssh_sftp_client::{Sftp, SftpOptions};
use pin_project::pin_project;
use sftp_test_common::launch_sftp;
use tokio::io::AsyncWrite;

#[pin_project]
struct HoldWriter<W> {
    #[pin]
    inner: W,
    buf: Vec<u8>,
    flushed: Arc<AtomicBool>,
}

impl<W> HoldWriter<W> {
    fn new(inner: W, flushed: Arc<AtomicBool>) -> Self {
        Self {
            inner,
            buf: Vec::new(),
            flushed,
        }
    }
}

impl<W: AsyncWrite> AsyncWrite for HoldWriter<W> {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.project().buf.extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut this = self.project();
        while !this.buf.is_empty() {
            let n = {
                match this.inner.as_mut().poll_write(cx, &this.buf) {
                    Poll::Ready(Ok(n)) => n,
                    Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                    Poll::Pending => return Poll::Pending,
                }
            };
            this.buf.drain(..n);
        }

        match this.inner.poll_flush(cx) {
            Poll::Ready(Ok(())) => {
                this.flushed.store(true, Ordering::SeqCst);
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.project().inner.poll_shutdown(cx)
    }
}

#[tokio::test]
async fn sftp_new_works_with_buf_writer() {
    let (mut child, stdin, stdout) = launch_sftp().await;

    let flushed = Arc::new(AtomicBool::new(false));
    let writer = HoldWriter::new(stdin, flushed.clone());

    let sftp = tokio::time::timeout(
        Duration::from_secs(5),
        Sftp::new(writer, stdout, SftpOptions::default()),
    )
    .await
    .expect(
        "Sftp::new timed out — the SSH_FXP_INIT packet was never flushed \
         through HoldWriter to sftp-server (missing flush() fix?)",
    )
    .expect("Sftp::new returned an error");

    assert!(
        flushed.load(Ordering::SeqCst),
        "Sftp::new completed but flush() was never called on the writer"
    );

    sftp.close().await.expect("Sftp::close failed");
    assert!(child.wait().await.unwrap().success());
}
