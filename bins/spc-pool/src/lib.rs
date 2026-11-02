//! `spc-pool` library surface. The binary is a thin role dispatcher.

#![deny(unsafe_code)]

pub mod api;
pub mod coinbase;
pub mod config;
pub mod identity;
pub mod sv1;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
