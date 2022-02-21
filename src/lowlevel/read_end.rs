#![forbid(unsafe_code)]

use super::awaitable_responses::ArenaArc;
use super::awaitable_responses::Response;
use super::connection::SharedData;
use super::Error;
use super::Extensions;
use super::ToBuffer;

use std::fmt::Debug;
use std::io;

use openssh_sftp_protocol::response::{self, ServerVersion};
use openssh_sftp_protocol::serde::Deserialize;
use openssh_sftp_protocol::ssh_format::from_bytes;

use tokio::io::{copy_buf, sink, AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio_io_utility::{read_exact_to_bytes, read_exact_to_vec};
use tokio_pipe::{PipeRead, PIPE_BUF};

#[derive(Debug)]
pub struct ReadEnd<Buffer, Auxiliary = ()> {
    reader: BufReader<PipeRead>,
    buffer: Vec<u8>,
    shared_data: SharedData<Buffer, Auxiliary>,
}

impl<Buffer: ToBuffer + 'static + Send + Sync, Auxiliary> ReadEnd<Buffer, Auxiliary> {
    pub(crate) fn new(reader: PipeRead, shared_data: SharedData<Buffer, Auxiliary>) -> Self {
        Self {
            reader: BufReader::with_capacity(PIPE_BUF, reader),
            buffer: Vec::with_capacity(64),
            shared_data,
        }
    }

    pub(crate) async fn receive_server_version(
        &mut self,
        version: u32,
    ) -> Result<Extensions, Error> {
        // Receive server version
        let len: u32 = self.read_and_deserialize(4).await?;
        if (len as usize) > 4096 {
            return Err(Error::SftpServerHelloMsgTooLong { len });
        }

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

    async fn consume_packet(&mut self, len: u32, err: Error) -> Result<(), Error> {
        if let Err(consumption_err) =
            copy_buf(&mut (&mut self.reader).take(len as u64), &mut sink()).await
        {
            Err(Error::RecursiveErrors(Box::new((
                err,
                consumption_err.into(),
            ))))
        } else {
            Err(err)
        }
    }

    async fn read_into_box(&mut self, len: usize) -> Result<Box<[u8]>, Error> {
        let mut vec = Vec::new();
        read_exact_to_vec(&mut self.reader, &mut vec, len as usize).await?;

        Ok(vec.into_boxed_slice())
    }

    async fn read_in_data_packet_fallback(
        &mut self,
        len: usize,
    ) -> Result<Response<Buffer>, Error> {
        self.read_into_box(len).await.map(Response::AllocatedBox)
    }

    /// * `len` - excludes packet_type and request_id.
    async fn read_in_data_packet(
        &mut self,
        len: u32,
        buffer: Option<Buffer>,
    ) -> Result<Response<Buffer>, Error> {
        // Since the data is sent as a string, we need to consume the 4-byte length first.
        self.reader.read_exact(&mut [0; 4]).await?;

        let len = (len - 4) as usize;

        if let Some(mut buffer) = buffer {
            match buffer.get_buffer() {
                super::Buffer::Vector(vec) => {
                    read_exact_to_vec(&mut self.reader, vec, len).await?;
                    Ok(Response::Buffer(buffer))
                }
                super::Buffer::Slice(slice) => {
                    if slice.len() >= len {
                        self.reader.read_exact(slice).await?;
                        Ok(Response::Buffer(buffer))
                    } else {
                        self.read_in_data_packet_fallback(len).await
                    }
                }
                super::Buffer::Bytes(bytes) => {
                    read_exact_to_bytes(&mut self.reader, bytes, len).await?;
                    Ok(Response::Buffer(buffer))
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

    /// * `len` - excludes packet_type and request_id.
    async fn read_in_extended_reply(&mut self, len: u32) -> Result<Response<Buffer>, Error> {
        self.read_into_box(len as usize)
            .await
            .map(Response::ExtendedReply)
    }

    /// Precondition: [`ReadEnd::wait_for_new_request`] must not be 0.
    ///
    /// # Restart on Error
    ///
    /// Only when the returned error is [`Error::InvalidResponseId`] or
    /// [`Error::AwaitableError`], can the function be restarted.
    ///
    /// Upon other errors [`Error::IOError`], [`Error::FormatError`] and
    /// [`Error::RecursiveErrors`], the sftp session has to be discarded.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let readend = ...;
    /// loop {
    ///     let new_requests_submit = readend.wait_for_new_request().await;
    ///     if new_requests_submit == 0 {
    ///         break;
    ///     }
    ///
    ///     // If attempt to read in more than new_requests_submit, then
    ///     // `read_in_one_packet` might block forever.
    ///     for _ in 0..new_requests_submit {
    ///         readend.read_in_one_packet().await.unwrap();
    ///     }
    /// }
    /// ```
    /// # Cancel Safety
    ///
    /// This function is not cancel safe.
    ///
    /// Dropping the future might cause the response packet to be partially read,
    /// and the next read would treat the partial response as a new response.
    pub async fn read_in_one_packet(&mut self) -> Result<(), Error> {
        let (len, packet_type, response_id): (u32, u8, u32) = self.read_and_deserialize(9).await?;

        let len = len - 5;

        let callback = match self.shared_data.responses().get(response_id) {
            Ok(callback) => callback,

            // Invalid response_id
            Err(err) => {
                // Consume the invalid data to return self to a valid state
                // where read_in_one_packet can be called again.
                return self.consume_packet(len, err).await;
            }
        };

        let response = if response::Response::is_data(packet_type) {
            let buffer = match callback.take_input() {
                Ok(buffer) => buffer,
                Err(err) => {
                    // Consume the invalid data to return self to a valid state
                    // where read_in_one_packet can be called again.
                    return self.consume_packet(len, err.into()).await;
                }
            };
            self.read_in_data_packet(len, buffer).await?
        } else if response::Response::is_extended_reply(packet_type) {
            self.read_in_extended_reply(len).await?
        } else {
            self.read_in_packet(len).await?
        };

        let res = callback.done(response);

        // If counter == 2, then it must be one of the following situation:
        //  - `ReadEnd` is the only holder other than the `Arena` itself;
        //  - `ReadEnd` and the `AwaitableInner` is the holder and `AwaitableInner::drop`
        //    has already `ArenaArc::remove`d it.
        //
        // In case 1, since there is no `AwaitableInner` holding reference to it,
        // it can be removed safely.
        //
        // In case 2, since it is already removed, remove it again is a no-op.
        //
        // NOTE that if the arc is dropped after this call while having the
        // `Awaitable*::drop` executed before `callback.done`, then the callback
        // would not be removed.
        //
        // Though this kind of situation is rare.
        if ArenaArc::strong_count(&callback) == 2 {
            ArenaArc::remove(&callback);
        }

        Ok(res?)
    }

    /// Return number of requests sent (including requests that are still in the write
    /// buffer and not yet flushed) and number of responses to read in.
    /// **Read 0 if the connection is closed.**
    ///
    /// You must call this function in a loop, break if this function returns
    /// 0, otherwise call [`ReadEnd::read_in_one_packet`] for `n` times where `n` in the
    /// return value of this function, then repeat.
    ///
    /// # Cancel Safety
    ///
    /// It is perfectly safe to cancel this future.
    #[inline(always)]
    pub async fn wait_for_new_request(&self) -> u32 {
        self.shared_data.wait_for_new_request().await
    }

    /// Wait for next packet to be readable.
    ///
    /// Return `Ok(())` if next packet is ready and readable, `Error::IOError(io_error)`
    /// where `io_error.kind() == ErrorKind::UnexpectedEof` if `EOF` is met.
    ///
    /// # Cancel Safety
    ///
    /// This function is cancel safe and thus can be used with `tokio::select!`
    /// to implement [`ReadEnd::flush_write_end_buffer`] on timeout.
    pub async fn ready_for_read(&mut self) -> Result<(), io::Error> {
        if self.reader.fill_buf().await?.is_empty() {
            // Empty buffer means EOF
            Err(io::Error::new(io::ErrorKind::UnexpectedEof, ""))
        } else {
            Ok(())
        }
    }

    pub fn get_shared_data(&self) -> &SharedData<Buffer, Auxiliary> {
        &self.shared_data
    }
}