#![forbid(unsafe_code)]

use crate::*;
use awaitable_responses::ArenaArc;
use awaitable_responses::Awaitable;
use awaitable_responses::Response;

use core::fmt::Debug;
use core::mem::replace;

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use std::path::Path;

use openssh_sftp_protocol::file_attrs::FileAttrs;
use openssh_sftp_protocol::response::{NameEntry, ResponseInner, StatusCode};
use openssh_sftp_protocol::ssh_format;
use openssh_sftp_protocol::HandleOwned;

use derive_destructure2::destructure;

#[derive(Debug, Clone)]
pub enum Data<Buffer: ToBuffer> {
    /// The buffer that stores the response of Read.
    ///
    /// It will be returned if you provided a buffer to
    /// [`crate::WriteEnd::send_read_request`].
    Buffer(Buffer),

    /// This is a fallback that is returned
    /// if `Buffer` isn't provided or it isn't large enough.
    AllocatedBox(Box<[u8]>),

    /// EOF is reached before any data can be read.
    Eof,
}

/// Provides drop impl
///
/// Store `ArenaArc` instead of `Id` or `IdInner` to have more control
/// over removal of `ArenaArc`.
#[derive(Debug, destructure)]
struct AwaitableInner<Buffer: ToBuffer + Debug + Send + Sync>(ArenaArc<Buffer>);

impl<Buffer: ToBuffer + Debug + Send + Sync> AwaitableInner<Buffer> {
    async fn wait_impl(self) -> Result<(Id<Buffer>, Response<Buffer>), Error> {
        struct WaitFuture<'a, Buffer: ToBuffer>(Option<&'a Awaitable<Buffer>>);

        impl<Buffer: ToBuffer + Debug> Future for WaitFuture<'_, Buffer> {
            type Output = ();

            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                if let Some(value) = self.0.take() {
                    let waker = cx.waker().clone();

                    let res = value
                        .install_waker(waker)
                        .expect("AwaitableResponse should either in state Ongoing or Done");

                    if res {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    }
                } else {
                    Poll::Ready(())
                }
            }
        }

        WaitFuture(Some(&self.0)).await;

        let response = self
            .0
            .take_output()
            .expect("The request should be done by now");

        // Reconstruct Id here so that it will be automatically
        // released on error.
        let id = Id::new(self.destructure().0);

        // Propagate failure
        match response {
            Response::Header(ResponseInner::Status {
                status_code: StatusCode::Failure(err_code),
                err_msg,
            }) => Err(Error::SftpError(err_code, err_msg)),

            response => Ok((id, response)),
        }
    }

    async fn wait<T>(
        self,
        post_processing: fn(Response<Buffer>) -> Result<T, Error>,
    ) -> Result<(Id<Buffer>, T), Error> {
        self.wait_impl()
            .await
            .and_then(|(id, response)| Ok((id, post_processing(response)?)))
    }
}

impl<Buffer: ToBuffer + Debug + Send + Sync> Drop for AwaitableInner<Buffer> {
    fn drop(&mut self) {
        // Remove ArenaArc only if the `AwaitableResponse` is done.
        //
        // If the ArenaArc is in `Consumed` state, then the user cannot have the future
        // cancelled unless they played with fire (`unsafe`).
        if self.0.is_done() {
            ArenaArc::remove(&self.0);
        }
    }
}

macro_rules! def_awaitable {
    ($name:ident, $res:ty, | $response_name:ident | $post_processing:block) => {
        #[derive(Debug)]
        pub struct $name<Buffer: ToBuffer + Debug + Send + Sync>(AwaitableInner<Buffer>);

        impl<Buffer: ToBuffer + Debug + Send + Sync> $name<Buffer> {
            pub(crate) fn new(arc: ArenaArc<Buffer>) -> Self {
                Self(AwaitableInner(arc))
            }

            /// Return (id, res).
            ///
            /// id can be reused in the next request.
            ///
            /// # Cancel Safety
            ///
            /// It is perfectly safe to cancel the future.
            pub async fn wait(self) -> Result<(Id<Buffer>, $res), Error> {
                self.0.wait(|$response_name| $post_processing).await
            }
        }
    };
}

def_awaitable!(AwaitableStatus, (), |response| {
    match response {
        Response::Header(ResponseInner::Status {
            status_code: StatusCode::Success,
            ..
        }) => Ok(()),
        _ => Err(Error::InvalidResponse(&"Expected Status response")),
    }
});

def_awaitable!(AwaitableHandle, HandleOwned, |response| {
    match response {
        Response::Header(ResponseInner::Handle(handle)) => {
            if handle.into_inner().len() > 256 {
                Err(Error::HandleTooLong)
            } else {
                Ok(handle)
            }
        }
        _ => Err(Error::InvalidResponse(
            &"Expected Handle or err Status response",
        )),
    }
});

def_awaitable!(AwaitableData, Data<Buffer>, |response| {
    match response {
        Response::Buffer(buffer) => Ok(Data::Buffer(buffer)),
        Response::AllocatedBox(allocated_box) => Ok(Data::AllocatedBox(allocated_box)),
        Response::Header(ResponseInner::Status {
            status_code: StatusCode::Eof,
            ..
        }) => Ok(Data::Eof),
        _ => Err(Error::InvalidResponse(
            &"Expected Buffer/AllocatedBox response",
        )),
    }
});

def_awaitable!(AwaitableNameEntries, Box<[NameEntry]>, |response| {
    match response {
        Response::Header(response_inner) => match response_inner {
            ResponseInner::Name(name) => Ok(name),
            ResponseInner::Status {
                status_code: StatusCode::Eof,
                ..
            } => Ok(Vec::new().into_boxed_slice()),

            _ => Err(Error::InvalidResponse(
                &"Expected Name or err Status response",
            )),
        },
        _ => Err(Error::InvalidResponse(
            &"Expected Name or err Status response",
        )),
    }
});

def_awaitable!(AwaitableAttrs, FileAttrs, |response| {
    match response {
        Response::Header(ResponseInner::Attrs(attrs)) => Ok(*attrs),
        _ => Err(Error::InvalidResponse(
            &"Expected Attrs or err Status response",
        )),
    }
});

def_awaitable!(AwaitableName, Box<Path>, |response| {
    match response {
        Response::Header(ResponseInner::Name(mut names)) => {
            if names.len() != 1 {
                Err(Error::InvalidResponse(
                    &"Got expected Name response, but it does not have exactly \
                        one and only one entry",
                ))
            } else {
                let name = &mut names[0];

                Ok(replace(&mut name.filename, Path::new("").into()))
            }
        }
        _ => Err(Error::InvalidResponse(
            &"Expected Name or err Status response",
        )),
    }
});

def_awaitable!(AwaitableLimits, Limits, |response| {
    match response {
        Response::ExtendedReply(boxed) => Ok(ssh_format::from_bytes(&boxed)?.0),
        _ => Err(Error::InvalidResponse(&"Expected extended reply response")),
    }
});
