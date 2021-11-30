use super::{awaitable::Awaitable, Response};

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use std::io;

use parking_lot::RwLock;
use thunderdome::Arena;

pub(crate) type Value = Option<Awaitable<(Response, Box<[u8]>)>>;

// TODO: Simplify this

#[derive(Debug, Default)]
pub(crate) struct Responses(RwLock<Arena<Value>>);

impl Responses {
    fn insert_impl(&self, waitable: bool) -> u32 {
        let val = if waitable {
            Some(Awaitable::new())
        } else {
            None
        };

        self.0.write().insert(val).slot()
    }

    async fn wait_impl(&self, slot: u32) {
        struct WaitFuture<'a>(Option<&'a RwLock<Arena<Value>>>, u32);

        impl Future for WaitFuture<'_> {
            type Output = ();

            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                let rwlock = if let Some(rwlock) = self.0.take() {
                    rwlock
                } else {
                    return Poll::Ready(());
                };

                let waker = cx.waker().clone();

                let guard = rwlock.read();
                let (_index, value) = guard.get_by_slot(self.1).expect("Invalid slot");

                if value.as_ref().unwrap().install_waker(waker) {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }
        }

        WaitFuture(Some(&self.0), slot).await;
    }

    pub fn insert(&self) -> SlotGuard {
        SlotGuard(Some(self), self.insert_impl(true))
    }

    /// It is recommended to use this function in `Drop::drop` implementation
    /// since it does not require any `.await`.
    pub fn insert_no_await(&self) -> SlotGuardNoWait {
        SlotGuardNoWait(self, self.insert_impl(false))
    }

    /// Prototype
    pub(crate) async fn do_callback(
        &self,
        slot: u32,
        response: Response,
        buffer: Box<[u8]>,
    ) -> io::Result<()> {
        let need_removal = match self.0.read().get_by_slot(slot) {
            None => return Ok(()),
            Some((_index, value)) => {
                if let Some(awaitable) = value {
                    awaitable.done((response, buffer));
                    false
                } else {
                    true
                }
            }
        };

        if need_removal {
            self.remove(slot);
        }

        Ok(())
    }

    fn remove(&self, slot: u32) -> Value {
        self.0
            .write()
            .remove_by_slot(slot)
            .expect("Slot is removed before SlotGuard is dropped or waited")
            .1
    }
}

#[derive(Debug)]
pub(crate) struct SlotGuard<'a>(Option<&'a Responses>, u32);

impl SlotGuard<'_> {
    pub(crate) fn get_slot_id(&self) -> u32 {
        self.1
    }

    pub(crate) async fn wait(mut self) -> (Response, Box<[u8]>) {
        let responses = self.0.take().unwrap();
        let slot = self.1;
        responses.wait_impl(slot).await;
        responses
            .remove(slot)
            .unwrap()
            .get_value()
            .expect("Response is already processed and get_value should return a value")
    }
}

impl Drop for SlotGuard<'_> {
    fn drop(&mut self) {
        if let Some(responses) = self.0.take() {
            responses.remove(self.1);
        }
    }
}

#[derive(Debug)]
pub(crate) struct SlotGuardNoWait<'a>(&'a Responses, u32);

impl SlotGuardNoWait<'_> {
    pub(crate) fn get_slot_id(&self) -> u32 {
        self.1
    }
}

impl Drop for SlotGuardNoWait<'_> {
    fn drop(&mut self) {
        self.0.remove(self.1);
    }
}
