use crate::awaitable_responses::ArenaArc;
use crate::awaitable_responses::Awaitable;
use crate::awaitable_responses::Response;
use crate::Error;
use crate::Id;
use crate::ToBuffer;

use core::fmt::Debug;
use core::mem::replace;

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use std::path::Path;

use openssh_sftp_protocol::file_attrs::FileAttrs;
use openssh_sftp_protocol::response::*;
use openssh_sftp_protocol::HandleOwned;

use derive_destructure2::destructure;

#[derive(Debug, Clone)]
pub enum Data<Buffer: ToBuffer> {
    /// The buffer that stores the response of Read,
    /// since its corresponding response type `ResponseInner::Data`
    /// does not contain any member, it doesn't have to be stored.
    Buffer(Buffer),

    /// Same as `Buffer`, this is a fallback
    /// if `Buffer` isn't provided or it isn't large enough.
    AllocatedBox(Box<[u8]>),

    /// EOF is reached before any data can be read.
    Eof,
}

#[derive(Debug, Clone)]
pub struct Name {
    pub filename: Box<Path>,
    pub longname: Box<str>,
}

/// Provides drop impl
///
/// Store `ArenaArc` instead of `Id` or `IdInner` to have more control
/// over removal of `ArenaArc`.
#[derive(Debug, destructure)]
struct AwaitableInner<Buffer: ToBuffer + Debug + Send + Sync>(ArenaArc<Buffer>);

impl<Buffer: ToBuffer + Debug + Send + Sync> AwaitableInner<Buffer> {
    async fn wait(&self) {
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
    }

    fn into_inner(self) -> ArenaArc<Buffer> {
        self.destructure().0
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
            pub async fn wait(self) -> Result<(Id<Buffer>, $res), Error> {
                let post_processing = |$response_name| $post_processing;

                self.0.wait().await;

                let response = self
                    .0
                     .0
                    .take_output()
                    .expect("The request should be done by now");

                let res = post_processing(response)?;

                Ok((Id::new(self.0.into_inner()), res))
            }
        }
    };
}

def_awaitable!(AwaitableStatus, (), |response| {
    match response {
        Response::Header(ResponseInner::Status {
            status_code,
            err_msg,
        }) => match status_code {
            StatusCode::Success => Ok(()),
            StatusCode::Failure(err_code) => Err(Error::SftpError(err_code, err_msg)),
        },
        _ => Err(Error::InvalidResponse(&"Expected Status response")),
    }
});

def_awaitable!(AwaitableHandle, HandleOwned, |response| {
    match response {
        Response::Header(response_inner) => match response_inner {
            ResponseInner::Handle(handle) => Ok(handle),
            ResponseInner::Status {
                status_code: StatusCode::Failure(err_code),
                err_msg,
            } => Err(Error::SftpError(err_code, err_msg)),

            _ => Err(Error::InvalidResponse(
                &"Expected Handle or err Status response",
            )),
        },
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
            status_code: StatusCode::Failure(err_code),
            err_msg,
        }) => match err_code {
            ErrorCode::Eof => Ok(Data::Eof),
            _ => Err(Error::SftpError(err_code, err_msg)),
        },
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
                status_code: StatusCode::Failure(err_code),
                err_msg,
            } => Err(Error::SftpError(err_code, err_msg)),

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
        Response::Header(response_inner) => match response_inner {
            // use replace to avoid allocation that might occur due to
            // `FileAttrs::clone`.
            ResponseInner::Attrs(mut attrs) => Ok(replace(&mut *attrs, FileAttrs::new())),
            ResponseInner::Status {
                status_code: StatusCode::Failure(err_code),
                err_msg,
            } => Err(Error::SftpError(err_code, err_msg)),

            _ => Err(Error::InvalidResponse(
                &"Expected Attrs or err Status response",
            )),
        },
        _ => Err(Error::InvalidResponse(
            &"Expected Attrs or err Status response",
        )),
    }
});

def_awaitable!(AwaitableName, Name, |response| {
    match response {
        Response::Header(response_inner) => match response_inner {
            ResponseInner::Name(mut names) => {
                if names.len() != 1 {
                    Err(Error::InvalidResponse(
                        &"Got expected Name response, but it does not have exactly \
                        one and only one entry",
                    ))
                } else {
                    let name = &mut names[0];

                    Ok(Name {
                        filename: replace(&mut name.filename, Path::new("").into()),
                        longname: replace(&mut name.longname, "".into()),
                    })
                }
            }
            ResponseInner::Status {
                status_code: StatusCode::Failure(err_code),
                err_msg,
            } => Err(Error::SftpError(err_code, err_msg)),

            _ => Err(Error::InvalidResponse(
                &"Expected Name or err Status response",
            )),
        },
        _ => Err(Error::InvalidResponse(
            &"Expected Name or err Status response",
        )),
    }
});
