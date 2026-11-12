//! Unit tests for the traffic wallet core (integration suites live in
//! `tests/`: independent signing verification, journal durability, the
//! lifecycle/reorg transitions, and the Fable review regressions).

use std::os::unix::fs::PermissionsExt;

use bitcoin::hashes::Hash as _;
use bitcoin::{OutPoint, ScriptBuf, Txid};

use crate::builder::{build_and_sign, DestinationSpec, FeePolicy, PaymentCategory, SignableUtxo};
use crate::journal::{FundingContext, FundingObservation, FundingSource, TrafficJournal, TxState};
use crate::keys::{
    Chain, TrafficKeyStore, TrafficPrivateKey, WalletRole, DEFAULT_SCRIPT_LOOKAHEAD,
};
use crate::wallet::{
    PaymentOutcome, PaymentRequest, ProvisionConfig, TrafficWalletSet, ValidatedAllocation,
};

/// A fixed run seed for reproducible fixtures.
pub const TEST_RUN_SEED: u64 = 0x5452_4146_4649_4321;

/// The fixed fixture run id (HMAC derivation domain; derivation v2).
pub const FIXTURE_RUN_ID: &str = "t1a-fixed-fixture";

/// The fixed fixture driver id (journal header binding).
pub const FIXTURE_DRIVER_ID: &str = "t1a-conformance";

/// A fixed synthetic funding outpoint (its txid is arbitrary fixture data).
pub fn fixture_outpoint(byte: u8) -> OutPoint {
    OutPoint::new(Txid::from_byte_array([byte; 32]), 0)
}

pub fn standard_test_config() -> ProvisionConfig {
    ProvisionConfig::standard(TEST_RUN_SEED, FIXTURE_RUN_ID, FIXTURE_DRIVER_ID)
}

fn conformance_set(dir: &std::path::Path) -> TrafficWalletSet {
    TrafficWalletSet::provision_for_conformance_tests(&standard_test_config(), dir)
        .expect("provision test wallet set")
}

/// Fund the bootstrap wallet with one synthetic fixture UTXO at the
/// role's next receive index and return the funded script.
pub fn fund_bootstrap(set: &TrafficWalletSet, outpoint: OutPoint, amount_sats: u64) -> ScriptBuf {
    let role = WalletRole::BootstrapFunding;
    let index = set.journal().next_receive_index(&role);
    let address = set.next_receive_address(&role).expect("receive address");
    let script = address.script_pubkey;
    set.observe_funding(&FundingObservation {
        outpoint,
        amount_sats,
        chain: Chain::Receive,
        index,
        owner_role: role,
        source: FundingSource::SyntheticFixture,
        context: FundingContext::Fixture,
    })
    .expect("observe fixture funding");
    script
}

fn payment_request(set: &TrafficWalletSet, command_id: &str, amount_sats: u64) -> PaymentRequest {
    let destination = set
        .next_receive_address(&WalletRole::NetworkWallet { index: 0 })
        .expect("destination address");
    PaymentRequest {
        command_id: command_id.to_string(),
        payer_role: WalletRole::BootstrapFunding,
        category: PaymentCategory::Ordinary,
        destinations: vec![DestinationSpec {
            script_pubkey: destination.script_pubkey,
            amount_sats,
            owner: Some(WalletRole::NetworkWallet { index: 0 }),
        }],
        fee_rate_sats_vb: 3,
    }
}

// ---------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------

#[test]
fn key_store_provision_reopen_roundtrip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let roles = standard_test_config().roles();
    let store = TrafficKeyStore::provision(
        TEST_RUN_SEED,
        FIXTURE_RUN_ID,
        bitcoin::Network::Regtest,
        &roles,
        dir.path(),
    )
    .expect("provision");
    assert_eq!(store.roles().len(), 13); // 1 bootstrap + 8 network + 4 user
    let reopened = TrafficKeyStore::reopen(
        TEST_RUN_SEED,
        FIXTURE_RUN_ID,
        bitcoin::Network::Regtest,
        dir.path(),
    )
    .expect("reopen");
    assert_eq!(reopened.roles(), store.roles());
    // Provisioning twice is refused.
    assert!(TrafficKeyStore::provision(
        TEST_RUN_SEED,
        FIXTURE_RUN_ID,
        bitcoin::Network::Regtest,
        &roles,
        dir.path()
    )
    .is_err());
    // A different run id derives different keys: fail closed, never
    // regenerated.
    assert!(TrafficKeyStore::reopen(
        TEST_RUN_SEED,
        "another-run-id",
        bitcoin::Network::Regtest,
        dir.path()
    )
    .is_err());
}

#[test]
fn key_store_reopen_wrong_seed_fails_closed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let roles = standard_test_config().roles();
    TrafficKeyStore::provision(
        TEST_RUN_SEED,
        FIXTURE_RUN_ID,
        bitcoin::Network::Regtest,
        &roles,
        dir.path(),
    )
    .expect("provision");
    let error = TrafficKeyStore::reopen(
        TEST_RUN_SEED + 1,
        FIXTURE_RUN_ID,
        bitcoin::Network::Regtest,
        dir.path(),
    )
    .expect_err("wrong seed must fail");
    assert!(matches!(
        error,
        crate::error::KeyError::FingerprintMismatch { .. }
    ));
    // The file was not regenerated: the same seed still reopens.
    assert!(TrafficKeyStore::reopen(
        TEST_RUN_SEED,
        FIXTURE_RUN_ID,
        bitcoin::Network::Regtest,
        dir.path()
    )
    .is_ok());
}

#[test]
fn key_store_corrupt_file_fails_closed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let roles = standard_test_config().roles();
    TrafficKeyStore::provision(
        TEST_RUN_SEED,
        FIXTURE_RUN_ID,
        bitcoin::Network::Regtest,
        &roles,
        dir.path(),
    )
    .expect("provision");
    let path = dir.path().join(TrafficKeyStore::FILE_NAME);
    let original = std::fs::read(&path).expect("read key file");
    // Truncate mid-file: corrupt.
    std::fs::write(&path, &original[..original.len() / 2]).expect("corrupt key file");
    let error = TrafficKeyStore::reopen(
        TEST_RUN_SEED,
        FIXTURE_RUN_ID,
        bitcoin::Network::Regtest,
        dir.path(),
    )
    .expect_err("corrupt file must fail");
    assert!(matches!(error, crate::error::KeyError::CorruptKeyFile(_)));
    // Tampered payload (same length) also fails: fingerprints must match
    // re-derivation.
    std::fs::write(&path, original).expect("restore key file");
    let mut tampered = std::fs::read(&path).expect("read");
    let last = tampered.len() - 1;
    tampered[last] = tampered[last].wrapping_add(1);
    std::fs::write(&path, tampered).expect("tamper");
    assert!(TrafficKeyStore::reopen(
        TEST_RUN_SEED,
        FIXTURE_RUN_ID,
        bitcoin::Network::Regtest,
        dir.path()
    )
    .is_err());
}

#[test]
fn key_file_is_mode_0600() {
    let dir = tempfile::tempdir().expect("tempdir");
    let roles = standard_test_config().roles();
    TrafficKeyStore::provision(
        TEST_RUN_SEED,
        FIXTURE_RUN_ID,
        bitcoin::Network::Regtest,
        &roles,
        dir.path(),
    )
    .expect("provision");
    let mode = std::fs::metadata(dir.path().join(TrafficKeyStore::FILE_NAME))
        .expect("stat key file")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn no_secret_material_in_debug() {
    let wallet = crate::keys::TrafficWallet::new(
        TEST_RUN_SEED,
        FIXTURE_RUN_ID,
        WalletRole::BootstrapFunding,
        bitcoin::Network::Regtest,
        DEFAULT_SCRIPT_LOOKAHEAD,
    )
    .expect("wallet");
    let master = TrafficPrivateKey::derive_from_run_seed(
        TEST_RUN_SEED,
        FIXTURE_RUN_ID,
        &WalletRole::BootstrapFunding,
        bitcoin::Network::Regtest,
    )
    .expect("master");
    let rendered = format!("{wallet:?} {master:?}");
    assert!(!rendered.contains("xprv"));
    assert!(!rendered.contains("tprv"));
    // The persisted encoding itself is, of course, secret.
    let encoded = master.master_xpriv_base58();
    assert!(encoded.starts_with("tprv"));
    assert!(!format!("{master:?}").contains(&encoded));
}

#[test]
fn foreign_script_fails_key_resolution() {
    let wallet = crate::keys::TrafficWallet::new(
        TEST_RUN_SEED,
        FIXTURE_RUN_ID,
        WalletRole::BootstrapFunding,
        bitcoin::Network::Regtest,
        DEFAULT_SCRIPT_LOOKAHEAD,
    )
    .expect("wallet");
    // A P2PKH script of another wallet (different seed).
    let other = crate::keys::TrafficWallet::new(
        TEST_RUN_SEED + 1,
        FIXTURE_RUN_ID,
        WalletRole::BootstrapFunding,
        bitcoin::Network::Regtest,
        DEFAULT_SCRIPT_LOOKAHEAD,
    )
    .expect("other wallet");
    let foreign = other
        .script_at(crate::keys::Chain::Receive, 0)
        .expect("script");
    assert!(matches!(
        wallet.key_for_script(&foreign.script_pubkey),
        Err(crate::error::KeyError::ScriptNotOwned { .. })
    ));
}

// ---------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------

#[test]
fn builder_rejects_dust_destination_and_dust_change() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    let script = fund_bootstrap(&set, fixture_outpoint(0x11), 1_000_000);
    let wallet = crate::keys::TrafficWallet::new(
        TEST_RUN_SEED,
        FIXTURE_RUN_ID,
        WalletRole::BootstrapFunding,
        bitcoin::Network::Regtest,
        DEFAULT_SCRIPT_LOOKAHEAD,
    )
    .expect("wallet");
    let utxo = SignableUtxo::resolve_indexed(
        &wallet,
        fixture_outpoint(0x11),
        Chain::Receive,
        0,
        &script,
        1_000_000,
    )
    .expect("resolve");
    let destination = set
        .next_receive_address(&WalletRole::NetworkWallet { index: 0 })
        .expect("destination");
    let secp = bitcoin::secp256k1::Secp256k1::new();

    // Dust destination.
    let error = build_and_sign(
        &secp,
        &wallet,
        std::slice::from_ref(&utxo),
        &[DestinationSpec {
            script_pubkey: destination.script_pubkey.clone(),
            amount_sats: 100,
            owner: None,
        }],
        3,
        &FeePolicy::default(),
        0,
        PaymentCategory::Ordinary,
    )
    .expect_err("dust destination");
    assert!(matches!(
        error,
        crate::error::BuilderError::DustDestination {
            amount: 100,
            dust: 546
        }
    ));

    // Dust change: destination sized so the remainder lands in (0, dust)
    // for any fee in the plausible range (about 600-900 sats here).
    let error = build_and_sign(
        &secp,
        &wallet,
        &[utxo],
        &[DestinationSpec {
            script_pubkey: destination.script_pubkey,
            // 1,000,000 - 999,000 = 1,000 minus the fee lands below 546.
            amount_sats: 999_000,
            owner: None,
        }],
        3,
        &FeePolicy::default(),
        0,
        PaymentCategory::Ordinary,
    )
    .expect_err("dust change");
    assert!(matches!(
        error,
        crate::error::BuilderError::DustChange { change, dust: 546 } if change > 0 && change < 546
    ));
}

#[test]
fn builder_rejects_fee_rate_outside_policy_and_insufficient_funds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    let script = fund_bootstrap(&set, fixture_outpoint(0x22), 1_000_000);
    let wallet = crate::keys::TrafficWallet::new(
        TEST_RUN_SEED,
        FIXTURE_RUN_ID,
        WalletRole::BootstrapFunding,
        bitcoin::Network::Regtest,
        DEFAULT_SCRIPT_LOOKAHEAD,
    )
    .expect("wallet");
    let utxo = SignableUtxo::resolve_indexed(
        &wallet,
        fixture_outpoint(0x22),
        Chain::Receive,
        0,
        &script,
        1_000_000,
    )
    .expect("resolve");
    let destination = set
        .next_receive_address(&WalletRole::NetworkWallet { index: 0 })
        .expect("destination");
    let secp = bitcoin::secp256k1::Secp256k1::new();
    let spec = DestinationSpec {
        script_pubkey: destination.script_pubkey.clone(),
        amount_sats: 50_000,
        owner: None,
    };

    let error = build_and_sign(
        &secp,
        &wallet,
        std::slice::from_ref(&utxo),
        std::slice::from_ref(&spec),
        100,
        &FeePolicy::default(),
        0,
        PaymentCategory::Ordinary,
    )
    .expect_err("rate above band");
    assert!(matches!(
        error,
        crate::error::BuilderError::FeeRateOutsidePolicy { rate: 100, .. }
    ));

    let error = build_and_sign(
        &secp,
        &wallet,
        std::slice::from_ref(&utxo),
        &[DestinationSpec {
            script_pubkey: destination.script_pubkey.clone(),
            amount_sats: 2_000_000,
            owner: None,
        }],
        3,
        &FeePolicy::default(),
        0,
        PaymentCategory::Ordinary,
    )
    .expect_err("insufficient");
    assert!(matches!(
        error,
        crate::error::BuilderError::InsufficientFunds { .. }
    ));

    // An affordable destination still builds.
    build_and_sign(
        &secp,
        &wallet,
        &[utxo],
        &[spec],
        3,
        &FeePolicy::default(),
        0,
        PaymentCategory::Ordinary,
    )
    .expect("affordable build");
}

#[test]
fn builder_is_deterministic_and_exact() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    let script = fund_bootstrap(&set, fixture_outpoint(0x33), 1_000_000);
    let wallet = crate::keys::TrafficWallet::new(
        TEST_RUN_SEED,
        FIXTURE_RUN_ID,
        WalletRole::BootstrapFunding,
        bitcoin::Network::Regtest,
        DEFAULT_SCRIPT_LOOKAHEAD,
    )
    .expect("wallet");
    let utxo = SignableUtxo::resolve_indexed(
        &wallet,
        fixture_outpoint(0x33),
        Chain::Receive,
        0,
        &script,
        1_000_000,
    )
    .expect("resolve");
    let destination = set
        .next_receive_address(&WalletRole::NetworkWallet { index: 0 })
        .expect("destination");
    let secp = bitcoin::secp256k1::Secp256k1::new();
    let spec = DestinationSpec {
        script_pubkey: destination.script_pubkey,
        amount_sats: 250_000,
        owner: Some(WalletRole::NetworkWallet { index: 0 }),
    };

    let first = build_and_sign(
        &secp,
        &wallet,
        std::slice::from_ref(&utxo),
        std::slice::from_ref(&spec),
        3,
        &FeePolicy::default(),
        0,
        PaymentCategory::Ordinary,
    )
    .expect("build");
    let second = build_and_sign(
        &secp,
        &wallet,
        &[utxo],
        &[spec],
        3,
        &FeePolicy::default(),
        0,
        PaymentCategory::Ordinary,
    )
    .expect("build again");

    // RFC 6979 determinism: byte-identical.
    assert_eq!(first.raw_bytes, second.raw_bytes);
    // Fee arithmetic on the final signed size: the band comparison uses
    // the measured vbytes with exact integer products, and the realized
    // rate is never below the requested rate (the estimate is an upper
    // bound of the signed size).
    assert_eq!(first.vbytes, first.tx.vsize() as u64);
    assert_eq!(first.weight_wu, first.tx.weight().to_wu());
    assert!(
        first.fee_sats >= 3 * first.vbytes,
        "realized rate below requested"
    );
    assert!(
        first.fee_sats <= 20 * first.vbytes,
        "realized rate above band"
    );
    // Exact destination and owned change.
    assert_eq!(first.tx.output[0].value.to_sat(), 250_000);
    let change = first.change.as_ref().expect("change output");
    assert_eq!(change.amount_sats + 250_000 + first.fee_sats, 1_000_000);
    assert!(wallet.owns_script(&change.script_pubkey));
    // Low-S signatures and SIGHASH_ALL: re-normalizing is a no-op and the
    // hashtype byte is 0x01. The raw scriptSig bytes are exactly two
    // pushes, with no outer length prefix (that lives in the tx
    // serialization, not the script):
    // <push sig_len> <DER sig> <hashtype> <push 33> <pubkey>
    for input in &first.tx.input {
        let bytes = input.script_sig.as_bytes();
        let sig_len = bytes[0] as usize;
        let hashtype = bytes[sig_len];
        assert_eq!(hashtype, 0x01, "SIGHASH_ALL hashtype byte");
        assert_eq!(bytes[sig_len + 1], 33, "compressed pubkey push");
        assert_eq!(bytes.len(), sig_len + 2 + 33, "two-push scriptSig shape");
        let der = bytes[1..sig_len].to_vec();
        let mut signature = bitcoin::secp256k1::ecdsa::Signature::from_der(&der)
            .expect("DER signature in scriptSig");
        signature.normalize_s();
        assert_eq!(
            signature.serialize_der().as_ref(),
            der.as_slice(),
            "signature must already be low-S"
        );
    }
}

// ---------------------------------------------------------------------
// Journal
// ---------------------------------------------------------------------

#[test]
fn journal_fixture_funding_refused_in_production_constructor() {
    let dir = tempfile::tempdir().expect("tempdir");
    let allocation = ProvisionConfig::validate_allocation(&[standard_test_config()])
        .expect("validated allocation");
    let set = TrafficWalletSet::provision(&allocation, FIXTURE_DRIVER_ID, dir.path())
        .expect("production set");
    let error = set
        .observe_funding(&FundingObservation {
            outpoint: fixture_outpoint(0x44),
            amount_sats: 1_000_000,
            chain: Chain::Receive,
            index: 0,
            owner_role: WalletRole::BootstrapFunding,
            source: FundingSource::SyntheticFixture,
            context: FundingContext::Fixture,
        })
        .expect_err("synthetic funding must be refused");
    assert!(matches!(
        error,
        crate::error::Error::Journal(crate::error::JournalError::FixtureFundingRefused)
    ));
}

#[test]
fn journal_duplicate_funding_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    fund_bootstrap(&set, fixture_outpoint(0x55), 1_000_000);
    let error = set
        .observe_funding(&FundingObservation {
            outpoint: fixture_outpoint(0x55),
            amount_sats: 1_000_000,
            chain: Chain::Receive,
            index: 1,
            owner_role: WalletRole::BootstrapFunding,
            source: FundingSource::SyntheticFixture,
            context: FundingContext::Fixture,
        })
        .expect_err("duplicate funding");
    assert!(matches!(
        error,
        crate::error::Error::Journal(crate::error::JournalError::DuplicateFunding { .. })
    ));
}

#[test]
fn journal_unknown_retry_send_cap_enforced() {
    // H3: a cap of N admits at most N retry sends (N+1 total sends);
    // recording unknown outcomes is never capped.
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    let caps = set.journal().caps();
    fund_bootstrap(&set, fixture_outpoint(0x66), 1_000_000);
    set.sign_payment(&payment_request(&set, "cap-cmd", 250_000))
        .expect("sign");
    // Original send.
    set.note_broadcast_sent("cap-cmd").expect("broadcast");
    // Unknown outcomes and retry sends, up to the cap.
    for _ in 0..caps.unknown_retry_sends {
        set.note_unknown("cap-cmd", "transport timeout")
            .expect("recording unknown outcomes is never capped");
        set.note_broadcast_sent("cap-cmd")
            .expect("retry send admitted below the cap");
    }
    // Total sends = 1 original + N retries.
    assert_eq!(
        set.journal().next_send_sequence() - 1,
        1 + caps.unknown_retry_sends as u64
    );
    // One more unknown outcome is still recordable (evidence only)...
    set.note_unknown("cap-cmd", "transport timeout")
        .expect("recording unknown outcomes is never capped");
    // ...but the next retry send is refused.
    let error = set
        .note_broadcast_sent("cap-cmd")
        .expect_err("retry-send cap exceeded");
    assert!(matches!(
        error,
        crate::error::Error::Journal(crate::error::JournalError::UnknownRetryCapExceeded { .. })
    ));
}

#[test]
fn journal_barrier_refuses_broadcasts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    fund_bootstrap(&set, fixture_outpoint(0x77), 1_000_000);
    fund_bootstrap(&set, fixture_outpoint(0x78), 1_000_000);
    set.sign_payment(&payment_request(&set, "bar-1", 100_000))
        .expect("sign 1");
    let sequence = set.note_broadcast_sent("bar-1").expect("broadcast 1");
    assert_eq!(sequence, 1);

    set.pause_sends("pause-cmd", 42_000).expect("barrier");
    assert!(set.journal().barrier_active());
    set.sign_payment(&payment_request(&set, "bar-2", 100_000))
        .expect("sign 2 (signing is allowed while paused)");
    let error = set
        .note_broadcast_sent("bar-2")
        .expect_err("broadcast must be refused after the barrier");
    assert!(matches!(
        error,
        crate::error::Error::Journal(crate::error::JournalError::IllegalTransition { .. })
    ));

    set.resume_sends("pause-cmd").expect("resume");
    assert!(!set.journal().barrier_active());
    assert_eq!(set.note_broadcast_sent("bar-2").expect("broadcast 2"), 2);
}

#[test]
fn journal_torn_final_line_is_tolerated_and_truncated() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = standard_test_config();
    {
        let set = TrafficWalletSet::provision_for_conformance_tests(&config, dir.path())
            .expect("provision");
        fund_bootstrap(&set, fixture_outpoint(0x88), 1_000_000);
        // The set is dropped here: the exclusive flock is released, so
        // the reopen below is legal (a live second owner would fail with
        // HeldByAnotherOwner, which the H4 regressions cover).
    }
    let path = TrafficJournal::path_in(dir.path());
    // Simulate a crash mid-write: append a torn fragment.
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open journal");
    use std::io::Write as _;
    file.write_all(br#"{"record":"funding_observe"#)
        .expect("torn write");
    drop(file);

    let reopened = TrafficJournal::reopen_for_conformance_tests(
        dir.path(),
        &config.caps,
        ValidatedAllocation::for_conformance_tests(&config)
            .expect("conformance allocation")
            .digest(),
        &config.driver_id,
    )
    .expect("torn tail is tolerated");
    assert_eq!(
        reopened.balance_sats(&WalletRole::BootstrapFunding),
        1_000_000
    );
    drop(reopened);
    // The torn fragment was truncated: the next record appends cleanly
    // (through a full set reopen) and the journal reopens again.
    let set = TrafficWalletSet::reopen_for_conformance_tests(&config, dir.path())
        .expect("reopen set after truncation");
    let index = set
        .journal()
        .next_receive_index(&WalletRole::BootstrapFunding);
    set.observe_funding(&FundingObservation {
        outpoint: fixture_outpoint(0x89),
        amount_sats: 500_000,
        chain: Chain::Receive,
        index,
        owner_role: WalletRole::BootstrapFunding,
        source: FundingSource::SyntheticFixture,
        context: FundingContext::Fixture,
    })
    .expect("clean append after truncation");
    drop(set);
    assert!(TrafficJournal::reopen_for_conformance_tests(
        dir.path(),
        &config.caps,
        ValidatedAllocation::for_conformance_tests(&config)
            .expect("conformance allocation")
            .digest(),
        &config.driver_id
    )
    .is_ok());
}

#[test]
fn journal_mode_is_0600() {
    let dir = tempfile::tempdir().expect("tempdir");
    let _set = conformance_set(dir.path());
    let path = TrafficJournal::path_in(dir.path());
    let mode = std::fs::metadata(&path)
        .expect("stat journal")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn journal_header_binds_run_identity() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = standard_test_config();
    {
        let set =
            TrafficWalletSet::provision_for_conformance_tests(&config, dir.path()).expect("set");
        let header = set.journal().header().expect("header");
        assert_eq!(header.format_magic, crate::journal::FORMAT_MAGIC);
        assert_eq!(
            header.history_version,
            crate::journal::SUPPORTED_HISTORY_VERSION
        );
        assert_eq!(header.protocol, crate::journal::PROTOCOL);
        assert_eq!(header.network, "regtest");
        assert_eq!(header.run_id, FIXTURE_RUN_ID);
        assert_eq!(header.driver_id, FIXTURE_DRIVER_ID);
        assert_eq!(header.key_mode, crate::keys::KEY_MODE);
        assert_eq!(header.derivation_version, crate::keys::DERIVATION_VERSION);
        assert_eq!(header.caps, config.caps);
    }
    // A reopen with different expected caps fails closed.
    let mut other = config.clone();
    other.caps.unknown_retry_sends = 1;
    let error = TrafficWalletSet::reopen_for_conformance_tests(&other, dir.path())
        .expect_err("caps mismatch must fail");
    assert!(matches!(
        error,
        crate::error::Error::Journal(crate::error::JournalError::CapsMismatch { .. })
    ));
    // A reopen under a different run id fails closed.
    let mut other = config.clone();
    other.run_id = "a-different-run".to_string();
    let error = TrafficWalletSet::reopen_for_conformance_tests(&other, dir.path())
        .expect_err("run id mismatch must fail");
    assert!(matches!(
        error,
        crate::error::Error::Key(_) | crate::error::Error::Journal(_)
    ));
}

#[test]
fn journal_conflict_and_replay_semantics() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    fund_bootstrap(&set, fixture_outpoint(0x99), 1_000_000);

    // Same id + same payload: stored outcome with identical bytes.
    let first = set
        .sign_payment(&payment_request(&set, "dup-cmd", 250_000))
        .expect("first");
    let again = set
        .sign_payment(&payment_request(&set, "dup-cmd", 250_000))
        .expect("replay");
    let (first_view, replay_view) = match (first, again) {
        (PaymentOutcome::Signed(a), PaymentOutcome::ReplayedStored(b)) => (a, b),
        _ => panic!("expected signed then replayed-stored"),
    };
    assert_eq!(first_view.raw_hex, replay_view.raw_hex);
    assert_eq!(first_view.txid, replay_view.txid);

    // Same id + different payload: journaled conflict, hard error.
    let error = set
        .sign_payment(&payment_request(&set, "dup-cmd", 100_000))
        .expect_err("conflict");
    assert!(matches!(
        error,
        crate::error::Error::Journal(crate::error::JournalError::CommandConflict { .. })
    ));

    // The outpoints of the signed command are still locked for it.
    assert_eq!(set.balance_sats(&WalletRole::BootstrapFunding), 0);
}

#[test]
fn journal_reject_releases_and_conflict_marks_spent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    fund_bootstrap(&set, fixture_outpoint(0xaa), 1_000_000);
    let outcome = set
        .sign_payment(&payment_request(&set, "rel-cmd", 250_000))
        .expect("sign");
    let view = match outcome {
        PaymentOutcome::Signed(view) => view,
        PaymentOutcome::ReplayedStored(_) => panic!("fresh"),
    };
    assert_eq!(set.balance_sats(&WalletRole::BootstrapFunding), 0);
    set.note_broadcast_sent("rel-cmd").expect("broadcast");
    set.note_rejected("rel-cmd", "bad-txns-inputs-missingorspent")
        .expect("rejected");
    // An explicit rejection of a never-uncertain payment releases the
    // reservation (H1: an ever-unknown/ever-accepted/resubmitted payment
    // would degrade to possibly-spent instead).
    assert_eq!(set.balance_sats(&WalletRole::BootstrapFunding), 1_000_000);
    assert_eq!(set.journal().tx_state(&view.txid), Some(TxState::Rejected));
}

#[test]
fn unknown_never_releases_inputs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    fund_bootstrap(&set, fixture_outpoint(0xbb), 1_000_000);
    set.sign_payment(&payment_request(&set, "unk-cmd", 250_000))
        .expect("sign");
    set.note_broadcast_sent("unk-cmd").expect("broadcast");
    set.note_unknown("unk-cmd", "timeout").expect("unknown");
    // Still locked.
    assert_eq!(set.balance_sats(&WalletRole::BootstrapFunding), 0);
    // And a second payment cannot reselect those inputs.
    let error = set
        .sign_payment(&payment_request(&set, "unk-cmd-2", 250_000))
        .expect_err("no other funding");
    assert!(matches!(
        error,
        crate::error::Error::Builder(crate::error::BuilderError::InsufficientFunds { .. })
    ));
}

// ---------------------------------------------------------------------
// Checked destinations (explicit address, wrong network/script class
// rejected before any journal record or signature)
// ---------------------------------------------------------------------

/// A P2PKH address of a set wallet, re-encoded for an arbitrary network.
fn wallet_address_on_network(
    set: &TrafficWalletSet,
    role: &WalletRole,
    network: bitcoin::Network,
) -> bitcoin::Address {
    let address = set.next_receive_address(role).expect("receive address");
    bitcoin::Address::p2pkh(bitcoin::PubkeyHash::from(&address.public_key), network)
}

#[test]
fn checked_destination_rejects_wrong_network_before_any_record() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    let seq_before = set.journal().last_seq();
    let mainnet = wallet_address_on_network(
        &set,
        &WalletRole::NetworkWallet { index: 0 },
        bitcoin::Network::Bitcoin,
    );
    let error = set
        .checked_destination(mainnet.as_unchecked().clone(), 50_000)
        .expect_err("wrong network must fail closed");
    assert!(matches!(
        error,
        crate::error::Error::Wallet(crate::error::WalletError::DestinationNetworkWrong { .. })
    ));
    // Nothing was journaled.
    assert_eq!(set.journal().last_seq(), seq_before);
}

#[test]
fn checked_destination_rejects_unsupported_script_before_any_record() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    let seq_before = set.journal().last_seq();
    // P2WSH is outside the bounded T1-DEST output policy.
    let witness = bitcoin::Address::p2wsh(&ScriptBuf::new(), bitcoin::Network::Regtest)
        .as_unchecked()
        .clone();
    let error = set
        .checked_destination(witness, 50_000)
        .expect_err("unsupported script class must fail closed");
    assert!(matches!(
        error,
        crate::error::Error::Builder(crate::error::BuilderError::UnsupportedDestination { .. })
    ));
    assert_eq!(set.journal().last_seq(), seq_before);
}

#[test]
fn checked_destination_tags_owned_wallets_and_external() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    // A set wallet's regtest address is owner-tagged.
    let owned = wallet_address_on_network(
        &set,
        &WalletRole::NetworkWallet { index: 1 },
        bitcoin::Network::Regtest,
    );
    let destination = set
        .checked_destination(owned.as_unchecked().clone(), 50_000)
        .expect("owned");
    assert_eq!(
        destination.owner,
        Some(WalletRole::NetworkWallet { index: 1 })
    );
    // An external regtest address (a key outside this set) is not.
    let secp = bitcoin::secp256k1::Secp256k1::new();
    let secret = bitcoin::secp256k1::SecretKey::from_slice(&[7u8; 32]).expect("external key");
    let external_public = bitcoin::PublicKey::from(bitcoin::secp256k1::PublicKey::from_secret_key(
        &secp, &secret,
    ));
    let external = bitcoin::Address::p2pkh(
        bitcoin::PubkeyHash::from(&external_public),
        bitcoin::Network::Regtest,
    );
    let destination = set
        .checked_destination(external.as_unchecked().clone(), 50_000)
        .expect("external");
    assert_eq!(destination.owner, None);
}

#[test]
fn request_with_unsupported_destination_rejected_before_journal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let set = conformance_set(dir.path());
    fund_bootstrap(&set, fixture_outpoint(0xcc), 1_000_000);
    let seq_before = set.journal().last_seq();
    let request = PaymentRequest {
        command_id: "bad-class-cmd".to_string(),
        payer_role: WalletRole::BootstrapFunding,
        category: PaymentCategory::Ordinary,
        destinations: vec![DestinationSpec {
            script_pubkey: ScriptBuf::from_bytes(vec![0x6a]), // OP_RETURN
            amount_sats: 50_000,
            owner: None,
        }],
        fee_rate_sats_vb: 3,
    };
    let error = set.sign_payment(&request).expect_err("must fail closed");
    assert!(matches!(
        error,
        crate::error::Error::Builder(crate::error::BuilderError::UnsupportedDestination { .. })
    ));
    // No journal record exists for the rejected command at all: the
    // rejection happened before `begin_command`.
    assert!(set.journal().command("bad-class-cmd").is_none());
    assert_eq!(set.journal().last_seq(), seq_before);
    // The funding is untouched and still spendable by a valid request.
    assert_eq!(set.balance_sats(&WalletRole::BootstrapFunding), 1_000_000);
}
