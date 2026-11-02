//! `spc-pool` library surface. The binary is a thin role dispatcher.

#![deny(unsafe_code)]

pub mod config;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
