#![forbid(unsafe_code)]

mod awaitable_responses;
mod buffer;
mod connection;
mod error;
mod read_end;
mod write_end;

pub use awaitable_responses::Id;

pub use buffer::*;

pub use openssh_sftp_protocol::file_attrs::{FileAttrs, FileAttrsBox};
pub use openssh_sftp_protocol::request::{CreateFlags, FileMode, OpenFile, RequestInner};
pub use openssh_sftp_protocol::response::ErrMsg as SftpErrMsg;
pub use openssh_sftp_protocol::response::ErrorCode as SftpErrorKind;
pub use openssh_sftp_protocol::response::StatusCode;
pub use openssh_sftp_protocol::response::{NameEntry, ResponseInner};
pub use openssh_sftp_protocol::{Handle, HandleOwned};

pub use connection::connect;
pub use error::Error;
pub use read_end::ReadEnd;
pub use write_end::*;
