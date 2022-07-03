#![forbid(unsafe_code)]

use bytes::BytesMut;

/// Any type that can act as a buffer.
pub trait ToBuffer {
    /// Returned the buffer.
    fn get_buffer(&mut self) -> Buffer<'_>;
}

/// Buffer that can be used to write data into.
#[derive(Debug)]
pub enum Buffer<'a> {
    /// A `Vec<u8>` acts as a buffer.
    Vector(&'a mut Vec<u8>),

    /// A byte slice acts as a buffer.
    Slice(&'a mut [u8]),

    /// A `BytesMut` acts as a buffer.
    Bytes(&'a mut BytesMut),
}

impl ToBuffer for Vec<u8> {
    #[inline(always)]
    fn get_buffer(&mut self) -> Buffer<'_> {
        Buffer::Vector(self)
    }
}

impl ToBuffer for BytesMut {
    #[inline(always)]
    fn get_buffer(&mut self) -> Buffer<'_> {
        Buffer::Bytes(self)
    }
}

impl ToBuffer for Box<[u8]> {
    #[inline(always)]
    fn get_buffer(&mut self) -> Buffer<'_> {
        Buffer::Slice(&mut *self)
    }
}

impl<const LEN: usize> ToBuffer for [u8; LEN] {
    #[inline(always)]
    fn get_buffer(&mut self) -> Buffer<'_> {
        Buffer::Slice(self)
    }
}
