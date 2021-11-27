use std::io;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    /// Server speaks multiplex protocol other than protocol 4.
    #[error("Server speaks multiplex protocol other than protocol 4.")]
    UnsupportedMuxProtocol,

    /// Server response with unexpected package type {0}: Response {1:#?}.
    #[error("Server response with unexpected package type {0}")]
    InvalidServerResponse(&'static str),

    /// Server response with a different id than the requested one.
    #[error("Server response with a different id than the requested one.")]
    UnmatchedRequestId,

    /// IO Error (Excluding `EWOULDBLOCK`): {0}.
    #[error("IO Error (Excluding `EWOULDBLOCK`): {0}.")]
    IOError(#[from] io::Error),

    /// Failed to serialize/deserialize the message: {0}.
    #[error("Failed to serialize/deserialize the message: {0}.")]
    FormatError(#[from] ssh_format::Error),
}
