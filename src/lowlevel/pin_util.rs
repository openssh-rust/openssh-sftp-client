use std::ops::Deref;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::{Mutex as MutexAsync, MutexGuard as MutexAsyncGuard};

#[derive(Debug)]
pub(super) struct PinnedArc<T>(Arc<T>);

impl<T> Clone for PinnedArc<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> PinnedArc<T> {
    pub(super) fn new(val: T) -> Self {
        Self(Arc::new(val))
    }

    pub(super) fn strong_count(arc: &Self) -> usize {
        Arc::strong_count(&arc.0)
    }

    pub(super) fn get_pinned_mut(arc: &mut Self) -> Option<Pin<&mut T>> {
        // Safety:
        //
        // Arc is essentially ref-counted box, which provides stable address for T,
        // moving the arc does not change the address of T.
        Arc::get_mut(&mut arc.0).map(|val| unsafe { Pin::new_unchecked(val) })
    }

    pub(super) fn deref_pinned(arc: &Self) -> Pin<&T> {
        // Safety:
        //
        // Arc is essentially ref-counted box, which provides stable address for T,
        // moving the arc does not change the address of T.
        unsafe { Pin::new_unchecked(&*arc.0) }
    }
}

impl<T> Deref for PinnedArc<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
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
