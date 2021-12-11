use super::Error;
use super::ToBuffer;

use core::fmt::Debug;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use concurrent_arena::Arena;
use concurrent_arena::ArenaArc;

use openssh_sftp_protocol::response::ResponseInner;

#[derive(Debug, Clone)]
pub enum Response<Buffer: ToBuffer> {
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
        Id(self.0.insert(Awaitable::new(buffer)))
    }

    pub(crate) fn get(&self, slot: u32) -> Result<Id<Buffer>, Error> {
        self.0
            .get(slot)
            .map(Id)
            .ok_or(Error::InvalidResponseId { response_id: slot })
    }
}

#[derive(Debug)]
pub struct Id<Buffer: ToBuffer + Send + Sync>(ArenaArc<Awaitable<Buffer>, BITARRAY_LEN, LEN>);

impl<Buffer: ToBuffer + Debug + Send + Sync> Id<Buffer> {
    pub(crate) fn slot(&self) -> u32 {
        ArenaArc::slot(&self.0)
    }

    pub(crate) fn reset(&self, input: Option<Buffer>) {
        self.0.reset(input);
    }

    pub(crate) fn get_input(&self) -> Option<Buffer> {
        self.0.take_input()
    }

    pub(crate) fn do_callback(&self, response: Response<Buffer>) {
        self.0.done(response);
    }

    pub(crate) async fn wait(&self) -> Response<Buffer> {
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

        WaitFuture(Some(&self.0)).await;

        self.0
            .take_output()
            .expect("The request should be done by now")
    }
}

impl<Buffer: ToBuffer + Send + Sync> Drop for Id<Buffer> {
    fn drop(&mut self) {
        ArenaArc::remove(&self.0);
    }
}
