use super::{awaitable::Awaitable, Response};

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use thunderdome::Arena;

pub(crate) type Value = Awaitable<(Response, Box<[u8]>)>;

#[derive(Debug, Default)]
pub(crate) struct AwaitableResponses(Arena<Value>);

impl AwaitableResponses {
    pub fn insert(&mut self) -> AwaitableResponse {
        let awaitable_response = Awaitable::new();

        AwaitableResponse(
            self.0.insert(awaitable_response.clone()).slot(),
            awaitable_response,
        )
    }

    pub(crate) async fn do_callback(&mut self, slot: u32, response: Response, buffer: Box<[u8]>) {
        self.remove(slot)
            .expect("Invalid slot")
            .done((response, buffer));
    }

    /// Precondition: There must not be an ongoing request for `slot`.
    pub(crate) fn remove(&mut self, slot: u32) -> Option<Value> {
        self.0
            .remove_by_slot(slot)
            .map(|(_index, awaitable_response)| awaitable_response)
    }
}

#[derive(Debug)]
pub(crate) struct AwaitableResponse(u32, Value);

impl AwaitableResponse {
    pub(crate) fn get_slot_id(&self) -> u32 {
        self.0
    }

    pub(crate) async fn wait(self) -> (Response, Box<[u8]>) {
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

        WaitFuture(Some(&self.1)).await;

        self.1
            .get_value()
            .expect("The request should be done by now")
    }
}
