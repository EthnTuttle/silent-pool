//! `spc-pool` library surface. The binary is a thin role dispatcher.

#![deny(unsafe_code)]

pub mod api;
pub mod api_backend;
pub mod ark;
pub mod block;
pub mod coinbase;
pub mod config;
mod derivation_observation;
pub mod evidence;
pub mod front;
pub mod identity;
pub mod jds;
pub mod jds_server;
pub mod metrics;
pub mod net;
pub mod operator;
pub mod payout;
mod payout_observation;
pub mod registration;
pub mod sv1;
pub mod sv1_server;
pub mod sv2_server;
pub mod template;
#[cfg(feature = "rpc")]
pub mod template_rpc;
pub mod type32;
pub mod witness;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
