//! This crate provides a set of APIs to access the remote filesystem using
//! the sftp protocol and is implemented in pure Rust.
//!
//! It supports sending multiple requests concurrently using [`WriteEnd`]
//! (it can be [`WriteEnd::clone`]d), however receiving responses have to be done
//! sequentially using [`ReadEnd::read_in_one_packet`].
//!
//! To create [`WriteEnd`] and [`ReadEnd`], simply pass the `stdin` and `stdout` of
//! the `sftp-server` launched at remote to [`connect`].
//!
//! This crate supports all operations supported by sftp v3, in additional to
//! the following extensions:
//!  - [`WriteEnd::send_limits_request`]
//!  - [`WriteEnd::send_expand_path_request`]
//!  - [`WriteEnd::send_fsync_request`]
//!  - [`WriteEnd::send_hardlink_request`]
//!  - [`WriteEnd::send_posix_rename_request`]

pub use openssh_sftp_error::{Error, SftpErrMsg, SftpErrorKind, UnixTimeStampError};
pub use openssh_sftp_protocol::{
    file_attrs::{FileAttrs, FileType, Permissions, UnixTimeStamp},
    open_options::{CreateFlags, OpenOptions},
    request::OpenFileRequest,
    response::{Extensions, Limits, NameEntry},
    Handle, HandleOwned,
};

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

#[cfg(doc)]
/// Changelog for this crate.
pub mod changelog;

mod awaitable_responses;
pub use awaitable_responses::Id;

mod awaitables;
pub use awaitables::{
    AwaitableAttrs, AwaitableAttrsFuture, AwaitableData, AwaitableDataFuture, AwaitableHandle,
    AwaitableHandleFuture, AwaitableLimits, AwaitableLimitsFuture, AwaitableName,
    AwaitableNameEntries, AwaitableNameEntriesFuture, AwaitableNameFuture, AwaitableStatus,
    AwaitableStatusFuture, Data,
};

mod buffer;
pub use buffer::{Buffer, ToBuffer};

mod connection;
pub use connection::{connect, SharedData};

mod queue;
pub use queue::Queue;

mod read_end;
pub use read_end::ReadEnd;

mod reader_buffered;

mod write_end;
pub use write_end::WriteEnd;
