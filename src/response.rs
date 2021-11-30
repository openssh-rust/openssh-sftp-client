use super::{constants, Extensions};

use core::fmt;

use serde::de::{Deserialize, Deserializer, Error, SeqAccess, Unexpected, Visitor};
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
        status_code: StatusCode,

        /// ISO-10646 UTF-8 [RFC-2279]
        err_msg: String,

        /// [RFC-1766]
        language_tag: String,
    },
}

#[derive(Debug)]
pub(crate) struct Response {
    pub(crate) response_id: u32,
    pub(crate) response_inner: ResponseInner,
}

impl<'de> Deserialize<'de> for Response {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ResponseVisitor(usize);

        impl ResponseVisitor {
            fn get_next<'de, T, V>(&mut self, seq: &mut V) -> Result<T, V::Error>
            where
                T: Deserialize<'de>,
                V: SeqAccess<'de>,
            {
                let res = seq
                    .next_element()?
                    .ok_or_else(|| Error::invalid_length(self.0, self));
                self.0 += 1;
                res
            }
        }

        impl<'de> Visitor<'de> for ResponseVisitor {
            type Value = Response;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(formatter, "Expects a u8 type and payload")
            }

            fn visit_seq<V>(mut self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                use constants::*;
                use ResponseInner::*;

                let discriminant: u8 = self.get_next(&mut seq)?;
                let response_id: u32 = self.get_next(&mut seq)?;

                let response_inner = match discriminant {
                    SSH_FXP_STATUS => Status {
                        status_code: self.get_next(&mut seq)?,
                        err_msg: self.get_next(&mut seq)?,
                        language_tag: self.get_next(&mut seq)?,
                    },

                    _ => {
                        return Err(Error::invalid_value(
                            Unexpected::Unsigned(discriminant as u64),
                            &"Invalid packet type",
                        ))
                    }
                };

                Ok(Response {
                    response_id,
                    response_inner,
                })
            }
        }

        // Pass a dummy size here since ssh_format doesn't care
        deserializer.deserialize_tuple(3, ResponseVisitor(0))
    }
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
