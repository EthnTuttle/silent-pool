//! `spc-agent` library surface: miner-side config and payout verification.

#![deny(unsafe_code)]

pub mod config;
pub mod jdc;
pub mod source_feed;
pub mod sv2_client;
pub mod translator;
pub mod verify;
#[cfg(feature = "ark")]
pub mod wallet;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Role-selection policy for `--roles`: the wallet role holds spendable
/// VTXO secrets and never co-runs with the mining-side roles in one
/// process (fail closed, SPEC 11.5 isolation).
pub fn validate_role_selection(roles: &[&str]) -> Result<(), String> {
    let has = |name: &str| roles.contains(&name);
    if has("wallet") && (has("translator") || has("jdc")) {
        return Err(
            "the wallet role cannot co-run with the translator or JDC roles; \
             run it as its own process"
                .into(),
        );
    }
    Ok(())
}
