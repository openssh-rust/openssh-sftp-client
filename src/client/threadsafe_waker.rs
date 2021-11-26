use core::mem;
use core::task::Waker;

use parking_lot::Mutex;

#[derive(Debug)]
enum InnerState {
    None,
    Done,
    Waiting(Waker),
}

#[derive(Debug)]
pub(crate) struct ThreadSafeWaker(Mutex<InnerState>);

impl ThreadSafeWaker {
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
            Done => true,
        };
        if !done {
            *guard = Waiting(waker);
        }
        done
    }

    pub(crate) fn done(&self) {
        use InnerState::*;

        // Release the lock ASAP
        let prev_state = mem::replace(&mut *self.0.lock(), Done);

        match prev_state {
            Done => panic!("ThreadSafeWaker is marked as done twice"),
            None => (),
            Waiting(waker) => waker.wake(),
        }
    }
}
