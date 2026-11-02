//! Pure SLICE/PPLNS-JD accounting. See docs/SPEC.md §4 and §11.1.
//!
//! No I/O. WP3 fills in slices, windows, scores, ledger, trim and distribution.

#![deny(unsafe_code)]

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
