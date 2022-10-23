#[allow(unused_imports)]
use crate::*;

#[doc(hidden)]
pub mod unreleased {}

/// ## Changed
///  - Ensure stable api: Create newtype wrapper of UnixTimeStamp (#53)
///
/// ## Other
///  - Bump [`openssh-sftp-error`] to v0.3.0
pub mod v_0_12_0 {}

/// ## Other change
///
/// Bump dep
///  - `ssh_format` to v0.13.0
///  - `openssh_sftp_protocol` to v0.22.0
///  - `openssh_sftp_error` to v0.2.0
///  - `openssh_sftp_client_lowlevel` to v0.3.0
pub mod v_0_11_3 {}

/// ## Other change
///  - Bump `openssh_sftp_client_lowlevel` version and optimize
///    write buffer implementation.
///  - Optimize: Reduce monomorphization
///  - Optimize latency: `create_flush_task` first in `Sftp::new`
///    and write the hello msg ASAP.
pub mod v_0_11_2 {}

/// Nothing has changed from [`v_0_11_0_rc_3`].
///
/// ## Other changes
///  - Dependency [`bytes`] bump to v1.2.0 for its optimizations.
pub mod v_0_11_1 {}

/// Nothing has changed from [`v_0_11_0_rc_3`].
pub mod v_0_11_0 {}

/// ## Changed
///  - Rename `SftpOptions::write_end_buffer_size` to
///    [`SftpOptions::requests_buffer_size`] and improve its
///    documentation.
///  - Rename `SftpOptions::read_end_buffer_size` to
///    [`SftpOptions::responses_buffer_size`] and improve its
///    documentation.
///
/// ## Removed
///  - `SftpOptions::max_read_len`
///  - `SftpOptions::max_write_len`
pub mod v_0_11_0_rc_3 {}

/// ## Fixed
///  - Changelog of v0.11.0-rc.1
///
/// ## Added
///  - [`file::File::copy_all_to`] to copy until EOF.
///    This function is extracted from the old `copy_to`
///    function.
///  - [`file::TokioCompatFile::capacity`]
///  - [`file::TokioCompatFile::reserve`]
///  - [`file::TokioCompatFile::shrink_to`]
///
/// ## Changed
///  - [`file::File::copy_to`] now takes [`std::num::NonZeroU64`]
///    instead of `u64`.
///  - [`file::TokioCompatFile::with_capacity`] does not take
///    `max_buffer_len` anymore.
///
/// ## Removed
///  - `file::DEFAULT_MAX_BUFLEN`
pub mod v_0_11_0_rc_2 {}

/// ## Added
///  - `SftpOptions::write_end_buffer_size`
///  - `SftpOptions::read_end_buffer_size`
///
/// ## Changed
///  - All types now does not have generic parameter `W`
///    except for `Sftp::new`
///
/// ## Removed
///  - Unused re-export `CancellationToken`.
///  - Backward compatibility alias `file::TokioCompactFile`.
///  - `Sftp::try_flush`
///  - `Sftp::flush`
///  - `file::File::max_write_len`
///  - `file::File::max_read_len`
///  - `file::File::max_buffered_write`
///
/// ## Moved
///  - `lowlevel` is now moved to be another crate [openssh_sftp_client_lowlevel].
///  - All items in `highlevel` is now moved into root.
pub mod v_0_11_0_rc_1 {}

/// ## Fixed
///  - Changelog of v0.10.2
pub mod v_0_10_3 {}

/// ## Added
///  - Async fn `lowlevel::WriteEnd::send_copy_data_request`
///  - Async fn `highlevel::file::File::copy_to`
pub mod v_0_10_2 {}

/// ## Fixed
///  - Changelog of v0.10.0
///  - Changelog of v0.9.0
pub mod v0_10_1 {}

/// ## Added
///  - Export mod `highlevel::file`
///  - Export mod `highlevel::fs`
///  - Export mod `highlevel::metadata`
///
/// ## Changed
///  - `lowlevel::WriteEnd` now requires `W: AsyncWrite + Unpin`
///  - `lowlevel::SharedData` now requires `W: AsyncWrite + Unpin`
///  - `lowlevel::ReadEnd` now requires `W: AsyncWrite + Unpin`
///  - `lowlevel::connect` now requires `W: AsyncWrite + Unpin`
///  - `lowlevel::connect_with_auxiliary` now requires `W: AsyncWrite + Unpin`
///  - All types in `highlevel` now requires `W: AsyncWrite + Unpin`
///    except for
///     - the re-exported type `highlevel::CancellationToken`
///     - `highlevel::SftpOptions`
///     - `highlevel::fs::DirEntry`
///     - `highlevel::fs::ReadDir`
///
/// ## Removed
///  - Trait `Writer`.
///  - `lowlevel::WriteEnd::send_write_request_direct_atomic`
///  - `lowlevel::WriteEnd::send_write_request_direct_atomic_vectored`
///  - `lowlevel::WriteEnd::send_write_request_direct_atomic_vectored2`
///  - Export of `highlevel::file::TokioCompactFile`
///  - Export of `highlevel::file::TokioCompatFile`
///  - Export of `highlevel::file::DEFAULT_BUFLEN`
///  - Export of `highlevel::file::DEFAULT_MAX_BUFLEN`
///  - Export of `highlevel::file::File`
///  - Export of `highlevel::file::OpenOptions`
///  - Export of `highlevel::fs::DirEntry`
///  - Export of `highlevel::fs::ReadDir`
///  - Export of `highlevel::fs::Dir`
///  - Export of `highlevel::fs::DirBuilder`
///  - Export of `highlevel::fs::Fs`
///  - Export of `highlevel::metadata::FileType`
///  - Export of `highlevel::metadata::MetaData`
///  - Export of `highlevel::metadata::MetaDataBuilder`
///  - Export of `highlevel::metadata::Permissions`
pub mod v0_10_0 {}

/// ## Removed
///  - `highlevel::Sftp::get_cancellation_token`
///  - `highlevel::Sftp::max_write_len`
///  - `highlevel::Sftp::max_read_len`
///  - `highlevel::Sftp::max_buffered_write`
pub mod v_0_9_0 {}

/// ## Added
///  - Type `highlevel::TokioCompatFile` to Replace
///    `highlevel::TokioCompactFile`.
pub mod v0_8_3 {}

/// ## Fixed
///  - Fix possible panic in `highlevel::max_atomic_write_len`
pub mod v0_8_2 {}

/// ## Added
///  - Reexport `highlevel::CancellationToken`.
pub mod v0_8_1 {}

/// ## Added
///  - Associated function `highlevel::FileType::is_fifo`.
///  - Associated function `highlevel::FileType::is_socket`.
///  - Associated function `highlevel::FileType::is_block_device`.
///  - Associated function `highlevel::FileType::is_char_device`.
///  - Trait `Writer`.
///
/// ## Changed
///  - Replace all use of `tokio_pipe::PipeRead` with generic bound
///    `tokio::io::AsyncRead` + `Unpin`.
///  - Replace all use of `tokio_pipe::PipeWrite` with generic bound
///    `Writer`.
///  - Replace constant `highlevel::MAX_ATOMIC_WRITE_LEN` with
///    non-`const` function `highlevel::max_atomic_write_len`.
///  - Associated function `highlevel::Sftp::fs` now only takes `&self`
///    as parameter.
///
/// ## Removed
///  - Trait `std::os::unix::fs::FileTypeExt` implementation for
///    `highlevel::FileType`.
///  - Trait `std::os::unix::fs::PermissionsExt` implementation for
///    `highlevel::Permissions`.
///  - Associated function `lowlevel::WriteEnd::send_write_request_direct`.
///  - Associated function
///    `lowlevel::WriteEnd::send_write_request_direct_vectored`.
pub mod v0_8_0 {}
