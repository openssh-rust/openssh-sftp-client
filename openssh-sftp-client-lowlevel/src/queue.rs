use bytes::Bytes;

pub trait Queue {
    type Pusher: QueuePusher;

    /// Return a pusher that guarantees that no other threads
    /// can push to the queue.
    fn get_pusher(&self) -> Self::Pusher;

    /// Push one `bytes`.
    fn push(&self, bytes: Bytes) {
        self.get_pusher().push(bytes);
    }
}

pub trait QueuePusher {
    /// Push one `bytes`.
    fn push(&mut self, bytes: Bytes);

    /// Push multiple `bytes` from `iter`.
    fn extend_from_iter<It>(&mut self, iter: It)
    where
        It: IntoIterator<Item = Bytes>;

    /// Push multiple `bytes` from `iter` but with exact size.
    fn extend_from_exact_size_iter<I, It>(&mut self, iter: I)
    where
        I: IntoIterator<Item = Bytes, IntoIter = It>,
        It: ExactSizeIterator + Iterator<Item = Bytes>;
}
