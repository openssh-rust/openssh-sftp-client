#![forbid(unsafe_code)]

use std::{io, num::TryFromIntError, process::ExitStatus};

pub use awaitable_error::Error as AwaitableError;
pub use openssh_sftp_protocol_error::{
    ErrMsg as SftpErrMsg, ErrorCode as SftpErrorKind, UnixTimeStampError,
};
pub use ssh_format_error::Error as SshFormatError;
use thiserror::Error as ThisError;

/// Error returned by
/// [`openssh-sftp-client-lowlevel`](https://docs.rs/openssh-sftp-client-lowlevel)
/// and [`openssh-sftp-client`](https://docs.rs/openssh-sftp-client)
#[non_exhaustive]
#[derive(Debug, ThisError)]
pub enum Error {
    /// Server speaks sftp protocol other than protocol 3.
    #[error("Server does not support sftp protocol v3: It only support sftp protocol newer than {version}.")]
    UnsupportedSftpProtocol {
        /// The minimal sftp protocol version the server supported.
        version: u32,
    },

    /// Server returned a hello message that is too long.
    #[error("sftp server returned hello message of length {len}, which is longer than 4096.")]
    SftpServerHelloMsgTooLong {
        /// The length of the hello mesage
        len: u32,
    },

    /// This error is meant to be a dummy error created by user of this crate
    /// to indicate that the sftp-server run on remote server failed.
    ///
    /// openssh-sftp-client would never return this error.
    #[error("sftp-server run on remote server failed: {0}.")]
    SftpServerFailure(ExitStatus),

    /// This error is meant to be a dummy error created by user of this crate
    /// to indicate that the sftp-server run on remote server failed.
    ///
    /// openssh-sftp-client would never return this error.
    #[error("Background task failed: {0}.")]
    BackgroundTaskFailure(&'static &'static str),

    /// This error is meant to be a dummy error created by user of this crate
    /// to indicate that the extension is not supported.
    ///
    /// openssh-sftp-client would never return this error.
    #[error("Unsupported extension {0}.")]
    UnsupportedExtension(&'static &'static str),

    /// IO Error (Excluding [`io::ErrorKind::WouldBlock`]): {0}.
    #[error("IO Error (Excluding `io::ErrorKind::WouldBlock`): {0}.")]
    IOError(#[from] io::Error),

    /// Failed to serialize/deserialize the message: {0}.
    #[error("Failed to serialize/deserialize the message: {0}.")]
    FormatError(#[from] SshFormatError),

    /// Error when waiting for response
    #[error("Error when waiting for response: {0}.")]
    AwaitableError(#[from] AwaitableError),

    /// Sftp protocol can only send and receive at most [`u32::MAX`] data in one request.
    #[error("Sftp protocol can only send and receive at most u32::MAX data in one request.")]
    BufferTooLong(#[from] TryFromIntError),

    /// The response id is invalid.
    ///
    /// The user can choose to log this error and continue operation.
    #[error("The response id {response_id} is invalid.")]
    InvalidResponseId {
        /// The invalid response id
        response_id: u32,
    },

    /// Raised 2 errors when cleaning up.
    #[error(transparent)]
    RecursiveErrors(Box<RecursiveError>),

    /// Raised 3 errors when cleaning up.
    #[error(transparent)]
    RecursiveErrors3(Box<RecursiveError3>),

    /// Sftp server error
    #[error("Sftp server reported error kind {0:#?}, msg: {1}")]
    SftpError(SftpErrorKind, SftpErrMsg),

    /// Invalid response from the sftp-server
    #[error("Response from sftp server is invalid: {0}")]
    InvalidResponse(
        // Use `&&str` since `&str` takes 16 bytes while `&str` only takes 8 bytes.
        &'static &'static str,
    ),

    /// Handle returned by server is longer than the limit 256 bytes specified in sftp v3.
    #[error("Handle returned by server is longer than the limit 256 bytes specified in sftp v3")]
    HandleTooLong,

    /// tokio join error
    #[error("Failed to join tokio task")]
    TaskJoinError(#[from] tokio::task::JoinError),

    #[cfg(feature = "openssh")]
    #[error("Failed to create sftp from session: {0}")]
    RemoteChildSpawnError(#[from] openssh::Error),
}

#[derive(Debug, ThisError)]
#[error("OriginalError: {original_error}, curr err raised when cleaning up: {occuring_error}.")]
pub struct RecursiveError {
    /// Original error
    pub original_error: Error,

    /// Current error raised when performing cleanup
    /// for original error.
    #[source]
    pub occuring_error: Error,
}

#[derive(Debug, ThisError)]
#[error("err1: {err1}, err2: {err2}, err3: {err3}.")]
pub struct RecursiveError3 {
    /// First error raised.
    pub err1: Error,

    /// Second error raised.
    pub err2: Error,

    /// Third error raised.
    #[source]
    pub err3: Error,
}
