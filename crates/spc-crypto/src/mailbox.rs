//! Mailbox blinding helpers (SPEC §2.3, MAILBOX.md, DESIGN.md "Mailbox delivery").
//!
//! Two constructions are provided:
//!
//! **Static / variant A** (one-time per Ark server):
//! - `mailbox_blinding_hint`: miner gives pool `D = b_ark * S_mb`.
//! - `pool_blinded_mailbox_pubkey`: pool computes `M + D + t_ark * S_mb`.
//!
//! **Epoch-derived / variant B** (default per SPEC §2.3, decision 2026-10-14):
//! - `derived_mailbox_tweak`: `t_mb = hash_SPc/Mailbox(ser_P(ecdh))`.
//! - `derived_epoch_mailbox_pubkey`: `M_i = M + t_mb * G`.

use bitcoin::hashes::{sha256t_hash_newtype, Hash};
use bitcoin::secp256k1::{PublicKey, Scalar, Secp256k1, SecretKey};

use crate::{kdf::SharedSecret, Error};

sha256t_hash_newtype! {
    /// Tag: `hash_SPc/Mailbox(ser_P(ecdh))` (variant B epoch mailbox tweak).
    pub struct MailboxTweakTag = hash_str("SPc/Mailbox");
    /// Variant-B per-epoch mailbox tweak hash.
    pub struct MailboxTweakHash(_);
}

/// `D = b_ark * S_mb`.
///
/// Miner-side, once per Ark server. `S_mb` is the server's mailbox pubkey
/// (`ArkInfo::mailbox_pubkey`). Registered privately with the pool alongside
/// the miner's static mailbox id `M`. Gives the pool nothing new (it already
/// knows every `U_i`), but enables blinding without `u_i` (DESIGN.md).
pub fn mailbox_blinding_hint(
    b_ark: &SecretKey,
    server_mailbox_pubkey: &PublicKey,
) -> Result<PublicKey, Error> {
    server_mailbox_pubkey
        .mul_tweak(&Secp256k1::new(), &Scalar::from(*b_ark))
        .map_err(|_| Error::PointAtInfinity)
}

/// Pool side: `blinded = M + D + t_ark * S_mb`.
///
/// Equals bark's `BlindedMailboxIdentifier::new(M, u, S_mb)` because
/// `D + t_ark * S_mb = b_ark * S_mb + t_ark * S_mb = (b_ark + t_ark) * S_mb = u * S_mb`.
/// The server unblinds to `M` (DESIGN.md "Recommended approach").
///
/// `M` and `S_mb` are `PublicKey` here; callers wrap them in the appropriate
/// ark-lib types when the `ark` feature is enabled.
pub fn pool_blinded_mailbox_pubkey(
    m: &PublicKey,
    d: &PublicKey,
    t_ark: &Scalar,
    server_mailbox_pubkey: &PublicKey,
) -> Result<PublicKey, Error> {
    let t_s = server_mailbox_pubkey
        .mul_tweak(&Secp256k1::new(), t_ark)
        .map_err(|_| Error::PointAtInfinity)?;
    PublicKey::combine_keys(&[m, d, &t_s]).map_err(|_| Error::PointAtInfinity)
}

/// `t_mb = hash_SPc/Mailbox(ser_P(ecdh))`, failing if 0 or >= n.
///
/// Variant B: derives the per-epoch mailbox tweak from the ECDH shared secret,
/// so no per-epoch registration is needed (SPEC §2.3, decision 2026-10-14).
pub fn derived_mailbox_tweak(ecdh: &SharedSecret) -> Result<Scalar, Error> {
    let bytes = MailboxTweakHash::hash(&ecdh.0).to_byte_array();
    let sk = SecretKey::from_slice(&bytes).map_err(|_| Error::InvalidScalar)?;
    Ok(Scalar::from(sk))
}

/// Pool side (variant B): `M_i = M + t_mb * G`.
///
/// `M` is the miner's static bark mailbox pubkey. No per-epoch registration
/// required. The miner derives the same `M_i` as `m + t_mb` on its side.
pub fn derived_epoch_mailbox_pubkey(
    m: &PublicKey,
    ecdh: &SharedSecret,
) -> Result<PublicKey, Error> {
    let t = derived_mailbox_tweak(ecdh)?;
    m.add_exp_tweak(&Secp256k1::new(), &t)
        .map_err(|_| Error::PointAtInfinity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::Secp256k1;

    fn setup() -> (SecretKey, PublicKey, SecretKey, PublicKey) {
        let secp = Secp256k1::new();
        let b_ark_sk = SecretKey::from_slice(&[0x02u8; 32]).unwrap();
        let b_ark_pub = PublicKey::from_secret_key(&secp, &b_ark_sk);
        let s_mb_sk = SecretKey::from_slice(&[0x0au8; 32]).unwrap();
        let s_mb_pub = PublicKey::from_secret_key(&secp, &s_mb_sk);
        (b_ark_sk, b_ark_pub, s_mb_sk, s_mb_pub)
    }

    #[test]
    fn blinding_hint_deterministic() {
        let (b_ark_sk, _, _, s_mb_pub) = setup();
        let d1 = mailbox_blinding_hint(&b_ark_sk, &s_mb_pub).unwrap();
        let d2 = mailbox_blinding_hint(&b_ark_sk, &s_mb_pub).unwrap();
        assert_eq!(d1, d2);
    }

    /// Pool-side blinding equals `u * S_mb` where `u = b_ark + t_ark`.
    #[test]
    fn pool_blinding_equals_u_times_s_mb() {
        use crate::kdf::{ark_user_secret, tier_tweaks, SharedSecret};

        let secp = Secp256k1::new();
        let (b_ark_sk, b_ark_pub, _, s_mb_pub) = setup();

        let a_sk = SecretKey::from_slice(&[0x07u8; 32]).unwrap();
        let b_scan_sk = SecretKey::from_slice(&[0x01u8; 32]).unwrap();
        let b_scan_pub = PublicKey::from_secret_key(&secp, &b_scan_sk);

        let ecdh = SharedSecret::pool_side(800_000, &a_sk, &b_scan_pub).unwrap();
        let tweaks = tier_tweaks(&ecdh, 0).unwrap();

        let m_sk = SecretKey::from_slice(&[0x05u8; 32]).unwrap();
        let m_pub = PublicKey::from_secret_key(&secp, &m_sk);

        let d = mailbox_blinding_hint(&b_ark_sk, &s_mb_pub).unwrap();
        let pool_blinded =
            pool_blinded_mailbox_pubkey(&m_pub, &d, &tweaks.t_ark, &s_mb_pub).unwrap();

        // u = b_ark + t_ark; u * S_mb + M = blinded_i
        let u_sk = ark_user_secret(&b_ark_sk, &tweaks.t_ark).unwrap();
        let u_times_s_mb = s_mb_pub.mul_tweak(&secp, &Scalar::from(u_sk)).unwrap();
        let expected = PublicKey::combine_keys(&[&m_pub, &u_times_s_mb]).unwrap();
        assert_eq!(pool_blinded, expected);

        // And ignore b_ark_pub in this test (it's derivable from b_ark_sk)
        let _ = b_ark_pub;
    }

    #[test]
    fn derived_mailbox_tweak_deterministic() {
        use crate::kdf::SharedSecret;
        let secp = Secp256k1::new();
        let a_sk = SecretKey::from_slice(&[0x07u8; 32]).unwrap();
        let b_scan_sk = SecretKey::from_slice(&[0x01u8; 32]).unwrap();
        let b_scan_pub = PublicKey::from_secret_key(&secp, &b_scan_sk);
        let ecdh = SharedSecret::pool_side(800_000, &a_sk, &b_scan_pub).unwrap();

        let t1 = derived_mailbox_tweak(&ecdh).unwrap();
        let t2 = derived_mailbox_tweak(&ecdh).unwrap();
        assert_eq!(t1, t2);
    }

    #[test]
    fn derived_epoch_mailbox_differs_across_heights() {
        use crate::kdf::SharedSecret;
        let secp = Secp256k1::new();
        let a_sk = SecretKey::from_slice(&[0x07u8; 32]).unwrap();
        let b_scan_sk = SecretKey::from_slice(&[0x01u8; 32]).unwrap();
        let b_scan_pub = PublicKey::from_secret_key(&secp, &b_scan_sk);
        let m_pub =
            PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[0x05u8; 32]).unwrap());

        let ecdh_h = SharedSecret::pool_side(800_000, &a_sk, &b_scan_pub).unwrap();
        let ecdh_h1 = SharedSecret::pool_side(800_001, &a_sk, &b_scan_pub).unwrap();

        let mi_h = derived_epoch_mailbox_pubkey(&m_pub, &ecdh_h).unwrap();
        let mi_h1 = derived_epoch_mailbox_pubkey(&m_pub, &ecdh_h1).unwrap();
        assert_ne!(mi_h, mi_h1);
    }
}
