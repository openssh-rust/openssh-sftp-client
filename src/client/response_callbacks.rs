use super::{CountedReader, ResponseCallback, ThreadSafeWaker};

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use std::io;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use thunderdome::Arena;

pub(crate) type Value = (ThreadSafeWaker, Mutex<Option<ResponseCallback>>);

#[derive(Debug, Default)]
pub(crate) struct ResponseCallbacks(RwLock<Arena<Value>>);

impl ResponseCallbacks {
    pub(crate) fn insert(&self, callback: ResponseCallback) -> u32 {
        let val = (ThreadSafeWaker::new(), Mutex::new(Some(callback)));

        self.0.write().insert(val).slot()
    }

    /// Return false if slot is invalid.
    pub(crate) fn remove(&self, slot: u32) -> bool {
        self.0.write().remove_by_slot(slot).is_some()
    }

    pub(crate) async fn wait(&self, slot: u32) {
        struct WaitFuture<'a>(&'a RwLock<Arena<Value>>, u32);

        impl Future for WaitFuture<'_> {
            type Output = ();

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                let waker = cx.waker().clone();

                let guard = self.0.read();
                let (_index, value) = guard.get_by_slot(self.1).expect("Invalid slot");

                if value.0.install_waker(waker) {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }
        }

        WaitFuture(&self.0, slot).await;
    }

    /// Prototype
    pub(crate) async fn do_callback(
        &self,
        slot: u32,
        response: u8,
        reader: CountedReader<'_>,
    ) -> io::Result<()> {
        let callback = match self.0.read().get_by_slot(slot) {
            None => return Ok(()),
            Some((_index, value)) => value.1.lock().take(),
        };

        if let Some(mut callback) = callback {
            callback.call(response, reader).await?;
        }

        match self.0.read().get_by_slot(slot) {
            None => return Ok(()),
            Some((_index, value)) => value.0.done(),
        };

        Ok(())
    }
}
