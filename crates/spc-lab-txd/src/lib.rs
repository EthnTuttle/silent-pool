//! `spc-lab-txd`: the lab traffic wallet core (traffic plan packet T1a).
//!
//! This crate is the **functional signer + reservation journal** slice of
//! the traffic plan: real legacy (P2PKH) traffic keys fully disjoint from
//! SPc master/child/Ark keys, deterministic exact-fee transaction
//! construction and signing, and the durable reservation/provenance
//! journal that every traffic spend (background churn, manual user
//! payments, bootstrap distribution, Ark chain effects, retries and crash
//! recovery) must pass through. The native actor uses this same core; the
//! bootstrap mining loop remains a separately supplied T2-L executor.
//!
//! Module map:
//!
//! - [`error`] - the fail-closed error surface (no donation fallback).
//! - [`money`] - exact BTC decimal parsing and checked satoshi arithmetic.
//! - [`keys`] - per-role traffic key derivation, provisioning and
//!   fail-closed reopen (mode-0600 private artifact).
//! - [`builder`] - legacy P2PKH construction, deterministic fee
//!   convergence, exact destination/fee/change verification.
//! - [`journal`] - the append-only reservation/provenance journal: dedup,
//!   reservations, lifecycle state machine, pause barrier, replay
//!   integrity.
//! - [`wallet`] - the wallet set and the payment flow with all
//!   crash-recovery paths (replay-stored / resume-reserved).
//! - [`adapter`] - the mapping contract to the frozen T0 traffic types
//!   and the explicit remainder owned by T1b/T2-L/T3/AP-5.
//!
//! Scope statement (T1a): the core is testable independently of any node
//! or RPC; nothing here performs or simulates a broadcast, and no Ark
//! readiness is claimed.

pub mod actor;
pub mod actor_config;
pub mod adapter;
pub mod admit;
pub mod bootstrap;
pub mod builder;
pub mod construction;
pub mod contextual_pow;
pub mod control;
pub mod error;
pub mod journal;
pub mod keys;
pub mod money;
pub mod native_bootstrap;
pub mod net_readiness;
pub mod observer;
pub mod payment_view;
pub mod schedule;
pub mod send_outcome;
pub mod wallet;

pub use journal::{
    ChainAnchor, ObserverOutpoint, ObserverSnapshot, PendingSendIntent, RecoverySnapshot,
    RecoveryTransaction, RollbackEvidence,
};

#[cfg(test)]
mod tests;

/// Wall-clock milliseconds since the Unix epoch (same convention as the
/// engine's `command::now_ms`); used only for journal timestamps.
pub fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
