use super::awaitable_responses::ArenaArc;
use super::awaitable_responses::Response;
use super::connection::SharedData;
use super::Error;
use super::Extensions;
use super::ToBuffer;

use core::fmt::Debug;

use std::sync::Arc;

use openssh_sftp_protocol::response::{self, ServerVersion};
use openssh_sftp_protocol::serde::Deserialize;
use openssh_sftp_protocol::ssh_format::from_bytes;

use tokio::io::{copy, sink, AsyncReadExt};
use tokio_io_utility::read_exact_to_vec;
use tokio_pipe::PipeRead;

#[derive(Debug)]
pub struct ReadEnd<Buffer: ToBuffer + 'static> {
    reader: PipeRead,
    buffer: Vec<u8>,
    shared_data: Arc<SharedData<Buffer>>,
}

impl<Buffer: ToBuffer + Debug + 'static + Send + Sync> ReadEnd<Buffer> {
    pub(crate) fn new(reader: PipeRead, shared_data: Arc<SharedData<Buffer>>) -> Self {
        Self {
            reader,
            buffer: Vec::new(),
            shared_data,
        }
    }

    pub(crate) async fn receive_server_version(
        &mut self,
        version: u32,
    ) -> Result<Extensions, Error> {
        // Receive server version
        let len: u32 = self.read_and_deserialize(4).await?;
        self.read_exact(len as usize).await?;
        let server_version = ServerVersion::deserialize(&self.buffer)?;

        if server_version.version != version {
            Err(Error::UnsupportedSftpProtocol {
                version: server_version.version,
            })
        } else {
            Ok(server_version.extensions)
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
        // Since the data is sent as a string, we need to consume the 4-byte length first.
        copy(&mut (&mut self.reader).take(4), &mut sink()).await?;

        let len = len - 4;

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

    /// Precondition: `self.wait_for_new_request()` must not be 0.
    pub async fn read_in_one_packet(&mut self) -> Result<(), Error> {
        let (len, packet_type, response_id): (u32, u8, u32) = self.read_and_deserialize(9).await?;

        let len = len - 5;

        let callback = match self.shared_data.responses.get(response_id) {
            Ok(callback) => callback,

            // Invalid response_id
            Err(err) => {
                // Consume the invalid data to return self to a valid state
                // where read_in_one_packet can be called again.
                if let Err(consumption_err) = self.consume_data_packet(len).await {
                    return Err(Error::RecursiveErrors(Box::new((err, consumption_err))));
                }
                return Err(err);
            }
        };

        let response = if response::Response::is_data(packet_type) {
            let buffer = match callback.take_input() {
                Ok(buffer) => buffer,
                Err(err) => {
                    let err = err.into();

                    // Consume the invalid data to return self to a valid state
                    // where read_in_one_packet can be called again.
                    if let Err(consumption_err) = self.consume_data_packet(len).await {
                        return Err(Error::RecursiveErrors(Box::new((err, consumption_err))));
                    }
                    return Err(err);
                }
            };
            self.read_in_data_packet(len, buffer).await?
        } else {
            self.read_in_packet(len).await?
        };

        let res = callback.done(response);

        // NOTE that if the arc is dropped after this call, then the callback
        // would not be removed.
        if ArenaArc::strong_count(&callback) == 2 {
            ArenaArc::remove(&callback);
        }

        Ok(res?)
    }

    /// Return number of requests sent and number of responses to read in.
    /// **Read 0 if the connection is closed.**
    ///
    /// You must call this function in a loop, break if this function returns
    /// 0, otherwise call `read_in_one_packet` for `n` times where `n` in the
    /// return value of this function, then repeat.
    pub async fn wait_for_new_request(&self) -> u32 {
        self.shared_data.wait_for_new_request().await
    }
}
