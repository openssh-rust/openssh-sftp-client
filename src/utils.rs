use std::convert::identity;

use crate::error::{Error, RecursiveError};

pub trait ErrorExt {
    fn error_on_cleanup(self, occuring_error: Error) -> Self;
}

impl ErrorExt for Error {
    fn error_on_cleanup(self, occuring_error: Error) -> Self {
        Error::RecursiveErrors(Box::new(RecursiveError {
            original_error: self,
            occuring_error,
        }))
    }
}

pub trait ResultExt<T, E> {
    fn flatten(self) -> Result<T, E>;
}

impl<T, E, E2> ResultExt<T, E> for Result<Result<T, E>, E2>
where
    E: From<E2>,
{
    fn flatten(self) -> Result<T, E> {
        self.map_err(E::from).and_then(identity)
    }
}
