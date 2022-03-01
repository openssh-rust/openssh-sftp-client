//! Highlevle API of sftp client
//!
//! All `async` functions in this module are cancel safe.
//!
//! Internally, this is archived by first writing requests into a write buffer
//! containing `bytes::Bytes` and then flush all buffers at once periodically
//! to archive cancel safety and improve efficiencies.
//!
//! However, cancelling the future does not actually has any effect,
//! since the requests are sent regardless of the cancellation.
//!
//! Thus, if you cancel a future that changes the remote filesystem in any way,
//! then the change would still happen regardless.

use crate::Writer;

use super::{lowlevel, Error};

pub use lowlevel::{UnixTimeStamp, UnixTimeStampError};

use bytes::BytesMut;

mod sftp;
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

mod file;
pub use file::TokioCompactFile;
pub use file::DEFAULT_BUFLEN;
pub use file::DEFAULT_MAX_BUFLEN;
pub use file::{max_atomic_write_len, File, OpenOptions};

mod fs;
pub use fs::DirEntry;
pub use fs::ReadDir;
pub use fs::{Dir, DirBuilder, Fs};

mod metadata;
pub use metadata::{FileType, MetaData, MetaDataBuilder, Permissions};

type Buffer = BytesMut;

type WriteEnd<W> = lowlevel::WriteEnd<W, Buffer, Auxiliary>;
type ReadEnd<W> = lowlevel::ReadEnd<W, Buffer, Auxiliary>;
type SharedData<W> = lowlevel::SharedData<W, Buffer, Auxiliary>;
type Id = lowlevel::Id<Buffer>;
type Data = lowlevel::Data<Buffer>;
