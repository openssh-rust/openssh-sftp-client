use super::CountedReader;

use std::io;

/// Prototype
#[derive(Debug)]
pub(crate) struct ResponseCallback {}

impl ResponseCallback {
    /// reader is used to read additional variable length data, especially
    /// one that can be very long (response body of read request).
    ///
    /// Return true if the callback is already called and should be removed.
    pub(crate) async fn call(
        &mut self,
        response: u8,
        reader: CountedReader<'_>,
    ) -> io::Result<bool> {
        todo!()
    }
}
