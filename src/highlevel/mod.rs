//! Highlevle API of sftp client
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

use super::{lowlevel, Error};

pub use lowlevel::{UnixTimeStamp, UnixTimeStampError};

use bytes::BytesMut;

mod sftp;
pub use sftp::CancellationToken;
pub use sftp::Sftp;

mod cancel_utility;
use cancel_utility::BoxedWaitForCancellationFuture;

mod options;
pub use options::SftpOptions;

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

type WriteEnd<W> = lowlevel::WriteEnd<W, Buffer, Auxiliary>;
type ReadEnd<R, W> = lowlevel::ReadEnd<R, W, Buffer, Auxiliary>;
type SharedData<W> = lowlevel::SharedData<W, Buffer, Auxiliary>;
type Id = lowlevel::Id<Buffer>;
type Data = lowlevel::Data<Buffer>;
