use core::fmt::Debug;
use core::mem;
use core::task::Waker;

use std::sync::Arc;

use parking_lot::Mutex;

#[derive(Debug)]
enum InnerState<Input, Output> {
    Ongoing(Option<Input>, Option<Waker>),

    /// The awaitable is done
    Done(Output),
}
use InnerState::*;

#[derive(Debug)]
pub(crate) struct Awaitable<Input, Output>(Arc<Mutex<InnerState<Input, Output>>>);

impl<Input, Output> Clone for Awaitable<Input, Output> {
    fn clone(&self) -> Self {
        Awaitable(self.0.clone())
    }
}

impl<Input: Debug, Output: Debug> Awaitable<Input, Output> {
    pub(crate) fn new(input: Option<Input>) -> Self {
        let state = Ongoing(input, None);
        Self(Arc::new(Mutex::new(state)))
    }

    /// Return true if the task is already done.
    pub(crate) fn install_waker(&self, waker: Waker) -> bool {
        let mut guard = self.0.lock();

        match &*guard {
            Ongoing(_input, stored_waker) => {
                if stored_waker.is_some() {
                    panic!("Waker is installed twice before the awaitable is done");
                }
                *stored_waker = Some(waker);
                false
            }
            Done(_) => true,
        }
    }

    pub(crate) fn take_input(&self) -> Option<Input> {
        let mut guard = self.0.lock();

        match &*guard {
            Ongoing(input, _stored_waker) => input.take(),
            Done(_) => None,
        }
    }

    pub(crate) fn done(&self, value: Output) {
        // Release the lock ASAP
        let prev_state = mem::replace(&mut *self.0.lock(), Done(value));

        match prev_state {
            Done(_) => panic!("Awaitable is marked as done twice"),
            Ongoing(_input, stored_waker) => {
                if let Some(waker) = stored_waker {
                    waker.wake();
                }
            }
        }
    }

    /// Precondition: This must be the last Awaitable.
    pub(crate) fn get_value(self) -> Option<Output> {
        match Arc::try_unwrap(self.0)
            .expect("get_value is called when there is other awaitable alive")
            .into_inner()
        {
            Done(value) => Some(value),
            _ => None,
        }
    }
}
