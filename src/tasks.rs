use super::{lowlevel::Extensions, Error, ReadEnd, SharedData};

use std::{
    num::NonZeroUsize,
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use bytes::Bytes;
use scopeguard::defer;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    pin,
    sync::oneshot,
    task::{spawn, JoinHandle},
    time,
};
use tokio_io_utility::{write_all_bytes, ReusableIoSlices};

async fn flush(
    shared_data: &SharedData,
    writer: Pin<&mut (dyn AsyncWrite + Send)>,
    buffer: &mut Vec<Bytes>,
    reusable_io_slices: &mut ReusableIoSlices,
) -> Result<(), Error> {
    shared_data.queue().swap(buffer);

    #[cfg(feature = "tracing")]
    tracing::debug!(
        "Flushing out {} bytes, shared_data = {shared_data:p}",
        buffer.len()
    );

    // `Queue` implementation for `MpscQueue` already removes
    // all empty `Bytes`s so that precond of write_all_bytes
    // is satisfied.
    write_all_bytes(writer, buffer, reusable_io_slices).await?;

    Ok(())
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
    write_end_buffer_size: NonZeroUsize,
    flush_interval: Duration,
) -> JoinHandle<Result<(), Error>> {
    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(name = "flush_task", skip(writer, shared_data), err)
    )]
    async fn inner(
        mut writer: Pin<&mut (dyn AsyncWrite + Send)>,
        shared_data: SharedData,
        write_end_buffer_size: NonZeroUsize,
        flush_interval: Duration,
    ) -> Result<(), Error> {
        let mut interval = if !flush_interval.is_zero() {
            let mut interval = time::interval(flush_interval);
            interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
            Some(interval)
        } else {
            None
        };

        let auxiliary = shared_data.get_auxiliary();
        let flush_end_notify = &auxiliary.flush_end_notify;
        let read_end_notify = &auxiliary.read_end_notify;
        let pending_requests = &auxiliary.pending_requests;
        let shutdown_stage = &auxiliary.shutdown_stage;
        let max_pending_requests = auxiliary.max_pending_requests();

        let cancel_guard = auxiliary.cancel_token.clone().drop_guard();

        let mut backup_queue_buffer = Vec::with_capacity(write_end_buffer_size.get());
        let mut reusable_io_slices = ReusableIoSlices::new(write_end_buffer_size);

        loop {
            #[cfg(feature = "tracing")]
            tracing::debug!(
                "Flushing out the initial hello msg from shared_data = {shared_data:p}"
            );

            // Flush the initial hello msg ASAP.
            let mut cnt = pending_requests.load(Ordering::Relaxed);

            loop {
                #[cfg(feature = "tracing")]
                tracing::debug!("Trigger read_task from shared_data = {shared_data:p}");

                read_end_notify.notify_one();

                // Wait until another thread is done or cancelled flushing
                // and try flush it again just in case the flushing is cancelled
                flush(
                    &shared_data,
                    writer.as_mut(),
                    &mut backup_queue_buffer,
                    &mut reusable_io_slices,
                )
                .await?;

                cnt = atomic_sub_assign(pending_requests, cnt);

                if cnt < max_pending_requests {
                    break;
                }
            }

            if shutdown_stage.load(Ordering::Relaxed) == 2 {
                #[cfg(feature = "tracing")]
                tracing::info!("flush_task graceful shutdown, shared_data = {shared_data:p}");

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

            flush_end_notify.notified().await;

            if let Some(interval) = interval.as_mut() {
                tokio::select! {
                    biased;

                    _ = interval.tick() => (),
                    // tokio::sync::Notify is cancel safe, however
                    // cancelling it would lose the place in the queue.
                    //
                    // However, since flush_task is the only one who
                    // calls `flush_immediately.notified()`, it
                    // is totally fine to cancel here.
                    _ = auxiliary.flush_immediately.notified() => (),
                };
            }
        }
    }

    spawn(async move {
        pin!(writer);

        inner(writer, shared_data, write_end_buffer_size, flush_interval).await
    })
}

pub(super) fn create_read_task<R: AsyncRead + Send + 'static>(
    stdout: R,
    read_end_buffer_size: NonZeroUsize,
    shared_data: SharedData,
) -> (oneshot::Receiver<Extensions>, JoinHandle<Result<(), Error>>) {
    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(name = "read_task", skip(stdout, tx, shared_data), err)
    )]
    async fn inner(
        stdout: Pin<&mut (dyn AsyncRead + Send)>,
        read_end_buffer_size: NonZeroUsize,
        shared_data: SharedData,
        tx: oneshot::Sender<Extensions>,
    ) -> Result<(), Error> {
        let read_end = ReadEnd::new(stdout, read_end_buffer_size, shared_data.clone());

        let auxiliary = shared_data.get_auxiliary();
        let read_end_notify = &auxiliary.read_end_notify;
        let requests_to_read = &auxiliary.requests_to_read;
        let shutdown_stage = &auxiliary.shutdown_stage;
        let cancel_guard = auxiliary.cancel_token.clone().drop_guard();

        pin!(read_end);

        defer! {
            #[cfg(feature = "tracing")]
            tracing::info!(
                "Requesting graceful shutdown of flush_task from read_task, shared_data = {shared_data:p}"
            );

            // Order the shutdown of flush_task.
            auxiliary.shutdown_stage.store(2, Ordering::Relaxed);

            auxiliary.flush_immediately.notify_one();
            auxiliary.flush_end_notify.notify_one();
        }

        #[cfg(feature = "tracing")]
        tracing::debug!("Receiving version and extensions, shared_data = {shared_data:p}");

        // Receive version and extensions
        let extensions = read_end.as_mut().receive_server_hello_pinned().await?;

        tx.send(extensions).unwrap();

        loop {
            read_end_notify.notified().await;

            let mut cnt = requests_to_read.load(Ordering::Relaxed);

            #[cfg(feature = "tracing")]
            tracing::debug!(
                "Attempting to read {cnt} responses in read_task, shared_data = {shared_data:p}"
            );

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

                break Ok(());
            }
        }
    }

    let (tx, rx) = oneshot::channel();

    let handle = spawn(async move {
        pin!(stdout);

        inner(stdout, read_end_buffer_size, shared_data, tx).await
    });

    (rx, handle)
}
