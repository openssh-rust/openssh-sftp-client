use super::*;

use std::io;
use thiserror::Error;

use openssh_sftp_protocol::ssh_format;

pub use openssh_sftp_protocol::response::ResponseInner;

#[derive(Debug, Error)]
pub enum Error {
    /// Server speaks sftp protocol other than protocol 3.
    #[error("Server speaks sftp protocol {version} other than protocol 3.")]
    UnsupportedSftpProtocol {
        /// The minimal sftp protocol version the server supported.
        version: u32,
    },

    /// IO Error (Excluding `EWOULDBLOCK`): {0}.
    #[error("IO Error (Excluding `EWOULDBLOCK`): {0}.")]
    IOError(#[from] io::Error),

    /// Failed to serialize/deserialize the message: {0}.
    #[error("Failed to serialize/deserialize the message: {0}.")]
    FormatError(#[from] ssh_format::Error),

    /// Error when waiting for response
    #[error("Error when waiting for response: {0}.")]
    AwaitableError(#[from] awaitable::Error),

    /// Sftp protocol can only send and receive at most u32::MAX data in one request.
    #[error("Sftp protocol can only send and receive at most u32::MAX data in one request.")]
    BufferTooLong,

    /// The response id is invalid.
    ///
    /// The user of `Connection` can choose to log this error and continue operation.
    #[error("The response id {response_id} is invalid.")]
    InvalidResponseId { response_id: u32 },

    /// (OriginalError, RecursiveError): {0:#?}.
    #[error("(OriginalError, RecursiveError): {0:#?}.")]
    RecursiveErrors(Box<(Error, Error)>),

    /// Sftp server error
    #[error("Sftp server reported error kind {0:#?}, msg: {1:#?}")]
    SftpError(SftpErrorKind, SftpErrMsg),

    /// Invalid response from the sftp-server
    ///
    /// Use &&str since &str takes 16 bytes while &str only takes 8 bytes.
    #[error("Response from sftp server is invalid: {0}")]
    InvalidResponse(&'static &'static str),
}
