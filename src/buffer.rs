/// Any type that can act as a buffer.
pub trait ToBuffer {
    fn get_buffer(&mut self) -> Buffer<'_>;
}

/// Buffer that can be used to write data into.
pub enum Buffer<'a> {
    Vector(&'a mut Vec<u8>),
    Slice(&'a mut [u8]),
}

impl ToBuffer for Vec<u8> {
    fn get_buffer(&mut self) -> Buffer<'_> {
        Buffer::Vector(self)
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
