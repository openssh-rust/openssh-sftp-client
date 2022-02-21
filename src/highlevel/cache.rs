use super::{Auxiliary, BoxedWaitForCancellationFuture, Error, Id, Sftp, WriteEnd};

use std::future::Future;
use std::ops::{Deref, DerefMut};

#[derive(Debug)]
pub(super) struct WriteEndWithCachedId<'s> {
    sftp: &'s Sftp,
    inner: WriteEnd,
    id: Option<Id>,
    /// WaitForCancellationFuture adds itself as an entry to the internal
    /// linked list of CancellationToken when `poll`ed.
    ///
    /// Thus, in its `Drop::drop` implementation, it is removed from the
    /// linked list.
    ///
    /// However, rust does not guarantee on 'no leaking', thus it is possible
    /// and safe for user to `mem::forget` the future returned, and thus
    /// causing the linked list to point to invalid memory locations.
    ///
    /// To avoid this, we have to box this future.
    ///
    /// However, allocate a new box each time a future is called is super
    /// expensive, thus we keep it cached so that we can reuse it.
    wait_for_cancell_future: BoxedWaitForCancellationFuture<'s>,
}

impl Clone for WriteEndWithCachedId<'_> {
    fn clone(&self) -> Self {
        Self {
            sftp: self.sftp,
            inner: self.inner.clone(),
            id: None,
            wait_for_cancell_future: BoxedWaitForCancellationFuture::new(),
        }
    }
}

impl Deref for WriteEndWithCachedId<'_> {
    type Target = WriteEnd;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for WriteEndWithCachedId<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<'s> WriteEndWithCachedId<'s> {
    pub(super) fn new(sftp: &'s Sftp, inner: WriteEnd) -> Self {
        Self {
            sftp,
            inner,
            id: None,
            wait_for_cancell_future: BoxedWaitForCancellationFuture::new(),
        }
    }

    pub(super) fn get_id_mut(&mut self) -> Id {
        self.id
            .take()
            .unwrap_or_else(|| self.inner.create_response_id())
    }

    pub(super) fn cache_id_mut(&mut self, id: Id) {
        if self.id.is_none() {
            self.id = Some(id);
        }
    }

    /// * `f` - the future must be cancel safe.
    pub(super) async fn cancel_if_task_failed<R, E, F>(&mut self, future: F) -> Result<R, Error>
    where
        F: Future<Output = Result<R, E>>,
        E: Into<Error>,
    {
        let cancel_err = || Err(BoxedWaitForCancellationFuture::cancel_error().into());
        let auxiliary = self.sftp.auxiliary();

        if auxiliary.cancel_token.is_cancelled() {
            return cancel_err();
        }

        tokio::select! {
            res = future => res.map_err(Into::into),
            _ = self.wait_for_cancell_future.wait(auxiliary) => cancel_err(),
        }
    }

    pub(super) async fn send_request<Func, F, R>(&mut self, f: Func) -> Result<R, Error>
    where
        Func: FnOnce(&mut WriteEnd, Id) -> Result<F, Error>,
        F: Future<Output = Result<(Id, R), Error>> + 'static,
    {
        let id = self.get_id_mut();
        let write_end = &mut self.inner;

        let future = f(write_end, id)?;

        // Requests is already added to write buffer, so wakeup
        // the `flush_task`.
        self.get_auxiliary().wakeup_flush_task();

        let (id, ret) = self.cancel_if_task_failed(future).await?;

        self.cache_id_mut(id);

        Ok(ret)
    }

    pub(super) fn get_auxiliary(&self) -> &'s Auxiliary {
        self.sftp.auxiliary()
    }

    pub(super) fn sftp(&self) -> &'s Sftp {
        self.sftp
    }
}
