//! Postgres settlement store, Redis stream codecs and bootstrap snapshot.
//! See docs/SPEC.md §5 and docs/WIRE-FORMATS.md §3.

#![deny(unsafe_code)]

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
