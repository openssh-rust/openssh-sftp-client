use super::{awaitable::Awaitable, Response};

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use std::io;

use parking_lot::RwLock;
use thunderdome::Arena;

pub(crate) type Value = Awaitable<(Response, Vec<u8>)>;

// TODO: Simplify this

#[derive(Debug, Default)]
pub(crate) struct Responses(RwLock<Arena<Value>>);

impl Responses {
    fn insert_impl(&self) -> u32 {
        let val = Awaitable::new();

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

                if value.install_waker(waker) {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }
        }

        WaitFuture(Some(&self.0), slot).await;
    }

    pub fn insert(&self) -> SlotGuard {
        SlotGuard(self, Some(self.insert_impl()))
    }

    /// Prototype
    pub(crate) async fn do_callback(
        &self,
        slot: u32,
        response: Response,
        buffer: Vec<u8>,
    ) -> io::Result<()> {
        match self.0.read().get_by_slot(slot) {
            None => return Ok(()),
            Some((_index, value)) => {
                value.done((response, buffer));
            }
        };

        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct SlotGuard<'a>(&'a Responses, Option<u32>);

impl SlotGuard<'_> {
    pub(crate) fn get_slot_id(&self) -> u32 {
        self.1.unwrap()
    }

    fn remove(&mut self, slot: u32) -> Value {
        self.0
             .0
            .write()
            .remove_by_slot(slot)
            .expect("Slot is removed before SlotGuard is dropped or waited")
            .1
    }

    pub(crate) async fn wait(mut self) -> (Response, Vec<u8>) {
        let slot = self.1.take().unwrap();
        self.0.wait_impl(slot).await;
        self.remove(slot)
            .get_value()
            .expect("Response is already processed and get_value should return a value")
    }
}

impl Drop for SlotGuard<'_> {
    fn drop(&mut self) {
        if let Some(slot) = self.1.take() {
            self.remove(slot);
        }
    }
}
