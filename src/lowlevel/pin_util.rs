use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::{Mutex as MutexAsync, MutexGuard as MutexAsyncGuard};

pub(super) fn pinned_arc_strong_count<T>(arc: &Pin<Arc<T>>) -> usize {
    let arc_ptr = arc as *const Pin<Arc<T>> as *const Arc<T>;

    // Safety:
    //
    // Pin is a transparent new type of Arc.
    //
    // Pin<Arc<T>> prevent users from moving from/into arc,
    // however since we hold a immutable reference here,
    // it is ok to dereference arc_ptr and holds a &Arc<T>.
    let arc = unsafe { &*arc_ptr };

    Arc::strong_count(arc)
}

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
