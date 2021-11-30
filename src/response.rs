use super::{constants, Extensions};

use ssh_format::from_bytes;

use vec_strings::Strings;

pub(crate) struct ServerVersion {
    version: u32,
    extensions: Extensions,
}
impl ServerVersion {
    /// * `bytes` - should not include the initial 4-byte which server
    ///   as the length of the whole packet.
    pub(crate) fn deserialize(bytes: &[u8]) -> ssh_format::Result<Self> {
        let (version, mut bytes) = from_bytes(bytes)?;

        let mut strings = Strings::new();
        while !bytes.is_empty() {
            let (string, bytes_left) = from_bytes(bytes)?;
            strings.push(string);

            bytes = bytes_left;
        }

        if let Some(extensions) = Extensions::new(strings) {
            Ok(Self {
                version,
                extensions,
            })
        } else {
            Err(ssh_format::Error::Eof)
        }
    }
}

#[derive(Debug)]
pub(crate) enum ResponseInner {
    Status {
        status_code: u32,

        /// ISO-10646 UTF-8 [RFC-2279]
        err_msg: String,

        /// [RFC-1766]
        language_tag: String,
    },
}

#[derive(Debug)]
pub(crate) struct Response {
    response_id: u32,
}

#[derive(Debug)]
#[repr(u32)]
pub(crate) enum StatusCode {
    Success = constants::SSH_FX_OK,
    Eof = constants::SSH_FX_EOF,
    NoSuchFile = constants::SSH_FX_NO_SUCH_FILE,
    PermDenied = constants::SSH_FX_PERMISSION_DENIED,
    Failure = constants::SSH_FX_FAILURE,
    BadMessage = constants::SSH_FX_BAD_MESSAGE,
    NoConnection = constants::SSH_FX_NO_CONNECTION,
    ConnectionLost = constants::SSH_FX_CONNECTION_LOST,
    OpUnsupported = constants::SSH_FX_OP_UNSUPPORTED,
}
