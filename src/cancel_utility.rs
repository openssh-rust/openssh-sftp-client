use super::{Auxiliary, Error};

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use tokio_util::sync::WaitForCancellationFuture;

/// lifetime 's is reference to `sftp::Sftp`
#[repr(transparent)]
#[derive(Debug)]
pub(super) struct BoxedWaitForCancellationFuture<'s>(
    /// WaitForCancellationFuture is boxed since it stores an intrusive node
    /// inline, which is removed from waitlist on drop.
    ///
    /// However, in rust, leaking is permitted, thus we have to box it.
    Option<Pin<Box<WaitForCancellationFuture<'s>>>>,
);

impl<'s> BoxedWaitForCancellationFuture<'s> {
    pub(super) fn new() -> Self {
        Self(None)
    }

    pub(super) fn cancel_error() -> Error {
        Error::BackgroundTaskFailure(&"read/flush task failed")
    }

    fn get_future(&mut self, auxiliary: &'s Auxiliary) -> Pin<&mut WaitForCancellationFuture<'s>> {
        if self.0.is_none() {
            self.0 = Some(Box::pin(auxiliary.cancel_token.cancelled()));
        }

        self.0
            .as_mut()
            .expect("self.0 is just set to Some")
            .as_mut()
    }

    /// Return `Ok(())` if the task hasn't failed yet and the context has
    /// already been registered.
    pub(super) fn poll_for_task_failure(
        &mut self,
        cx: &mut Context<'_>,
        auxiliary: &'s Auxiliary,
    ) -> Result<(), Error> {
        if auxiliary.cancel_token.is_cancelled() {
            return Err(Self::cancel_error());
        }

        match self.get_future(auxiliary).poll(cx) {
            Poll::Ready(_) => {
                // Drop future since a completed future cannot be called again.
                self.0 = None;

                Err(Self::cancel_error())
            }
            Poll::Pending => Ok(()),
        }
    }
}
