#![warn(
    missing_docs,
    missing_debug_implementations,
    rustdoc::broken_intra_doc_links,
    rust_2018_idioms,
    unreachable_pub
)]

mod error;
pub use error::Error;

pub mod highlevel;
pub mod lowlevel;
