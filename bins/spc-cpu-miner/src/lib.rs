#![forbid(unsafe_code)]

pub mod budget;
// Keep the frozen inline AddConnection/source-audit DTO construction API.
// Config/control frames are bounded and are not per-hash allocations.
#[allow(clippy::large_enum_variant)]
pub mod config;
pub mod connection;
#[allow(clippy::large_enum_variant)]
pub mod control;
pub mod hash;
pub mod ids;
pub mod miner;
pub mod native_gate;
pub mod report;
pub mod session;
pub mod target;
pub mod worker;
