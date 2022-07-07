use super::lowlevel::Extensions;

use once_cell::sync::OnceCell;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Copy, Clone)]
pub(super) struct Limits {
    pub(super) read_len: u32,
    pub(super) write_len: u32,
}

#[derive(Debug)]
pub(super) struct ConnInfo {
    pub(super) limits: Limits,
    pub(super) extensions: Extensions,
}

#[derive(Debug)]
pub(super) struct Auxiliary {
    pub(super) conn_info: OnceCell<ConnInfo>,

    /// cancel_token is used to cancel `Awaitable*Future`
    /// when the read_task/flush_task has failed.
    pub(super) cancel_token: CancellationToken,

    /// flush_end_notify is used to avoid unnecessary wakeup
    /// in flush_task.
    pub(super) flush_end_notify: Notify,
    /// `Notify::notify_one` is called if
    /// pending_requests == max_pending_requests.
    pub(super) flush_immediately: Notify,

    /// There can be at most `u32::MAX` pending requests, since each request
    /// requires a request id that is 32 bits.
    pub(super) pending_requests: AtomicUsize,
    pub(super) max_pending_requests: u16,

    pub(super) read_end_notify: Notify,
    pub(super) requests_to_read: AtomicUsize,

    /// 0 means no shutdown is requested
    /// 1 means the read task should shutdown
    /// 2 means the flush task should shutdown
    pub(super) shutdown_stage: AtomicU8,
}

impl Auxiliary {
    pub(super) fn new(max_pending_requests: u16) -> Self {
        Self {
            conn_info: OnceCell::new(),

            cancel_token: CancellationToken::new(),

            flush_end_notify: Notify::new(),
            flush_immediately: Notify::new(),

            pending_requests: AtomicUsize::new(0),
            max_pending_requests,

            read_end_notify: Notify::new(),
            requests_to_read: AtomicUsize::new(0),

            shutdown_stage: AtomicU8::new(0),
        }
    }

    pub(super) fn wakeup_flush_task(&self) {
        // Must increment requests_to_read first, since
        // flush_task might wakeup read_end once it done flushing.
        self.requests_to_read.fetch_add(1, Ordering::Relaxed);

        let pending_requests = self.pending_requests.fetch_add(1, Ordering::Relaxed);

        self.flush_end_notify.notify_one();
        // Use `==` here to avoid unnecessary wakeup of flush_task.
        if pending_requests == self.max_pending_requests() {
            self.flush_immediately.notify_one();
        }
    }

    fn conn_info(&self) -> &ConnInfo {
        self.conn_info
            .get()
            .expect("auxiliary.conn_info shall be initialized by sftp::Sftp::new")
    }

    pub(super) fn extensions(&self) -> Extensions {
        // since writing to conn_info is only done in `Sftp::new`,
        // reading these variable should never block.
        self.conn_info().extensions
    }

    pub(super) fn limits(&self) -> Limits {
        // since writing to conn_info is only done in `Sftp::new`,
        // reading these variable should never block.
        self.conn_info().limits
    }

    pub(super) fn max_pending_requests(&self) -> usize {
        self.max_pending_requests as usize
    }

    pub(super) fn order_shutdown(&self) {
        // Order the shutdown of read_task.
        //
        // Once it shutdowns, it will automatically order
        // shutdown of flush_task.
        self.shutdown_stage.store(1, Ordering::Relaxed);

        self.flush_immediately.notify_one();
        self.flush_end_notify.notify_one();
    }
}
