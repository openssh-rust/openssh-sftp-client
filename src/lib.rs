#![forbid(unsafe_code)]

mod awaitable_responses;
mod awaitables;
mod buffer;
mod connection;
mod error;
mod read_end;
mod write_end;
mod writer;

/// Default size of buffer for up/download in openssh-portable
pub const OPENSSH_PORTABLE_DEFAULT_COPY_BUFLEN: usize = 32768;

/// Default number of concurrent outstanding requests in openssh-portable
pub const OPENSSH_PORTABLE_DEFAULT_NUM_REQUESTS: usize = 64;

/// Minimum amount of data to read at a time in openssh-portable
pub const OPENSSH_PORTABLE_MIN_READ_SIZE: usize = 512;

/// Maximum depth to descend in directory trees in openssh-portable
pub const OPENSSH_PORTABLE_MAX_DIR_DEPTH: usize = 64;

/// Default length of download buffer in openssh-portable
pub const OPENSSH_PORTABLE_DEFAULT_DOWNLOAD_BUFLEN: usize = 20480;

/// Default length of upload buffer in openssh-portable
pub const OPENSSH_PORTABLE_DEFAULT_UPLOAD_BUFLEN: usize = 20480;

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
    AwaitableStatus, Data,
};
