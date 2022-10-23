use super::{lowlevel, UnixTimeStampError};
use std::time::{Duration, SystemTime};

/// Default value is 1970-01-01 00:00:00 UTC.
///
/// UnixTimeStamp stores number of seconds elapsed since 1970-01-01 00:00:00 UTC
/// as `u32`.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct UnixTimeStamp(pub(crate) lowlevel::UnixTimeStamp);

impl UnixTimeStamp {
    /// Create new unix timestamp from `system_time`.
    pub fn new(system_time: SystemTime) -> Result<Self, UnixTimeStampError> {
        lowlevel::UnixTimeStamp::new(system_time).map(Self)
    }

    /// Return unix epoch, same as [`UnixTimeStamp::default`]
    pub const fn unix_epoch() -> Self {
        Self(lowlevel::UnixTimeStamp::unix_epoch())
    }

    /// Return `None` if [`std::time::SystemTime`] cannot hold the timestamp.
    pub fn from_raw(elapsed: u32) -> Option<Self> {
        lowlevel::UnixTimeStamp::from_raw(elapsed).map(Self)
    }

    /// Into `u32` which is used to internally store the timestamp in seconds.
    pub fn into_raw(self) -> u32 {
        self.0.into_raw()
    }

    /// Convert timestamp to [`Duration`].
    pub fn as_duration(self) -> Duration {
        self.0.as_duration()
    }

    /// Convert timestamp back to [`SystemTime`].
    pub fn as_system_time(self) -> SystemTime {
        self.0.as_system_time()
    }
}
