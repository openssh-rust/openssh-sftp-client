use std::convert::identity;

use crate::error::{Error, RecursiveError, RecursiveError3};

pub(super) trait ErrorExt {
    fn error_on_cleanup(self, occuring_error: Self) -> Self;

    fn error_on_cleanup3(self, err2: Self, err3: Self) -> Self;
}

impl ErrorExt for Error {
    fn error_on_cleanup(self, occuring_error: Error) -> Self {
        Error::RecursiveErrors(Box::new(RecursiveError {
            original_error: self,
            occuring_error,
        }))
    }

    fn error_on_cleanup3(self, err2: Self, err3: Self) -> Self {
        Error::RecursiveErrors3(Box::new(RecursiveError3 {
            err1: self,
            err2,
            err3,
        }))
    }
}

pub(super) trait ResultExt<T, E> {
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
