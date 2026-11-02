//! `spc-agent` library surface: miner-side config and payout verification.

#![deny(unsafe_code)]

pub mod config;
pub mod sv2_client;
pub mod translate;
pub mod verify;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
