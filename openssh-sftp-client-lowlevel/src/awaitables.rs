#![forbid(unsafe_code)]

use super::*;
use awaitable_responses::ArenaArc;
use awaitable_responses::Response;

use std::fmt::Debug;
use std::future::Future;
use std::mem::replace;
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::openssh_sftp_protocol::file_attrs::FileAttrs;
use crate::openssh_sftp_protocol::response::{NameEntry, ResponseInner, StatusCode};
use crate::openssh_sftp_protocol::ssh_format;
use crate::openssh_sftp_protocol::HandleOwned;
use derive_destructure2::destructure;

/// The data returned by [`WriteEnd::send_read_request`].
#[derive(Debug, Clone)]
pub enum Data<Buffer> {
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

type AwaitableInnerRes<Buffer> = (Id<Buffer>, Response<Buffer>);

#[repr(transparent)]
#[derive(Debug)]
struct AwaitableInnerFuture<Buffer: Send + Sync>(Option<AwaitableInner<Buffer>>);

impl<Buffer: Send + Sync> AwaitableInnerFuture<Buffer> {
    fn new(awaitable_inner: AwaitableInner<Buffer>) -> Self {
        Self(Some(awaitable_inner))
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Result<AwaitableInnerRes<Buffer>, Error>> {
        let errmsg = "AwaitableInnerFuture::poll is called after completed";

        let waker = cx.waker().clone();

        let res = self
            .0
            .as_ref()
            .expect(errmsg)
            .0
            .install_waker(waker)
            .expect("AwaitableResponse should either in state Ongoing or Done");

        if !res {
            return Poll::Pending;
        }

        let awaitable = self.0.take().expect(errmsg);

        let response = awaitable
            .0
            .take_output()
            .expect("The request should be done by now");

        // Reconstruct Id here so that it will be automatically
        // released on error.
        let id = Id::new(awaitable.destructure().0);

        // Propagate failure
        Poll::Ready(match response {
            Response::Header(ResponseInner::Status {
                status_code: StatusCode::Failure(err_code),
                err_msg,
            }) => Err(Error::SftpError(err_code, err_msg)),

            response => Ok((id, response)),
        })
    }
}

/// Provides drop impl
///
/// Store `ArenaArc` instead of `Id` or `IdInner` to have more control
/// over removal of `ArenaArc`.
#[repr(transparent)]
#[derive(Debug, destructure)]
struct AwaitableInner<Buffer: Send + Sync>(ArenaArc<Buffer>);

impl<Buffer: Send + Sync> Drop for AwaitableInner<Buffer> {
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
    ($name:ident, $future_name:ident, $res:ty, | $response_name:ident | $post_processing:block) => {
        /// Return (id, res).
        ///
        /// id can be reused in the next request.
        ///
        /// # Cancel Safety
        ///
        /// It is perfectly safe to cancel the future.
        #[repr(transparent)]
        #[derive(Debug)]
        pub struct $future_name<Buffer: Send + Sync>(AwaitableInnerFuture<Buffer>);

        impl<Buffer: Send + Sync> Future for $future_name<Buffer> {
            type Output = Result<(Id<Buffer>, $res), Error>;

            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                let post_processing = |$response_name: Response<Buffer>| $post_processing;

                self.0.poll(cx).map(|res| {
                    let (id, response) = res?;
                    Ok((id, post_processing(response)?))
                })
            }
        }

        /// Awaitable
        #[repr(transparent)]
        #[derive(Debug)]
        pub struct $name<Buffer: Send + Sync>(AwaitableInner<Buffer>);

        impl<Buffer: Send + Sync> $name<Buffer> {
            #[inline(always)]
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
            pub fn wait(self) -> $future_name<Buffer> {
                $future_name(AwaitableInnerFuture::new(self.0))
            }
        }
    };
}

def_awaitable!(AwaitableStatus, AwaitableStatusFuture, (), |response| {
    match response {
        Response::Header(ResponseInner::Status {
            status_code: StatusCode::Success,
            ..
        }) => Ok(()),
        _ => Err(Error::InvalidResponse(&"Expected Status response")),
    }
});

def_awaitable!(
    AwaitableHandle,
    AwaitableHandleFuture,
    HandleOwned,
    |response| {
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
    }
);

def_awaitable!(
    AwaitableData,
    AwaitableDataFuture,
    Data<Buffer>,
    |response| {
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
    }
);

def_awaitable!(
    AwaitableNameEntries,
    AwaitableNameEntriesFuture,
    Box<[NameEntry]>,
    |response| {
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
    }
);

def_awaitable!(
    AwaitableAttrs,
    AwaitableAttrsFuture,
    FileAttrs,
    |response| {
        match response {
            Response::Header(ResponseInner::Attrs(attrs)) => Ok(attrs),
            _ => Err(Error::InvalidResponse(
                &"Expected Attrs or err Status response",
            )),
        }
    }
);

def_awaitable!(AwaitableName, AwaitableNameFuture, Box<Path>, |response| {
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

def_awaitable!(AwaitableLimits, AwaitableLimitsFuture, Limits, |response| {
    match response {
        Response::ExtendedReply(boxed) => Ok(ssh_format::from_bytes(&boxed)?.0),
        _ => Err(Error::InvalidResponse(&"Expected extended reply response")),
    }
});
