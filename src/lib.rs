#![forbid(unsafe_code)]

mod client;
mod constants;
mod error;

pub use client::Client;
pub use error::Error;
