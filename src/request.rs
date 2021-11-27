use super::constants;

use bitflags::bitflags;
use serde::Serialize;

#[derive(Debug)]
pub(crate) enum Request {
    /// Response with `Response::Version`.
    Hello { version: u32 },
}

bitflags! {
    #[derive(Serialize)]
    pub(crate) struct OpenFlags: u32 {
        /// Open the file for reading.
        const Read = constants::SSH_FXF_READ;

        /// Open the file for writing.
        /// If both this and Read are specified; the file is opened for both
        /// reading and writing.
        const Write = constants::SSH_FXF_WRITE;

        /// Force all writes to append data at the end of the file.
        const Append = constants::SSH_FXF_APPEND;

        /// If this flag is specified; then a new file will be created if one does not
        /// already exist (if Trunc is specified; the new file will
        /// be truncated to zero length if it previously exists).
        const Create = constants::SSH_FXF_CREAT;

        /// Forces an existing file with the same name to be truncated to zero
        /// length when creating a file by specifying Create.
        /// Create MUST also be specified if this flag is used.
        const Trunc = constants::SSH_FXF_TRUNC;

        /// Causes the request to fail if the named file already exists.
        /// Create MUST also be specified if this flag is used.
        const Excl = constants::SSH_FXF_EXCL;
    }
}
