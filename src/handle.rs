use super::{
    lowlevel::{Handle, HandleOwned},
    {Error, Id, WriteEnd, WriteEndWithCachedId},
};

use std::{
    borrow::Cow,
    future::Future,
    ops::{Deref, DerefMut},
    sync::Arc,
};

use derive_destructure2::destructure;

/// Remote Directory
#[derive(Debug, Clone, destructure)]
pub(super) struct OwnedHandle {
    pub(super) write_end: WriteEndWithCachedId,
    pub(super) handle: Arc<HandleOwned>,
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        let write_end = &mut self.write_end;
        let handle = &self.handle;

        if Arc::strong_count(handle) == 1 {
            // This is the last reference to the arc
            let id = write_end.get_id_mut();
            if let Ok(response) = write_end.send_close_request(id, Cow::Borrowed(handle)) {
                // Requests is already added to write buffer, so wakeup
                // the `flush_task`.
                self.get_auxiliary().wakeup_flush_task();
                tokio::spawn(async move {
                    #[cfg(not(feature = "tracing"))]
                    {
                        let _ = response.wait().await;
                    }
                    #[cfg(feature = "tracing")]
                    {
                        let res = response.wait().await;
                        match res {
                            Ok(_) => tracing::debug!("close handle success"),
                            Err(err) => tracing::error!(?err, "failed to close handle"),
                        }
                    }
                });
            }
        }
    }
}

impl OwnedHandle {
    pub(super) fn new(write_end: WriteEndWithCachedId, handle: HandleOwned) -> Self {
        Self {
            write_end,
            handle: Arc::new(handle),
        }
    }

    pub(super) async fn send_request<Func, F, R>(&mut self, f: Func) -> Result<R, Error>
    where
        Func: FnOnce(&mut WriteEnd, Cow<'_, Handle>, Id) -> Result<F, Error> + Send,
        F: Future<Output = Result<(Id, R), Error>> + Send + 'static,
        R: Send,
    {
        let handle = &self.handle;

        self.write_end
            .send_request(|write_end, id| f(write_end, Cow::Borrowed(handle), id))
            .await
    }

    /// Close the [`OwnedHandle`], send the close request
    /// if this is the last reference.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe.
    pub(super) async fn close(self) -> Result<(), Error> {
        if Arc::strong_count(&self.handle) == 1 {
            // This is the last reference to the arc

            // Release resources without running `Drop::drop`
            let (mut write_end, handle) = self.destructure();

            write_end
                .send_request(|write_end, id| {
                    Ok(write_end
                        .send_close_request(id, Cow::Borrowed(&handle))?
                        .wait())
                })
                .await
        } else {
            Ok(())
        }
    }
}

impl Deref for OwnedHandle {
    type Target = WriteEndWithCachedId;

    fn deref(&self) -> &Self::Target {
        &self.write_end
    }
}

impl DerefMut for OwnedHandle {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.write_end
    }
}
