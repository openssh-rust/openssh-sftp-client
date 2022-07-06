use std::pin::Pin;

use tokio::sync::{Mutex as MutexAsync, MutexGuard as MutexAsyncGuard};

#[derive(Debug)]
pub(super) struct PinnedMutexAsync<T>(MutexAsync<T>);

impl<T> PinnedMutexAsync<T> {
    pub(super) fn new(val: T) -> Self {
        Self(MutexAsync::new(val))
    }

    pub(super) async fn lock(self: Pin<&Self>) -> PinnedMutexAsyncGuard<'_, T> {
        let this = self.get_ref();
        let guard = this.0.lock().await;

        PinnedMutexAsyncGuard(guard)
    }
}

#[derive(Debug)]
pub(super) struct PinnedMutexAsyncGuard<'a, T>(MutexAsyncGuard<'a, T>);

impl<'a, T> PinnedMutexAsyncGuard<'a, T> {
    pub(super) fn deref_mut_pinned(&mut self) -> Pin<&mut T> {
        // Safety:
        //
        // PinnedMutexAsyncGuard can only be obtained via `PinnedMutexAsync.lock()`,
        // which only takes `Pin<&Self>`.
        //
        // Since `PinnedMutexAsync` is pinned and `T` is stored inline, so `T`
        // is also pinned.
        unsafe { Pin::new_unchecked(&mut *self.0) }
    }
}
