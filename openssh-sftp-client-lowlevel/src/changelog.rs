#[allow(unused_imports)]
use crate::*;

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
