use super::constants;

use core::fmt;

use serde::de::{Deserialize, Deserializer, Error, SeqAccess, Visitor};
use serde::Serialize;

use vec_strings::Strings;

#[derive(Debug, Default, Serialize)]
pub struct FileAttrs {
    flags: u32,

    /// present only if flag SSH_FILEXFER_ATTR_SIZE
    size: Option<u64>,

    /// present only if flag SSH_FILEXFER_ATTR_UIDGID
    ///
    /// Stores uid and gid.
    id: Option<(u32, u32)>,

    /// present only if flag SSH_FILEXFER_ATTR_PERMISSIONS
    permissions: Option<u32>,

    /// present only if flag SSH_FILEXFER_ATTR_ACMODTIME
    ///
    /// Stores atime and mtime.
    time: Option<(u32, u32)>,

    /// present only if flag SSH_FILEXFER_ATTR_EXTENDED
    extensions: Option<Strings>,
}

impl FileAttrs {
    pub fn set_size(&mut self, size: u64) {
        self.flags |= constants::SSH_FILEXFER_ATTR_SIZE;
        self.size = Some(size);
    }

    pub fn set_uid(&mut self, uid: u32, gid: u32) {
        self.flags |= constants::SSH_FILEXFER_ATTR_UIDGID;
        self.id = Some((uid, gid));
    }

    pub fn set_permissions(&mut self, permissions: u32) {
        self.flags |= constants::SSH_FILEXFER_ATTR_PERMISSIONS;
        self.permissions = Some(permissions);
    }

    pub fn set_time(&mut self, atime: u32, mtime: u32) {
        self.flags |= constants::SSH_FILEXFER_ATTR_ACMODTIME;
        self.time = Some((atime, mtime));
    }

    pub fn set_extensions(&mut self, extensions: Strings) {
        self.flags |= constants::SSH_FILEXFER_ATTR_EXTENDED;
        self.extensions = Some(extensions);
    }

    pub fn get_size(&self) -> Option<u64> {
        self.size
    }

    /// Return uid and gid
    pub fn get_id(&self) -> Option<(u32, u32)> {
        self.id
    }

    pub fn get_permissions(&self) -> Option<u32> {
        self.permissions
    }

    /// Return atime and mtime
    pub fn get_time(&self) -> Option<(u32, u32)> {
        self.time
    }

    pub fn get_extensions(&self) -> &Option<Strings> {
        &self.extensions
    }

    pub fn get_extensions_mut(&mut self) -> &mut Option<Strings> {
        &mut self.extensions
    }
}

/// This implementation is only usuable for ssh_format::Deserializer
impl<'de> Deserialize<'de> for FileAttrs {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct FileAttrVisitor(usize);

        impl FileAttrVisitor {
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

        impl<'de> Visitor<'de> for FileAttrVisitor {
            type Value = FileAttrs;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(formatter, "A u32 length and &[str]")
            }

            fn visit_seq<V>(mut self, mut seq: V) -> Result<Self::Value, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let mut attrs = FileAttrs::default();

                let flags = self.get_next(&mut seq)?;

                attrs.flags = flags;

                if (flags & constants::SSH_FILEXFER_ATTR_SIZE) != 0 {
                    attrs.size = Some(self.get_next(&mut seq)?);
                }
                if (flags & constants::SSH_FILEXFER_ATTR_UIDGID) != 0 {
                    attrs.id = Some((self.get_next(&mut seq)?, self.get_next(&mut seq)?));
                }
                if (flags & constants::SSH_FILEXFER_ATTR_PERMISSIONS) != 0 {
                    attrs.permissions = Some(self.get_next(&mut seq)?);
                }
                if (flags & constants::SSH_FILEXFER_ATTR_ACMODTIME) != 0 {
                    attrs.time = Some((self.get_next(&mut seq)?, self.get_next(&mut seq)?));
                }
                if (flags & constants::SSH_FILEXFER_ATTR_EXTENDED) != 0 {
                    attrs.extensions = Some(self.get_next(&mut seq)?);
                }

                Ok(attrs)
            }
        }

        deserializer.deserialize_tuple(1, FileAttrVisitor(0))
    }
}
