//! openssh sftp client, implements [sftp v3] accodring to
//! [`openssh-portable/sftp-client.c`].
//!
//! ## Usage
//!
//! It is recommended that you use this crate with [openssh].
//!
//! You can also use this crate directly by using whatever ssh library
//! to launch the `sftp` subsystem, then pass the stdin/stdout to
//! [`highlevel::Sftp::new`].
//!
//! ## Extensions
//!
//! This crate support the following extensions:
//!  - limits
//!  - expand path
//!  - fsync
//!  - hardlink
//!  - posix rename
//!
//! [openssh]: https://crates.io/crates/openssh
//! [sftp v3]: https://www.openssh.com/txt/draft-ietf-secsh-filexfer-02.txt
//! [`openssh-portable/sftp-client.c`]: https://github.com/openssh/openssh-portable/blob/19b3d846f06697c85957ab79a63454f57f8e22d6/sftp-client.c

#![warn(
    missing_docs,
    missing_debug_implementations,
    rustdoc::broken_intra_doc_links,
    rust_2018_idioms,
    unreachable_pub
)]

#[cfg(doc)]
/// Changelog for this crate.
pub mod changelog;

pub use openssh_sftp_error::Error;
use openssh_sftp_error::{awaitable, openssh_sftp_protocol};

pub mod highlevel;
pub mod lowlevel;
