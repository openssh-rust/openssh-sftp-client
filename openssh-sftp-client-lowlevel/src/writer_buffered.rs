#![forbid(unsafe_code)]

use std::io::IoSlice;

use crate::openssh_sftp_protocol::ssh_format::SerOutput;
use bytes::{BufMut, Bytes, BytesMut};

#[repr(transparent)]
#[derive(Debug, Default)]
pub(crate) struct WriteBuffer(pub(crate) BytesMut);

impl WriteBuffer {
    /// split out one buffer
    pub(crate) fn split(&mut self) -> Bytes {
        self.0.split().freeze()
    }

    pub(crate) fn put_io_slices(&mut self, io_slices: &[IoSlice<'_>]) {
        for io_slice in io_slices {
            self.0.put_slice(io_slice);
        }
    }
}

impl SerOutput for WriteBuffer {
    #[inline(always)]
    fn extend_from_slice(&mut self, other: &[u8]) {
        self.0.extend_from_slice(other);
    }

    #[inline(always)]
    fn push(&mut self, byte: u8) {
        self.0.put_u8(byte);
    }

    #[inline(always)]
    fn reserve(&mut self, additional: usize) {
        self.0.reserve(additional);
    }
}
