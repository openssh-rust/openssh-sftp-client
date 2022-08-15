use bytes::Bytes;

pub trait Queue {
    /// Push one `bytes`.
    fn push(&self, bytes: Bytes);

    /// Push multiple bytes atomically
    fn extend(&self, header: Bytes, body: &[&[Bytes]]);
}
