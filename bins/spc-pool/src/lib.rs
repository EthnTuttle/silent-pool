//! `spc-pool` library surface. The binary is a thin role dispatcher.

#![deny(unsafe_code)]

pub mod api;
pub mod block;
pub mod coinbase;
pub mod config;
pub mod front;
pub mod identity;
pub mod sv1;
pub mod sv1_server;
pub mod template;
#[cfg(feature = "rpc")]
pub mod template_rpc;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
