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
///  - `lowlevel::WriteEnd` now does not require `W`
///  - `lowlevel::ReadEnd` now does not require `W`
///  - `lowlevel::SharedData` now does not require `W`
///  - `lowlevel::connect` now does not require `W` and does not return
///    the extension.
///  - `lowlevel::connect_with_auxiliary` now does not require `W` and
///    does not return the extension.
///
/// ## Removed
///  - `SharedData::get_auxiliary_mut`
///  - `SharedData::strong_count`
///  - `ReadEnd::wait_for_new_request`
pub mod v0_1_0 {}
