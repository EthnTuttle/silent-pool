//! SPc derivation and identity primitives. See docs/SPEC.md §2 and DESIGN.md.
//!
//! This crate is pure: no I/O, no networking, no database. WP1 fills in the
//! derivation, carriers and mailbox blinding; the frozen vectors in
//! `tests/vectors.rs` are the contract.

#![deny(unsafe_code)]

/// Crate version, exposed so binaries can report a consistent build.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
