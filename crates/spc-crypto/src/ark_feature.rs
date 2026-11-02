//! Ark address building and mailbox identifier wrapping (SPEC §2.3, §11.8).
//!
//! Enabled only with `--features ark`. The default build MUST NOT require
//! ark-lib, so everything in this module is feature-gated.
//!
//! Ported from `spc_ark/src/lib.rs` (`build_bark_address`, `pool_blinded_mailbox_id`).

use ark::mailbox::{BlindedMailboxIdentifier, MailboxIdentifier};
use bitcoin::secp256k1::{PublicKey, Scalar, Secp256k1};

use crate::Error;

/// Pool side: wrap the miner's static or derived mailbox id into the
/// `BlindedMailboxIdentifier` that bark's server unblinds to `M`.
///
/// This is the ark-lib-typed version of `pool_blinded_mailbox_pubkey`. The
/// result is `M + D + t_ark * S_mb`.
///
/// Arguments:
/// - `mailbox_id`: miner's `MailboxIdentifier` (static or derived).
/// - `hint`: `D = b_ark * S_mb` (blinding hint).
/// - `t_ark`: tier tweak scalar.
/// - `server_mailbox_pubkey`: the Ark server's mailbox public key.
pub fn pool_blinded_mailbox_id(
    mailbox_id: &MailboxIdentifier,
    hint: &PublicKey,
    t_ark: &Scalar,
    server_mailbox_pubkey: &PublicKey,
) -> Result<BlindedMailboxIdentifier, Error> {
    let secp = Secp256k1::new();
    let t_s = server_mailbox_pubkey
        .mul_tweak(&secp, t_ark)
        .map_err(|_| Error::PointAtInfinity)?;
    let blinded = PublicKey::combine_keys(&[&mailbox_id.as_pubkey(), hint, &t_s])
        .map_err(|_| Error::PointAtInfinity)?;
    Ok(BlindedMailboxIdentifier::from_pubkey(blinded))
}

/// Build a payable bark `ark1` address for the tweaked Ark user pubkey `U`.
///
/// Constructs: `ArkId` from the server pubkey, a `Pubkey` policy with `U`,
/// and a `ServerMailbox` delivery with the given blinded id.
///
/// Ported from `spc_ark::build_bark_address`.
pub fn build_bark_address(
    server_pubkey: &PublicKey,
    ark_user_pubkey: &PublicKey,
    blinded_id: BlindedMailboxIdentifier,
    testnet: bool,
) -> ark::Address {
    use ark::address::VtxoDelivery;

    ark::Address::builder()
        .testnet(testnet)
        .server_pubkey(*server_pubkey)
        .pubkey_policy(*ark_user_pubkey)
        .delivery(VtxoDelivery::ServerMailbox { blinded_id })
        .into_address()
        // Safety: all required fields (server_pubkey, policy, delivery) are set.
        .expect("all required fields set")
}
