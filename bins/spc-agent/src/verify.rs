//! Client-side own-payout verification (SPEC §3.6).
//!
//! Both roles know the scan key, the spend public key, the height and
//! `A_send`. Before hashing they derive their own `P` and check it appears in
//! the coinbase outputs (pool-template) or the distribution (JD).

use bitcoin::secp256k1::{PublicKey, SecretKey};
use spc_crypto::{spc_output_pubkey, tier_tweaks, SharedSecret};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnPayout {
    /// Index into the supplied output list, if found.
    pub vout: Option<u32>,
    pub amount: Option<u64>,
    pub p_x: [u8; 32],
    /// The tweak needed by a wallet that can spend the output.
    pub tweak: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    Warn,
    Refuse,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VerifyError {
    #[error("own output not found")]
    Absent,
    #[error("derivation failed")]
    Derivation,
    #[error("own output set or denomination entitlement mismatch")]
    Entitlement,
}

/// A minimal output view: `script_pubkey` and `value`.
pub use spc_scan::{OutputView, PayoutFormat};

/// Runtime adapter for a source-bound proof from the private/operator or
/// ownership-authenticated collector. The trust roots are independently
/// captured collector/chain-history results; never take them from the bundle.
/// Actual issued SV2 job bytes must be supplied by the live job reader.
pub fn verify_source_entitlement(
    bundle: &[u8],
    trusted: &spc_scan::source_audit::TrustedSource,
    issued: &spc_scan::source_audit::JobProof,
    b_scan: &SecretKey,
    b_spend: &PublicKey,
) -> Result<(Vec<spc_scan::Found>, u64), VerifyError> {
    let verified = spc_scan::source_audit::verify_owned(bundle, trusted, issued, b_scan, b_spend)
        .map_err(|_| VerifyError::Entitlement)?;
    Ok((verified.outputs, verified.remainder))
}

/// Full own-payout verification from an independently audited frozen window.
/// Returns every owned output/tweak and the exact Ark/error-held remainder.
/// Absence warnings never turn an invalid amount/composition into success.
pub fn verify_entitlement(
    height: u32,
    a_x: &[u8; 32],
    outputs: &[OutputView<'_>],
    b_scan: &SecretKey,
    b_spend: &PublicKey,
    owed: u64,
    format: PayoutFormat,
) -> Result<(Vec<spc_scan::Found>, u64), VerifyError> {
    let found = spc_scan::scan_outputs(height, a_x, outputs, b_scan, b_spend)
        .map_err(|_| VerifyError::Entitlement)?;
    let remainder =
        spc_scan::audit_outputs(&found, owed, format).map_err(|_| VerifyError::Entitlement)?;
    Ok((found, remainder))
}

/// Derive the miner's `P` for `(height, A_x)`.
pub fn derive_own(
    height: u32,
    a_x: &[u8; 32],
    b_scan: &SecretKey,
    b_spend: &PublicKey,
) -> Result<OwnPayout, VerifyError> {
    let mut a_bytes = [0u8; 33];
    a_bytes[0] = 0x02;
    a_bytes[1..].copy_from_slice(a_x);
    let a_send = PublicKey::from_slice(&a_bytes).map_err(|_| VerifyError::Derivation)?;
    let ecdh =
        SharedSecret::miner_side(height, b_scan, &a_send).map_err(|_| VerifyError::Derivation)?;
    let tweaks = tier_tweaks(&ecdh, 0).map_err(|_| VerifyError::Derivation)?;
    let p = spc_output_pubkey(b_spend, &tweaks.t_onchain).map_err(|_| VerifyError::Derivation)?;
    let p_x = p.x_only_public_key().0.serialize();
    let tweak = tweaks.t_onchain.to_be_bytes();
    Ok(OwnPayout {
        vout: None,
        amount: None,
        p_x,
        tweak,
    })
}

/// Find the derived `P` among `outputs` (`OP_1 <32-byte>` scripts).
pub fn find_own(outputs: &[OutputView<'_>], own: &OwnPayout) -> Option<(u32, u64)> {
    outputs.iter().enumerate().find_map(|(i, o)| {
        if o.script_pubkey.len() == 34
            && o.script_pubkey[0] == 0x51
            && o.script_pubkey[1] == 0x20
            && o.script_pubkey[2..] == own.p_x
        {
            Some((i as u32, o.value))
        } else {
            None
        }
    })
}

/// Apply the configured policy: absence is a warning by default, because a
/// small miner is legitimately below the threshold or trimmed some blocks.
pub fn check_policy(policy: Policy, found: bool) -> Result<(), VerifyError> {
    if found {
        return Ok(());
    }
    match policy {
        Policy::Warn => Ok(()),
        Policy::Refuse => Err(VerifyError::Absent),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::{Secp256k1, SecretKey};
    use spc_crypto::SendKey;

    #[test]
    fn derives_and_finds_own_output() {
        let secp = Secp256k1::new();
        let b_scan = SecretKey::from_slice(&[0x11; 32]).unwrap();
        let b_spend = SecretKey::from_slice(&[0x12; 32]).unwrap();
        let b_spend_pub = PublicKey::from_secret_key(&secp, &b_spend);

        let send = SendKey::generate(800_000).unwrap();
        let own = derive_own(800_000, send.x_only(), &b_scan, &b_spend_pub).unwrap();

        let mut script = vec![0x51, 0x20];
        script.extend_from_slice(&own.p_x);
        let outputs = vec![
            OutputView {
                script_pubkey: &[0x51, 0x20, 0, 0],
                value: 1,
            },
            OutputView {
                script_pubkey: &script,
                value: 330,
            },
        ];
        assert_eq!(find_own(&outputs, &own), Some((1, 330)));
        assert!(check_policy(Policy::Refuse, true).is_ok());
        assert_eq!(
            check_policy(Policy::Refuse, false),
            Err(VerifyError::Absent)
        );
        assert!(check_policy(Policy::Warn, false).is_ok());
    }

    #[test]
    fn wrong_height_is_not_found() {
        let secp = Secp256k1::new();
        let b_scan = SecretKey::from_slice(&[0x11; 32]).unwrap();
        let b_spend = SecretKey::from_slice(&[0x12; 32]).unwrap();
        let b_spend_pub = PublicKey::from_secret_key(&secp, &b_spend);
        let send = SendKey::generate(800_000).unwrap();
        let own = derive_own(800_000, send.x_only(), &b_scan, &b_spend_pub).unwrap();
        let mut script = vec![0x51, 0x20];
        script.extend_from_slice(&own.p_x);
        let other = derive_own(800_001, send.x_only(), &b_scan, &b_spend_pub).unwrap();
        assert_ne!(other.p_x, own.p_x);
        let outputs = vec![OutputView {
            script_pubkey: &script,
            value: 330,
        }];
        assert_eq!(find_own(&outputs, &other), None);
    }

    #[test]
    fn input_hash_is_used() {
        use spc_crypto::input_hash;
        let secp = Secp256k1::new();
        let b_scan = SecretKey::from_slice(&[0x11; 32]).unwrap();
        let b_spend_pub =
            PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[0x12; 32]).unwrap());
        let send = SendKey::generate(800_000).unwrap();
        let a = send.public_key();
        let ih = input_hash(800_000, &a);
        assert_eq!(ih.len(), 32);
        assert!(derive_own(800_000, send.x_only(), &b_scan, &b_spend_pub).is_ok());
    }
}
