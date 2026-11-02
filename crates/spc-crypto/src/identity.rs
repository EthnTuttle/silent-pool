//! Miner identity functions (SPEC §2.1).

use bitcoin::hashes::{sha256t_hash_newtype, Hash, HashEngine};
use bitcoin::secp256k1::PublicKey;

sha256t_hash_newtype! {
    /// Tag for `sp_id = hash_SPc/Id(ser_P(B_scan) || ser_P(B_spend))`.
    pub struct SpIdTag = hash_str("SPc/Id");
    /// The internal ledger key for one miner identity.
    pub struct SpIdHash(_);
}

/// Strip the `.worker` suffix from a miner identity string at the first `.`
/// (SPEC §2.1). Returns a subslice; no allocation.
///
/// This is the only transformation applied to `user_identity` before
/// `parse_sp_address`. Stats-only: the worker label never affects accounting.
pub fn strip_worker(identity: &str) -> &str {
    match identity.find('.') {
        Some(pos) => &identity[..pos],
        None => identity,
    }
}

/// `sp_id = tagged_hash("SPc/Id", ser_P(B_scan) || ser_P(B_spend))`.
///
/// This is the pool's internal ledger key. It MUST NOT leave the pool
/// (SPEC §0). The 32-byte output is not a consensus or wire value.
pub fn sp_id(b_scan: &PublicKey, b_spend: &PublicKey) -> [u8; 32] {
    let mut eng = SpIdHash::engine();
    eng.input(&b_scan.serialize());
    eng.input(&b_spend.serialize());
    SpIdHash::from_engine(eng).to_byte_array()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_worker_no_dot() {
        assert_eq!(strip_worker("tsp1qfoo"), "tsp1qfoo");
    }

    #[test]
    fn strip_worker_with_dot() {
        assert_eq!(strip_worker("tsp1qfoo.worker1"), "tsp1qfoo");
    }

    #[test]
    fn strip_worker_multiple_dots() {
        // Only the first dot is the separator.
        assert_eq!(strip_worker("tsp1qfoo.w.sub"), "tsp1qfoo");
    }

    #[test]
    fn strip_worker_empty() {
        assert_eq!(strip_worker(""), "");
    }

    #[test]
    fn sp_id_deterministic() {
        use bitcoin::secp256k1::{Secp256k1, SecretKey};
        let secp = Secp256k1::new();
        let k1 = SecretKey::from_slice(&[1u8; 32]).unwrap();
        let k2 = SecretKey::from_slice(&[2u8; 32]).unwrap();
        let b_scan = PublicKey::from_secret_key(&secp, &k1);
        let b_spend = PublicKey::from_secret_key(&secp, &k2);
        let id = sp_id(&b_scan, &b_spend);
        assert_eq!(id, sp_id(&b_scan, &b_spend), "must be deterministic");
    }

    #[test]
    fn sp_id_order_matters() {
        use bitcoin::secp256k1::{Secp256k1, SecretKey};
        let secp = Secp256k1::new();
        let k1 = SecretKey::from_slice(&[1u8; 32]).unwrap();
        let k2 = SecretKey::from_slice(&[2u8; 32]).unwrap();
        let b_scan = PublicKey::from_secret_key(&secp, &k1);
        let b_spend = PublicKey::from_secret_key(&secp, &k2);
        assert_ne!(sp_id(&b_scan, &b_spend), sp_id(&b_spend, &b_scan));
    }
}
