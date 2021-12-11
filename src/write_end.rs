use super::connection::SharedData;
use super::Error;
use super::Id;
use super::Response;
use super::ToBuffer;

use core::fmt::Debug;
use core::marker::Unpin;

use std::borrow::Cow;
use std::path::Path;

use std::sync::Arc;

use openssh_sftp_protocol::extensions::Extensions;
use openssh_sftp_protocol::file_attrs::FileAttrs;
use openssh_sftp_protocol::request::*;
use openssh_sftp_protocol::response::ErrorCode;
use openssh_sftp_protocol::response::ResponseInner;
use openssh_sftp_protocol::response::StatusCode;
use openssh_sftp_protocol::serde::Serialize;
use openssh_sftp_protocol::ssh_format::Serializer;
use openssh_sftp_protocol::{Handle, HandleOwned};

use std::io::IoSlice;
use tokio::io::AsyncWrite;
use tokio_io_utility::AsyncWriteUtility;

/// TODO:
///  - Support IoSlice for data in `send_write_request`

#[derive(Debug)]
pub struct WriteEnd<Writer, Buffer: ToBuffer + 'static> {
    serializer: Serializer,
    shared_data: Arc<SharedData<Writer, Buffer>>,
}

impl<Writer, Buffer: ToBuffer + 'static> Clone for WriteEnd<Writer, Buffer> {
    fn clone(&self) -> Self {
        Self::new(self.shared_data.clone())
    }
}

impl<Writer, Buffer: ToBuffer + 'static> Drop for WriteEnd<Writer, Buffer> {
    fn drop(&mut self) {
        let shared_data = &self.shared_data;

        // If this is the last reference, except for `ReadEnd`, to the SharedData,
        // then the connection is closed.
        if Arc::strong_count(shared_data) == 2 {
            shared_data.notify_conn_closed();
        }
    }
}

impl<Writer, Buffer: ToBuffer + 'static> WriteEnd<Writer, Buffer> {
    pub(crate) fn new(shared_data: Arc<SharedData<Writer, Buffer>>) -> Self {
        Self {
            serializer: Serializer::new(),
            shared_data,
        }
    }
}

impl<Writer: AsyncWrite + Unpin, Buffer: ToBuffer + Debug + Send + Sync + 'static>
    WriteEnd<Writer, Buffer>
{
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

        let mut writer = self.shared_data.writer.lock().await;

        writer
            .write_vectored_all(&mut [IoSlice::new(self.serializer.get_output()?)])
            .await?;

        Ok(())
    }

    /// Create a useable response id.
    pub fn create_response_id(&self) -> Id<Buffer> {
        self.shared_data.responses.insert(None)
    }

    /// Send requests.
    ///
    /// **Please use `Self::send_write_request` for sending write requests.**
    async fn send_request(
        &mut self,
        id: Id<Buffer>,
        request: RequestInner<'_>,
        buffer: Option<Buffer>,
    ) -> Result<Id<Buffer>, Error> {
        id.reset(buffer);

        self.write(Request {
            request_id: id.slot(),
            inner: request,
        })
        .await?;

        self.shared_data.notify_new_packet_event();

        Ok(id)
    }

    pub async fn send_open_file_request(
        &mut self,
        id: Id<Buffer>,
        params: OpenFile<'_>,
    ) -> Result<AwaitableHandle<Buffer>, Error> {
        Ok(AwaitableHandle(
            self.send_request(id, RequestInner::Open(params), None)
                .await?,
        ))
    }

    pub async fn send_close_request(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        Ok(AwaitableStatus(
            self.send_request(id, RequestInner::Close(handle), None)
                .await?,
        ))
    }

    pub async fn send_read_request(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
        offset: u64,
        len: u32,
        buffer: Buffer,
    ) -> Result<AwaitableData<Buffer>, Error> {
        Ok(AwaitableData(
            self.send_request(
                id,
                RequestInner::Read {
                    handle,
                    offset,
                    len,
                },
                Some(buffer),
            )
            .await?,
        ))
    }

    pub async fn send_remove_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        Ok(AwaitableStatus(
            self.send_request(id, RequestInner::Remove(path), None)
                .await?,
        ))
    }

    pub async fn send_rename_request(
        &mut self,
        id: Id<Buffer>,
        oldpath: Cow<'_, Path>,
        newpath: Cow<'_, Path>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        Ok(AwaitableStatus(
            self.send_request(id, RequestInner::Rename { oldpath, newpath }, None)
                .await?,
        ))
    }

    pub async fn send_mkdir_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
        attrs: FileAttrs,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        Ok(AwaitableStatus(
            self.send_request(id, RequestInner::Mkdir { path, attrs }, None)
                .await?,
        ))
    }

    pub async fn send_rmdir_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        Ok(AwaitableStatus(
            self.send_request(id, RequestInner::Rmdir(path), None)
                .await?,
        ))
    }

    pub async fn send_opendir_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableHandle<Buffer>, Error> {
        Ok(AwaitableHandle(
            self.send_request(id, RequestInner::Opendir(path), None)
                .await?,
        ))
    }

    // TODO: Add one function for every ResponseInner

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

        let mut writer = self.shared_data.writer.lock().await;

        let mut slices = [IoSlice::new(header), IoSlice::new(data)];
        writer.write_vectored_all(&mut slices).await?;

        Ok(())
    }

    /// Send write requests
    pub async fn send_write_request(
        &mut self,
        id: Id<Buffer>,
        handle: &[u8],
        offset: u64,
        data: &[u8],
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        self.send_write_request_impl(id.slot(), handle, offset, data)
            .await?;

        self.shared_data.notify_new_packet_event();

        Ok(AwaitableStatus(id))
    }
}

#[derive(Debug, Clone)]
pub enum Data<Buffer: ToBuffer> {
    /// The buffer that stores the response of Read,
    /// since its corresponding response type `ResponseInner::Data`
    /// does not contain any member, it doesn't have to be stored.
    Buffer(Buffer),

    /// Same as `Buffer`, this is a fallback
    /// if `Buffer` isn't provided or it isn't large enough.
    AllocatedBox(Box<[u8]>),

    /// EOF is reached before any data can be read.
    Eof,
}

macro_rules! def_awaitable {
    ($name:ident, $res:ty, $response_name:ident, $post_processing:block) => {
        pub struct $name<Buffer: ToBuffer + Send + Sync>(Id<Buffer>);

        impl<Buffer: ToBuffer + Debug + Send + Sync> $name<Buffer> {
            pub async fn wait(self) -> (Id<Buffer>, Result<$res, Error>) {
                let id = self.0;

                let $response_name = id.wait().await;
                (id, $post_processing)
            }
        }
    };
}

def_awaitable!(AwaitableStatus, (), response, {
    match response {
        Response::Header(ResponseInner::Status {
            status_code,
            err_msg,
        }) => match status_code {
            StatusCode::Success => Ok(()),
            StatusCode::Failure(err_code) => Err(Error::SftpError(err_code, err_msg)),
        },
        _ => Err(Error::InvalidResponse(&"Expected Status response")),
    }
});

def_awaitable!(AwaitableHandle, HandleOwned, response, {
    match response {
        Response::Header(response_inner) => match response_inner {
            ResponseInner::Handle(handle) => Ok(handle),
            ResponseInner::Status {
                status_code: StatusCode::Failure(err_code),
                err_msg,
            } => Err(Error::SftpError(err_code, err_msg)),

            _ => Err(Error::InvalidResponse(&"Unexpected Data response")),
        },
        _ => Err(Error::InvalidResponse(
            &"Expected Handle or err Status response",
        )),
    }
});

def_awaitable!(AwaitableData, Data<Buffer>, response, {
    match response {
        Response::Buffer(buffer) => Ok(Data::Buffer(buffer)),
        Response::AllocatedBox(allocated_box) => Ok(Data::AllocatedBox(allocated_box)),
        Response::Header(ResponseInner::Status {
            status_code: StatusCode::Failure(err_code),
            err_msg,
        }) => match err_code {
            ErrorCode::Eof => Ok(Data::Eof),
            _ => Err(Error::SftpError(err_code, err_msg)),
        },
        _ => Err(Error::InvalidResponse(
            &"Expected Buffer/AllocatedBox response",
        )),
    }
});
