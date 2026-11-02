//! SPc derivation and identity primitives. See docs/SPEC.md §2 and DESIGN.md.
//!
//! This crate is pure: no I/O, no networking, no database. Every pool and agent
//! path keys off these functions; the frozen vectors in `tests/vectors.rs` are
//! the contract.
//!
//! ## Derivation summary (SPEC §2.3, DESIGN.md "Derivation scheme")
//!
//! ```text
//! input_hash = hash_SPc/Inputs(ser32(H) || ser_P(A_send))
//! ecdh       = ser_P(input_hash * a_send * B_scan)   -- pool side
//!            = ser_P(input_hash * b_scan * A_send)   -- miner side
//! t_onchain  = hash_SPc/TierSpc(ecdh || ser32(k))  -> P = B_spend + t_onchain * G
//! t_ark      = hash_SPc/TierArk(ecdh || ser32(k))  -> U = B_ark   + t_ark    * G
//! ```
//!
//! Every hash-to-scalar **fails** if the digest is 0 or >= n (BIP 352 semantics).

#![deny(unsafe_code)]

pub mod address;
pub mod carriers;
pub mod identity;
pub mod kdf;
pub mod mailbox;
pub mod send_key;

#[cfg(feature = "ark")]
pub mod ark_feature;

pub use address::{parse_sp_address, Network, SpAddress};
pub use carriers::{
    emit_op_return, emit_script_sig, height_push_bytes, parse_carrier, parse_op_return,
    parse_script_sig, script_sig_budget,
};
pub use identity::{sp_id, strip_worker};
pub use kdf::{
    ark_user_pubkey, ark_user_secret, input_hash, spc_output_pubkey, spc_output_secret,
    tier_tweaks, SharedSecret, TierTweaks,
};
pub use mailbox::{
    derived_epoch_mailbox_pubkey, derived_mailbox_tweak, mailbox_blinding_hint,
    pool_blinded_mailbox_pubkey,
};
pub use send_key::SendKey;

/// Crate version, exposed so binaries can report a consistent build.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Derivation error type. Per BIP 352, an invalid scalar or point at infinity
/// means fail, never reduce mod n.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// A hash output was zero or >= n (BIP 352 fail-not-reduce).
    #[error("hash output is not a valid scalar (0 or >= n)")]
    InvalidScalar,
    /// A point addition produced the point at infinity.
    #[error("result is the point at infinity")]
    PointAtInfinity,
    /// Address parsing failed.
    #[error("invalid SP address: {0}")]
    InvalidAddress(String),
    /// Carrier parsing/building failed.
    #[error("carrier error: {0}")]
    Carrier(String),
    /// Key generation failed (OS CSPRNG or normalization).
    #[error("key generation failed: {0}")]
    KeyGen(String),
}
