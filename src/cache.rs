use crate::{cancel_error, Auxiliary, Error, Id, WriteEnd};

use std::{
    future::Future,
    ops::{Deref, DerefMut},
    pin::Pin,
};

#[derive(Debug)]
pub(super) struct WriteEndWithCachedId {
    pub(super) inner: WriteEnd,
    id: Option<Id>,
}

impl Clone for WriteEndWithCachedId {
    fn clone(&self) -> Self {
        self.inner.get_auxiliary().inc_active_user_count();

        Self {
            inner: self.inner.clone(),
            id: None,
        }
    }
}

impl Drop for WriteEndWithCachedId {
    fn drop(&mut self) {
        self.inner.get_auxiliary().dec_active_user_count();
    }
}

impl Deref for WriteEndWithCachedId {
    type Target = WriteEnd;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for WriteEndWithCachedId {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl WriteEndWithCachedId {
    pub(super) fn new(write_end: WriteEnd) -> Self {
        Self {
            inner: write_end,
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
        F: Future<Output = Result<R, E>> + Send,
        E: Into<Error> + Send,
    {
        let future = async move { future.await.map_err(Into::into) };
        tokio::pin!(future);

        self.cancel_if_task_failed_inner(future).await
    }

    pub(super) async fn cancel_if_task_failed_inner<R>(
        &mut self,
        future: Pin<&mut (dyn Future<Output = Result<R, Error>> + Send)>,
    ) -> Result<R, Error> {
        let cancel_err = || Err(cancel_error());
        let auxiliary = self.inner.get_auxiliary();

        let cancel_token = &auxiliary.cancel_token;

        if cancel_token.is_cancelled() {
            return cancel_err();
        }

        tokio::select! {
            biased;

            _ = cancel_token.cancelled() => cancel_err(),
            res = future => res,
        }
    }

    pub(super) fn get_auxiliary(&self) -> &Auxiliary {
        self.inner.get_auxiliary()
    }
}

impl WriteEndWithCachedId {
    pub(super) async fn send_request<Func, F, R>(&mut self, f: Func) -> Result<R, Error>
    where
        Func: FnOnce(&mut WriteEnd, Id) -> Result<F, Error> + Send,
        F: Future<Output = Result<(Id, R), Error>> + Send + 'static,
    {
        let id = self.get_id_mut();
        let write_end = &mut self.inner;

        let future = f(write_end, id)?;
        tokio::pin!(future);

        async fn inner<R>(
            this: &mut WriteEndWithCachedId,
            future: Pin<&mut (dyn Future<Output = Result<(Id, R), Error>> + Send)>,
        ) -> Result<R, Error> {
            // Requests is already added to write buffer, so wakeup
            // the `flush_task` if necessary.
            this.get_auxiliary().wakeup_flush_task();

            let (id, ret) = this.cancel_if_task_failed(future).await?;

            this.cache_id_mut(id);

            Ok(ret)
        }

        inner(self, future).await
    }
}
