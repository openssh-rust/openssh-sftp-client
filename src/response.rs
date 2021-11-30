use super::{constants, Extensions};

use serde::de::{Deserialize, Deserializer, Error, Unexpected};
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
pub(crate) enum StatusCode {
    Success,
    Eof,
    NoSuchFile,
    PermDenied,
    Failure,
    BadMessage,
    NoConnection,
    ConnectionLost,
    OpUnsupported,
}
impl<'de> Deserialize<'de> for StatusCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use constants::*;
        use StatusCode::*;

        let discriminent = <u32 as Deserialize>::deserialize(deserializer)?;

        match discriminent {
            SSH_FX_OK => Ok(Success),
            SSH_FX_EOF => Ok(Eof),
            SSH_FX_NO_SUCH_FILE => Ok(NoSuchFile),
            SSH_FX_PERMISSION_DENIED => Ok(PermDenied),
            SSH_FX_FAILURE => Ok(Failure),
            SSH_FX_BAD_MESSAGE => Ok(BadMessage),
            SSH_FX_NO_CONNECTION => Ok(NoConnection),
            SSH_FX_CONNECTION_LOST => Ok(ConnectionLost),
            SSH_FX_OP_UNSUPPORTED => Ok(OpUnsupported),

            _ => Err(Error::invalid_value(
                Unexpected::Unsigned(discriminent as u64),
                &"Invalid status code",
            )),
        }
    }
}
