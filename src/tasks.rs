use super::{Error, ReadEnd, SharedData};
use crate::lowlevel::Extensions;

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::pin;
use tokio::sync::oneshot;
use tokio::task::{spawn, JoinHandle};
use tokio::time;

async fn flush<W: AsyncWrite>(shared_data: &SharedData<W>) -> Result<(), Error> {
    shared_data.flush().await.map_err(Error::from)
}

/// Return the size after substraction.
///
/// # Panic
///
/// If it underflows, thenit will panic.
fn atomic_sub_assign(atomic: &AtomicU32, val: u32) -> u32 {
    atomic.fetch_sub(val, Ordering::Relaxed) - val
}

pub(super) fn create_flush_task<W: AsyncWrite + Send + Sync + 'static>(
    shared_data: SharedData<W>,
    flush_interval: Duration,
) -> JoinHandle<Result<(), Error>> {
    spawn(async move {
        let mut interval = time::interval(flush_interval);
        interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

        let auxiliary = shared_data.get_auxiliary();
        let flush_end_notify = &auxiliary.flush_end_notify;
        let read_end_notify = &auxiliary.read_end_notify;
        let pending_requests = &auxiliary.pending_requests;
        let shutdown_requested = &auxiliary.shutdown_requested;
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

            let mut prev_pending_requests = pending_requests.load(Ordering::Relaxed);

            loop {
                read_end_notify.notify_one();

                // Wait until another thread is done or cancelled flushing
                // and try flush it again just in case the flushing is cancelled
                flush(&shared_data).await?;

                prev_pending_requests =
                    pending_requests.fetch_sub(prev_pending_requests, Ordering::Relaxed);

                if prev_pending_requests < max_pending_requests {
                    break;
                }
            }

            if shutdown_requested.load(Ordering::Relaxed) {
                read_end_notify.notify_one();

                // Once shutdown_requested is sent, there will be no
                // new requests.
                //
                // Flushing here will ensure all pending requests is sent.
                flush(&shared_data).await?;

                cancel_guard.disarm();

                break Ok(());
            }
        }
    })
}

pub(super) fn create_read_task<
    R: AsyncRead + Send + Sync + 'static,
    W: AsyncWrite + Send + Sync + 'static,
>(
    read_end: ReadEnd<R, W>,
) -> (oneshot::Receiver<Extensions>, JoinHandle<Result<(), Error>>) {
    let (tx, rx) = oneshot::channel();

    let handle = spawn(async move {
        let shared_data = read_end.get_shared_data().clone();
        let auxiliary = shared_data.get_auxiliary();
        let read_end_notify = &auxiliary.read_end_notify;
        let requests_to_read = &auxiliary.requests_to_read;
        let shutdown_requested = &auxiliary.shutdown_requested;
        let cancel_guard = auxiliary.cancel_token.clone().drop_guard();

        pin!(read_end);

        // Receive version and extensions
        let extensions = read_end.as_mut().receive_server_hello_pinned().await?;

        tx.send(extensions).unwrap();

        loop {
            read_end_notify.notified().await;

            let mut cnt = requests_to_read.load(Ordering::Relaxed);

            while cnt != 0 {
                eprintln!("Enter while loop cnt = {cnt}");

                // If attempt to read in more than new_requests_submit, then
                // `read_in_one_packet` might block forever.
                for _ in 0..cnt {
                    read_end.as_mut().read_in_one_packet_pinned().await?;
                }

                cnt = atomic_sub_assign(requests_to_read, cnt);
                eprintln!("Updated requests_to_read: new_cnt = {cnt}");
            }

            if shutdown_requested.load(Ordering::Relaxed) {
                // All responses is read in and there is no
                // write_end/shared_data left.
                cancel_guard.disarm();
                break Ok(());
            }
        }
    });

    (rx, handle)
}
