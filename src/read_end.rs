use super::awaitable_responses::{AwaitableResponses, Response};
use super::Error;
use super::ToBuffer;

use core::fmt::Debug;
use core::marker::Unpin;

use std::sync::Arc;

use openssh_sftp_protocol::response::{self, ServerVersion};
use openssh_sftp_protocol::serde::Deserialize;
use openssh_sftp_protocol::ssh_format::from_bytes;

use tokio::io::{copy, sink, AsyncRead, AsyncReadExt};
use tokio_io_utility::read_exact_to_vec;

#[derive(Debug)]
pub struct ReadEnd<Reader: AsyncRead + Unpin, Buffer: ToBuffer + 'static> {
    reader: Reader,
    buffer: Vec<u8>,
    responses: Arc<AwaitableResponses<Buffer>>,
}

impl<Reader: AsyncRead + Unpin, Buffer: ToBuffer + Debug + 'static> ReadEnd<Reader, Buffer> {
    pub(crate) fn new(reader: Reader, responses: Arc<AwaitableResponses<Buffer>>) -> Self {
        Self {
            reader,
            buffer: Vec::new(),
            responses,
        }
    }

    pub(crate) async fn receive_server_version(&mut self, version: u32) -> Result<(), Error> {
        // Receive server version
        let len: u32 = self.read_and_deserialize(4).await?;
        self.read_exact(len as usize).await?;
        let server_version = ServerVersion::deserialize(&self.buffer)?;

        if server_version.version != version {
            Err(Error::UnsupportedSftpProtocol)
        } else {
            Ok(())
        }
    }

    async fn read_exact(&mut self, size: usize) -> Result<(), Error> {
        self.buffer.clear();
        read_exact_to_vec(&mut self.reader, &mut self.buffer, size).await?;

        Ok(())
    }

    fn deserialize<'a, T: Deserialize<'a>>(&'a self) -> Result<T, Error> {
        // Ignore any trailing bytes to be forward compatible
        Ok(from_bytes(&self.buffer)?.0)
    }

    async fn read_and_deserialize<'a, T>(&'a mut self, size: usize) -> Result<T, Error>
    where
        T: Deserialize<'a>,
    {
        self.read_exact(size).await?;
        self.deserialize()
    }

    async fn consume_data_packet(&mut self, len: u32) -> Result<(), Error> {
        copy(&mut (&mut self.reader).take(len as u64), &mut sink()).await?;
        Ok(())
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
        self.buffer.drain(0..4);

        // read_exact_to_vec does not overwrite any existing data.
        read_exact_to_vec(&mut self.reader, &mut self.buffer, len as usize).await?;

        let response: response::Response = self.deserialize()?;

        Ok(Response::Header(response.response_inner))
    }

    pub async fn read_in_one_packet(&mut self) -> Result<(), Error> {
        let (len, packet_type, response_id): (u32, u8, u32) = self.read_and_deserialize(9).await?;

        let len = len - 5;

        let callback = match self.responses.remove(response_id) {
            Ok(callback) => callback,

            // Invalid response_id
            Err(err) => {
                // Consume the invalid data to return self to a valid state
                // where read_in_one_packet can be called again.
                if let Err(consumption_err) = self.consume_data_packet(len).await {
                    return Err(Error::RecursiveErrors(
                        Box::new(err),
                        Box::new(consumption_err),
                    ));
                }
                return Err(err);
            }
        };

        let response = if response::Response::is_data(packet_type) {
            self.read_in_data_packet(len, callback.get_input()).await?
        } else {
            self.read_in_packet(len).await?
        };

        callback.do_callback(response);

        Ok(())
    }
}
