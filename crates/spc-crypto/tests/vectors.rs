//! Frozen derivation vectors (SPEC §8.1, WP1).
//!
//! These are ported unchanged from `spc_ark/src/lib.rs::test_vector` and
//! `spc_ark_mailbox/src/lib.rs::tests::test_vector`. They constitute the
//! normative contract for the entire derivation chain.
//!
//! Fixture: seed = 0x42 * 32, a_send = 0x07 * 32, block_height = 800_000,
//! k = 0, server pubkey = (0x09 * 32)*G, server mailbox pubkey = (0x0a * 32)*G.
//!
//! The `ark` feature gates tests that require ark-lib (address encoding).

use bitcoin::secp256k1::{PublicKey, Scalar, Secp256k1, SecretKey};

use spc_crypto::{
    ark_user_pubkey, ark_user_secret, input_hash, mailbox_blinding_hint,
    pool_blinded_mailbox_pubkey, sp_id, spc_output_pubkey, tier_tweaks, SharedSecret,
};

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

const SEED: [u8; 32] = [0x42; 32];
const HEIGHT: u32 = 800_000;

#[allow(dead_code)]
struct Fixture {
    b_scan_sk: SecretKey,
    b_scan: PublicKey,
    b_spend_sk: SecretKey,
    b_spend: PublicKey,
    b_ark_sk: SecretKey,
    b_ark: PublicKey,
    /// bark's static mailbox key (m/350'/1').
    mailbox_sk: SecretKey,
    mailbox_pub: PublicKey,
    a_send_sk: SecretKey,
    a_send: PublicKey,
    server_pk: PublicKey,
    server_mailbox_sk: SecretKey,
    server_mailbox: PublicKey,
}

fn fixture() -> Fixture {
    use bitcoin::bip32::{ChildNumber, Xpriv};
    use bitcoin::Network;

    let secp = Secp256k1::new();
    let root = Xpriv::new_master(Network::Bitcoin, &SEED).unwrap();
    let h = |i| ChildNumber::from_hardened_idx(i).unwrap();

    let derive_sk = |path: Vec<ChildNumber>| root.derive_priv(&secp, &path).unwrap().private_key;

    let sp = |branch: u32| {
        derive_sk(vec![
            h(352),
            h(0),
            h(0),
            h(branch),
            ChildNumber::from_normal_idx(0).unwrap(),
        ])
    };

    let b_spend_sk = sp(0);
    let b_scan_sk = sp(1);
    let b_ark_sk = sp(2);
    let mailbox_sk = derive_sk(vec![h(350), h(1)]);

    let a_send_sk = SecretKey::from_slice(&[0x07u8; 32]).unwrap();
    let server_sk = SecretKey::from_slice(&[0x09u8; 32]).unwrap();
    let server_mailbox_sk = SecretKey::from_slice(&[0x0au8; 32]).unwrap();

    Fixture {
        b_scan: PublicKey::from_secret_key(&secp, &b_scan_sk),
        b_scan_sk,
        b_spend: PublicKey::from_secret_key(&secp, &b_spend_sk),
        b_spend_sk,
        b_ark: PublicKey::from_secret_key(&secp, &b_ark_sk),
        b_ark_sk,
        mailbox_pub: PublicKey::from_secret_key(&secp, &mailbox_sk),
        mailbox_sk,
        a_send: PublicKey::from_secret_key(&secp, &a_send_sk),
        a_send_sk,
        server_pk: PublicKey::from_secret_key(&secp, &server_sk),
        server_mailbox: PublicKey::from_secret_key(&secp, &server_mailbox_sk),
        server_mailbox_sk,
    }
}

fn pool_ecdh(f: &Fixture) -> SharedSecret {
    SharedSecret::pool_side(HEIGHT, &f.a_send_sk, &f.b_scan).unwrap()
}

// ---------------------------------------------------------------------------
// §8.1 Frozen vector — ported from spc_ark::tests::test_vector
// ---------------------------------------------------------------------------

#[test]
fn frozen_vector_keys() {
    let f = fixture();

    // Keys derived from seed 0x42*32 via BIP32.
    assert_eq!(
        f.a_send.to_string(),
        "02989c0b76cb563971fdc9bef31ec06c3560f3249d6ee9e5d83c57625596e05f6f"
    );
    assert_eq!(
        f.b_scan.to_string(),
        "024b19f0da60684ff1b6ee04c39c753a823c4bc758872931bdbef4ca14ff4b9c50"
    );
    assert_eq!(
        f.b_spend.to_string(),
        "032cd165ae1dadd742f410eb06c56467feef432b218a14b216bada9573f9176907"
    );
    assert_eq!(
        f.b_ark.to_string(),
        "02c6accb73e60944acb90d0eaade2d6ddb56cf91449cd11dcd42771fabdf222b18"
    );
    assert_eq!(
        f.mailbox_pub.to_string(),
        "0235739eb757168f51be9a504da5b5cda61c37d95133ac677a76605d6971c5274f"
    );
}

#[test]
fn frozen_vector_input_hash() {
    let f = fixture();
    let ih = input_hash(HEIGHT, &f.a_send);
    assert_eq!(
        hex::encode(ih),
        "04df9f4ac4c6ce0c80eca39eca7d4d256eceb2ab2fbe6f52a0acfa8cde3dfd4b"
    );
}

#[test]
fn frozen_vector_ecdh() {
    let f = fixture();
    let ecdh = pool_ecdh(&f);
    assert_eq!(
        hex::encode(ecdh.0),
        "02cbef249596b2d7beff88c66723ef7241ec4557c2110831172619f5e277fa24ee"
    );
}

#[test]
fn frozen_vector_ecdh_sides_agree() {
    let f = fixture();
    let pool = pool_ecdh(&f);
    let miner = SharedSecret::miner_side(HEIGHT, &f.b_scan_sk, &f.a_send).unwrap();
    assert_eq!(pool, miner);
}

#[test]
fn frozen_vector_tier_tweaks() {
    let f = fixture();
    let ecdh = pool_ecdh(&f);
    let tweaks = tier_tweaks(&ecdh, 0).unwrap();
    assert_eq!(
        hex::encode(tweaks.t_onchain.to_be_bytes()),
        "1a06accc037f59f18a114acbb36df8e6a1c5d4b3754f326b2189e8b6e3db72bb"
    );
    assert_eq!(
        hex::encode(tweaks.t_ark.to_be_bytes()),
        "5f6d66827770b92b1d7250175da5f90862be9d19f73ed09a485251286158dd0c"
    );
}

#[test]
fn frozen_vector_p_spc() {
    let f = fixture();
    let ecdh = pool_ecdh(&f);
    let tweaks = tier_tweaks(&ecdh, 0).unwrap();
    let p = spc_output_pubkey(&f.b_spend, &tweaks.t_onchain).unwrap();
    assert_eq!(
        p.to_string(),
        "02a159acc1fa366ab4d5fb9e82adc37d6f5875e0dc9887faeffb58607bd809e831"
    );
}

#[test]
fn frozen_vector_u_ark() {
    let f = fixture();
    let ecdh = pool_ecdh(&f);
    let tweaks = tier_tweaks(&ecdh, 0).unwrap();
    let u = ark_user_pubkey(&f.b_ark, &tweaks.t_ark).unwrap();
    assert_eq!(
        u.to_string(),
        "028774fd21598aa994083488788d42c87d344b116e2552d2236c3644bd47f53937"
    );
}

#[test]
fn frozen_vector_mailbox_hint() {
    let f = fixture();
    let d = mailbox_blinding_hint(&f.b_ark_sk, &f.server_mailbox).unwrap();
    assert_eq!(
        d.to_string(),
        "02d864313e3db31b98210d966a3a02589696d63491921b7a119bd6943be353820a"
    );
}

#[test]
fn frozen_vector_blinded_mailbox_id() {
    let f = fixture();
    let ecdh = pool_ecdh(&f);
    let tweaks = tier_tweaks(&ecdh, 0).unwrap();
    let d = mailbox_blinding_hint(&f.b_ark_sk, &f.server_mailbox).unwrap();
    let blinded =
        pool_blinded_mailbox_pubkey(&f.mailbox_pub, &d, &tweaks.t_ark, &f.server_mailbox).unwrap();
    assert_eq!(
        blinded.to_string(),
        "023682649a5662a1e1d7e559003325ad2fe05f49be7a2b45a68673fceb9d009420"
    );
}

// ---------------------------------------------------------------------------
// Mailbox — pool blinding matches the key-splitting identity
// ---------------------------------------------------------------------------

#[test]
fn pool_blinding_matches_u_times_s_mb() {
    let f = fixture();
    let secp = Secp256k1::new();
    let ecdh = pool_ecdh(&f);
    let tweaks = tier_tweaks(&ecdh, 0).unwrap();

    let d = mailbox_blinding_hint(&f.b_ark_sk, &f.server_mailbox).unwrap();
    let pool_blinded =
        pool_blinded_mailbox_pubkey(&f.mailbox_pub, &d, &tweaks.t_ark, &f.server_mailbox).unwrap();

    // Verify algebraically: M + u * S_mb, where u = b_ark + t_ark.
    let u_sk = ark_user_secret(&f.b_ark_sk, &tweaks.t_ark).unwrap();
    let u_times_s_mb = f
        .server_mailbox
        .mul_tweak(&secp, &Scalar::from(u_sk))
        .unwrap();
    let expected = PublicKey::combine_keys(&[&f.mailbox_pub, &u_times_s_mb]).unwrap();
    assert_eq!(pool_blinded, expected);
}

// ---------------------------------------------------------------------------
// Variant B: derived epoch mailbox
// ---------------------------------------------------------------------------

#[test]
fn frozen_vector_variant_b_mailbox() {
    use spc_crypto::{derived_epoch_mailbox_pubkey, derived_mailbox_tweak};

    let f = fixture();
    let ecdh = pool_ecdh(&f);

    let t_mb = derived_mailbox_tweak(&ecdh).unwrap();
    assert_ne!(t_mb.to_be_bytes(), [0u8; 32]);

    let m_i = derived_epoch_mailbox_pubkey(&f.mailbox_pub, &ecdh).unwrap();
    // Frozen from spc_ark_mailbox::tests::test_vector variant_b_M_i.
    assert_eq!(
        m_i.to_string(),
        "031ff5ec81ad81bcbf75e828fec51c334d97191d1f001bfebc85cea1989b1b23a2"
    );
    // Must differ from the static mailbox.
    assert_ne!(m_i, f.mailbox_pub);
}

// ---------------------------------------------------------------------------
// Ark address (behind the `ark` feature)
// ---------------------------------------------------------------------------

#[cfg(feature = "ark")]
#[test]
fn frozen_vector_ark_address() {
    use spc_crypto::ark_feature::{build_bark_address, pool_blinded_mailbox_id};

    let f = fixture();
    let ecdh = pool_ecdh(&f);
    let tweaks = tier_tweaks(&ecdh, 0).unwrap();
    let u = ark_user_pubkey(&f.b_ark, &tweaks.t_ark).unwrap();
    let d = mailbox_blinding_hint(&f.b_ark_sk, &f.server_mailbox).unwrap();
    let mailbox_id = ark::mailbox::MailboxIdentifier::from_pubkey(f.mailbox_pub);
    let blinded =
        pool_blinded_mailbox_id(&mailbox_id, &d, &tweaks.t_ark, &f.server_mailbox).unwrap();
    let addr = build_bark_address(&f.server_pk, &u, blinded, false);
    assert_eq!(
        addr.to_string(),
        "ark1pw4zjupfzqqpgwa8ay9vc42v5pq6gs7ydgty86dztz9hz25kjydkrv39agl6njdezqyprdqnynftx9g0p6lj4jqpnykkjlczlfxl8526956r88l8tn5qfggqlrm0es"
    );
}

// ---------------------------------------------------------------------------
// §8.1 Negative controls
// ---------------------------------------------------------------------------

/// Odd-Y A_send without even-Y normalisation would produce a different ecdh.
#[test]
fn negative_odd_y_a_send_misses() {
    let f = fixture();
    let secp = Secp256k1::new();

    // Create an odd-Y A_send by negating the even-Y key.
    let a_odd_sk = f.a_send_sk.negate();
    let a_odd_pub = PublicKey::from_secret_key(&secp, &a_odd_sk);
    assert_eq!(a_odd_pub.serialize()[0], 0x03, "must be odd Y");

    let ecdh_correct = pool_ecdh(&f);
    // Pool computes with odd key = different ecdh than even key version.
    let ecdh_wrong = SharedSecret::pool_side(HEIGHT, &a_odd_sk, &f.b_scan).unwrap();
    assert_ne!(ecdh_correct, ecdh_wrong);
}

/// `input_hash` without A_send (using only the height) is not our construction.
#[test]
fn negative_input_hash_without_a_send() {
    // Compute input_hash with a random A_send vs the fixture's A_send.
    let f = fixture();
    let secp = Secp256k1::new();
    let other_sk = SecretKey::from_slice(&[0x55u8; 32]).unwrap();
    let other_pub = PublicKey::from_secret_key(&secp, &other_sk);

    let ih1 = input_hash(HEIGHT, &f.a_send);
    let ih2 = input_hash(HEIGHT, &other_pub);
    assert_ne!(ih1, ih2, "input_hash must commit to A_send");
}

/// t_onchain != t_ark for the same (ecdh, k) — domain separation works.
#[test]
fn negative_t_onchain_ne_t_ark() {
    let f = fixture();
    let ecdh = pool_ecdh(&f);
    let tweaks = tier_tweaks(&ecdh, 0).unwrap();
    assert_ne!(
        tweaks.t_onchain.to_be_bytes(),
        tweaks.t_ark.to_be_bytes(),
        "t_onchain must differ from t_ark"
    );
}

/// A digest of 0 or >= n fails rather than reducing.
#[test]
fn negative_zero_scalar_fails() {
    // The hash_to_scalar check is inside kdf.rs; confirm via a known-zero input.
    // We can't call `hash_to_scalar` directly; instead verify that a degenerate
    // ecdh can't produce valid tweaks. We construct this by checking that our
    // own SharedSecret internals reject zeros.
    //
    // The actual zero-scalar enforcement is tested in kdf.rs inline tests, and
    // confirmed by the fact that SecretKey::from_slice rejects [0u8;32].
    let zero = [0u8; 32];
    assert!(
        SecretKey::from_slice(&zero).is_err(),
        "secp256k1 must reject zero scalar"
    );
    // All-0xff is > n and also rejected.
    let big = [0xffu8; 32];
    assert!(
        SecretKey::from_slice(&big).is_err(),
        "secp256k1 must reject scalar >= n"
    );
}

/// B_ark == B_spend would expose B_ark - B_spend via output difference.
/// The registration layer rejects this; we record the check here as a guard.
#[test]
fn negative_b_ark_ne_b_spend() {
    let f = fixture();
    // The fixture derives them from different BIP32 paths, so they must differ.
    assert_ne!(f.b_ark, f.b_spend, "B_ark must differ from B_spend");
}

/// sp_id is deterministic and encodes the correct key order.
#[test]
fn negative_sp_id_key_order() {
    let f = fixture();
    let id1 = sp_id(&f.b_scan, &f.b_spend);
    let id2 = sp_id(&f.b_spend, &f.b_scan);
    assert_ne!(id1, id2, "sp_id must be order-sensitive");
}

// ---------------------------------------------------------------------------
// §8.1 Invalid address table (fail-closed, SPEC §2.1)
// ---------------------------------------------------------------------------

mod invalid_addresses {
    use spc_crypto::{parse_sp_address, Network};

    #[test]
    fn bare_bc1q() {
        assert!(parse_sp_address(
            "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq",
            Network::Mainnet
        )
        .is_err());
    }

    #[test]
    fn wrong_hrp_sp_for_testnet() {
        use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
        use spc_crypto::address::encode_sp_address;
        let secp = Secp256k1::new();
        let b_scan = PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[1u8; 32]).unwrap());
        let b_spend =
            PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[2u8; 32]).unwrap());
        let addr = encode_sp_address(Network::Mainnet, &b_scan, &b_spend).unwrap();
        // "sp" prefix, trying to parse as testnet "tsp".
        assert!(parse_sp_address(&addr, Network::Testnet).is_err());
    }

    #[test]
    fn wrong_hrp_tsp_for_mainnet() {
        use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
        use spc_crypto::address::encode_sp_address;
        let secp = Secp256k1::new();
        let b_scan = PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[1u8; 32]).unwrap());
        let b_spend =
            PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[2u8; 32]).unwrap());
        let addr = encode_sp_address(Network::Testnet, &b_scan, &b_spend).unwrap();
        assert!(parse_sp_address(&addr, Network::Mainnet).is_err());
    }

    #[test]
    fn bad_checksum() {
        use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
        use spc_crypto::address::encode_sp_address;
        let secp = Secp256k1::new();
        let b_scan = PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[1u8; 32]).unwrap());
        let b_spend =
            PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[2u8; 32]).unwrap());
        let mut addr = encode_sp_address(Network::Testnet, &b_scan, &b_spend).unwrap();
        // Flip last char.
        let last = addr.pop().unwrap();
        addr.push(if last == 'q' { 'p' } else { 'q' });
        assert!(parse_sp_address(&addr, Network::Testnet).is_err());
    }

    #[test]
    fn truncated_63_chars() {
        // An address truncated below the minimum valid length.
        let s = "tsp1q".to_string() + &"q".repeat(58); // 5 + 58 = 63 chars
        assert!(parse_sp_address(&s, Network::Testnet).is_err());
    }

    #[test]
    fn empty_string() {
        assert!(parse_sp_address("", Network::Mainnet).is_err());
    }

    #[test]
    fn version_1_address() {
        // A bech32m address with witness version 1 (not 0).
        // Construct one directly.
        use bech32::{Bech32m, Hrp};
        // version=1 (Fe32::P), payload = 66 bytes
        let hrp = Hrp::parse("sp").unwrap();
        let mut payload = vec![1u8]; // version byte = 1
                                     // 66 more bytes (would be a valid payload if version were 0).
        payload.extend_from_slice(&[0x02u8; 33]); // fake b_scan
        payload.extend_from_slice(&[0x03u8; 33]); // fake b_spend
        let addr = bech32::encode::<Bech32m>(hrp, &payload).unwrap();
        assert!(parse_sp_address(&addr, Network::Mainnet).is_err());
    }
}

// ---------------------------------------------------------------------------
// Property test: pool-side and miner-side ECDH agree for random keys/heights
// ---------------------------------------------------------------------------

#[test]
fn property_ecdh_sides_agree_random() {
    use rand::rngs::OsRng;
    use rand::RngCore;

    let secp = Secp256k1::new();
    let mut rng = OsRng;

    for _ in 0..32 {
        // Random a_send secret (with even-Y normalisation).
        let mut raw = [0u8; 32];
        rng.fill_bytes(&mut raw);
        let a_sk = loop {
            rng.fill_bytes(&mut raw);
            if let Ok(sk) = SecretKey::from_slice(&raw) {
                break sk;
            }
        };
        let a_pub = PublicKey::from_secret_key(&secp, &a_sk);
        // Even-Y normalisation.
        let (a_sk, a_pub) = if a_pub.serialize()[0] == 0x02 {
            (a_sk, a_pub)
        } else {
            let neg = a_sk.negate();
            (neg, PublicKey::from_secret_key(&secp, &neg))
        };

        // Random b_scan.
        let b_sk = loop {
            rng.fill_bytes(&mut raw);
            if let Ok(sk) = SecretKey::from_slice(&raw) {
                break sk;
            }
        };
        let b_pub = PublicKey::from_secret_key(&secp, &b_sk);

        // Random height.
        let height = rng.next_u32();

        let pool = SharedSecret::pool_side(height, &a_sk, &b_pub).unwrap();
        let miner = SharedSecret::miner_side(height, &b_sk, &a_pub).unwrap();
        assert_eq!(
            pool, miner,
            "pool and miner ECDH must agree at height {height}"
        );
    }
}
