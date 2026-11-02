//! Pool-side ephemeral sending key (`a_send`, `A_send`) (SPEC §2.2).
//!
//! `SendKey` wraps an OS-CSPRNG-generated secret with even-Y normalisation,
//! zeroising on drop, and a `Debug` impl that never prints the secret.

use std::fmt;

use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
use rand::rngs::OsRng;
use zeroize::Zeroizing;

use crate::{kdf::SharedSecret, Error};

/// Pool-side ephemeral ECDH key for one block height.
///
/// - Secret bytes are held in a `Zeroizing<[u8; 32]>` and erased on drop.
/// - `Debug` prints only `h` and the x-coordinate of `A_send`.
/// - There is no `Clone` for the secret; use `derive_pool_side` to share work.
pub struct SendKey {
    /// The block height this key was generated for.
    pub h: u32,
    /// Zeroising secret scalar (even-Y normalised).
    secret: Zeroizing<[u8; 32]>,
    /// `ser_x(A_send)`: 32-byte x-coordinate, always even-Y.
    pub a_x: [u8; 32],
    /// Full compressed `A_send` (always 0x02 prefix). Stored so `public_key`
    /// cannot fail on a corrupted secret.
    public: PublicKey,
}

impl fmt::Debug for SendKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // MUST NOT print the secret scalar.
        f.debug_struct("SendKey")
            .field("h", &self.h)
            .field(
                "A_x",
                &self
                    .a_x
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
            )
            .finish_non_exhaustive()
    }
}

// No Clone for the secret; explicit restriction.
impl SendKey {
    /// Generate a fresh `SendKey` for height `h` from the OS CSPRNG.
    ///
    /// Applies even-Y normalisation: if `a_raw·G` has odd Y, use `n - a_raw`.
    /// The resulting `A_send` always has even Y (`0x02` prefix in `ser_P`).
    ///
    /// SPEC §2.2: MUST be from an OS CSPRNG; MUST NOT be derived from any
    /// seed or earlier `a_send`.
    pub fn generate(h: u32) -> Result<Self, Error> {
        let secp = Secp256k1::new();
        // Use rand + secp256k1's generate_keypair, which sources from OS.
        let (sk, pk) = secp.generate_keypair(&mut OsRng);

        // Even-Y normalisation.
        let (sk, pk) = if pk.serialize()[0] == 0x02 {
            (sk, pk)
        } else {
            // Negate: n - sk.
            let sk_neg = sk.negate();
            let pk_neg = PublicKey::from_secret_key(&secp, &sk_neg);
            debug_assert_eq!(pk_neg.serialize()[0], 0x02);
            (sk_neg, pk_neg)
        };

        let mut secret = Zeroizing::new([0u8; 32]);
        secret.copy_from_slice(&sk.secret_bytes());

        let pub_bytes = pk.serialize();
        debug_assert_eq!(pub_bytes[0], 0x02);
        let mut a_x = [0u8; 32];
        a_x.copy_from_slice(&pub_bytes[1..]);

        Ok(SendKey {
            h,
            secret,
            a_x,
            public: pk,
        })
    }

    /// Return the full compressed public key `A_send` (always even-Y, `0x02` prefix).
    pub fn public_key(&self) -> PublicKey {
        self.public
    }

    /// Return `ser_x(A_send)`: the 32-byte x-coordinate.
    pub fn x_only(&self) -> &[u8; 32] {
        &self.a_x
    }

    /// Compute the pool-side ECDH shared secret for miner `b_scan` at height `self.h`.
    pub fn derive_pool_side(&self, b_scan: &PublicKey) -> Result<SharedSecret, Error> {
        let sk = SecretKey::from_slice(&*self.secret).map_err(|_| Error::InvalidScalar)?;
        SharedSecret::pool_side(self.h, &sk, b_scan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_produces_even_y() {
        for _ in 0..16 {
            let key = SendKey::generate(800_000).unwrap();
            let pk = key.public_key();
            assert_eq!(pk.serialize()[0], 0x02, "A_send must have even Y");
        }
    }

    #[test]
    fn debug_hides_secret() {
        let key = SendKey::generate(1).unwrap();
        let dbg = format!("{key:?}");
        // The secret bytes should not appear.
        assert!(dbg.contains("SendKey"));
        assert!(dbg.contains("h:"));
        assert!(dbg.contains("A_x:"));
        // Must not contain the secret scalar as hex.
        let secret_hex = hex::encode(*key.secret);
        assert!(!dbg.contains(&secret_hex), "debug must not reveal secret");
    }

    #[test]
    fn derive_pool_side_matches_miner() {
        use bitcoin::secp256k1::Secp256k1;

        let key = SendKey::generate(800_000).unwrap();
        let secp = Secp256k1::new();
        let b_scan_sk = bitcoin::secp256k1::SecretKey::from_slice(&[0x01u8; 32]).unwrap();
        let b_scan_pub = PublicKey::from_secret_key(&secp, &b_scan_sk);

        let pool_side = key.derive_pool_side(&b_scan_pub).unwrap();
        let a_pub = key.public_key();
        let miner_side = SharedSecret::miner_side(key.h, &b_scan_sk, &a_pub).unwrap();
        assert_eq!(pool_side, miner_side);
    }
}
