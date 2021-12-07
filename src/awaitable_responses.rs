use super::awaitable::{Awaitable, AwaitableFactory};
use super::Error;
use super::ToBuffer;

use core::fmt::{Debug, Display};
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use openssh_sftp_protocol::response::ResponseInner;
use parking_lot::Mutex;
use thunderdome::Arena;

#[derive(Debug, Clone)]
pub enum Response<Buffer: ToBuffer> {
    Header(ResponseInner),

    /// The buffer that stores the response of Read,
    /// since its corresponding response type `ResponseInner::Data`
    /// does not contain any member, it doesn't have to be stored.
    Buffer(Buffer),

    /// Same as `Buffer`, this is a fallback
    /// if `Buffer` isn't provided or it isn't large enough.
    AllocatedBox(Box<[u8]>),
}

impl<Buffer: ToBuffer> Response<Buffer> {
    pub fn expect_header<T: Display>(self, err_msg: T) -> ResponseInner {
        match self {
            Response::Header(response) => response,
            _ => panic!("{}", err_msg),
        }
    }

    pub fn expect_buffer<T: Display>(self, err_msg: T) -> Buffer {
        match self {
            Response::Buffer(buffer) => buffer,
            _ => panic!("{}", err_msg),
        }
    }

    pub fn expect_alloated_box<T: Display>(self, err_msg: T) -> Box<[u8]> {
        match self {
            Response::AllocatedBox(allocated_box) => allocated_box,
            _ => panic!("{}", err_msg),
        }
    }

    /// Return true if `self` contains `Response::Buffer`.
    pub fn is_buffer(&self) -> bool {
        matches!(self, Response::Buffer(_))
    }
}

pub(crate) type Value<Buffer> = Awaitable<Buffer, Response<Buffer>>;

#[derive(Debug)]
pub(crate) struct AwaitableResponseFactory<Buffer: ToBuffer + 'static>(
    AwaitableFactory<Buffer, Response<Buffer>>,
);

impl<Buffer: ToBuffer + 'static> AwaitableResponseFactory<Buffer> {
    pub(crate) fn new() -> Self {
        Self(AwaitableFactory::new())
    }

    pub(crate) fn create(&self) -> AwaitableResponses<Buffer> {
        AwaitableResponses(Mutex::new(Arena::new()), self.0.clone())
    }
}

impl<Buffer: ToBuffer + 'static> Clone for AwaitableResponseFactory<Buffer> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

#[derive(Debug)]
pub(crate) struct AwaitableResponses<Buffer: ToBuffer + 'static>(
    Mutex<Arena<Value<Buffer>>>,
    AwaitableFactory<Buffer, Response<Buffer>>,
);

impl<Buffer: Debug + ToBuffer> AwaitableResponses<Buffer> {
    /// Return (slot_id, awaitable_response)
    pub(crate) fn insert(&self, buffer: Option<Buffer>) -> (u32, AwaitableResponse<Buffer>) {
        let awaitable_response = self.1.create(buffer);

        let awaitable_response_clone = awaitable_response.clone();
        let res = self.0.lock().insert(awaitable_response_clone);

        (res.slot(), AwaitableResponse(awaitable_response))
    }

    pub(crate) fn remove(&self, slot: u32) -> Result<AwaitableResponse<Buffer>, Error> {
        let res = self.0.lock().remove_by_slot(slot);
        res.map(|(_index, awaitable_response)| AwaitableResponse(awaitable_response))
            .ok_or(Error::InvalidResponseId)
    }
}

#[derive(Debug)]
pub struct AwaitableResponse<Buffer: ToBuffer>(Value<Buffer>);

impl<Buffer: ToBuffer + Debug> AwaitableResponse<Buffer> {
    pub(crate) fn get_input(&self) -> Option<Buffer> {
        self.0.take_input()
    }

    pub(crate) fn do_callback(self, response: Response<Buffer>) {
        self.0.done(response);
    }

    pub async fn wait(self) -> Response<Buffer> {
        struct WaitFuture<'a, Buffer: ToBuffer>(Option<&'a Value<Buffer>>);

        impl<Buffer: ToBuffer + Debug> Future for WaitFuture<'_, Buffer> {
            type Output = ();

            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                if let Some(value) = self.0.take() {
                    let waker = cx.waker().clone();

                    if value.install_waker(waker) {
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

        self.0
            .take_output()
            .expect("The request should be done by now")
    }
}
