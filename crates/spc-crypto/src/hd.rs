//! HD key tree for the SPc agent wallet (SPEC 11.5, 11.8; plan 3.3).
//!
//! Two trees meet here:
//!
//! 1. The **SPc tree** `m/352'/0'/0'/{branch}'/0` (SLIP-352 style), which
//!    holds `b_spend` (branch 0, cold), `b_scan` (branch 1) and `b_ark`
//!    (branch 2). The agent is provisioned with the branch *account*
//!    xprvs (`m/352'/0'/0'/1'` and `m/352'/0'/0'/2'`) and derives the
//!    final `/0` child itself; the master seed and the spend branch never
//!    reach the mining daemon (SPEC 11.5).
//! 2. The **BIP 85 child wallet**: `m/83696968'/39'/0'/12'/0'` derives a
//!    12-word English mnemonic with an empty passphrase; its 64-byte
//!    BIP 39 seed is the *child seed* the embedded stock bark wallet runs
//!    on. The child seed owns bark's `m/350'/1'` mailbox key `M` and the
//!    normal (non-SPc) wallet keys `m/350'/0'/idx`; it cannot derive the
//!    SPc scan/ark/spend branches.
//!
//! [`provision`] is the pure core of the offline/lab provisioner: from one
//! master seed it produces exactly the material the agent is allowed to
//! hold (plus the cold spend branch, which stays in the provisioning
//! store). The production agent never loads a master seed, so it never
//! calls it.
//!
//! BIP 85 is implemented exactly (application 39, BIP 85 v2 entropy
//! derivation), not "BIP85-style": test vectors from BIP 85 are pinned in
//! the tests below.

use bip39::Mnemonic;
use bitcoin::bip32::{ChildNumber, Xpriv};
use bitcoin::hashes::sha512;
use bitcoin::hashes::{hmac::HmacEngine, Hash, HashEngine};
use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};

use crate::{Error, Network};

/// BIP 85 purpose (ASCII "ASCIIbits" -> 83696968).
pub const BIP85_PURPOSE: u32 = 83_696_968;
/// BIP 85 application number for BIP 39 mnemonics.
pub const BIP85_APP_BIP39: u32 = 39;
/// BIP 85 language code for English.
pub const BIP85_LANG_ENGLISH: u32 = 0;
/// BIP 85 app-39 word count the SPc child wallet uses.
pub const BIP85_CHILD_WORDS: u32 = 12;
/// BIP 85 app-39 index the SPc child wallet uses.
pub const BIP85_CHILD_INDEX: u32 = 0;

/// SPc key-tree purpose (SLIP 352).
pub const SPC_PURPOSE: u32 = 352;
/// SPc key-tree coin type.
pub const SPC_COIN: u32 = 0;
/// SPc key-tree account.
pub const SPC_ACCOUNT: u32 = 0;
/// SPc spend branch (`b_spend`; COLD, never on the agent).
pub const SPC_BRANCH_SPEND: u32 = 0;
/// SPc scan branch (`b_scan`).
pub const SPC_BRANCH_SCAN: u32 = 1;
/// SPc ark branch (`b_ark`).
pub const SPC_BRANCH_ARK: u32 = 2;

/// bark wallet purpose (`BARK_PURPOSE_INDEX` in bark at c419159).
pub const BARK_PURPOSE: u32 = 350;
/// bark mailbox key index (`MAILBOX_KEY_INDEX`): `M = m/350'/1'`.
pub const BARK_MAILBOX_INDEX: u32 = 1;
/// bark normal VTXO key index (`VTXO_KEYS_INDEX`): `m/350'/0'/idx`.
pub const BARK_VTXO_INDEX: u32 = 0;

/// Map the crate network selector onto `bitcoin::Network` (used only as
/// BIP 32 metadata; derivation itself is network-independent).
fn bitcoin_network(network: Network) -> bitcoin::Network {
    match network {
        Network::Mainnet => bitcoin::Network::Bitcoin,
        Network::Testnet => bitcoin::Network::Testnet,
        Network::Signet => bitcoin::Network::Signet,
        Network::Regtest => bitcoin::Network::Regtest,
    }
}

fn hardened(index: u32) -> Result<ChildNumber, Error> {
    ChildNumber::from_hardened_idx(index).map_err(|_| Error::Hd(format!("hardened index {index}")))
}

fn normal(index: u32) -> Result<ChildNumber, Error> {
    ChildNumber::from_normal_idx(index).map_err(|_| Error::Hd(format!("normal index {index}")))
}

/// BIP 85 master entropy: `HMAC-SHA512(key = "bip-entropy-from-k",
/// msg = ser256(k))` where `k` is the private key at `path` (all steps
/// hardened per BIP 85). Applications truncate the 64-byte output.
fn bip85_entropy(root: &Xpriv, path: &[ChildNumber]) -> [u8; 64] {
    let secp = Secp256k1::new();
    let key = root
        .derive_priv(&secp, &path)
        .expect("BIP 85 path is fully hardened, derivation cannot fail");
    let mut engine = HmacEngine::<sha512::Hash>::new(b"bip-entropy-from-k");
    engine.input(&key.private_key[..]);
    let hmac = bitcoin::hashes::hmac::Hmac::from_engine(engine);
    hmac.to_byte_array()
}

/// BIP 85 application 39 (BIP 39 mnemonic) entropy, truncated to the
/// word-count's BIP 39 entropy length (12/15/18/21/24 words ->
/// 16/20/24/28/32 bytes). Fails closed on any other word count or
/// language code.
pub fn bip85_app39_entropy(
    root: &Xpriv,
    language: u32,
    words: u32,
    index: u32,
) -> Result<Vec<u8>, Error> {
    if language != BIP85_LANG_ENGLISH {
        // BIP 85 lists ten languages; SPc pins English. Anything else is a
        // provisioning error, not a silent fallback.
        return Err(Error::Hd(format!(
            "unsupported BIP85 language code {language}"
        )));
    }
    let entropy_len = match words {
        12 => 16,
        15 => 20,
        18 => 24,
        21 => 28,
        24 => 32,
        other => {
            return Err(Error::Hd(format!(
                "BIP85 word count must be 12, 15, 18, 21 or 24, got {other}"
            )))
        }
    };
    let path = [
        hardened(BIP85_PURPOSE)?,
        hardened(BIP85_APP_BIP39)?,
        hardened(language)?,
        hardened(words)?,
        hardened(index)?,
    ];
    let entropy = bip85_entropy(root, &path);
    Ok(entropy[..entropy_len].to_vec())
}

/// The SPc child wallet material derived from the master seed:
/// a 12-word English mnemonic (empty passphrase) and its 64-byte
/// BIP 39 seed. This is *defined*, not "BIP85-style" (plan 3.3):
///
/// ```text
/// child_mnemonic = BIP85(master, m/83696968'/39'/0'/12'/0')  # English, 12 words
/// child_seed     = BIP39(child_mnemonic).to_seed("")         # 64 bytes
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct ChildWalletMaterial {
    pub mnemonic: Mnemonic,
    pub seed: [u8; 64],
}

impl std::fmt::Debug for ChildWalletMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The mnemonic and seed are wallet secrets: print neither.
        f.debug_struct("ChildWalletMaterial")
            .finish_non_exhaustive()
    }
}

/// Derive the SPc child wallet (BIP 85 app 39, 12 English words, index 0,
/// empty passphrase) from the master seed.
pub fn spc_child_wallet_seed(
    master_seed: &[u8; 64],
    network: Network,
) -> Result<ChildWalletMaterial, Error> {
    let root = Xpriv::new_master(bitcoin_network(network), master_seed)
        .map_err(|e| Error::Hd(format!("master seed rejected: {e}")))?;
    let entropy = bip85_app39_entropy(
        &root,
        BIP85_LANG_ENGLISH,
        BIP85_CHILD_WORDS,
        BIP85_CHILD_INDEX,
    )?;
    let mnemonic = Mnemonic::from_entropy(&entropy).map_err(|e| {
        Error::Hd(format!(
            "BIP85 entropy is not a valid 12-word mnemonic: {e}"
        ))
    })?;
    let seed = mnemonic.to_seed("");
    Ok(ChildWalletMaterial { mnemonic, seed })
}

/// The SPc branch account xprv `m/352'/0'/0'/{branch}'` (depth 4).
///
/// The provisioner serializes this for the agent's key files; the agent
/// derives the final `/0` child with [`spc_branch_secret`].
pub fn spc_branch_xprv(root: &Xpriv, branch: u32) -> Result<Xpriv, Error> {
    let path = [
        hardened(SPC_PURPOSE)?,
        hardened(SPC_COIN)?,
        hardened(SPC_ACCOUNT)?,
        hardened(branch)?,
    ];
    root.derive_priv(&Secp256k1::new(), &path)
        .map_err(|e| Error::Hd(format!("SPc branch derivation failed: {e}")))
}

/// The SPc branch key `m/352'/0'/0'/{branch}'/0`: `b_scan` from branch 1,
/// `b_ark` from branch 2 (`b_spend` from branch 0 stays cold).
pub fn spc_branch_secret(branch: &Xpriv) -> Result<SecretKey, Error> {
    let child = branch
        .derive_priv(&Secp256k1::new(), &[normal(0)?])
        .map_err(|e| Error::Hd(format!("SPc branch child derivation failed: {e}")))?;
    Ok(child.private_key)
}

/// bark's wallet root under a seed: `m/350'` (parity with
/// `WalletSeed::new_from_seed` at c419159, which derives this purpose
/// step before any wallet key).
pub fn bark_root_xpriv(seed: &[u8; 64], network: Network) -> Result<Xpriv, Error> {
    let master = Xpriv::new_master(bitcoin_network(network), seed)
        .map_err(|e| Error::Hd(format!("seed rejected: {e}")))?;
    master
        .derive_priv(&Secp256k1::new(), &[hardened(BARK_PURPOSE)?])
        .map_err(|e| Error::Hd(format!("bark purpose derivation failed: {e}")))
}

/// bark's static mailbox key `M` = `m/350'/1'` under the child seed
/// (parity with bark `WalletSeed::to_mailbox_keypair`). The agent reads
/// `epoch_derived` mailboxes `M_i = M + t_mb*G` from this key.
pub fn bark_mailbox_keypair(child_seed: &[u8; 64], network: Network) -> Result<Keypair, Error> {
    let root = bark_root_xpriv(child_seed, network)?;
    let xpriv = root
        .derive_priv(&Secp256k1::new(), &[hardened(BARK_MAILBOX_INDEX)?])
        .map_err(|e| Error::Hd(format!("mailbox key derivation failed: {e}")))?;
    Ok(xpriv.to_keypair(&Secp256k1::new()))
}

/// bark's normal (non-SPc) wallet VTXO key `m/350'/0'/index` under the
/// child seed (parity with bark `WalletSeed::derive_vtxo_keypair`).
/// Refreshed normal VTXOs, refresh outputs and offboard change live
/// here; the SPc `U_i -> u_i` keystore is separate.
pub fn bark_vtxo_keypair(
    child_seed: &[u8; 64],
    network: Network,
    index: u32,
) -> Result<Keypair, Error> {
    let root = bark_root_xpriv(child_seed, network)?;
    let path = [hardened(BARK_VTXO_INDEX)?, normal(index)?];
    let xpriv = root
        .derive_priv(&Secp256k1::new(), &path)
        .map_err(|e| Error::Hd(format!("normal wallet key derivation failed: {e}")))?;
    Ok(xpriv.to_keypair(&Secp256k1::new()))
}

/// The full provisioning set for one SPc identity, from one master seed.
///
/// This is the pure core of the offline/lab provisioner (plan 3.3): it
/// produces the two branch xprvs the agent is allowed to hold, the cold
/// spend branch (which the provisioner keeps; the agent never sees it)
/// and the BIP 85 child wallet. The production agent consumes
/// already-provisioned files and never calls this.
pub struct ProvisionedKeys {
    /// `m/352'/0'/0'/0'` - `b_spend` branch. COLD: stays in the
    /// provisioning store, never written to the agent.
    pub spend_branch: Xpriv,
    /// `m/352'/0'/0'/1'` - `b_scan` branch, provisioned to the agent.
    pub scan_branch: Xpriv,
    /// `m/352'/0'/0'/2'` - `b_ark` branch, provisioned to the agent.
    pub ark_branch: Xpriv,
    /// The BIP 85 child wallet (mnemonic + 64-byte seed), provisioned to
    /// the agent for the embedded stock bark wallet.
    pub child: ChildWalletMaterial,
}

impl std::fmt::Debug for ProvisionedKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProvisionedKeys").finish_non_exhaustive()
    }
}

/// Provision one SPc identity from a master seed (pure; see
/// [`ProvisionedKeys`]).
pub fn provision(master_seed: &[u8; 64], network: Network) -> Result<ProvisionedKeys, Error> {
    let root = Xpriv::new_master(bitcoin_network(network), master_seed)
        .map_err(|e| Error::Hd(format!("master seed rejected: {e}")))?;
    Ok(ProvisionedKeys {
        spend_branch: spc_branch_xprv(&root, SPC_BRANCH_SPEND)?,
        scan_branch: spc_branch_xprv(&root, SPC_BRANCH_SCAN)?,
        ark_branch: spc_branch_xprv(&root, SPC_BRANCH_ARK)?,
        child: spc_child_wallet_seed(master_seed, network)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The BIP 85 test-vector master root key (BIP 85 "12 English words").
    const BIP85_MASTER_XPRV: &str = "xprv9s21ZrQH143K2LBWUUQRFXhucrQqBpKdRRxNVq2zBqsx8HVqFk2uYo8kmbaLLHRdqtQpUm98uKfu3vca1LqdGhUtyoFnCNkfmXRyPXLjbKb";

    const BIP85_ENTROPY_12W_0: &str = "6250b68daf746d12a24d58b4787a714b";
    const BIP85_MNEMONIC_12W_0: &str =
        "girl mad pet galaxy egg matter matrix prison refuse sense ordinary nose";

    #[test]
    fn bip85_vector_12_words_index_0() {
        let root: Xpriv = BIP85_MASTER_XPRV.parse().unwrap();
        let entropy = bip85_app39_entropy(&root, BIP85_LANG_ENGLISH, 12, 0).unwrap();
        assert_eq!(hex::encode(&entropy), BIP85_ENTROPY_12W_0);
        let mnemonic = Mnemonic::from_entropy(&entropy).unwrap();
        assert_eq!(mnemonic.to_string(), BIP85_MNEMONIC_12W_0);
    }

    #[test]
    fn bip85_rejects_unknown_word_counts_and_languages() {
        let root: Xpriv = BIP85_MASTER_XPRV.parse().unwrap();
        // 13 and 11 are not BIP 39 word counts.
        for words in [11u32, 13, 16, 0] {
            assert!(
                bip85_app39_entropy(&root, BIP85_LANG_ENGLISH, words, 0).is_err(),
                "word count {words} must be rejected"
            );
        }
        // SPc pins English; any other language code is a provisioning error.
        for language in [1u32, 2, 9] {
            assert!(
                bip85_app39_entropy(&root, language, 12, 0).is_err(),
                "language {language} must be rejected"
            );
        }
    }

    #[test]
    fn child_wallet_seed_is_exact_and_deterministic() {
        let seed_a = [0x42u8; 64];
        let first = spc_child_wallet_seed(&seed_a, Network::Regtest).unwrap();
        let second = spc_child_wallet_seed(&seed_a, Network::Regtest).unwrap();
        assert_eq!(first, second, "same master seed must give the same child");
        assert_eq!(first.mnemonic.word_count(), 12);
        assert_eq!(first.seed.len(), 64);
        // Empty passphrase, BIP 39: seed is the PBKDF2 of the mnemonic.
        assert_eq!(first.seed, first.mnemonic.to_seed(""));

        // A different master seed must give a different child wallet.
        let other = spc_child_wallet_seed(&[0x43u8; 64], Network::Regtest).unwrap();
        assert_ne!(first.seed, other.seed);
        assert_ne!(first.mnemonic, other.mnemonic);

        // Network is BIP 32 metadata only: derivation is identical.
        let mainnet = spc_child_wallet_seed(&seed_a, Network::Mainnet).unwrap();
        assert_eq!(first.mnemonic, mainnet.mnemonic);
        assert_eq!(first.seed, mainnet.seed);
    }

    #[test]
    fn branch_xprvs_have_the_exact_sp_c_paths() {
        let master_seed = [0x42u8; 64];
        let keys = provision(&master_seed, Network::Regtest).unwrap();

        // Depth 4 account xprvs at m/352'/0'/0'/{0,1,2}'.
        for (branch, xprv) in [
            (SPC_BRANCH_SPEND, &keys.spend_branch),
            (SPC_BRANCH_SCAN, &keys.scan_branch),
            (SPC_BRANCH_ARK, &keys.ark_branch),
        ] {
            assert_eq!(xprv.depth, 4);
            assert_eq!(xprv.child_number, hardened(branch).unwrap());
        }

        // The agent-side branch keys are m/352'/0'/0'/{1,2}'/0.
        let secp = Secp256k1::new();
        let root = Xpriv::new_master(bitcoin::Network::Regtest, &master_seed).unwrap();
        let h = |i| hardened(i).unwrap();
        let scan = root
            .derive_priv(&secp, &[h(352), h(0), h(0), h(1), normal(0).unwrap()])
            .unwrap();
        assert_eq!(
            spc_branch_secret(&keys.scan_branch).unwrap(),
            scan.private_key
        );
        let ark = root
            .derive_priv(&secp, &[h(352), h(0), h(0), h(2), normal(0).unwrap()])
            .unwrap();
        assert_eq!(
            spc_branch_secret(&keys.ark_branch).unwrap(),
            ark.private_key
        );

        // Branches are distinct keys.
        assert_ne!(scan.private_key, ark.private_key);

        // Serialization round-trip: the provisioner writes the xprv string,
        // the agent reads it back.
        let text = keys.scan_branch.to_string();
        let back: Xpriv = text.parse().unwrap();
        assert_eq!(back, keys.scan_branch);
    }

    #[test]
    fn bark_keys_under_the_child_seed() {
        let master_seed = [0x42u8; 64];
        let keys = provision(&master_seed, Network::Regtest).unwrap();
        let child_seed = keys.child.seed;

        // M = m/350'/1' under the child seed, exactly as the plan defines.
        let secp = Secp256k1::new();
        let child_root = Xpriv::new_master(bitcoin::Network::Regtest, &child_seed).unwrap();
        let expected_mailbox = child_root
            .derive_priv(&secp, &[hardened(350).unwrap(), hardened(1).unwrap()])
            .unwrap()
            .to_keypair(&secp);
        let mailbox = bark_mailbox_keypair(&child_seed, Network::Regtest).unwrap();
        assert_eq!(mailbox.public_key(), expected_mailbox.public_key());

        // Normal wallet key m/350'/0'/0 is a different key.
        let normal0 = bark_vtxo_keypair(&child_seed, Network::Regtest, 0).unwrap();
        assert_ne!(mailbox.public_key(), normal0.public_key());

        // The child seed cannot derive the SPc branches: the mailbox key is
        // not any SPc branch key (different trees, same seed space).
        let scan = spc_branch_secret(&keys.scan_branch).unwrap();
        assert_ne!(mailbox.secret_key(), scan);
        assert_ne!(normal0.secret_key(), scan);

        // Determinism across calls.
        assert_eq!(
            bark_mailbox_keypair(&child_seed, Network::Regtest)
                .unwrap()
                .public_key(),
            mailbox.public_key()
        );
    }
}
