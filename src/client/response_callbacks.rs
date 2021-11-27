use super::{CountedReader, ResponseCallback, ThreadSafeWaker};

use std::io;
use std::sync::Arc;

use parking_lot::RwLock;
use thunderdome::Arena;

pub(crate) type Value = (ThreadSafeWaker, Option<ResponseCallback>);

#[derive(Debug, Default)]
pub(crate) struct ResponseCallbacks(RwLock<Arena<Value>>);

impl ResponseCallbacks {
    pub(crate) fn insert(&self, callback: ResponseCallback) -> u32 {
        let val = (ThreadSafeWaker::new(), Some(callback));

        self.0.write().insert(val).slot()
    }

    /// Prototype
    pub(crate) async fn do_callback(
        &self,
        slot: u32,
        response: u8,
        reader: CountedReader<'_>,
    ) -> io::Result<()> {
        let callback = match self.0.write().get_by_slot_mut(slot) {
            None => return Ok(()),
            Some((_index, value)) => value.1.take(),
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

    /// Return false if slot is invalid.
    pub(crate) fn remove(&self, slot: u32) -> bool {
        self.0.write().remove_by_slot(slot).is_some()
    }
}
