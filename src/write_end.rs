use super::awaitable_responses::ArenaArc;
use super::awaitable_responses::Response;
use super::connection::SharedData;
use super::Error;
use super::Id;
use super::ToBuffer;

use core::fmt::Debug;
use core::marker::Unpin;
use core::mem::replace;

use std::borrow::Cow;
use std::path::Path;

use std::sync::Arc;

use openssh_sftp_protocol::extensions::Extensions;
use openssh_sftp_protocol::file_attrs::FileAttrs;
use openssh_sftp_protocol::request::*;
use openssh_sftp_protocol::response::*;
use openssh_sftp_protocol::serde::Serialize;
use openssh_sftp_protocol::ssh_format::Serializer;
use openssh_sftp_protocol::{Handle, HandleOwned};

use std::io::IoSlice;
use tokio::io::AsyncWrite;
use tokio_io_utility::AsyncWriteUtility;

/// TODO:
///  - Support IoSlice for data in `send_write_request`

/// It is recommended to create at most one `WriteEnd` per thread.
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
    ) -> Result<ArenaArc<Buffer>, Error> {
        // Call id.into_inner to prevent id from being removed
        // if the future is cancelled.
        let arc = id.into_inner();

        arc.reset(buffer);

        self.write(Request {
            request_id: ArenaArc::slot(&arc),
            inner: request,
        })
        .await?;

        self.shared_data.notify_new_packet_event();

        Ok(arc)
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
        buffer: Option<Buffer>,
    ) -> Result<AwaitableData<Buffer>, Error> {
        Ok(AwaitableData(
            self.send_request(
                id,
                RequestInner::Read {
                    handle,
                    offset,
                    len,
                },
                buffer,
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

    pub async fn send_readdir_request(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
    ) -> Result<AwaitableNameEntries<Buffer>, Error> {
        Ok(AwaitableNameEntries(
            self.send_request(id, RequestInner::Readdir(handle), None)
                .await?,
        ))
    }

    pub async fn send_stat_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableAttrs<Buffer>, Error> {
        Ok(AwaitableAttrs(
            self.send_request(id, RequestInner::Stat(path), None)
                .await?,
        ))
    }

    /// Does not follow symlink
    pub async fn send_lstat_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableAttrs<Buffer>, Error> {
        Ok(AwaitableAttrs(
            self.send_request(id, RequestInner::Lstat(path), None)
                .await?,
        ))
    }

    /// Does not follow symlink
    pub async fn send_fstat_request(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
    ) -> Result<AwaitableAttrs<Buffer>, Error> {
        Ok(AwaitableAttrs(
            self.send_request(id, RequestInner::Fstat(handle), None)
                .await?,
        ))
    }

    pub async fn send_setstat_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
        attrs: FileAttrs,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        Ok(AwaitableStatus(
            self.send_request(id, RequestInner::Setstat { path, attrs }, None)
                .await?,
        ))
    }

    pub async fn send_fsetstat_request(
        &mut self,
        id: Id<Buffer>,
        handle: Cow<'_, Handle>,
        attrs: FileAttrs,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        Ok(AwaitableStatus(
            self.send_request(id, RequestInner::Fsetstat { handle, attrs }, None)
                .await?,
        ))
    }

    pub async fn send_readlink_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableName<Buffer>, Error> {
        Ok(AwaitableName(
            self.send_request(id, RequestInner::Readlink(path), None)
                .await?,
        ))
    }

    pub async fn send_realpath_request(
        &mut self,
        id: Id<Buffer>,
        path: Cow<'_, Path>,
    ) -> Result<AwaitableName<Buffer>, Error> {
        Ok(AwaitableName(
            self.send_request(id, RequestInner::Realpath(path), None)
                .await?,
        ))
    }

    /// Create symlink
    pub async fn send_symlink_request(
        &mut self,
        id: Id<Buffer>,
        linkpath: Cow<'_, Path>,
        targetpath: Cow<'_, Path>,
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        Ok(AwaitableStatus(
            self.send_request(
                id,
                RequestInner::Symlink {
                    linkpath,
                    targetpath,
                },
                None,
            )
            .await?,
        ))
    }

    /// Send write requests
    pub async fn send_write_request(
        &mut self,
        id: Id<Buffer>,
        handle: &[u8],
        offset: u64,
        data: &[u8],
    ) -> Result<AwaitableStatus<Buffer>, Error> {
        // Call id.into_inner to prevent id from being removed
        // if the future is cancelled.
        let arc = id.into_inner();

        let header = Request::serialize_write_request(
            &mut self.serializer,
            ArenaArc::slot(&arc),
            handle,
            offset,
            data.len().try_into().map_err(|_| Error::BufferTooLong)?,
        )?;

        let mut writer = self.shared_data.writer.lock().await;

        let mut slices = [IoSlice::new(header), IoSlice::new(data)];
        writer.write_vectored_all(&mut slices).await?;

        self.shared_data.notify_new_packet_event();

        Ok(AwaitableStatus(arc))
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

#[derive(Debug, Clone)]
pub struct Name {
    pub filename: Box<str>,
    pub longname: Box<str>,
}

macro_rules! def_awaitable {
    ($name:ident, $res:ty, $response_name:ident, $post_processing:block) => {
        pub struct $name<Buffer: ToBuffer + Send + Sync>(ArenaArc<Buffer>);

        impl<Buffer: ToBuffer + Debug + Send + Sync> $name<Buffer> {
            /// Return (id, res).
            ///
            /// id can be reused in the next request.
            pub async fn wait(self) -> (Id<Buffer>, Result<$res, Error>) {
                let arc = self.0;

                let $response_name = Id::wait(&arc).await;
                (Id::new(arc), $post_processing)
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

            _ => Err(Error::InvalidResponse(
                &"Expected Handle or err Status response",
            )),
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

def_awaitable!(AwaitableNameEntries, Box<[NameEntry]>, response, {
    match response {
        Response::Header(response_inner) => match response_inner {
            ResponseInner::Name(name) => Ok(name),
            ResponseInner::Status {
                status_code: StatusCode::Failure(err_code),
                err_msg,
            } => Err(Error::SftpError(err_code, err_msg)),

            _ => Err(Error::InvalidResponse(
                &"Expected Name or err Status response",
            )),
        },
        _ => Err(Error::InvalidResponse(
            &"Expected Name or err Status response",
        )),
    }
});

def_awaitable!(AwaitableAttrs, FileAttrs, response, {
    match response {
        Response::Header(response_inner) => match response_inner {
            // use replace to avoid allocation that might occur due to
            // `FileAttrs::clone`.
            ResponseInner::Attrs(mut attrs) => Ok(replace(&mut attrs, FileAttrs::new())),
            ResponseInner::Status {
                status_code: StatusCode::Failure(err_code),
                err_msg,
            } => Err(Error::SftpError(err_code, err_msg)),

            _ => Err(Error::InvalidResponse(
                &"Expected Attrs or err Status response",
            )),
        },
        _ => Err(Error::InvalidResponse(
            &"Expected Attrs or err Status response",
        )),
    }
});

def_awaitable!(AwaitableName, Name, response, {
    match response {
        Response::Header(response_inner) => match response_inner {
            ResponseInner::Name(mut names) => {
                if names.len() != 1 {
                    Err(Error::InvalidResponse(
                        &"Got expected Name response, but it does not have exactly \
                        one and only one entry",
                    ))
                } else {
                    let name = &mut names[0];

                    Ok(Name {
                        filename: replace(&mut name.filename, "".into()),
                        longname: replace(&mut name.longname, "".into()),
                    })
                }
            }
            ResponseInner::Status {
                status_code: StatusCode::Failure(err_code),
                err_msg,
            } => Err(Error::SftpError(err_code, err_msg)),

            _ => Err(Error::InvalidResponse(
                &"Expected Name or err Status response",
            )),
        },
        _ => Err(Error::InvalidResponse(
            &"Expected Name or err Status response",
        )),
    }
});
