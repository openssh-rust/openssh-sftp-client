#![forbid(unsafe_code)]

mod awaitable_responses;
mod awaitables;
mod buffer;
mod connection;
mod error;
mod read_end;
mod write_end;
mod writer;

pub use awaitable_responses::Id;

pub use buffer::*;

pub use openssh_sftp_protocol::file_attrs::{
    FileAttrs, Permissions, UnixTimeStamp, UnixTimeStampError,
};
pub use openssh_sftp_protocol::request::{CreateFlags, FileMode, OpenFile};
pub use openssh_sftp_protocol::response::ErrMsg as SftpErrMsg;
pub use openssh_sftp_protocol::response::ErrorCode as SftpErrorKind;
pub use openssh_sftp_protocol::response::NameEntry;
pub use openssh_sftp_protocol::response::StatusCode;
pub use openssh_sftp_protocol::{Handle, HandleOwned};

pub use connection::connect;
pub use error::Error;
pub use read_end::ReadEnd;
pub use write_end::*;

pub use awaitables::{
    AwaitableAttrs, AwaitableData, AwaitableHandle, AwaitableName, AwaitableNameEntries,
    AwaitableStatus, Data, Name,
};
