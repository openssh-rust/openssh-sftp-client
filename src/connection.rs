use super::*;

use awaitable_responses::AwaitableResponses;

use openssh_sftp_protocol::constants::SSH2_FILEXFER_VERSION;
use openssh_sftp_protocol::request::{Hello, Request};
use openssh_sftp_protocol::response::{self, ServerVersion};
use openssh_sftp_protocol::serde::{Deserialize, Serialize};
use ssh_format::Transformer;

use std::io::IoSlice;

use tokio::io::AsyncReadExt;
use tokio_io_utility::AsyncWriteUtility;
use tokio_pipe::{PipeRead, PipeWrite};

#[derive(Debug)]
pub struct Connection {
    writer: PipeWrite,
    reader: PipeRead,
    transformer: Transformer,
    responses: AwaitableResponses,
}
impl Connection {
    async fn write<T>(&mut self, value: T) -> Result<(), Error>
    where
        T: Serialize,
    {
        self.writer
            .write_vectored_all(&mut [IoSlice::new(self.transformer.serialize(value)?)])
            .await?;

        Ok(())
    }

    async fn read_exact(&mut self, size: usize) -> Result<(), Error> {
        self.transformer.get_buffer().resize(size, 0);
        self.reader
            .read_exact(&mut self.transformer.get_buffer())
            .await?;

        Ok(())
    }

    fn deserialize<'a, T: Deserialize<'a>>(&'a self) -> Result<T, Error> {
        // Ignore any trailing bytes to be forward compatible
        Ok(self.transformer.deserialize()?.0)
    }

    async fn read_and_deserialize<'a, T>(&'a mut self, size: usize) -> Result<T, Error>
    where
        T: Deserialize<'a>,
    {
        self.read_exact(size).await?;
        self.deserialize()
    }

    async fn negotiate(&mut self) -> Result<(), Error> {
        let version = SSH2_FILEXFER_VERSION;

        // Sent hello message
        self.write(Hello {
            version,
            extensions: Default::default(),
        })
        .await?;

        // Receive server version
        let len: u32 = self.read_and_deserialize(4).await?;
        self.read_exact(len as usize).await?;
        let server_version = ServerVersion::deserialize(self.transformer.get_buffer())?;

        if server_version.version != version {
            Err(Error::UnsupportedSftpProtocol)
        } else {
            Ok(())
        }
    }

    pub async fn new(reader: PipeRead, writer: PipeWrite) -> Result<Self, Error> {
        let mut val = Self {
            reader,
            writer,
            transformer: Transformer::default(),
            responses: AwaitableResponses::default(),
        };

        val.negotiate().await?;

        Ok(val)
    }

    /// Send requests.
    ///
    /// **Please use `Self::send_write_request` for sending write requests.**
    pub async fn send_request(
        &mut self,
        request: RequestInner<'_>,
    ) -> Result<AwaitableResponse, Error> {
        let (request_id, awaitable_response) = self.responses.insert();
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
            self.transformer.get_ser(),
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
    ) -> Result<AwaitableResponse, Error> {
        let (request_id, awaitable_response) = self.responses.insert();

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

    /// * `len` - includes packet_type.
    async fn read_in_data_packet(&mut self, len: u32) -> Result<Response, Error> {
        // 1 byte for the packet_type and 4 byte for the request_id
        let mut vec = vec![0; (len - 5) as usize];
        self.reader.read_exact(&mut vec).await?;

        Ok(Response::Buffer(vec.into_boxed_slice()))
    }

    /// * `len` - includes the packet type
    async fn read_in_packet(&mut self, len: u32) -> Result<Response, Error> {
        // Remove the len
        self.transformer.get_buffer().drain(0..4);

        // Read in the rest of the packets, but do not overwrite the packet_type
        // and the response_id
        self.transformer.get_buffer().resize(len as usize, 0);
        self.reader
            // Skip packet_type (1 byte) and response_id (4 byte)
            .read_exact(&mut self.transformer.get_buffer()[5..])
            .await?;

        let response: response::Response = self.deserialize()?;

        Ok(Response::Header(response.response_inner))
    }

    pub async fn read_in_one_packet(&mut self) -> Result<(), Error> {
        let (len, packet_type, response_id): (u32, u8, u32) = self.read_and_deserialize(9).await?;

        let response = if response::Response::is_data(packet_type) {
            self.read_in_data_packet(len).await?
        } else {
            self.read_in_packet(len).await?
        };

        self.responses.do_callback(response_id, response);

        Ok(())
    }
}
