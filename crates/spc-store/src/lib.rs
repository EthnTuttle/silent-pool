//! Postgres settlement store, Redis stream codecs and bootstrap snapshot.
//! See docs/SPEC.md §5 and docs/WIRE-FORMATS.md §3.

#![deny(unsafe_code)]

pub mod codec;
pub mod models;
pub mod postgres;
pub mod wire;

pub use codec::{CodecError, Reader, Writer};
pub use postgres::Store;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
