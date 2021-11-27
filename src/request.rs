use super::constants;

#[derive(Debug)]
pub(crate) enum Request {
    /// Response with `Response::Version`.
    Hello { version: u32 },
}
