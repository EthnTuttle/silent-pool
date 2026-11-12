//! AP-3a wallet recovery tests (plan AP-3 recovery acceptance).
//!
//! The committed fixture `tests/fixtures/ap3a-recovery-fixture.json`
//! holds a frozen provisioning set: the master seed, its BIP 85 child
//! seed, the scan/ark branch xprvs and every expected public value
//! (`sp_id`, `M`, normal-wallet key, per-block `U_i`, VTXO
//! ids/amounts). The recovery run never mints a new seed; it recreates
//! the agent from the same inputs.
//!
//! Honest scope (should-fix 8): the fixture is **synthetic**. The
//! blocks are deterministic derivations with explicit expiry constants
//! (see each block's `expiry_provenance`), and the recovery proven here
//! is **key derivation plus ledger replay** - not real VTXO ancestry
//! parsing, not a live mailbox read, and not stock bark VTXO recovery
//! (all of those are AP-3b's network backend). `VtxoState::Spendable`
//! in the observations is caller-asserted and faithfully persisted,
//! never verified against a chain.
//!
//! Provenance: the fixture was generated once by
//! `cargo test --features ark --test wallet_recovery -- --ignored
//! regenerate_fixture` from the deterministic master seed below (two
//! SHA-256 halves, no entropy source) and then frozen. Regenerating it
//! would produce byte-identical output; the committed file is the
//! reference.

#![cfg(feature = "ark")]

mod wallet_common;

use std::net::SocketAddr;
use std::path::Path;

use bitcoin::hashes::{sha256, Hash as _};
use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
use serde::Deserialize;
use spc_agent::wallet::receive::{Expiry, ExpirySource, ObservedVtxo, VtxoState};
use spc_agent::wallet::registration::RegistrationOutcome;
use spc_agent::wallet::{WalletRole, WalletSettings};
use spc_crypto::Network;
use wallet_common::{provision_files, MockTransport, RecordWriteExt as _, WalletCommonExt as _};

#[derive(Clone, Debug, Deserialize)]
struct Fixture {
    seed_source: String,
    /// Honest provenance label (should-fix 8): the fixture is
    /// synthetic - deterministic derivations and ledger replay, not
    /// live ancestry/mailbox/VTXO recovery.
    #[serde(default)]
    provenance: Option<String>,
    network: String,
    master_seed_hex: String,
    scan_xprv: String,
    ark_xprv: String,
    child_seed_hex: String,
    sp_address: String,
    sp_id_hex: String,
    wallet_fingerprint_hex: String,
    mailbox_pubkey: String,
    normal_vtxo_pubkey: String,
    b_ark_pubkey: String,
    tip_height: u32,
    blocks: Vec<FixtureBlock>,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize)]
struct FixtureBlock {
    height: u32,
    a_send: String,
    u_pubkey: String,
    vtxo_id_hex: String,
    amount_sats: u64,
    expiry_height: u32,
    /// Honest provenance label (should-fix 8): the expiry is an
    /// explicit constant chosen for the refresh-window scenario, not a
    /// value parsed from real VTXO ancestry.
    expiry_provenance: String,
}

fn fixture_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ap3a-recovery-fixture.json")
}

fn load_fixture() -> Fixture {
    let text = std::fs::read_to_string(fixture_path()).expect(
        "tests/fixtures/ap3a-recovery-fixture.json is committed; \
         regenerate with --ignored regenerate_fixture",
    );
    let fixture: Fixture = serde_json::from_str(&text).expect("fixture JSON is valid");
    // The recovery scenario is regtest-only; fail closed if the frozen
    // file ever says otherwise instead of silently re-targeting it.
    assert_eq!(fixture.network, "regtest", "fixture network changed");
    // The provenance label is part of the fixture's contract: it must
    // declare itself synthetic (should-fix 8), so a hand-crafted
    // "real-looking" fixture replacement is caught here.
    assert!(
        fixture
            .provenance
            .as_deref()
            .is_some_and(|p| p.contains("SYNTHETIC")),
        "fixture must carry the synthetic provenance label"
    );
    fixture
}

/// The deterministic fixture master seed (no entropy source; two
/// SHA-256 halves).
fn fixture_master() -> [u8; 64] {
    let left = sha256::Hash::hash(b"spc-ap3a-recovery-fixture-left-v1");
    let right = sha256::Hash::hash(b"spc-ap3a-recovery-fixture-right-v1");
    let mut seed = [0u8; 64];
    seed[..32].copy_from_slice(&left.to_byte_array());
    seed[32..].copy_from_slice(&right.to_byte_array());
    seed
}

fn fixture_vtxo_id(height: u32, a_send: &PublicKey) -> [u8; 32] {
    use bitcoin::hashes::{Hash, HashEngine};
    let mut eng = bitcoin::hashes::sha256::HashEngine::default();
    eng.input(b"spc-ap3a-vtxo");
    eng.input(&height.to_be_bytes());
    eng.input(&a_send.serialize());
    bitcoin::hashes::sha256::Hash::from_engine(eng).to_byte_array()
}

/// Explicit, independent expiry constants for the synthetic fixture
/// blocks (should-fix 8). They are deliberately NOT derived from the
/// finding height: `FIXTURE_EXPIRY_IN_WINDOW` sits inside the
/// tip+96 refresh window (tip 800_400, so anything <= 800_496 is due)
/// and `FIXTURE_EXPIRY_OUT_OF_WINDOW` sits far outside it, so the
/// refresh-window aggregation is exercised in both directions.
const FIXTURE_EXPIRY_IN_WINDOW: u32 = 800_432;
const FIXTURE_EXPIRY_OUT_OF_WINDOW: u32 = 900_000;
const FIXTURE_TIP_HEIGHT: u32 = 800_400;

/// One-off generator for the committed fixture. Ignored in normal runs.
#[test]
#[ignore = "fixture generation; run explicitly to (re)create the committed file"]
fn regenerate_fixture() {
    let network = Network::Regtest;
    let master = fixture_master();
    let keys = spc_crypto::provision(&master, network).unwrap();
    let secp = Secp256k1::new();
    let scan = spc_crypto::hd::spc_branch_secret(&keys.scan_branch).unwrap();
    let ark = spc_crypto::hd::spc_branch_secret(&keys.ark_branch).unwrap();
    let spend = spc_crypto::hd::spc_branch_secret(&keys.spend_branch).unwrap();
    let scan_pub = PublicKey::from_secret_key(&secp, &scan);
    let ark_pub = PublicKey::from_secret_key(&secp, &ark);
    let spend_pub = PublicKey::from_secret_key(&secp, &spend);
    let sp_address =
        spc_crypto::address::encode_sp_address(network, &scan_pub, &spend_pub).unwrap();
    let mailbox = spc_crypto::hd::bark_mailbox_keypair(&keys.child.seed, network).unwrap();
    let normal = spc_crypto::hd::bark_vtxo_keypair(&keys.child.seed, network, 0).unwrap();

    let blocks: Vec<FixtureBlock> = [0x11u8, 0x22]
        .iter()
        .enumerate()
        .map(|(i, seed)| {
            let a_send =
                PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[*seed; 32]).unwrap());
            let height = 800_000 + i as u32;
            // The same derivation the agent's receive path runs.
            let ecdh = spc_crypto::SharedSecret::miner_side(height, &scan, &a_send).unwrap();
            let tweaks = spc_crypto::tier_tweaks(&ecdh, 0).unwrap();
            let u = spc_crypto::ark_user_pubkey(&ark_pub, &tweaks.t_ark).unwrap();
            // Explicit expiry constants (see their definitions): the
            // first block's expiry is inside the refresh window, the
            // second's far outside. Neither is derived from the finding
            // height - these are synthetic scenario inputs.
            let (expiry_height, expiry_provenance) = if i == 0 {
                (
                    FIXTURE_EXPIRY_IN_WINDOW,
                    format!(
                        "synthetic constant FIXTURE_EXPIRY_IN_WINDOW ({FIXTURE_EXPIRY_IN_WINDOW}); \
                         inside the tip+96 window, unrelated to the finding height"
                    ),
                )
            } else {
                (
                    FIXTURE_EXPIRY_OUT_OF_WINDOW,
                    format!(
                        "synthetic constant FIXTURE_EXPIRY_OUT_OF_WINDOW ({FIXTURE_EXPIRY_OUT_OF_WINDOW}); \
                         far outside the tip+96 window, unrelated to the finding height"
                    ),
                )
            };
            FixtureBlock {
                height,
                a_send: a_send.to_string(),
                u_pubkey: u.to_string(),
                vtxo_id_hex: hex::encode(fixture_vtxo_id(height, &a_send)),
                amount_sats: 21_000_000 + i as u64 * 5_000_000,
                expiry_height,
                expiry_provenance,
            }
        })
        .collect();

    let fixture = serde_json::json!({
        "seed_source": "frozen-fixture (deterministic master, spc-ap3a-recovery-fixture-*-v1)",
        "provenance": "SYNTHETIC: deterministic key derivations and ledger replay; the blocks are \
                       not real VTXOs, the expiries are explicit scenario constants (see each \
                       block's expiry_provenance), and the scenario is not live ancestry parsing, \
                       mailbox recovery, or stock bark VTXO recovery (those are AP-3b). VtxoState \
                       values are caller-asserted claims, faithfully persisted.",
        "network": "regtest",
        "master_seed_hex": hex::encode(master),
        "scan_xprv": keys.scan_branch.to_string(),
        "ark_xprv": keys.ark_branch.to_string(),
        "child_seed_hex": hex::encode(keys.child.seed),
        "sp_address": sp_address,
        "sp_id_hex": hex::encode(spc_crypto::sp_id(&scan_pub, &spend_pub)),
        "wallet_fingerprint_hex": hex::encode(
            spc_crypto::hd::bark_root_xpriv(&keys.child.seed, network)
                .unwrap()
                .fingerprint(&secp)
                .to_bytes()
        ),
        "mailbox_pubkey": mailbox.public_key().to_string(),
        "normal_vtxo_pubkey": normal.public_key().to_string(),
        "b_ark_pubkey": ark_pub.to_string(),
        "tip_height": FIXTURE_TIP_HEIGHT,
        "blocks": blocks,
    });
    std::fs::create_dir_all(fixture_path().parent().unwrap()).unwrap();
    std::fs::write(
        fixture_path(),
        serde_json::to_string_pretty(&fixture).unwrap(),
    )
    .unwrap();
    panic!(
        "fixture written to {}; committing it now",
        fixture_path().display()
    );
}

/// Write the fixture's key files (exactly what the provisioner handed
/// the agent) and build settings against `datadir`.
fn fixture_settings(fixture: &Fixture, dir: &Path, datadir: &Path) -> WalletSettings {
    wallet_common::write_secret(&dir.join("scan.xprv"), &fixture.scan_xprv);
    wallet_common::write_secret(&dir.join("ark.xprv"), &fixture.ark_xprv);
    wallet_common::write_secret(&dir.join("child.seed"), &fixture.child_seed_hex);
    WalletSettings {
        scan_xprv: dir.join("scan.xprv"),
        ark_xprv: dir.join("ark.xprv"),
        child_seed: dir.join("child.seed"),
        datadir: datadir.to_path_buf(),
        status_listen: SocketAddr::from(([127, 0, 0, 1], 0)),
        mailbox_mode: spc_agent::wallet::MailboxMode::EpochDerived,
        epoch_fallback: spc_agent::wallet::EpochFallback::Hold,
        ark_id: [0xa1, 0xb2, 0xc3, 0xd4],
        network: Network::Regtest,
        sp_address: fixture.sp_address.clone(),
    }
}

fn observation(block: &FixtureBlock, state: VtxoState, expiry: Expiry) -> ObservedVtxo {
    ObservedVtxo {
        // Frozen synthetic txid is unchanged; supply vout 0 for the Bark
        // outpoint-shaped record. This still is not a real server receipt.
        vtxo_id: {
            let mut id = [0u8; 36];
            id[..32].copy_from_slice(&hex::decode(&block.vtxo_id_hex).unwrap());
            id
        },
        u_pubkey: block.u_pubkey.parse().unwrap(),
        amount_sats: block.amount_sats,
        expiry,
        state,
    }
}

/// Phase 1: create the agent from the fixture, register, receive both
/// VTXOs, record the scan cursor - then verify identity, balances,
/// status redaction and a graceful stop.
#[tokio::test]
async fn first_run_registers_claims_and_stops_gracefully() {
    let fixture = load_fixture();
    let dir = tempfile::tempdir().unwrap();
    let datadir = dir.path().join("datadir");
    let settings = fixture_settings(&fixture, dir.path(), &datadir);
    let role = WalletRole::start(settings).await.unwrap();

    // Identity: the fixture's frozen public values, exactly.
    let keys = role.keys();
    assert_eq!(hex::encode(keys.sp_id), fixture.sp_id_hex);
    assert_eq!(
        hex::encode(keys.wallet_fingerprint.to_bytes()),
        fixture.wallet_fingerprint_hex
    );
    assert_eq!(keys.mailbox_public().to_string(), fixture.mailbox_pubkey);
    assert_eq!(
        keys.normal_vtxo_public().to_string(),
        fixture.normal_vtxo_pubkey
    );
    assert_eq!(keys.b_ark_public().to_string(), fixture.b_ark_pubkey);

    // Register: one transport failure, then applied. Same attempt, one
    // request id.
    let mut transport = MockTransport::new(vec![
        Err("no route to front".into()),
        Ok(RegistrationOutcome::Applied),
    ]);
    let a1 = role
        .with_registration(|r| r.attempt(&mut transport))
        .unwrap();
    assert_eq!(a1.request_id, 1);
    assert_eq!(a1.outcome, RegistrationOutcome::Unavailable);
    let a2 = role
        .with_registration(|r| r.attempt(&mut transport))
        .unwrap();
    assert_eq!(a2.request_id, 1, "retry reuses the pending id");
    assert_eq!(a2.outcome, RegistrationOutcome::Applied);
    assert!(role
        .store()
        .is_registered(&role.registration_digest())
        .unwrap());
    assert_eq!(transport.sent.len(), 2);

    // Receive: derive U per block, then run the observations through
    // the ledger exactly as the (AP-3b) mailbox poll would.
    for block in &fixture.blocks {
        let a_send: PublicKey = block.a_send.parse().unwrap();
        let u = role.derive_u(block.height, &a_send);
        assert_eq!(
            u.to_string(),
            block.u_pubkey,
            "U derivation matches the frozen fixture"
        );
    }
    let first = &fixture.blocks[0];
    let second = &fixture.blocks[1];
    // Block 1: spendable with an ancestry expiry.
    assert!(role
        .ledger()
        .record(&observation(
            first,
            VtxoState::Spendable,
            Expiry::At {
                height: first.expiry_height,
                source: ExpirySource::VtxoAncestry
            }
        ))
        .unwrap()
        .is_inserted());
    // Block 2: pending with unknown expiry first, then validated
    // spendable with a server-reported expiry.
    assert!(role
        .ledger()
        .record(&observation(second, VtxoState::Pending, Expiry::Unknown))
        .unwrap()
        .is_inserted());
    assert!(role
        .ledger()
        .record(&observation(
            second,
            VtxoState::Spendable,
            Expiry::At {
                height: second.expiry_height,
                source: ExpirySource::ServerReport
            }
        ))
        .unwrap()
        .is_updated());
    role.store().set_scan_height(fixture.tip_height).unwrap();

    // Balances: honest aggregates, unknown is null (never zero).
    let snapshot = role.status().snapshot().unwrap();
    assert!(snapshot.registered);
    assert_eq!(snapshot.spendable_sats, Some(47_000_000));
    assert_eq!(snapshot.pending_sats, Some(0));
    assert_eq!(snapshot.min_expiry_height, Some(first.expiry_height));
    assert_eq!(snapshot.last_scan_height, Some(fixture.tip_height));
    // needs_refresh: tip 800_400, threshold 96 -> due at expiry <= 800_496.
    // Only the first block's expiry (the explicit in-window constant
    // 800_432) falls inside the window; the second (the explicit
    // out-of-window constant 900_000) does not, so only its amount
    // counts. One VTXO per direction - both aggregation branches are
    // exercised.
    assert_eq!(snapshot.needs_refresh_sats, Some(21_000_000));

    // Reconciliation against the pool-reported total.
    assert_eq!(
        role.ledger().reconcile(47_000_000).unwrap(),
        spc_agent::wallet::Reconciliation::Ok
    );

    // Status JSON over HTTP: honest fields, redacted payload.
    let body = role.http_status_json().await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["registered"], true);
    assert_eq!(json["spendable_sats"], 47_000_000);
    assert_eq!(json["min_expiry_height"], first.expiry_height);
    let lowered = body.to_lowercase();
    for banned in ["sp_id", "secret", "b_scan", "b_ark", "vtxo_id", "u_pubkey"] {
        assert!(!lowered.contains(banned), "status leaked '{banned}'");
    }

    // Graceful stop: run() observes shutdown, state -> stopped.
    let status = role.status().clone();
    role.shutdown();
    role.run(None).await.unwrap();
    assert_eq!(status.state(), spc_agent::wallet::WalletState::Stopped);
    let snapshot = status.snapshot().unwrap();
    assert_eq!(snapshot.state, "stopped");
}

/// Phase 2: restart without destroying state. `last_request_id`
/// persists, a pending retry reuses the same id, the next registration
/// increments (never wraps back).
#[tokio::test]
async fn restart_persists_request_ids_and_reuses_pending() {
    let fixture = load_fixture();
    let dir = tempfile::tempdir().unwrap();
    let datadir = dir.path().join("datadir");
    let settings = fixture_settings(&fixture, dir.path(), &datadir);

    // First run: reserve id 1 (transport failure leaves it pending).
    {
        let role = WalletRole::start(settings.clone()).await.unwrap();
        let mut transport = MockTransport::new(vec![Err("front down".into())]);
        let a = role
            .with_registration(|r| r.attempt(&mut transport))
            .unwrap();
        assert_eq!(a.request_id, 1);
        assert_eq!(role.store().last_request_id().unwrap(), 1);
    }

    // "Restart": a fresh process handle on the same datadir.
    let role = WalletRole::start(settings).await.unwrap();
    assert_eq!(role.store().last_request_id().unwrap(), 1, "persists");
    // The retry after restart reuses the same request id.
    let mut transport = MockTransport::new(vec![
        Ok(RegistrationOutcome::Retryable {
            error_code: "registration-commit-unknown".into(),
        }),
        Ok(RegistrationOutcome::AlreadyApplied),
    ]);
    let a = role
        .with_registration(|r| r.attempt(&mut transport))
        .unwrap();
    assert_eq!(a.request_id, 1, "pending id reused across restart");
    assert!(!a.journaled, "retryable keeps the attempt open");
    let a = role
        .with_registration(|r| r.attempt(&mut transport))
        .unwrap();
    assert_eq!(a.request_id, 1);
    assert_eq!(a.outcome, RegistrationOutcome::AlreadyApplied);
    assert!(role
        .store()
        .is_registered(&role.registration_digest())
        .unwrap());

    // The next registration increments rather than wrapping back.
    let mut transport = MockTransport::new(vec![Ok(RegistrationOutcome::Applied)]);
    let a = role
        .with_registration(|r| r.attempt(&mut transport))
        .unwrap();
    assert_eq!(a.request_id, 2, "fresh attempt increments");
    assert_eq!(role.store().last_request_id().unwrap(), 2);
}

/// Phase 3: destroy the datadirs (not the seed fixture), recreate from
/// the same inputs, reset the scan cursor and re-claim. The same
/// `sp_id`, `M`, `U_i` set, normal-wallet key and VTXO records come
/// back; a re-poll does not duplicate claims.
#[tokio::test]
async fn recovery_after_datadir_destruction() {
    let fixture = load_fixture();
    let dir = tempfile::tempdir().unwrap();
    let datadir_a = dir.path().join("datadir-a");
    let datadir_b = dir.path().join("datadir-b");

    // Run 1 on datadir A.
    let captured = {
        let settings = fixture_settings(&fixture, dir.path(), &datadir_a);
        let role = WalletRole::start(settings).await.unwrap();
        let mut transport = MockTransport::new(vec![Ok(RegistrationOutcome::Applied)]);
        role.with_registration(|r| r.attempt(&mut transport))
            .unwrap();
        for block in &fixture.blocks {
            let a_send: PublicKey = block.a_send.parse().unwrap();
            role.derive_u(block.height, &a_send);
            assert!(role
                .ledger()
                .record(&observation(
                    block,
                    VtxoState::Spendable,
                    Expiry::At {
                        height: block.expiry_height,
                        source: ExpirySource::VtxoAncestry
                    }
                ))
                .unwrap()
                .is_inserted());
        }
        role.store().set_scan_height(fixture.tip_height).unwrap();
        role.status().snapshot().unwrap()
    };

    // Destroy the agent datadir entirely (the seed files survive).
    std::fs::remove_dir_all(&datadir_a).unwrap();
    assert!(!datadir_a.exists(), "agent state destroyed");

    // Run 2 from the same inputs, a fresh datadir: the scan cursor
    // resets and everything re-derives deterministically.
    let settings = fixture_settings(&fixture, dir.path(), &datadir_b);
    let role = WalletRole::start(settings).await.unwrap();
    let keys = role.keys();
    assert_eq!(hex::encode(keys.sp_id), fixture.sp_id_hex, "same sp_id");
    assert_eq!(
        keys.mailbox_public().to_string(),
        fixture.mailbox_pubkey,
        "same M"
    );
    assert_eq!(
        keys.normal_vtxo_public().to_string(),
        fixture.normal_vtxo_pubkey,
        "same normal wallet key"
    );
    // Scan cursor reset, balances unknown (null) until re-claimed.
    assert_eq!(role.store().last_scan_height().unwrap(), None);
    let fresh = role.status().snapshot().unwrap();
    assert_eq!(fresh.spendable_sats, None);
    assert!(
        !fresh.registered,
        "client journal was destroyed with the datadir"
    );

    // Re-register: the server journal answers for (sp_id, 1).
    let mut transport = MockTransport::new(vec![Ok(RegistrationOutcome::AlreadyApplied)]);
    let a = role
        .with_registration(|r| r.attempt(&mut transport))
        .unwrap();
    assert_eq!(a.request_id, 1, "fresh client counter restarts at 1");
    assert_eq!(a.outcome, RegistrationOutcome::AlreadyApplied);
    assert!(role
        .store()
        .is_registered(&role.registration_digest())
        .unwrap());

    // Re-claim the same observations.
    for block in &fixture.blocks {
        let a_send: PublicKey = block.a_send.parse().unwrap();
        let u = role.derive_u(block.height, &a_send);
        assert_eq!(u.to_string(), block.u_pubkey, "same U set");
        assert!(role
            .ledger()
            .record(&observation(
                block,
                VtxoState::Spendable,
                Expiry::At {
                    height: block.expiry_height,
                    source: ExpirySource::VtxoAncestry
                }
            ))
            .unwrap()
            .is_inserted());
    }
    // A re-poll of the same mailbox does not duplicate the claim.
    for block in &fixture.blocks {
        let write = role
            .ledger()
            .record(&observation(
                block,
                VtxoState::Spendable,
                Expiry::At {
                    height: block.expiry_height,
                    source: ExpirySource::VtxoAncestry,
                },
            ))
            .unwrap();
        assert!(write.is_already_present(), "no duplicate claim");
    }
    role.store().set_scan_height(fixture.tip_height).unwrap();

    // The recovered snapshot matches the captured one field for field.
    let recovered = role.status().snapshot().unwrap();
    assert_eq!(recovered.registered, captured.registered);
    assert_eq!(recovered.spendable_sats, captured.spendable_sats);
    assert_eq!(recovered.pending_sats, captured.pending_sats);
    assert_eq!(recovered.needs_refresh_sats, captured.needs_refresh_sats);
    assert_eq!(recovered.min_expiry_height, captured.min_expiry_height);
    assert_eq!(recovered.next_refresh_height, captured.next_refresh_height);
    assert_eq!(recovered.last_scan_height, captured.last_scan_height);
    assert_eq!(recovered.mailbox_mode, captured.mailbox_mode);

    // Evidence hygiene: no key material in the shareable status output.
    let body = role.http_status_json().await;
    let lowered = body.to_lowercase();
    assert!(!lowered.contains(&fixture.child_seed_hex.to_lowercase()));
    assert!(!lowered.contains("secret"));
}

/// The fixture is internally consistent: its values are exactly what
/// the deterministic derivation produces from the frozen master seed.
/// This guards the fixture against accidental edits.
#[test]
fn fixture_matches_its_own_master_seed() {
    let fixture = load_fixture();
    assert!(fixture.seed_source.contains("frozen-fixture"));
    let master: [u8; 64] = hex::decode(&fixture.master_seed_hex)
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(
        master,
        fixture_master(),
        "master seed is the deterministic one"
    );

    // Cross-check the whole provisioning set.
    let dir = tempfile::tempdir().unwrap();
    let (sources, sp_address) = provision_files(dir.path(), &master, Network::Regtest);
    assert_eq!(sp_address, fixture.sp_address);
    let keys = spc_agent::wallet::WalletKeys::load(&sources).unwrap();
    assert_eq!(hex::encode(keys.sp_id), fixture.sp_id_hex);
    assert_eq!(
        hex::encode(keys.wallet_fingerprint.to_bytes()),
        fixture.wallet_fingerprint_hex
    );
    assert_eq!(keys.mailbox_public().to_string(), fixture.mailbox_pubkey);
    assert_eq!(
        keys.normal_vtxo_public().to_string(),
        fixture.normal_vtxo_pubkey
    );
    assert_eq!(keys.b_ark_public().to_string(), fixture.b_ark_pubkey);

    // And the per-block U values (the receive-path derivation).
    for block in &fixture.blocks {
        let a_send: PublicKey = block.a_send.parse().unwrap();
        let ecdh =
            spc_crypto::SharedSecret::miner_side(block.height, &keys.b_scan, &a_send).unwrap();
        let tweaks = spc_crypto::tier_tweaks(&ecdh, 0).unwrap();
        let u = spc_crypto::ark_user_pubkey(&keys.b_ark_public(), &tweaks.t_ark).unwrap();
        assert_eq!(u.to_string(), block.u_pubkey);
        assert_eq!(
            hex::encode(fixture_vtxo_id(block.height, &a_send)),
            block.vtxo_id_hex
        );
    }
}
