use super::Error;
use super::ToBuffer;

use core::fmt::Debug;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use concurrent_arena::Arena;

use openssh_sftp_protocol::response::ResponseInner;

use derive_destructure::destructure;

#[derive(Debug, Clone)]
pub(crate) enum Response<Buffer: ToBuffer> {
    Header(ResponseInner),

    /// The buffer that stores the response of Read,
    /// since its corresponding response type `ResponseInner::Data`
    /// does not contain any member, it doesn't have to be stored.
    Buffer(Buffer),

    /// Same as `Buffer`, this is a fallback
    /// if `Buffer` isn't provided or it isn't large enough.
    AllocatedBox(Box<[u8]>),
}

pub(crate) type Awaitable<Buffer> = awaitable::Awaitable<Buffer, Response<Buffer>>;

/// BITARRAY_LEN must be LEN / usize::BITS and LEN must be divisble by usize::BITS.
const BITARRAY_LEN: usize = 1;
const LEN: usize = 64;

pub(crate) type ArenaArc<Buffer> = concurrent_arena::ArenaArc<Awaitable<Buffer>, BITARRAY_LEN, LEN>;

/// Check `concurrent_arena::Arena` for `BITARRAY_LEN` and `LEN`.
#[derive(Debug)]
pub(crate) struct AwaitableResponses<Buffer: ToBuffer + 'static>(
    Arena<Awaitable<Buffer>, BITARRAY_LEN, LEN>,
);

impl<Buffer: Debug + ToBuffer + Send + Sync> AwaitableResponses<Buffer> {
    pub(crate) fn new() -> Self {
        Self(Arena::with_capacity(3))
    }

    /// Return (slot_id, awaitable_response)
    pub(crate) fn insert(&self, buffer: Option<Buffer>) -> Id<Buffer> {
        Id::new(self.0.insert(Awaitable::new(buffer)))
    }

    pub(crate) fn get(&self, slot: u32) -> Result<Id<Buffer>, Error> {
        self.0
            .get(slot)
            .map(Id::new)
            .ok_or(Error::InvalidResponseId { response_id: slot })
    }
}

#[derive(Debug, destructure)]
struct IdInner<Buffer: ToBuffer + Send + Sync>(ArenaArc<Buffer>);

impl<Buffer: ToBuffer + Debug + Send + Sync> IdInner<Buffer> {
    pub(crate) fn get_input(&self) -> Option<Buffer> {
        self.0.take_input()
    }

    pub(crate) fn do_callback(&self, response: Response<Buffer>) {
        self.0.done(response);
    }

    pub(crate) fn remove_if_cancelled(&self) {
        // There is only two references:
        //  - self.0
        //  - AwaitableResponses
        //
        // Then the callback is cancelled.
        if ArenaArc::strong_count(&self.0) == 2 {
            ArenaArc::remove(&self.0);
        }
    }
}

impl<Buffer: ToBuffer + Send + Sync> Drop for IdInner<Buffer> {
    fn drop(&mut self) {
        ArenaArc::remove(&self.0);
    }
}

#[derive(Debug)]
pub struct Id<Buffer: ToBuffer + Send + Sync>(IdInner<Buffer>);

impl<Buffer: ToBuffer + Debug + Send + Sync> Id<Buffer> {
    pub(crate) fn new(arc: ArenaArc<Buffer>) -> Self {
        Id(IdInner(arc))
    }

    pub(crate) fn into_inner(self) -> ArenaArc<Buffer> {
        self.0.destructure().0
    }

    pub(crate) fn get_input(&self) -> Option<Buffer> {
        self.0.get_input()
    }

    pub(crate) fn do_callback(&self, response: Response<Buffer>) {
        self.0.do_callback(response);
    }

    pub(crate) async fn wait(this: &ArenaArc<Buffer>) -> Response<Buffer> {
        struct WaitFuture<'a, Buffer: ToBuffer>(Option<&'a Awaitable<Buffer>>);

        impl<Buffer: ToBuffer + Debug> Future for WaitFuture<'_, Buffer> {
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

        WaitFuture(Some(this)).await;

        this.take_output()
            .expect("The request should be done by now")
    }

    pub(crate) fn remove_if_cancelled(&self) {
        self.0.remove_if_cancelled();
    }
}
