use super::constants;

#[derive(Debug)]
pub(crate) enum Response {
    Version { version: u32 },
}
