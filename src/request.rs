use super::{constants, Extensions, FileAttrs};

use std::borrow::Cow;
use std::path::Path;

use bitflags::bitflags;
use serde::{Serialize, Serializer};

#[derive(Debug)]
pub(crate) enum Request<'a> {
    /// Response with `Response::Version`.
    Hello {
        version: u32,
        extensions: Extensions,
    },

    /// The response to this message will be either SSH_FXP_HANDLE
    /// (if the operation is successful) or SSH_FXP_STATUS
    /// (if the operation fails).
    Open {
        request_id: u32,
        params: OpenFile<'a>,
    },

    /// Response will be SSH_FXP_STATUS.
    Close { request_id: u32, handle: &'a [u8] },

    /// In response to this request, the server will read as many bytes as it
    /// can from the file (up to `len'), and return them in a SSH_FXP_DATA
    /// message.  If an error occurs or EOF is encountered before reading any
    /// data, the server will respond with SSH_FXP_STATUS.
    ///
    /// For normal disk files, it is guaranteed that this will read the specified
    /// number of bytes, or up to end of file.
    ///
    /// For e.g. device files this may return fewer bytes than requested.
    Read {
        request_id: u32,
        handle: &'a [u8],
        offset: u64,
        len: u32,
    },

    /// The write will extend the file if writing beyond the end of the file.
    ///
    /// It is legal to write way beyond the end of the file, the semantics
    /// are to write zeroes from the end of the file to the specified offset
    /// and then the data.
    ///
    /// On most operating systems, such writes do not allocate disk space but
    /// instead leave "holes" in the file.
    ///
    /// The server responds to a write request with a SSH_FXP_STATUS message.
    ///
    /// The Write also includes any amount of custom data and its size is
    /// included in the size of the entire packet sent.
    Write {
        request_id: u32,
        handle: &'a [u8],
        offset: u64,
    },

    /// Responds with a SSH_FXP_STATUS message.
    Remove {
        request_id: u32,
        filename: Cow<'a, Path>,
    },

    /// Responds with a SSH_FXP_STATUS message.
    Rename {
        request_id: u32,
        oldpath: Cow<'a, Path>,
        newpath: Cow<'a, Path>,
    },

    /// Responds with a SSH_FXP_STATUS message.
    Mkdir {
        request_id: u32,
        path: Cow<'a, Path>,
        attrs: FileAttrs,
    },

    /// Responds with a SSH_FXP_STATUS message.
    Rmdir {
        request_id: u32,
        path: Cow<'a, Path>,
    },

    /// Responds with a SSH_FXP_HANDLE or a SSH_FXP_STATUS message.
    Opendir {
        request_id: u32,
        path: Cow<'a, Path>,
    },

    /// Responds with a SSH_FXP_NAME or a SSH_FXP_STATUS message
    Readdir { request_id: u32, handle: &'a [u8] },

    /// Responds with SSH_FXP_ATTRS or SSH_FXP_STATUS.
    Stat {
        request_id: u32,
        path: Cow<'a, Path>,
    },

    /// Responds with SSH_FXP_ATTRS or SSH_FXP_STATUS.
    ///
    /// Does not follow symlink.
    Lstat {
        request_id: u32,
        path: Cow<'a, Path>,
    },

    /// Responds with SSH_FXP_ATTRS or SSH_FXP_STATUS.
    Fstat { request_id: u32, handle: &'a [u8] },

    /// Responds with SSH_FXP_STATUS
    Setstat {
        request_id: u32,
        path: Cow<'a, Path>,
        attrs: FileAttrs,
    },

    /// Responds with SSH_FXP_STATUS
    Fsetstat {
        request_id: u32,
        handle: &'a [u8],
        attrs: FileAttrs,
    },

    /// Responds with SSH_FXP_NAME with a name and dummy attribute value
    /// or SSH_FXP_STATUS on error.
    Readlink {
        request_id: u32,
        path: Cow<'a, Path>,
    },

    /// Responds with SSH_FXP_STATUS.
    Symlink {
        request_id: u32,
        linkpath: Cow<'a, Path>,
        targetpath: Cow<'a, Path>,
    },

    /// Responds with SSH_FXP_NAME with a name and dummy attribute value
    /// or SSH_FXP_STATUS on error.
    Realpath {
        request_id: u32,
        path: Cow<'a, Path>,
    },
}
impl Serialize for Request<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use Request::*;

        match self {
            Hello {
                version,
                extensions,
            } => (constants::SSH_FXP_INIT, *version, extensions.iter()).serialize(serializer),

            Open { request_id, params } => {
                (constants::SSH_FXP_OPEN, *request_id, params).serialize(serializer)
            }
            Close { request_id, handle } => {
                (constants::SSH_FXP_CLOSE, *request_id, *handle).serialize(serializer)
            }
            Read {
                request_id,
                handle,
                offset,
                len,
            } => {
                (constants::SSH_FXP_READ, *request_id, *handle, *offset, *len).serialize(serializer)
            }
            Write {
                request_id,
                handle,
                offset,
            } => (constants::SSH_FXP_WRITE, *request_id, *handle, *offset).serialize(serializer),

            Remove {
                request_id,
                filename,
            } => (constants::SSH_FXP_REMOVE, *request_id, filename).serialize(serializer),

            Rename {
                request_id,
                oldpath,
                newpath,
            } => (constants::SSH_FXP_RENAME, *request_id, oldpath, newpath).serialize(serializer),

            Mkdir {
                request_id,
                path,
                attrs,
            } => (constants::SSH_FXP_MKDIR, *request_id, path, attrs).serialize(serializer),

            Rmdir { request_id, path } => {
                (constants::SSH_FXP_MKDIR, *request_id, path).serialize(serializer)
            }

            Opendir { request_id, path } => {
                (constants::SSH_FXP_OPENDIR, *request_id, path).serialize(serializer)
            }

            Readdir { request_id, handle } => {
                (constants::SSH_FXP_READDIR, *request_id, *handle).serialize(serializer)
            }

            Stat { request_id, path } => {
                (constants::SSH_FXP_STAT, *request_id, path).serialize(serializer)
            }

            Lstat { request_id, path } => {
                (constants::SSH_FXP_LSTAT, *request_id, path).serialize(serializer)
            }

            Fstat { request_id, handle } => {
                (constants::SSH_FXP_FSTAT, *request_id, *handle).serialize(serializer)
            }

            Setstat {
                request_id,
                path,
                attrs,
            } => (constants::SSH_FXP_SETSTAT, *request_id, path, attrs).serialize(serializer),

            Fsetstat {
                request_id,
                handle,
                attrs,
            } => (constants::SSH_FXP_FSETSTAT, *request_id, *handle, attrs).serialize(serializer),

            Readlink { request_id, path } => {
                (constants::SSH_FXP_READLINK, *request_id, path).serialize(serializer)
            }

            Symlink {
                request_id,
                linkpath,
                targetpath,
            } => (
                constants::SSH_FXP_SYMLINK,
                *request_id,
                linkpath,
                targetpath,
            )
                .serialize(serializer),

            Realpath { request_id, path } => {
                (constants::SSH_FXP_REALPATH, *request_id, path).serialize(serializer)
            }
        }
    }
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
