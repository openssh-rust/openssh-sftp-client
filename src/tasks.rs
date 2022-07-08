use super::{Error, ReadEnd, SharedData};
use crate::lowlevel::Extensions;

use std::io;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::pin;
use tokio::sync::oneshot;
use tokio::task::{spawn, JoinHandle};
use tokio::time;

async fn flush<W: AsyncWrite>(
    shared_data: &SharedData,
    mut writer: Pin<&mut W>,
) -> Result<(), Error> {
    let mut buffers = shared_data.get_buffers().await;

    if buffers.is_empty() {
        return Ok(());
    }

    loop {
        let n = writer
            .as_mut()
            .write_vectored(buffers.get_io_slices())
            .await?;

        // Since `MpScBytesQueue::get_buffers` guarantees that every `IoSlice`
        // returned must be non-empty, having `0` bytes written is an error
        // likely caused by the close of the read end.
        let n = NonZeroUsize::new(n).ok_or_else(|| io::Error::new(io::ErrorKind::WriteZero, ""))?;

        // buffers.advance returns false when there is nothing
        // left to flush.
        if !buffers.advance(n) {
            break Ok(());
        }
    }
}

/// Return the size after substraction.
///
/// # Panic
///
/// If it underflows, thenit will panic.
fn atomic_sub_assign(atomic: &AtomicUsize, val: usize) -> usize {
    atomic.fetch_sub(val, Ordering::Relaxed) - val
}

pub(super) fn create_flush_task<W: AsyncWrite + Send + 'static>(
    writer: W,
    shared_data: SharedData,
    flush_interval: Duration,
) -> JoinHandle<Result<(), Error>> {
    spawn(async move {
        tokio::pin!(writer);

        let mut interval = time::interval(flush_interval);
        interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

        let auxiliary = shared_data.get_auxiliary();
        let flush_end_notify = &auxiliary.flush_end_notify;
        let read_end_notify = &auxiliary.read_end_notify;
        let pending_requests = &auxiliary.pending_requests;
        let shutdown_stage = &auxiliary.shutdown_stage;
        let max_pending_requests = auxiliary.max_pending_requests();

        let cancel_guard = auxiliary.cancel_token.clone().drop_guard();

        // The loop can only return `Err`
        loop {
            flush_end_notify.notified().await;

            tokio::select! {
                _ = interval.tick() => (),
                // tokio::sync::Notify is cancel safe, however
                // cancelling it would lose the place in the queue.
                //
                // However, since flush_task is the only one who
                // calls `flush_immediately.notified()`, it
                // is totally fine to cancel here.
                _ = auxiliary.flush_immediately.notified() => (),
            };

            let mut cnt = pending_requests.load(Ordering::Relaxed);

            loop {
                read_end_notify.notify_one();

                // Wait until another thread is done or cancelled flushing
                // and try flush it again just in case the flushing is cancelled
                flush(&shared_data, writer.as_mut()).await?;

                cnt = atomic_sub_assign(pending_requests, cnt);

                if cnt < max_pending_requests {
                    break;
                }
            }

            if shutdown_stage.load(Ordering::Relaxed) == 2 {
                // Read tasks have read in all responses, thus
                // write task can exit now.
                //
                // Since sftp-server implementation from openssh-portable
                // will quit once its cannot read anything, we have to keep
                // the write task alive until all requests have been processed
                // by sftp-server and each response is handled by the read task.

                debug_assert_eq!(cnt, 0);

                cancel_guard.disarm();

                break Ok(());
            }
        }
    })
}

pub(super) fn create_read_task<R: AsyncRead + Send + 'static>(
    read_end: ReadEnd<R>,
) -> (oneshot::Receiver<Extensions>, JoinHandle<Result<(), Error>>) {
    let (tx, rx) = oneshot::channel();

    let handle = spawn(async move {
        let shared_data = read_end.get_shared_data().clone();
        let auxiliary = shared_data.get_auxiliary();
        let read_end_notify = &auxiliary.read_end_notify;
        let requests_to_read = &auxiliary.requests_to_read;
        let shutdown_stage = &auxiliary.shutdown_stage;
        let cancel_guard = auxiliary.cancel_token.clone().drop_guard();

        pin!(read_end);

        // Receive version and extensions
        let extensions = read_end.as_mut().receive_server_hello_pinned().await?;

        tx.send(extensions).unwrap();

        loop {
            read_end_notify.notified().await;

            let mut cnt = requests_to_read.load(Ordering::Relaxed);

            while cnt != 0 {
                // If attempt to read in more than new_requests_submit, then
                // `read_in_one_packet` might block forever.
                for _ in 0..cnt {
                    read_end.as_mut().read_in_one_packet_pinned().await?;
                }

                cnt = atomic_sub_assign(requests_to_read, cnt);
            }

            if shutdown_stage.load(Ordering::Relaxed) == 1 {
                // All responses is read in and there is no
                // write_end/shared_data left.
                cancel_guard.disarm();

                // Order the shutdown of flush_task.
                auxiliary.shutdown_stage.store(2, Ordering::Relaxed);

                auxiliary.flush_immediately.notify_one();
                auxiliary.flush_end_notify.notify_one();

                break Ok(());
            }
        }
    });

    (rx, handle)
}
