#![forbid(unsafe_code)]

mod awaitable;
mod awaitable_responses;
mod buffer;
mod connection;
mod error;

pub use awaitable_responses::{AwaitableResponse, Response};

pub use buffer::*;

pub use openssh_sftp_protocol::file_attrs::FileAttrs;
pub use openssh_sftp_protocol::request::{CreateFlags, FileMode, OpenFile, RequestInner};
pub use openssh_sftp_protocol::response::ErrMsg as SftpErrMsg;
pub use openssh_sftp_protocol::response::ErrorCode as SftpErrorKind;
pub use openssh_sftp_protocol::response::StatusCode;
pub use openssh_sftp_protocol::response::{NameEntry, ResponseInner};
pub use openssh_sftp_protocol::{Handle, HandleOwned};

pub use connection::{Connection, ConnectionFactory};
pub use error::Error;
