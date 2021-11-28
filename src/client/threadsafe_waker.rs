use core::mem;
use core::task::Waker;

use parking_lot::Mutex;

#[derive(Debug)]
enum InnerState<T> {
    None,
    Done(T),
    Waiting(Waker),
}

#[derive(Debug)]
pub(crate) struct ThreadSafeWaker<T>(Mutex<InnerState<T>>);

impl<T> ThreadSafeWaker<T> {
    pub(crate) fn new() -> Self {
        Self(Mutex::new(InnerState::None))
    }

    /// Return true if the task is already done.
    pub(crate) fn install_waker(&self, waker: Waker) -> bool {
        use InnerState::*;

        let mut guard = self.0.lock();

        let done = match &*guard {
            Waiting(_waker) => panic!("Waker is installed twice"),
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
            Done(_) => panic!("ThreadSafeWaker is marked as done twice"),
            None => (),
            Waiting(waker) => waker.wake(),
        }
    }

    pub(crate) fn get_value(self) -> Option<T> {
        use InnerState::Done;

        match self.0.into_inner() {
            Done(value) => Some(value),
            _ => None,
        }
    }
}
