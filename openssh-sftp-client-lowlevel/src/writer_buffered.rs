#![forbid(unsafe_code)]

use std::convert::TryInto;
use std::io::IoSlice;

use crate::openssh_sftp_protocol::ssh_format::SerBacker;
use bytes::{BufMut, Bytes, BytesMut};

#[repr(transparent)]
#[derive(Debug)]
pub(crate) struct WriteBuffer(BytesMut);

impl WriteBuffer {
    /// split out one buffer
    pub(crate) fn split(&mut self) -> Bytes {
        self.0.split().freeze()
    }

    pub(crate) fn put_io_slices(&mut self, io_slices: &[IoSlice<'_>]) {
        for io_slice in io_slices {
            self.0.put_slice(&*io_slice);
        }
    }
}

impl SerBacker for WriteBuffer {
    fn new() -> Self {
        // Since `BytesMut` v1.1.0 does not reuse the underlying `Vec` that is shared
        // with other `BytesMut`/`Bytes` if it is too small.
        let mut bytes = BytesMut::with_capacity(256);
        bytes.put([0_u8, 0_u8, 0_u8, 0_u8].as_ref());
        Self(bytes)
    }

    #[inline(always)]
    fn len(&self) -> usize {
        self.0.len()
    }

    fn get_first_4byte_slice(&mut self) -> &mut [u8; 4] {
        let slice = &mut self.0[..4];
        slice.try_into().unwrap()
    }

    #[inline(always)]
    fn extend_from_slice(&mut self, other: &[u8]) {
        self.0.extend_from_slice(other);
    }

    #[inline(always)]
    fn push(&mut self, byte: u8) {
        self.0.put_u8(byte);
    }

    #[inline(always)]
    fn reset(&mut self) {
        self.0.resize(4, 0);
    }

    #[inline(always)]
    fn reserve(&mut self, additional: usize) {
        self.0.reserve(additional);
    }
}
