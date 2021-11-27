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
    async fn negotiate(reader: &mut PipeRead, writer: &mut PipeWrite) {}

    pub async fn connect(mut reader: PipeRead, mut writer: PipeWrite) -> Self {
        Self::negotiate(&mut reader, &mut writer).await;

        Self {
            read_end: read_end::ReadEnd::new(reader),
            write_end: write_end::WriteEnd::new(writer),
        }
    }
}
