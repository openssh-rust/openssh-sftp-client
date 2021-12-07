use super::awaitable_responses::{AwaitableResponse, AwaitableResponses};
use super::Error;
use super::ToBuffer;

use core::fmt::Debug;
use core::marker::Unpin;

use std::sync::Arc;

use openssh_sftp_protocol::extensions::Extensions;
use openssh_sftp_protocol::request::{Hello, Request, RequestInner};
use openssh_sftp_protocol::serde::Serialize;
use openssh_sftp_protocol::ssh_format::Serializer;

use std::io::IoSlice;
use tokio::io::AsyncWrite;
use tokio_io_utility::AsyncWriteUtility;

/// TODO:
///  - Support IoSlice for data in `send_write_request`

#[derive(Debug)]
pub struct WriteEnd<Writer: AsyncWrite + Unpin, Buffer: ToBuffer + 'static> {
    writer: Writer,
    serializer: Serializer,
    responses: Arc<AwaitableResponses<Buffer>>,
}

impl<Writer: AsyncWrite + Unpin, Buffer: ToBuffer + Debug + 'static> WriteEnd<Writer, Buffer> {
    pub(crate) fn new(writer: Writer, responses: Arc<AwaitableResponses<Buffer>>) -> Self {
        Self {
            writer,
            serializer: Serializer::new(),
            responses,
        }
    }

    pub(crate) async fn send_hello(
        &mut self,
        version: u32,
        extensions: Extensions,
    ) -> Result<(), Error> {
        self.write(Hello {
            version,
            extensions,
        })
        .await
    }

    async fn write<T>(&mut self, value: T) -> Result<(), Error>
    where
        T: Serialize,
    {
        self.serializer.reset();
        value.serialize(&mut self.serializer)?;
        self.writer
            .write_vectored_all(&mut [IoSlice::new(self.serializer.get_output()?)])
            .await?;

        Ok(())
    }

    /// Send requests.
    ///
    /// **Please use `Self::send_write_request` for sending write requests.**
    pub async fn send_request(
        &mut self,
        request: RequestInner<'_>,
        buffer: Option<Buffer>,
    ) -> Result<AwaitableResponse<Buffer>, Error> {
        let (request_id, awaitable_response) = self.responses.insert(buffer);
        match self
            .write(Request {
                request_id,
                inner: request,
            })
            .await
        {
            Ok(_) => Ok(awaitable_response),
            Err(err) => {
                self.responses.remove(request_id).unwrap();

                Err(err)
            }
        }
    }

    async fn send_write_request_impl(
        &mut self,
        request_id: u32,
        handle: &[u8],
        offset: u64,
        data: &[u8],
    ) -> Result<(), Error> {
        let header = Request::serialize_write_request(
            &mut self.serializer,
            request_id,
            handle,
            offset,
            data.len().try_into().map_err(|_| Error::BufferTooLong)?,
        )?;

        let mut slices = [IoSlice::new(header), IoSlice::new(data)];
        self.writer.write_vectored_all(&mut slices).await?;

        Ok(())
    }

    /// Send write requests
    pub async fn send_write_request(
        &mut self,
        handle: &[u8],
        offset: u64,
        data: &[u8],
    ) -> Result<AwaitableResponse<Buffer>, Error> {
        let (request_id, awaitable_response) = self.responses.insert(None);

        match self
            .send_write_request_impl(request_id, handle, offset, data)
            .await
        {
            Ok(_) => Ok(awaitable_response),
            Err(err) => {
                self.responses.remove(request_id).unwrap();

                Err(err)
            }
        }
    }
}
