use super::{Auxiliary, BoxedWaitForCancellationFuture, Error, Id, Sftp, WriteEnd};

use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;

#[derive(Debug)]
pub(super) struct WriteEndWithCachedId<'s> {
    sftp: &'s Sftp,
    inner: WriteEnd,
    id: Option<Id>,
}

impl Clone for WriteEndWithCachedId<'_> {
    fn clone(&self) -> Self {
        Self {
            sftp: self.sftp,
            inner: self.inner.clone(),
            id: None,
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
        async fn inner<R>(
            this: &mut WriteEndWithCachedId<'_>,
            future: Pin<&mut (dyn Future<Output = Result<R, Error>>)>,
        ) -> Result<R, Error> {
            let cancel_err = || Err(BoxedWaitForCancellationFuture::cancel_error());
            let auxiliary = this.sftp.auxiliary();

            let cancel_token = &auxiliary.cancel_token;

            if cancel_token.is_cancelled() {
                return cancel_err();
            }

            tokio::select! {
                res = future => res,
                _ = cancel_token.cancelled() => cancel_err(),
            }
        }

        let future = async move { future.await.map_err(Into::into) };
        tokio::pin!(future);

        inner(self, future).await
    }

    pub(super) fn get_auxiliary(&self) -> &'s Auxiliary {
        self.sftp.auxiliary()
    }

    pub(super) fn sftp(&self) -> &'s Sftp {
        self.sftp
    }
}

impl<'s> WriteEndWithCachedId<'s> {
    pub(super) async fn send_request<Func, F, R>(&mut self, f: Func) -> Result<R, Error>
    where
        Func: FnOnce(&mut WriteEnd, Id) -> Result<F, Error>,
        F: Future<Output = Result<(Id, R), Error>> + 'static,
    {
        let id = self.get_id_mut();
        let write_end = &mut self.inner;

        let future = f(write_end, id)?;
        tokio::pin!(future);

        async fn inner<R>(
            this: &mut WriteEndWithCachedId<'_>,
            future: Pin<&mut (dyn Future<Output = Result<(Id, R), Error>>)>,
        ) -> Result<R, Error> {
            // Requests is already added to write buffer, so wakeup
            // the `flush_task`.
            this.get_auxiliary().wakeup_flush_task();

            let (id, ret) = this.cancel_if_task_failed(future).await?;

            this.cache_id_mut(id);

            Ok(ret)
        }

        inner(self, future).await
    }
}
