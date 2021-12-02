use core::fmt::Debug;
use core::mem;
use core::task::Waker;

use std::sync::Arc;

use parking_lot::Mutex;

#[derive(Debug)]
enum InnerState<T> {
    None,

    /// The wakeup callback is registered.
    Waiting(Waker),

    /// The awaitable is done
    Done(T),
}

#[derive(Debug, Clone)]
pub(crate) struct Awaitable<T: Debug>(Arc<Mutex<InnerState<T>>>);

impl<T: Debug> Awaitable<T> {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Mutex::new(InnerState::None)))
    }

    /// Return true if the task is already done.
    pub(crate) fn install_waker(&self, waker: Waker) -> bool {
        use InnerState::*;

        let mut guard = self.0.lock();

        let done = match &*guard {
            Waiting(_waker) => panic!("Waker is installed twice before the awaitable is done"),
            None => false,
            Done(_) => true,
        };
        if !done {
            *guard = Waiting(waker);
        }
        done
    }

    pub(crate) fn done(&self, value: T) {
        use InnerState::*;

        // Release the lock ASAP
        let prev_state = mem::replace(&mut *self.0.lock(), Done(value));

        match prev_state {
            Done(_) => panic!("Awaitable is marked as done twice"),
            None => (),
            Waiting(waker) => waker.wake(),
        }
    }

    /// Precondition: This must be the last Awaitable.
    pub(crate) fn get_value(self) -> Option<T> {
        use InnerState::Done;

        match Arc::try_unwrap(self.0)
            .expect("get_value is called when there is other awaitable alive")
            .into_inner()
        {
            Done(value) => Some(value),
            _ => None,
        }
    }
}
