use super::{ResponseCallback, ThreadSafeWaker};

use dashmap::DashMap;

#[derive(Debug, Default)]
pub(crate) struct ResponseCallbacks(DashMap<u32, (ThreadSafeWaker, ResponseCallback)>);

impl ResponseCallbacks {}
