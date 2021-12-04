use super::awaitable::Awaitable;
use super::ToBuffer;

use core::fmt::Debug;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use openssh_sftp_protocol::response::ResponseInner;
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

pub(crate) type Value<Buffer> = Awaitable<Buffer, Response<Buffer>>;

#[derive(Debug)]
pub(crate) struct AwaitableResponses<Buffer: ToBuffer>(Arena<Value<Buffer>>);

impl<Buffer: Debug + ToBuffer> AwaitableResponses<Buffer> {
    pub(crate) fn new() -> Self {
        Self(Arena::new())
    }

    /// Return (slot_id, awaitable_response)
    pub(crate) fn insert(&mut self, buffer: Option<Buffer>) -> (u32, AwaitableResponse<Buffer>) {
        let awaitable_response = Awaitable::new(buffer);

        (
            self.0.insert(awaitable_response.clone()).slot(),
            AwaitableResponse(awaitable_response),
        )
    }

    pub(crate) fn get_input(&self, slot: u32) -> Option<Buffer> {
        self.0
            .get_by_slot(slot)
            .expect("Invalid slot")
            .1
            .take_input()
    }

    pub(crate) fn do_callback(&mut self, slot: u32, response: Response<Buffer>) {
        self.remove(slot).expect("Invalid slot").done(response);
    }

    /// Precondition: There must not be an ongoing request for `slot`.
    pub(crate) fn remove(&mut self, slot: u32) -> Option<Value<Buffer>> {
        self.0
            .remove_by_slot(slot)
            .map(|(_index, awaitable_response)| awaitable_response)
    }
}

#[derive(Debug)]
pub struct AwaitableResponse<Buffer: ToBuffer>(Value<Buffer>);

impl<Buffer: ToBuffer + Debug> AwaitableResponse<Buffer> {
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
            .get_value()
            .expect("The request should be done by now")
    }
}
