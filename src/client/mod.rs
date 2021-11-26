use tokio_pipe::{PipeRead, PipeWrite};

mod read_end;
mod threadsafe_waker;
mod write_end;

#[derive(Debug)]
pub struct Client {
    write_end: write_end::WriteEnd,
    read_end: read_end::ReadEnd,
}
impl Client {
    fn new(reader: PipeRead, writer: PipeWrite) -> Self {
        Self {
            read_end: read_end::ReadEnd::new(reader),
            write_end: write_end::WriteEnd::new(writer),
        }
    }
}
