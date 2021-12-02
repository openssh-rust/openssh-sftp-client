use std::io;
use thiserror::Error;

use openssh_sftp_protocol::ssh_format;

pub use openssh_sftp_protocol::response::ResponseInner;

#[derive(Debug, Error)]
pub enum Error {
    /// Server speaks sftp protocol other than protocol 4.
    #[error("Server speaks sftp protocol other than protocol 4.")]
    UnsupportedSftpProtocol,

    /// Server response with unexpected package type.
    #[error("Server response with unexpected package type {0}: Response {1:#?}")]
    InvalidServerResponse(&'static str, ResponseInner),

    /// Server response with a different id than the requested one.
    #[error("Server response with a different id than the requested one.")]
    UnmatchedRequestId,

    /// IO Error (Excluding `EWOULDBLOCK`): {0}.
    #[error("IO Error (Excluding `EWOULDBLOCK`): {0}.")]
    IOError(#[from] io::Error),

    /// Failed to serialize/deserialize the message: {0}.
    #[error("Failed to serialize/deserialize the message: {0}.")]
    FormatError(#[from] ssh_format::Error),

    /// Sftp protocol can only send and receive at most u32::MAX data in one request.
    #[error("Sftp protocol can only send and receive at most u32::MAX data in one request.")]
    BufferTooLong,
}
