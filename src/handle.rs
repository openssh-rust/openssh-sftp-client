use super::lowlevel::{Handle, HandleOwned};
use super::{Error, Id, WriteEnd, WriteEndWithCachedId};

use std::borrow::Cow;
use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use derive_destructure2::destructure;

/// Remote Directory
#[derive(Debug, destructure)]
pub(super) struct OwnedHandle<'s> {
    pub(super) write_end: WriteEndWithCachedId<'s>,
    pub(super) handle: Arc<HandleOwned>,
}

impl Clone for OwnedHandle<'_> {
    fn clone(&self) -> Self {
        Self {
            write_end: self.write_end.clone(),
            handle: self.handle.clone(),
        }
    }
}

impl Drop for OwnedHandle<'_> {
    fn drop(&mut self) {
        let write_end = &mut self.write_end;
        let handle = &self.handle;

        if Arc::strong_count(handle) == 1 {
            // This is the last reference to the arc
            let id = write_end.get_id_mut();
            let _ = write_end.send_close_request(id, Cow::Borrowed(handle));

            // Requests is already added to write buffer, so wakeup
            // the `flush_task`.
            self.get_auxiliary().wakeup_flush_task();
        }
    }
}

impl<'s> OwnedHandle<'s> {
    pub(super) fn new(write_end: WriteEndWithCachedId<'s>, handle: HandleOwned) -> Self {
        Self {
            write_end,
            handle: Arc::new(handle),
        }
    }

    pub(super) async fn send_request<Func, F, R>(&mut self, f: Func) -> Result<R, Error>
    where
        Func: FnOnce(&mut WriteEnd, Cow<'_, Handle>, Id) -> Result<F, Error>,
        F: Future<Output = Result<(Id, R), Error>> + 'static,
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

impl<'s> Deref for OwnedHandle<'s> {
    type Target = WriteEndWithCachedId<'s>;

    fn deref(&self) -> &Self::Target {
        &self.write_end
    }
}

impl DerefMut for OwnedHandle<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.write_end
    }
}
