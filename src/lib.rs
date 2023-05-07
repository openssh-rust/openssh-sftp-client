//! openssh sftp client, implements [sftp v3] accodring to
//! [`openssh-portable/sftp-client.c`] and provides
//! an easy-to-use highlevel API.
//!
//! All `async` functions in this module are cancel safe.
//!
//! Internally, this is archived by first writing requests into a write buffer
//! containing [`bytes::Bytes`] and then flush all buffers at once periodically
//! to archive cancel safety and improve efficiencies.
//!
//! However, cancelling the future does not actually has any effect,
//! since the requests are sent regardless of the cancellation.
//!
//! Thus, if you cancel a future that changes the remote filesystem in any way,
//! then the change would still happen regardless.
//!
//! ## Usage
//!
//! It is recommended that you use this crate with [openssh].
//!
//! You can also use this crate directly by using whatever ssh library
//! to launch the `sftp` subsystem, then pass the stdin/stdout to
//! [`Sftp::new`].
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
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

#[cfg(doc)]
/// Changelog for this crate.
pub mod changelog;

pub use error::{Error, UnixTimeStampError};
use openssh_sftp_client_lowlevel as lowlevel;
pub use openssh_sftp_error as error;

use bytes::BytesMut;

mod unix_timestamp;
pub use unix_timestamp::UnixTimeStamp;

mod sftp;
use sftp::SftpHandle;
pub use sftp::{Sftp, SftpAuxiliaryData};

mod options;
pub use options::SftpOptions;

mod queue;
use queue::MpscQueue;

mod tasks;

mod auxiliary;
use auxiliary::Auxiliary;

mod cache;
use cache::WriteEndWithCachedId;

mod handle;
use handle::OwnedHandle;

/// Module contains types for manipulating files.
pub mod file;

/// Module contains types for manipulating directories.
pub mod fs;

/// Module contains types for manipulating metadata of files or directories.
pub mod metadata;

type Buffer = BytesMut;

type WriteEnd = lowlevel::WriteEnd<Buffer, MpscQueue, Auxiliary>;
type ReadEnd<R> = lowlevel::ReadEnd<R, Buffer, MpscQueue, Auxiliary>;
type SharedData = lowlevel::SharedData<Buffer, MpscQueue, Auxiliary>;
type Id = lowlevel::Id<Buffer>;
type Data = lowlevel::Data<Buffer>;

fn cancel_error() -> Error {
    Error::BackgroundTaskFailure(&"read/flush task failed")
}
