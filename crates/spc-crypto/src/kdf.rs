//! SPc key derivation functions (SPEC §2.3, DESIGN.md "Derivation scheme").
//!
//! All hash tags are BIP 340 tagged hashes. Hash-to-scalar MUST fail (never
//! reduce mod n) if the result is 0 or >= n.

use bitcoin::hashes::{sha256t_hash_newtype, Hash, HashEngine};
use bitcoin::secp256k1::{PublicKey, Scalar, Secp256k1, SecretKey};

use crate::Error;

sha256t_hash_newtype! {
    /// Tag: `hash_SPc/Inputs(ser32(H) || ser_P(A_send))`.
    pub struct InputsTag = hash_str("SPc/Inputs");
    /// The `input_hash` preimage hash.
    pub struct InputsHash(_);
}

sha256t_hash_newtype! {
    /// Tag: `hash_SPc/SharedSecret(ser_P(ecdh))`. Reserved for the optional
    /// 32-byte digest; NOT an input to any tier tweak.
    pub struct SharedSecretTag = hash_str("SPc/SharedSecret");
    /// The optional ECDH digest.
    pub struct SharedSecretHash(_);
}

sha256t_hash_newtype! {
    /// Tag: `hash_SPc/TierSpc(ser_P(ecdh) || ser32(k))`.
    pub struct TierSpcTag = hash_str("SPc/TierSpc");
    /// On-chain tier tweak hash.
    pub struct TierSpcHash(_);
}

sha256t_hash_newtype! {
    /// Tag: `hash_SPc/TierArk(ser_P(ecdh) || ser32(k))`.
    pub struct TierArkTag = hash_str("SPc/TierArk");
    /// Ark tier tweak hash.
    pub struct TierArkHash(_);
}

/// Interpret a 32-byte digest as a scalar in `[1, n-1]`, failing otherwise.
/// This implements the BIP 352 "fail rather than reduce" rule.
fn hash_to_scalar(bytes: [u8; 32]) -> Result<Scalar, Error> {
    let sk = SecretKey::from_slice(&bytes).map_err(|_| Error::InvalidScalar)?;
    Ok(Scalar::from(sk))
}

/// `input_hash = hash_SPc/Inputs(ser32(height) || ser_P(a_send))`.
///
/// `ser_P` is the 33-byte compressed SEC1 encoding. Because `A_send` always
/// has even Y (SPEC §2.2), the prefix byte is always `0x02`.
pub fn input_hash(height: u32, a_send: &PublicKey) -> [u8; 32] {
    let mut eng = InputsHash::engine();
    eng.input(&height.to_be_bytes());
    eng.input(&a_send.serialize());
    InputsHash::from_engine(eng).to_byte_array()
}

/// The SPc ECDH shared secret: `ser_P(input_hash * a_send * B_scan)`.
///
/// Both sides must compute the same 33 bytes for the derivation to agree.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SharedSecret(pub [u8; 33]);

impl std::fmt::Debug for SharedSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Display as hex; this is a public derivation intermediate, not a secret.
        let hex: String = self.0.iter().map(|b| format!("{b:02x}")).collect();
        write!(f, "SharedSecret({hex})")
    }
}

impl SharedSecret {
    /// Pool side: `input_hash * a_send * B_scan`.
    ///
    /// Computes `input_hash(H, A_send)` first, then `(ih * a_send) * B_scan`.
    pub fn pool_side(height: u32, a_send: &SecretKey, b_scan: &PublicKey) -> Result<Self, Error> {
        let secp = Secp256k1::new();
        let a_pub = PublicKey::from_secret_key(&secp, a_send);
        let ih = hash_to_scalar(input_hash(height, &a_pub))?;
        // ih * a_send
        let s = a_send.mul_tweak(&ih).map_err(|_| Error::InvalidScalar)?;
        // (ih * a_send) * B_scan
        let point = b_scan
            .mul_tweak(&secp, &Scalar::from(s))
            .map_err(|_| Error::PointAtInfinity)?;
        Ok(SharedSecret(point.serialize()))
    }

    /// Miner side: `input_hash * b_scan * A_send`.
    pub fn miner_side(height: u32, b_scan: &SecretKey, a_send: &PublicKey) -> Result<Self, Error> {
        let secp = Secp256k1::new();
        let ih = hash_to_scalar(input_hash(height, a_send))?;
        // ih * b_scan
        let s = b_scan.mul_tweak(&ih).map_err(|_| Error::InvalidScalar)?;
        // (ih * b_scan) * A_send
        let point = a_send
            .mul_tweak(&secp, &Scalar::from(s))
            .map_err(|_| Error::PointAtInfinity)?;
        Ok(SharedSecret(point.serialize()))
    }

    /// `hash_SPc/SharedSecret(ser_P(ecdh))`: an optional 32-byte digest.
    /// NOT used as input to any tier tweak (SPEC §2.3).
    pub fn digest(&self) -> [u8; 32] {
        SharedSecretHash::hash(&self.0).to_byte_array()
    }
}

/// Domain-separated tier tweaks from one shared secret (SPEC §2.3).
pub struct TierTweaks {
    /// `t_onchain = hash_SPc/TierSpc(ser_P(ecdh) || ser32(k))`
    pub t_onchain: Scalar,
    /// `t_ark = hash_SPc/TierArk(ser_P(ecdh) || ser32(k))`
    pub t_ark: Scalar,
}

/// Compute both tier tweaks for output index `k`.
///
/// Both fail individually if the digest is 0 or >= n (BIP 352 semantics).
/// `k = 0` in v0.1 (SPEC §2.3).
pub fn tier_tweaks(ecdh: &SharedSecret, k: u32) -> Result<TierTweaks, Error> {
    let t_onchain = {
        let mut eng = TierSpcHash::engine();
        eng.input(&ecdh.0);
        eng.input(&k.to_be_bytes());
        hash_to_scalar(TierSpcHash::from_engine(eng).to_byte_array())?
    };
    let t_ark = {
        let mut eng = TierArkHash::engine();
        eng.input(&ecdh.0);
        eng.input(&k.to_be_bytes());
        hash_to_scalar(TierArkHash::from_engine(eng).to_byte_array())?
    };
    Ok(TierTweaks { t_onchain, t_ark })
}

/// `P = B_spend + t_onchain * G` (coinbase tier output pubkey).
pub fn spc_output_pubkey(b_spend: &PublicKey, t_onchain: &Scalar) -> Result<PublicKey, Error> {
    b_spend
        .add_exp_tweak(&Secp256k1::new(), t_onchain)
        .map_err(|_| Error::PointAtInfinity)
}

/// `p = b_spend + t_onchain` (coinbase tier output secret key).
pub fn spc_output_secret(b_spend: &SecretKey, t_onchain: &Scalar) -> Result<SecretKey, Error> {
    b_spend
        .add_tweak(t_onchain)
        .map_err(|_| Error::PointAtInfinity)
}

/// `U = B_ark + t_ark * G` (Ark tier user pubkey).
pub fn ark_user_pubkey(b_ark: &PublicKey, t_ark: &Scalar) -> Result<PublicKey, Error> {
    b_ark
        .add_exp_tweak(&Secp256k1::new(), t_ark)
        .map_err(|_| Error::PointAtInfinity)
}

/// `u = b_ark + t_ark` (Ark tier user secret key).
pub fn ark_user_secret(b_ark: &SecretKey, t_ark: &Scalar) -> Result<SecretKey, Error> {
    b_ark.add_tweak(t_ark).map_err(|_| Error::PointAtInfinity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::Secp256k1;

    fn a_send() -> SecretKey {
        SecretKey::from_slice(&[0x07u8; 32]).unwrap()
    }

    fn b_scan_sk() -> SecretKey {
        SecretKey::from_slice(&[0x01u8; 32]).unwrap()
    }

    #[test]
    fn hash_to_scalar_zero_fails() {
        assert_eq!(hash_to_scalar([0u8; 32]), Err(Error::InvalidScalar));
    }

    #[test]
    fn hash_to_scalar_n_minus_one_ok() {
        // n - 1 = FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364140
        let n_minus_one: [u8; 32] = [
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xfe, 0xba, 0xae, 0xdc, 0xe6, 0xaf, 0x48, 0xa0, 0x3b, 0xbf, 0xd2, 0x5e, 0x8c,
            0xd0, 0x36, 0x41, 0x40,
        ];
        assert!(hash_to_scalar(n_minus_one).is_ok());
        let mut one = [0u8; 32];
        one[31] = 1;
        assert!(hash_to_scalar(one).is_ok());
    }

    #[test]
    fn hash_to_scalar_overflow_fails() {
        // All 0xff is definitely > n.
        assert_eq!(hash_to_scalar([0xffu8; 32]), Err(Error::InvalidScalar));
    }

    #[test]
    fn ecdh_sides_agree() {
        let secp = Secp256k1::new();
        let a = a_send();
        let a_pub = PublicKey::from_secret_key(&secp, &a);
        let b = b_scan_sk();
        let b_pub = PublicKey::from_secret_key(&secp, &b);

        let pool = SharedSecret::pool_side(800_000, &a, &b_pub).unwrap();
        let miner = SharedSecret::miner_side(800_000, &b, &a_pub).unwrap();
        assert_eq!(pool, miner);
    }

    #[test]
    fn tier_tweaks_differ() {
        let secp = Secp256k1::new();
        let a = a_send();
        let a_pub = PublicKey::from_secret_key(&secp, &a);
        let b = b_scan_sk();
        let b_pub = PublicKey::from_secret_key(&secp, &b);
        let ecdh = SharedSecret::pool_side(800_000, &a, &b_pub).unwrap();
        let _ = SharedSecret::miner_side(800_000, &b, &a_pub).unwrap(); // keep both sides tested
        let t0 = tier_tweaks(&ecdh, 0).unwrap();
        let t1 = tier_tweaks(&ecdh, 1).unwrap();
        // t_onchain != t_ark for the same (ecdh, k)
        assert_ne!(t0.t_onchain, t0.t_ark);
        // different k gives different tweaks
        assert_ne!(t0.t_onchain, t1.t_onchain);
        assert_ne!(t0.t_ark, t1.t_ark);
    }

    #[test]
    fn spc_output_spendable() {
        let secp = Secp256k1::new();
        let a = a_send();
        let b = b_scan_sk();
        let b_pub = PublicKey::from_secret_key(&secp, &b);
        let b_spend_sk = SecretKey::from_slice(&[0x02u8; 32]).unwrap();
        let b_spend_pub = PublicKey::from_secret_key(&secp, &b_spend_sk);

        let ecdh = SharedSecret::pool_side(800_000, &a, &b_pub).unwrap();
        let tweaks = tier_tweaks(&ecdh, 0).unwrap();
        let p_pub = spc_output_pubkey(&b_spend_pub, &tweaks.t_onchain).unwrap();
        let p_sk = spc_output_secret(&b_spend_sk, &tweaks.t_onchain).unwrap();
        assert_eq!(p_pub, PublicKey::from_secret_key(&secp, &p_sk));
    }

    #[test]
    fn ark_user_spendable() {
        let secp = Secp256k1::new();
        let a = a_send();
        let b = b_scan_sk();
        let b_pub = PublicKey::from_secret_key(&secp, &b);
        let b_ark_sk = SecretKey::from_slice(&[0x03u8; 32]).unwrap();
        let b_ark_pub = PublicKey::from_secret_key(&secp, &b_ark_sk);

        let ecdh = SharedSecret::pool_side(800_000, &a, &b_pub).unwrap();
        let tweaks = tier_tweaks(&ecdh, 0).unwrap();
        let u_pub = ark_user_pubkey(&b_ark_pub, &tweaks.t_ark).unwrap();
        let u_sk = ark_user_secret(&b_ark_sk, &tweaks.t_ark).unwrap();
        assert_eq!(u_pub, PublicKey::from_secret_key(&secp, &u_sk));
    }

    #[test]
    fn shared_secret_digest_uses_reserved_tag() {
        // digest() must use SPc/SharedSecret, not a tier tag.
        let secp = Secp256k1::new();
        let a = a_send();
        let b = b_scan_sk();
        let b_pub = PublicKey::from_secret_key(&secp, &b);
        let ecdh = SharedSecret::pool_side(800_000, &a, &b_pub).unwrap();
        let d = ecdh.digest();
        // Just check it's deterministic and non-zero.
        assert_eq!(d, ecdh.digest());
        assert_ne!(d, [0u8; 32]);
        // Digest MUST NOT equal tier tweaks.
        let tweaks = tier_tweaks(&ecdh, 0).unwrap();
        assert_ne!(d, tweaks.t_onchain.to_be_bytes());
        assert_ne!(d, tweaks.t_ark.to_be_bytes());
    }
}
