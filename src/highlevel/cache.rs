use super::{Auxiliary, BoxedWaitForCancellationFuture, Error, Id, Sftp, WriteEnd, Writer};

use std::future::Future;
use std::ops::{Deref, DerefMut};

#[derive(Debug)]
pub(super) struct WriteEndWithCachedId<'s, W> {
    sftp: &'s Sftp<W>,
    inner: WriteEnd<W>,
    id: Option<Id>,
}

impl<W> Clone for WriteEndWithCachedId<'_, W> {
    fn clone(&self) -> Self {
        Self {
            sftp: self.sftp,
            inner: self.inner.clone(),
            id: None,
        }
    }
}

impl<W> Deref for WriteEndWithCachedId<'_, W> {
    type Target = WriteEnd<W>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<W> DerefMut for WriteEndWithCachedId<'_, W> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<'s, W> WriteEndWithCachedId<'s, W> {
    pub(super) fn new(sftp: &'s Sftp<W>, inner: WriteEnd<W>) -> Self {
        Self {
            sftp,
            inner,
            id: None,
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
        let cancel_err = || Err(BoxedWaitForCancellationFuture::cancel_error());
        let auxiliary = self.sftp.auxiliary();

        if auxiliary.cancel_token.is_cancelled() {
            return cancel_err();
        }

        tokio::select! {
            res = future => res.map_err(Into::into),
            _ = self.wait_for_cancell_future.wait(auxiliary) => cancel_err(),
        }
    }

    pub(super) fn get_auxiliary(&self) -> &'s Auxiliary {
        self.sftp.auxiliary()
    }

    pub(super) fn sftp(&self) -> &'s Sftp<W> {
        self.sftp
    }
}

impl<'s, W: Writer> WriteEndWithCachedId<'s, W> {
    pub(super) async fn send_request<Func, F, R>(&mut self, f: Func) -> Result<R, Error>
    where
        Func: FnOnce(&mut WriteEnd<W>, Id) -> Result<F, Error>,
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
}
