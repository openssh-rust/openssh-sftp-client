use super::{ResponseCallback, ThreadSafeWaker};

use parking_lot::RwLock;
use thunderdome::Arena;

#[derive(Debug, Default)]
pub(crate) struct ResponseCallbacks(RwLock<Arena<(ThreadSafeWaker, ResponseCallback)>>);

impl ResponseCallbacks {}
