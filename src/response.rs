use super::{constants, Extensions};

pub(crate) struct ServerVersion {
    version: u32,
    extensions: Extensions,
}

#[derive(Debug)]
pub(crate) enum Response {}
