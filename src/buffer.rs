/// Any type that can be converted to a buffer.
pub trait ToBuffer {
    fn get_buffer(&mut self) -> Buffer<'_>;
}

/// Buffer that can be used to write data into.
pub enum Buffer<'a> {
    Vec(&'a mut Vec<u8>),
    Slice(&'a mut [u8]),
}

impl ToBuffer for Vec<u8> {
    fn get_buffer(&mut self) -> Buffer<'_> {
        Buffer::Vec(self)
    }
}

impl ToBuffer for Box<[u8]> {
    fn get_buffer(&mut self) -> Buffer<'_> {
        Buffer::Slice(&mut *self)
    }
}

impl<const LEN: usize> ToBuffer for [u8; LEN] {
    fn get_buffer(&mut self) -> Buffer<'_> {
        Buffer::Slice(self)
    }
}
