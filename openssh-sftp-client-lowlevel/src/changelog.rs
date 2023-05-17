#[allow(unused_imports)]
use crate::*;

/// ## Changed
///  - Fix [`ReaderBuffered`]: Leave error of exceeding buffer len in `consume` to handle by `BytesMut`
#[doc(hidden)]
pub mod unreleased {}

/// ## Changed
///  - Fix [`openssh-sftp-error`]: Ensure stable api (#49)
///  - Create newtype RecursiveError: Impls error::Error (#52)
///
/// ## Other
///  - Bump [`openssh-sftp-protocol`] to v0.23.0
///  - Bump dep awaitable to v0.4.0 (#48)
pub mod v0_4_0 {}

/// ## Internal
///  - Rm WriteBuffer: ssh_format now supports BytesMut as SerOutput
///  - Enable feature "bytes" of dep [`openssh-sftp-error`] which
///    enables "bytes" of dep `ssh_format`.
///
/// ## Other
///  - Bump [`openssh-sftp-protocol`] to v0.22.1
pub mod v0_3_1 {}

/// ## Other
///  - Bump [`openssh-sftp-protocol`] to v0.22.0
pub mod v0_3_0 {}

/// ## Added
///  - new trait [`Queue`]
///  - [`ReadEnd::new`] is now public
///
/// ## Changed
///  - [`connect`] now takes `queue` instead of `write_end_buffer_size`
///  - [`connect`] does not take `reader` and `reader_buffer_len` and it
///    does not return [`ReadEnd`] anymore.
///
///    User has to manually call [`ReadEnd::new`] to create [`ReadEnd`].
///
///    This is done to give the user more freedom on how and when [`ReadEnd`]
///    is created.
///  - [`ReadEnd`], [`WriteEnd`] and [`SharedData`] now takes an additional generic
///    parameter `Q`.
pub mod v0_2_0 {}

/// This is the first release!
///
/// This crate has been extracted out from
/// [openssh-sftp-client](https://docs.rs/openssh-sftp-client).
///
/// # Changes from v0.10.2 of `openssh_sftp_client::lowlevel`:
///
/// ## Added
///  - `ReadEnd::receive_server_hello`
///  - `ReadEnd::receive_server_hello_pinned`
///  - `ReadEnd::read_in_one_packet_pinned`
///  - `ReadEnd::ready_for_read_pinned`
///
/// ## Changed
///
///  - `lowlevel::WriteEnd` now does not require `W`
///  - `lowlevel::ReadEnd` now does not require `W`
///  - `lowlevel::SharedData` now does not require `W`
///  - `lowlevel::connect` removed parameter `writer` and generic paramter `W`,
///    it now also requires user to call `ReadEnd::receive_server_hello`
///    and flush the buffer themselves.
///
/// ## Removed
///  - `SharedData::get_auxiliary_mut`
///  - `SharedData::strong_count`
///  - `ReadEnd::wait_for_new_request`
///  - `lowlevel::connect_with_auxiliary`
pub mod v0_1_0 {}
