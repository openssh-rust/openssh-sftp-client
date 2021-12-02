use super::awaitable::Awaitable;

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use openssh_sftp_protocol::response::ResponseInner;
use thunderdome::Arena;

#[derive(Debug, Clone)]
pub enum Response {
    Header(ResponseInner),

    /// The buffer that stores the response of Read,
    /// since its corresponding response type `ResponseInner::Data`
    /// does not contain any member, it doesn't have to be stored.
    Buffer(Box<[u8]>),
}

pub(crate) type Value = Awaitable<Response>;

#[derive(Debug, Default)]
pub(crate) struct AwaitableResponses(Arena<Value>);

impl AwaitableResponses {
    /// Return (slot_id, awaitable_response)
    pub(crate) fn insert(&mut self) -> (u32, AwaitableResponse) {
        let awaitable_response = Awaitable::new();

        (
            self.0.insert(awaitable_response.clone()).slot(),
            AwaitableResponse(awaitable_response),
        )
    }

    pub(crate) async fn do_callback(&mut self, slot: u32, response: Response) {
        self.remove(slot).expect("Invalid slot").done(response);
    }

    /// Precondition: There must not be an ongoing request for `slot`.
    pub(crate) fn remove(&mut self, slot: u32) -> Option<Value> {
        self.0
            .remove_by_slot(slot)
            .map(|(_index, awaitable_response)| awaitable_response)
    }
}

#[derive(Debug)]
pub struct AwaitableResponse(Value);

impl AwaitableResponse {
    pub async fn wait(self) -> Response {
        struct WaitFuture<'a>(Option<&'a Value>);

        impl Future for WaitFuture<'_> {
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
