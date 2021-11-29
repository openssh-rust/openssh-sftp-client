use super::{constants, FileAttrs};

use std::borrow::Cow;
use std::path::Path;

use bitflags::bitflags;
use serde::Serialize;

#[derive(Debug)]
pub(crate) enum Request<'a> {
    /// Response with `Response::Version`.
    Hello {
        version: u32,
    },

    Open {
        request_id: u32,
        params: OpenFile<'a>,
    },

    Close {
        request_id: u32,
        handle: &'a str,
    },
}

bitflags! {
    pub struct FileMode: u32 {
        /// Open the file for reading.
        const READ = constants::SSH_FXF_READ;

        /// Open the file for writing.
        /// If both this and Read are specified; the file is opened for both
        /// reading and writing.
        const WRITE = constants::SSH_FXF_WRITE;

        /// Force all writes to append data at the end of the file.
        const APPEND = constants::SSH_FXF_APPEND;
    }

    pub struct CreateFlags: u32 {
        /// Forces an existing file with the same name to be truncated to zero
        /// length when creating a file by specifying Create.
        const TRUNC = constants::SSH_FXF_TRUNC;

        /// Causes the request to fail if the named file already exists.
        const EXCL = constants::SSH_FXF_EXCL;
    }
}

#[derive(Debug, Serialize)]
pub struct OpenFile<'a> {
    filename: Cow<'a, Path>,
    flags: u32,
    attrs: Option<FileAttrs>,
}
impl<'a> OpenFile<'a> {
    pub fn open(filename: Cow<'a, Path>, mode: FileMode) -> Self {
        Self {
            filename,
            flags: mode.bits(),
            attrs: None,
        }
    }

    pub fn create(
        filename: Cow<'a, Path>,
        mode: FileMode,
        create_flags: CreateFlags,
        attrs: FileAttrs,
    ) -> Self {
        Self {
            filename,
            flags: mode.bits() | constants::SSH_FXF_CREAT | create_flags.bits(),
            attrs: Some(attrs),
        }
    }
}
