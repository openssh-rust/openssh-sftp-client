use std::io;
use thiserror::Error;

use openssh_sftp_protocol::response::StatusCode;
use openssh_sftp_protocol::ssh_format;

pub use openssh_sftp_protocol::response::ErrMsg as SftpErrMsg;
pub use openssh_sftp_protocol::response::ErrorCode as SftpErrorKind;

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

    /// Sftp error.
    #[error("Sftp error: {0:#?}, {1:#?}.")]
    SftpError(SftpErrorKind, SftpErrMsg),
}

impl Error {
    pub(crate) fn check_response(response: ResponseInner) -> Result<ResponseInner, Error> {
        match response {
            ResponseInner::Status {
                status_code,
                err_msg,
            } => match status_code {
                StatusCode::Success => Ok(ResponseInner::Status {
                    status_code: StatusCode::Success,
                    err_msg,
                }),
                StatusCode::Failure(err_kind) => Err(Error::SftpError(err_kind, err_msg)),
            },
            response => Ok(response),
        }
    }
}
