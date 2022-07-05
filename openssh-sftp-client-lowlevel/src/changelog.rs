#[allow(unused_imports)]
use crate::*;

/// This is the first release!
///
/// This crate has been extracted out from
/// [openssh-sftp-client](https://docs.rs/openssh-sftp-client).
///
/// # Changes from v0.10.2 of `openssh_sftp_client::lowlevel`:
///
/// ## Changed
///
///  - `lowlevel::WriteEnd` now does not require `W: Unpin`
///  - `lowlevel::ReadEnd` now does not require `W: Unpin`
///  - `lowlevel::SharedData` now does not require `W: Unpin`
///  - `lowlevel::connect` now does not require `W: Unpin`
///  - `lowlevel::connect_with_auxiliary` now does not require `W: Unpin`
///
/// ## Removed
///  - `SharedData::get_auxiliary_mut`
///  - `SharedData::strong_count`
///  - `ReadEnd::wait_for_new_request`
pub mod v0_1_0 {}
