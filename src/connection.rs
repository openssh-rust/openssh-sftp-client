use super::*;

use awaitable_responses::AwaitableResponses;

use openssh_sftp_protocol::constants::SSH2_FILEXFER_VERSION;
use openssh_sftp_protocol::request::{Hello, Request};
use openssh_sftp_protocol::response::{self, ServerVersion};
use openssh_sftp_protocol::serde::{Deserialize, Serialize};
use openssh_sftp_protocol::ssh_format::Transformer;

use core::fmt::Debug;

use std::io::IoSlice;

use tokio::io::{copy, sink, AsyncReadExt};
use tokio_io_utility::{read_exact_to_vec, AsyncWriteUtility};
use tokio_pipe::{PipeRead, PipeWrite};

#[derive(Debug)]
pub struct Connection<Buffer: ToBuffer> {
    writer: PipeWrite,
    reader: PipeRead,
    transformer: Transformer,
    responses: AwaitableResponses<Buffer>,
}
impl<Buffer: Debug + ToBuffer> Connection<Buffer> {
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
        self.transformer.get_buffer().clear();
        read_exact_to_vec(&mut self.reader, self.transformer.get_buffer(), size).await?;

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
            responses: AwaitableResponses::new(),
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

    async fn consume_data_packet(&mut self, len: u32) -> Result<(), Error> {
        copy(&mut (&mut self.reader).take(len as u64), &mut sink()).await?;
        todo!()
    }

    async fn read_in_data_packet_fallback(&mut self, len: u32) -> Result<Response<Buffer>, Error> {
        let mut vec = Vec::new();
        read_exact_to_vec(&mut self.reader, &mut vec, len as usize).await?;

        Ok(Response::AllocatedBox(vec.into_boxed_slice()))
    }

    /// * `len` - excludes packet_type and request_id.
    async fn read_in_data_packet(
        &mut self,
        len: u32,
        buffer: Option<Buffer>,
    ) -> Result<Response<Buffer>, Error> {
        if let Some(mut buffer) = buffer {
            match buffer.get_buffer() {
                crate::Buffer::Vector(vec) => {
                    read_exact_to_vec(&mut self.reader, vec, len as usize).await?;
                    Ok(Response::Buffer(buffer))
                }
                crate::Buffer::Slice(slice) => {
                    if slice.len() >= len as usize {
                        self.reader.read_exact(slice).await?;
                        Ok(Response::Buffer(buffer))
                    } else {
                        self.read_in_data_packet_fallback(len).await
                    }
                }
            }
        } else {
            self.read_in_data_packet_fallback(len).await
        }
    }

    /// * `len` - excludes packet_type and request_id.
    async fn read_in_packet(&mut self, len: u32) -> Result<Response<Buffer>, Error> {
        // Remove the len
        self.transformer.get_buffer().drain(0..4);

        // read_exact_to_vec does not overwrite any existing data.
        read_exact_to_vec(
            &mut self.reader,
            &mut self.transformer.get_buffer(),
            len as usize,
        )
        .await?;

        let response: response::Response = self.deserialize()?;

        Ok(Response::Header(response.response_inner))
    }

    pub async fn read_in_one_packet(&mut self) -> Result<(), Error> {
        let (len, packet_type, response_id): (u32, u8, u32) = self.read_and_deserialize(9).await?;

        let len = len - 5;

        let response = if response::Response::is_data(packet_type) {
            let buffer = match self.responses.get_input(response_id) {
                Ok(buffer) => buffer,

                // Invalid response_id
                Err(err) => {
                    if let Err(consumption_err) = self.consume_data_packet(len).await {
                        return Err(Error::RecursiveErrors(
                            Box::new(err),
                            Box::new(consumption_err),
                        ));
                    }
                    return Err(err);
                }
            };
            self.read_in_data_packet(len, buffer).await?
        } else {
            self.read_in_packet(len).await?
        };

        self.responses.do_callback(response_id, response)?;

        Ok(())
    }
}
