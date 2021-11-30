use core::ops::Deref;

use serde::de::{Deserialize, Deserializer, Error};
use serde::Serialize;

pub use vec_strings::Strings;

#[derive(Debug, Default, Eq, PartialEq, Clone, Hash, Serialize)]
#[serde(transparent)]
pub struct Extensions(Strings);

impl Extensions {
    /// Return Some(...) if strs.len() is even.
    /// None otherwise.
    pub fn new(strs: Strings) -> Option<Self> {
        if strs.len() % 2 == 0 {
            Some(Self(strs))
        } else {
            None
        }
    }

    pub fn add_extension(&mut self, name: &str, data: &str) {
        self.0.push(name);
        self.0.push(data);
    }

    #[inline(always)]
    pub fn reserve(&mut self, extensions: usize) {
        self.0.reserve(extensions * 2);
    }

    #[inline(always)]
    pub fn reserve_strs(&mut self, cnt: usize) {
        self.0.reserve(cnt);
    }

    pub fn shrink_to_fit(&mut self) {
        self.0.shrink_to_fit();
        self.0.shrink_to_fit();
    }
}

impl Deref for Extensions {
    type Target = Strings;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Extensions {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let strs = Strings::deserialize(deserializer)?;

        let len = strs.len() as usize;

        Extensions::new(strs)
            .ok_or_else(|| Error::invalid_length(len, &"Expected even number of strings"))
    }
}
