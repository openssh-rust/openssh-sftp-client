//! openssh sftp client, implements [sftp v3] accodring to
//! [`openssh-portable/sftp-client.c`].
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
//! [sftp v3]: https://www.openssh.com/txt/draft-ietf-secsh-filexfer-02.txt
//! [`openssh-portable/sftp-client.c`]: https://github.com/openssh/openssh-portable/blob/19b3d846f06697c85957ab79a63454f57f8e22d6/sftp-client.c

#![warn(
    missing_docs,
    missing_debug_implementations,
    rustdoc::broken_intra_doc_links,
    rust_2018_idioms,
    unreachable_pub
)]

mod error;
pub use error::Error;

pub mod highlevel;
pub mod lowlevel;
