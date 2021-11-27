use super::{ResponseCallback, ThreadSafeWaker};

use thunderdome::Arena;

#[derive(Debug, Default)]
pub(crate) struct ResponseCallbacks(Arena<(ThreadSafeWaker, ResponseCallback)>);

impl ResponseCallbacks {}
