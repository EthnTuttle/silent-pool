//! Channel identity parsing, shared by the SV1 and SV2 listeners
//! (SPEC §2.1, §3.1). There is no default payout mode.

use spc_crypto::{parse_sp_address, Network};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelIdentity {
    pub sp_id: [u8; 32],
    /// 33-byte compressed scan key.
    pub b_scan: [u8; 33],
    /// 33-byte compressed spend key.
    pub b_spend: [u8; 33],
    /// Statistics-only label from a `.worker` suffix.
    pub worker: Option<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IdentityError {
    #[error("invalid-user-identity")]
    Invalid,
}

/// Parse a `user_identity` (SV2) or SV1 username. A `.worker` suffix is
/// stripped at the first `.`; the remainder must be a valid BIP 352 address
/// for this pool's network. Any failure is `invalid-user-identity`.
pub fn parse_identity(raw: &str, network: Network) -> Result<ChannelIdentity, IdentityError> {
    let stripped = spc_crypto::strip_worker(raw);
    let address = parse_sp_address(stripped, network).map_err(|_| IdentityError::Invalid)?;
    let sp_id = spc_crypto::sp_id(&address.b_scan, &address.b_spend);
    let worker = if stripped.len() == raw.len() {
        None
    } else {
        let suffix = &raw[stripped.len() + 1..];
        if suffix.is_empty() {
            None
        } else {
            Some(suffix.to_string())
        }
    };
    Ok(ChannelIdentity {
        sp_id,
        b_scan: address.b_scan.serialize(),
        b_spend: address.b_spend.serialize(),
        worker,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};

    fn addr(network: Network, seed: u8) -> String {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[seed; 32]).unwrap();
        let sp = SecretKey::from_slice(&[seed.wrapping_add(1); 32]).unwrap();
        spc_crypto::address::encode_sp_address(
            network,
            &PublicKey::from_secret_key(&secp, &sk),
            &PublicKey::from_secret_key(&secp, &sp),
        )
        .unwrap()
    }

    #[test]
    fn accepts_both_hrps_for_their_networks() {
        let main = addr(Network::Mainnet, 3);
        assert!(parse_identity(&main, Network::Mainnet).is_ok());
        assert!(parse_identity(&main, Network::Testnet).is_err());
        let test = addr(Network::Testnet, 3);
        assert!(parse_identity(&test, Network::Testnet).is_ok());
        assert!(parse_identity(&test, Network::Signet).is_ok());
        assert!(parse_identity(&test, Network::Regtest).is_ok());
        assert!(parse_identity(&test, Network::Mainnet).is_err());
    }

    #[test]
    fn strips_worker_suffix_at_the_first_dot() {
        let a = addr(Network::Testnet, 3);
        let id = parse_identity(&format!("{a}.rig.1"), Network::Testnet).unwrap();
        assert_eq!(id.worker.as_deref(), Some("rig.1"));
        let id = parse_identity(&a, Network::Testnet).unwrap();
        assert!(id.worker.is_none());
    }

    #[test]
    fn rejects_the_spec_table() {
        for raw in [
            "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq",
            "sri/solo/abc",
            "tr(tpub.../0/*)",
            "",
            "tsp1q",
        ] {
            assert_eq!(
                parse_identity(raw, Network::Testnet),
                Err(IdentityError::Invalid),
                "{raw}"
            );
        }
    }

    #[test]
    fn identity_is_stable_across_suffixes() {
        let a = addr(Network::Testnet, 3);
        let one = parse_identity(&format!("{a}.one"), Network::Testnet).unwrap();
        let two = parse_identity(&format!("{a}.two"), Network::Testnet).unwrap();
        assert_eq!(one.sp_id, two.sp_id);
        assert_ne!(one.worker, two.worker);
    }
}
