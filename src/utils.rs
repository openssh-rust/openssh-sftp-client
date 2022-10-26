pub(super) fn assert_send<T>(val: T) -> T
where
    T: Send,
{
    val
}
