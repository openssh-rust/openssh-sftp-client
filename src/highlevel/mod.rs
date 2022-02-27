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
pub use file::{File, OpenOptions, MAX_ATOMIC_WRITE_LEN};

mod fs;
pub use fs::DirEntry;
pub use fs::ReadDir;
pub use fs::{Dir, DirBuilder, Fs};

mod metadata;
pub use metadata::{FileType, MetaData, MetaDataBuilder, Permissions};

type Buffer = BytesMut;

type WriteEnd = lowlevel::WriteEnd<Buffer, Auxiliary>;
type ReadEnd = lowlevel::ReadEnd<Buffer, Auxiliary>;
type SharedData = lowlevel::SharedData<Buffer, Auxiliary>;
type Id = lowlevel::Id<Buffer>;
type Data = lowlevel::Data<Buffer>;
