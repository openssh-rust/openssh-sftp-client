#[allow(unused_imports)]
use crate::*;

#[doc(hidden)]
pub mod unreleased {}

/// ## Fixed
///  - Fix possible panic in [`highlevel::max_atomic_write_len`]
pub mod v0_8_2 {}

/// ## Added
///  - Reexport [`highlevel::CancellationToken`].
pub mod v0_8_1 {}

/// ## Added
///  - Associated function [`highlevel::FileType::is_fifo`].
///  - Associated function [`highlevel::FileType::is_socket`].
///  - Associated function [`highlevel::FileType::is_block_device`].
///  - Associated function [`highlevel::FileType::is_char_device`].
///  - Trait [`Writer`].
///
/// ## Changed
///  - Replace all use of [`tokio_pipe::PipeRead`] with generic bound
///    [`tokio::io::AsyncRead`] + [`Unpin`].
///  - Replace all use of [`tokio_pipe::PipeWrite`] with generic bound
///    [`Writer`].
///  - Replace constant `highlevel::MAX_ATOMIC_WRITE_LEN` with
///    non-`const` function [`highlevel::max_atomic_write_len`].
///  - Associated function [`highlevel::Sftp::fs`] now only takes `&self`
///    as parameter.
///
/// ## Removed
///  - Trait [`std::os::unix::fs::FileTypeExt`] implementation for
///    [`highlevel::FileType`].
///  - Trait [`std::os::unix::fs::PermissionsExt`] implementation for
///    [`highlevel::Permissions`].
///  - Associated function `lowlevel::WriteEnd::send_write_request_direct`.
///  - Associated function
///    `lowlevel::WriteEnd::send_write_request_direct_vectored`.
pub mod v0_8_0 {}
