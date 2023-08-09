use crate::{
    cancel_error,
    lowlevel::NameEntry,
    metadata::{FileType, MetaData},
    Error,
};

use super::Dir;

use std::{
    borrow::Cow,
    future::Future,
    path::Path,
    pin::Pin,
    task::{ready, Context, Poll},
    vec::IntoIter,
};

use futures_core::stream::{FusedStream, Stream};
use pin_project::{pin_project, pinned_drop};
use tokio_util::sync::WaitForCancellationFutureOwned;

type ResponseFuture = crate::lowlevel::AwaitableNameEntriesFuture<crate::Buffer>;

/// Entries returned by the [`ReadDir`].
///
/// This is a specialized version of [`std::fs::DirEntry`].
#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct DirEntry(NameEntry);

impl DirEntry {
    /// Return filename of the dir entry.
    pub fn filename(&self) -> &Path {
        &self.0.filename
    }

    /// Return filename of the dir entry as a mutable reference.
    pub fn filename_mut(&mut self) -> &mut Box<Path> {
        &mut self.0.filename
    }

    /// Return metadata for the dir entry.
    pub fn metadata(&self) -> MetaData {
        MetaData::new(self.0.attrs)
    }

    /// Return the file type for the dir entry.
    pub fn file_type(&self) -> Option<FileType> {
        self.metadata().file_type()
    }
}

/// Reads the the entries in a directory.
#[derive(Debug)]
#[pin_project(PinnedDrop)]
pub struct ReadDir {
    dir: Dir,

    // future and entries contain the state
    //
    // Invariant:
    //  - entries.is_none() => future.is_none()
    //  - If entries.is_some(), then future.is_none() ^ entries.unwrap().as_slice().is_empty()
    future: Option<ResponseFuture>,
    entries: Option<IntoIter<NameEntry>>,

    /// cancellation_fut is not only cancel-safe, but also can be polled after
    /// it is ready.
    ///
    /// Once it is ready, all polls after that immediately return Poll::Ready(())
    #[pin]
    cancellation_fut: WaitForCancellationFutureOwned,
}

impl ReadDir {
    pub(super) fn new(dir: Dir) -> Self {
        Self {
            cancellation_fut: dir.0.get_auxiliary().cancel_token.clone().cancelled_owned(),
            dir,
            future: None,
            entries: Some(Vec::new().into_iter()),
        }
    }

    fn new_request(dir: &mut Dir) -> Result<ResponseFuture, Error> {
        let owned_handle = &mut dir.0;

        let id = owned_handle.get_id_mut();
        let handle = &owned_handle.handle;
        let write_end = &mut owned_handle.write_end.inner;

        let future = write_end
            .send_readdir_request(id, Cow::Borrowed(handle))?
            .wait();

        // Requests is already added to write buffer, so wakeup
        // the `flush_task` if necessary.
        owned_handle.get_auxiliary().wakeup_flush_task();

        Ok(future)
    }
}

impl Stream for ReadDir {
    type Item = Result<DirEntry, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        let future = this.future;

        let entries = match &mut *this.entries {
            Some(entries) => entries,
            None => return Poll::Ready(None),
        };

        if entries.as_slice().is_empty() {
            let dir = &mut *this.dir;
            let cancellation_fut = this.cancellation_fut;

            let fut = match future {
                Some(future) => future,
                None => {
                    *future = Some(Self::new_request(dir)?);
                    future.as_mut().unwrap()
                }
            };

            let res = {
                let fut = async move {
                    tokio::select! {
                        biased;

                        _ = cancellation_fut => Err(cancel_error()),
                        res = fut => res,
                    }
                };

                tokio::pin!(fut);

                ready!(fut.poll(cx))
            };
            *future = None; // future is ready, reset it to None
            let (id, ret) = res?;

            this.dir.0.cache_id_mut(id);
            if ret.is_empty() {
                *this.entries = None;
                return Poll::Ready(None);
            } else {
                *entries = Vec::from(ret).into_iter();
            }
        }

        debug_assert!(future.is_none());
        debug_assert!(!entries.as_slice().is_empty());

        Poll::Ready(entries.next().map(DirEntry).map(Ok))
    }
}

impl FusedStream for ReadDir {
    fn is_terminated(&self) -> bool {
        self.entries.is_none()
    }
}

impl ReadDir {
    async fn do_drop(mut dir: Dir, future: Option<ResponseFuture>) {
        if let Some(future) = future {
            if let Ok((id, _)) = future.await {
                dir.0.cache_id_mut(id);
            }
        }

        if let Err(_err) = dir.close().await {
            #[cfg(feature = "tracing")]
            tracing::error!(?_err, "failed to close handle");
        }
    }
}

/// We need to keep polling the future stored internally, otherwise it would
/// drop the internal request ids too early, causing read task to fail
/// when they should not fail.
#[pinned_drop]
impl PinnedDrop for ReadDir {
    fn drop(self: Pin<&mut Self>) {
        let this = self.project();

        let dir = this.dir.clone();
        let future = this.future.take();

        let cancellation_fut = dir.0.get_auxiliary().cancel_token.clone().cancelled_owned();
        let do_drop_fut = Self::do_drop(dir, future);

        this.dir.0.get_auxiliary().tokio_handle().spawn(async move {
            tokio::select! {
                biased;

                _ = cancellation_fut => (),
                _ = do_drop_fut => (),
            }
        });
    }
}
