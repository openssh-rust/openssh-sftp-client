#![forbid(unsafe_code)]

use super::Error;
use super::ToBuffer;

use core::fmt::Debug;

use concurrent_arena::Arena;

use openssh_sftp_protocol::response::ResponseInner;

use derive_destructure2::destructure;

#[derive(Debug)]
pub(crate) enum Response<Buffer: ToBuffer> {
    Header(ResponseInner),

    /// The buffer that stores the response of Read.
    ///
    /// It will be returned if you provided a buffer to
    /// [`crate::WriteEnd::send_read_request`].
    Buffer(Buffer),

    /// This is a fallback that is returned
    /// if `Buffer` isn't provided or it isn't large enough.
    AllocatedBox(Box<[u8]>),

    /// Extended reply
    ExtendedReply(Box<[u8]>),
}

pub(crate) type Awaitable<Buffer> = awaitable::Awaitable<Buffer, Response<Buffer>>;

/// BITARRAY_LEN must be LEN / usize::BITS and LEN must be divisble by usize::BITS.
const BITARRAY_LEN: usize = 2;
const LEN: usize = 128;

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
    pub(crate) fn insert(&self) -> Id<Buffer> {
        Id::new(self.0.insert(Awaitable::new()))
    }

    pub(crate) fn try_reserve(&self, new_id_cnt: u32) -> bool {
        self.0.try_reserve(new_id_cnt / (LEN as u32))
    }

    pub(crate) fn reserve(&self, new_id_cnt: u32) {
        self.0.reserve(new_id_cnt / (LEN as u32));
    }

    pub(crate) fn get(&self, slot: u32) -> Result<ArenaArc<Buffer>, Error> {
        self.0
            .get(slot)
            .ok_or(Error::InvalidResponseId { response_id: slot })
    }
}

#[derive(Debug, destructure)]
pub struct Id<Buffer: ToBuffer + Send + Sync>(pub(crate) ArenaArc<Buffer>);

impl<Buffer: ToBuffer + Debug + Send + Sync> Id<Buffer> {
    pub(crate) fn new(arc: ArenaArc<Buffer>) -> Self {
        Id(arc)
    }

    pub(crate) fn into_inner(self) -> ArenaArc<Buffer> {
        self.destructure().0
    }
}
impl<Buffer: ToBuffer + Send + Sync> Drop for Id<Buffer> {
    fn drop(&mut self) {
        ArenaArc::remove(&self.0);
    }
}
