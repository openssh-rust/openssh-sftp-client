use core::fmt::Debug;
use core::mem;
use core::task::Waker;

use generic_global_variables::{Entry, GenericGlobal};
use once_cell::sync::OnceCell;
use openssh_sftp_protocol::shared_arena::{ArenaArc, SharedArena};

use parking_lot::Mutex;

#[derive(Debug)]
enum InnerState<Input, Output> {
    Ongoing(Option<Input>, Option<Waker>),

    /// The awaitable is done
    Done(Output),

    Consumed,
}
impl<Input, Output> InnerState<Input, Output> {
    fn new(input: Option<Input>) -> Mutex<Self> {
        Mutex::new(Ongoing(input, None))
    }
}

#[derive(Debug)]
pub(crate) struct AwaitableFactory<Input: 'static, Output: 'static>(
    Entry<SharedArena<Mutex<InnerState<Input, Output>>>>,
);
impl<Input, Output> AwaitableFactory<Input, Output> {
    pub(crate) fn create(&self, input: Option<Input>) -> Awaitable<Input, Output> {
        Awaitable(self.0.alloc_arc(InnerState::new(input)))
    }
}

impl<Input, Output> Clone for AwaitableFactory<Input, Output> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

use InnerState::*;

#[derive(Debug)]
pub(crate) struct Awaitable<Input, Output>(ArenaArc<Mutex<InnerState<Input, Output>>>);

impl<Input, Output> Clone for Awaitable<Input, Output> {
    fn clone(&self) -> Self {
        Awaitable(self.0.clone())
    }
}

impl<Input: Debug, Output: Debug> Awaitable<Input, Output> {
    pub(crate) fn get_factory() -> AwaitableFactory<Input, Output> {
        static GLOBALS: OnceCell<GenericGlobal> = OnceCell::new();

        let globals = GLOBALS.get_or_init(GenericGlobal::new);

        AwaitableFactory(globals.get_or_init(SharedArena::new))
    }

    /// Return true if the task is already done.
    pub(crate) fn install_waker(&self, waker: Waker) -> bool {
        let mut guard = self.0.lock();

        match &mut *guard {
            Ongoing(_input, stored_waker) => {
                if stored_waker.is_some() {
                    panic!("Waker is installed twice before the awaitable is done");
                }
                *stored_waker = Some(waker);
                false
            }
            Done(_) => true,
            Consumed => {
                panic!("Waker is installed after the awaitable is done and its result consumed")
            }
        }
    }

    pub(crate) fn take_input(&self) -> Option<Input> {
        let mut guard = self.0.lock();

        match &mut *guard {
            Ongoing(input, _stored_waker) => input.take(),
            Done(_) => None,
            Consumed => {
                panic!("Task attempts to retrieve input after the awaitable is done and its result consumed")
            }
        }
    }

    pub(crate) fn done(self, value: Output) {
        let prev_state = mem::replace(&mut *self.0.lock(), Done(value));

        match prev_state {
            Done(_) => panic!("Awaitable is marked as done twice"),
            Ongoing(_input, stored_waker) => {
                if let Some(waker) = stored_waker {
                    waker.wake();
                }
            }
            Consumed => {
                panic!("Awaitable is marked as done again after its result consumed")
            }
        };
    }

    /// Return `Some(output)` if the awaitable is done.
    pub(crate) fn take_output(self) -> Option<Output> {
        let prev_state = mem::replace(&mut *self.0.lock(), Consumed);

        match prev_state {
            Done(value) => Some(value),
            _ => None,
        }
    }
}
