use super::{awaitable::Awaitable, Response};

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use std::io;

use thunderdome::Arena;

pub(crate) type Value = Awaitable<(Response, Box<[u8]>)>;

// TODO: Simplify this

#[derive(Debug, Default)]
pub(crate) struct Responses(Arena<Value>);

impl Responses {
    fn insert_impl(&mut self) -> (u32, Value) {
        let val = Awaitable::new();

        (self.0.insert(val.clone()).slot(), val)
    }

    pub fn insert(&mut self) -> SlotGuard {
        let (slot, response) = self.insert_impl();
        SlotGuard(slot, response)
    }

    pub(crate) async fn do_callback(
        &mut self,
        slot: u32,
        response: Response,
        buffer: Box<[u8]>,
    ) -> io::Result<()> {
        self.remove(slot)
            .expect("Invalid slot")
            .done((response, buffer));

        Ok(())
    }

    /// Precondition: There must not be an ongoing request for `slot`.
    pub(crate) fn remove(&mut self, slot: u32) -> Option<Value> {
        self.0.remove_by_slot(slot).map(|(_index, value)| value)
    }
}

#[derive(Debug)]
pub(crate) struct SlotGuard(u32, Value);

impl SlotGuard {
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
