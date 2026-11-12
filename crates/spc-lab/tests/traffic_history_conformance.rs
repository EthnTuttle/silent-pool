//! History-header conformance between the T0 contract and the real T1a
//! core (`crates/spc-lab-txd`, Fable-approved traffic plan T1a).
//!
//! The T0 `TrafficHistoryHeader` claims to BE the exact first line the
//! T1a core writes to `<run_dir>/private/traffic/history.jsonl`. These
//! tests prove that claim against the real writer - not a hand-built
//! replica:
//!
//! - a real journal is provisioned with the frozen conformance fixture
//!   (`ProvisionConfig::standard` with the fixed seed / run id / driver
//!   id), its actual first line is parsed through the strict T0
//!   consumer, re-serialized and compared **byte for byte**;
//! - every typed fact is compared against the core's own `HeaderFacts`
//!   view, the `ProvisionConfig::validate_allocation` digest and the
//!   `TrafficPrivateKey::run_seed_fingerprint` computation;
//! - the T0 constants are asserted equal to the core constants they
//!   mirror, so the two crates cannot drift apart silently;
//! - a fully hard-coded golden line pins the deterministic fields (the
//!   digests, the ids, the caps and the field order);
//! - every mutation of the real line rejects fail-closed with the
//!   documented T0 error code;
//! - the history-3 core and T0 both reject unknown record fields.
//!
//! The conformance-only dev-dependency on `spc-lab-txd` never couples a
//! production `spc-lab` path to the signer or the journal
//! (`crates/spc-lab/Cargo.toml`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{json, Value};

use spc_lab::traffic::protocol::{
    TrafficHistoryHeader, TRAFFIC_HISTORY_NETWORK, TRAFFIC_HISTORY_VERSION,
    TRAFFIC_KEY_DERIVATION_VERSION, TRAFFIC_KEY_MODE, TX_JOURNAL_FORMAT_MAGIC,
    TX_JOURNAL_PROTOCOL_STRING,
};
use spc_lab_txd::journal::JournalCaps;
use spc_lab_txd::keys::TrafficPrivateKey;
use spc_lab_txd::wallet::{ProvisionConfig, TrafficWalletSet};

// The frozen conformance fixture (mirrors `spc-lab-txd`'s own test
// constants, `tests/common/mod.rs`).

/// The fixed run seed of every conformance fixture (reproducible keys).
const TEST_RUN_SEED: u64 = 0x5452_4146_4649_4321;

/// The fixed fixture run id (HMAC derivation domain; derivation v2).
const FIXTURE_RUN_ID: &str = "t1a-fixed-fixture";

/// The fixed fixture driver id (journal header binding).
const FIXTURE_DRIVER_ID: &str = "t1a-conformance";

/// The canonical allocation digest of the standard single-actor fixture
/// config (SHA-256 over the sorted-key JSON the core's
/// `ProvisionConfig::validate_allocation` hashes). Deterministic for the
/// fixed seed / run id / driver id / standard population; cross-checked
/// against the live core computation below.
const GOLDEN_ALLOCATION_DIGEST: &str =
    "f804614a49714ccd348854b3b6d4261d518e011a3a8ce1d55255dbb123bf4978";

/// The run-seed fingerprint of the fixture seed (SHA-256 of the 8
/// big-endian seed bytes). Deterministic; cross-checked against the
/// live `TrafficPrivateKey::run_seed_fingerprint` below.
const GOLDEN_RUN_SEED_FINGERPRINT: &str =
    "13e2ee5f45f3cb4fa7018afb0e6a11d7286288a93112d484728008ea712f96da";

/// The fixed creation timestamp of the hard-coded golden line.
const GOLDEN_CREATED_UNIX_MS: u64 = 1_762_572_800_000;

/// The exact 14 fields of the history header, in the core's declaration
/// order (the field order is part of the byte-identity contract).
const HEADER_FIELDS: [&str; 14] = [
    "record",
    "seq",
    "format_magic",
    "history_version",
    "protocol",
    "network",
    "key_mode",
    "derivation_version",
    "run_id",
    "driver_id",
    "run_seed_fingerprint",
    "allocation_digest",
    "caps",
    "created_unix_ms",
];

/// The hard-coded golden line: the exact bytes the core writes for the
/// frozen fixture, with the deterministic fields pinned (only
/// `created_unix_ms` is wall-clock in a real run; here it is fixed).
const PRESERVED_V2_LINE: &str = concat!(
    r#"{"record":"header","seq":0,"format_magic":"SPC-LAB-TRAFFIC-JOURNAL","#,
    r#""history_version":2,"protocol":"spc-lab-tx-journal/2","network":"regtest","#,
    r#""key_mode":"lab_deterministic_run_seed_v1","derivation_version":2,"#,
    r#""run_id":"t1a-fixed-fixture","driver_id":"t1a-conformance","#,
    r#""run_seed_fingerprint":"13e2ee5f45f3cb4fa7018afb0e6a11d7286288a93112d484728008ea712f96da","#,
    r#""allocation_digest":"f804614a49714ccd348854b3b6d4261d518e011a3a8ce1d55255dbb123bf4978","#,
    r#""caps":{"unknown_retry_sends":3,"reorg_resubmits":3},"created_unix_ms":1762572800000}"#
);

// Format-only activation: identical key/seed/allocation bytes to the
// preserved approved v2 golden, explicitly versioned rather than re-keyed.
const GOLDEN_LINE: &str = concat!(
    r#"{"record":"header","seq":0,"format_magic":"SPC-LAB-TRAFFIC-JOURNAL","#,
    r#""history_version":3,"protocol":"spc-lab-tx-journal/3","network":"regtest","#,
    r#""key_mode":"lab_deterministic_run_seed_v1","derivation_version":2,"#,
    r#""run_id":"t1a-fixed-fixture","driver_id":"t1a-conformance","#,
    r#""run_seed_fingerprint":"13e2ee5f45f3cb4fa7018afb0e6a11d7286288a93112d484728008ea712f96da","#,
    r#""allocation_digest":"f804614a49714ccd348854b3b6d4261d518e011a3a8ce1d55255dbb123bf4978","#,
    r#""caps":{"unknown_retry_sends":3,"reorg_resubmits":3},"created_unix_ms":1762572800000}"#
);

/// A bounded, self-cleaning fixture run directory (the only files are
/// the conformance key store and the one-line journal). Dropping it
/// removes the tree; keep it in scope for the lifetime of the fixture.
struct TestDir(PathBuf);

impl TestDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "spc-lab-traffic-history-conformance-{tag}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("create the conformance run dir");
        Self(path)
    }

    fn journal_path(&self) -> PathBuf {
        self.0.join("private").join("traffic").join("history.jsonl")
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        // Bounded cleanup: the fixture never outlives the test.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A provisioned conformance fixture: the real T1a wallet set plus its
/// REAL first journal line. The `TestDir` guard stays in the fixture so
/// the run directory lives as long as the assertions need it and is
/// removed afterwards.
struct Fixture {
    line: String,
    set: TrafficWalletSet,
    _dir: TestDir,
}

/// Provision the frozen conformance fixture and read the REAL first
/// journal line the T1a core wrote.
fn real_fixture(tag: &str) -> Fixture {
    let dir = TestDir::new(tag);
    let config = ProvisionConfig::standard(TEST_RUN_SEED, FIXTURE_RUN_ID, FIXTURE_DRIVER_ID);
    let set = TrafficWalletSet::provision_for_conformance_tests(&config, &dir.0).expect(
        "the T1a core provisions the frozen conformance fixture (its own suites do the same)",
    );
    let text = std::fs::read_to_string(dir.journal_path())
        .expect("the provisioned journal file exists at private/traffic/history.jsonl");
    let line = text
        .split('\n')
        .next()
        .unwrap_or("")
        .trim_end_matches('\r')
        .to_string();
    assert!(!line.is_empty(), "the journal's first line is the header");
    Fixture {
        line,
        set,
        _dir: dir,
    }
}

// ---------------------------------------------------------------------------
// Constants: the two crates cannot drift apart
// ---------------------------------------------------------------------------

#[test]
fn t0_constants_equal_the_t1a_core_constants() {
    use spc_lab_txd::journal as core_journal;
    use spc_lab_txd::keys as core_keys;
    assert_eq!(
        TRAFFIC_HISTORY_VERSION,
        core_journal::SUPPORTED_HISTORY_VERSION,
        "the T0 history version must equal the core's supported version"
    );
    assert_eq!(
        TX_JOURNAL_FORMAT_MAGIC,
        core_journal::FORMAT_MAGIC,
        "the T0 format magic must equal the core's magic"
    );
    assert_eq!(
        TX_JOURNAL_PROTOCOL_STRING,
        core_journal::PROTOCOL,
        "the T0 journal protocol string must equal the core's protocol"
    );
    assert_eq!(
        TRAFFIC_KEY_MODE,
        core_keys::KEY_MODE,
        "the T0 key mode must equal the core's key mode"
    );
    assert_eq!(
        TRAFFIC_KEY_DERIVATION_VERSION,
        core_keys::DERIVATION_VERSION,
        "the T0 derivation version must equal the core's derivation version"
    );
    // The lab network is the only network the core's derivation accepts.
    assert_eq!(TRAFFIC_HISTORY_NETWORK, "regtest");
    // The standard fixture caps are the core defaults (3/3), inside the
    // T0 cap bounds.
    let caps = JournalCaps::default();
    let header_caps = spc_lab::traffic::TrafficHistoryCaps {
        unknown_retry_sends: caps.unknown_retry_sends,
        reorg_resubmits: caps.reorg_resubmits,
    };
    header_caps
        .validate()
        .expect("the core default caps satisfy the T0 cap bounds");
}

// ---------------------------------------------------------------------------
// The real first line IS the T0 contract
// ---------------------------------------------------------------------------

#[test]
fn real_t1a_first_line_roundtrips_through_the_t0_contract() {
    let fixture = real_fixture("roundtrip");
    let line = &fixture.line;

    // The strict T0 consumer parses and validates the real line.
    let header = TrafficHistoryHeader::from_journal_line(line)
        .expect("the real core first line satisfies the strict T0 history-3 header contract");

    // Byte identity: re-serializing the T0 mirror reproduces the core's
    // line exactly (same tag, same field order, transparent ids).
    assert_eq!(
        serde_json::to_string(&header).unwrap(),
        *line,
        "re-serialization must be byte-identical to the core's line"
    );

    // Exactly the 14 contract fields, no more, no less. (The Value parse
    // sorts keys - a BTreeMap - so this checks the SET; the field ORDER
    // is proven by the byte-identity assertion above.)
    let value: Value = serde_json::from_str(line).unwrap();
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    let mut expected: Vec<&str> = HEADER_FIELDS.to_vec();
    expected.sort_unstable();
    assert_eq!(keys, expected, "the field set is the contract");

    // Typed facts agree with the core's own HeaderFacts view.
    let facts = fixture
        .set
        .journal()
        .header()
        .expect("the core replays its header");
    assert_eq!(header.record, "header");
    assert_eq!(header.seq, 0);
    assert_eq!(header.format_magic, facts.format_magic);
    assert_eq!(header.history_version, facts.history_version);
    assert_eq!(header.protocol, facts.protocol);
    assert_eq!(header.network, facts.network);
    assert_eq!(header.key_mode, facts.key_mode);
    assert_eq!(header.derivation_version, facts.derivation_version);
    assert_eq!(header.run_id.as_str(), facts.run_id);
    assert_eq!(header.driver_id.as_str(), facts.driver_id);
    assert_eq!(header.run_seed_fingerprint, facts.run_seed_fingerprint);
    assert_eq!(header.allocation_digest, facts.allocation_digest);
    assert_eq!(
        header.caps.unknown_retry_sends, facts.caps.unknown_retry_sends,
        "caps: unknown_retry_sends"
    );
    assert_eq!(
        header.caps.reorg_resubmits, facts.caps.reorg_resubmits,
        "caps: reorg_resubmits"
    );
    assert_eq!(header.created_unix_ms, facts.created_unix_ms);

    // The allocation digest is the real validate_allocation digest of
    // the fixture config (no second hash, no subset derivation).
    let config = ProvisionConfig::standard(TEST_RUN_SEED, FIXTURE_RUN_ID, FIXTURE_DRIVER_ID);
    let allocation = ProvisionConfig::validate_allocation(&[config])
        .expect("the single standard config is a valid whole allocation");
    assert_eq!(header.allocation_digest, allocation.digest());

    // The run-seed fingerprint is the core's own computation.
    assert_eq!(
        header.run_seed_fingerprint,
        TrafficPrivateKey::run_seed_fingerprint(TEST_RUN_SEED)
    );
}

// ---------------------------------------------------------------------------
// The golden line
// ---------------------------------------------------------------------------

#[test]
fn golden_known_line_is_accepted_and_reproduced_byte_for_byte() {
    let header = TrafficHistoryHeader::from_journal_line(GOLDEN_LINE)
        .expect("the golden line satisfies the strict T0 history-3 header contract");
    assert_eq!(header.run_id.as_str(), FIXTURE_RUN_ID);
    assert_eq!(header.driver_id.as_str(), FIXTURE_DRIVER_ID);
    assert_eq!(header.network, "regtest");
    assert_eq!(header.caps.unknown_retry_sends, 3);
    assert_eq!(header.caps.reorg_resubmits, 3);
    assert_eq!(header.created_unix_ms, GOLDEN_CREATED_UNIX_MS);
    assert_eq!(header.allocation_digest, GOLDEN_ALLOCATION_DIGEST);
    assert_eq!(header.run_seed_fingerprint, GOLDEN_RUN_SEED_FINGERPRINT);

    // Byte-for-byte reproduction: parse -> serialize is the identity,
    // and a second roundtrip loses no field.
    assert_eq!(serde_json::to_string(&header).unwrap(), GOLDEN_LINE);
    let again = TrafficHistoryHeader::from_journal_line(GOLDEN_LINE).unwrap();
    assert_eq!(again, header);

    // The golden digests are the LIVE core computations for the frozen
    // fixture (the hard-coded values above can never silently drift).
    let config = ProvisionConfig::standard(TEST_RUN_SEED, FIXTURE_RUN_ID, FIXTURE_DRIVER_ID);
    let allocation = ProvisionConfig::validate_allocation(&[config]).unwrap();
    assert_eq!(GOLDEN_ALLOCATION_DIGEST, allocation.digest());
    assert_eq!(
        GOLDEN_RUN_SEED_FINGERPRINT,
        TrafficPrivateKey::run_seed_fingerprint(TEST_RUN_SEED)
    );
}

#[test]
fn preserved_v2_golden_is_refused_without_mutating_real_writer_files() {
    let fixture = real_fixture("v2-refusal");
    let Fixture { set, _dir: dir, .. } = fixture;
    let key_path = dir.0.join("private/traffic-wallets.json");
    let keys = std::fs::read(&key_path).unwrap();
    drop(set);
    // These are the exact preserved approved-v2 golden bytes above, not a
    // freshly generated key/header fixture mislabeled as historical output.
    let mut old = PRESERVED_V2_LINE.as_bytes().to_vec();
    old.extend_from_slice(b"\n{\"record\":");
    std::fs::write(dir.journal_path(), &old).unwrap();
    assert!(TrafficHistoryHeader::from_journal_line(PRESERVED_V2_LINE).is_err());
    let config = ProvisionConfig::standard(TEST_RUN_SEED, FIXTURE_RUN_ID, FIXTURE_DRIVER_ID);
    assert!(TrafficWalletSet::reopen_for_conformance_tests(&config, &dir.0).is_err());
    assert_eq!(std::fs::read(dir.journal_path()).unwrap(), old);
    assert_eq!(std::fs::read(key_path).unwrap(), keys);
    let mut prior: Value = serde_json::from_str(PRESERVED_V2_LINE).unwrap();
    prior["history_version"] = json!(3);
    prior["protocol"] = json!("spc-lab-tx-journal/3");
    assert_eq!(prior, serde_json::from_str::<Value>(GOLDEN_LINE).unwrap());
}

// ---------------------------------------------------------------------------
// Fail-closed mutations of the real line
// ---------------------------------------------------------------------------

#[test]
fn real_line_mutations_are_rejected_fail_closed() {
    let fixture = real_fixture("mutations");
    let base: Value = serde_json::from_str(&fixture.line).unwrap();

    let rejects = |label: &str, mutate: &dyn Fn(&mut Value), code: &str| {
        let mut value = base.clone();
        mutate(&mut value);
        let err = match TrafficHistoryHeader::from_journal_line(&value.to_string()) {
            Err(err) => err,
            Ok(_) => panic!("{label}: expected a rejection"),
        };
        assert_eq!(
            err.code(),
            code,
            "{label}: unexpected detail: {}",
            err.detail()
        );
    };

    // Record identity: the first line is the history-3 header, at seq 0, with
    // the journal file magic.
    rejects(
        "non-header record kind",
        &|v| v["record"] = json!("session_started"),
        "traffic_history_record_invalid",
    );
    rejects(
        "nonzero seq",
        &|v| v["seq"] = json!(1),
        "traffic_history_record_invalid",
    );
    rejects(
        "wrong format magic",
        &|v| v["format_magic"] = json!("SOME-OTHER-FILE"),
        "traffic_history_record_invalid",
    );

    // History version: the retired v1 shape and any later version are
    // refused, never migrated.
    rejects(
        "history version 1",
        &|v| v["history_version"] = json!(1),
        "traffic_protocol_unsupported",
    );
    rejects(
        "history version 2",
        &|v| v["history_version"] = json!(2),
        "traffic_protocol_unsupported",
    );
    // The CONTROL protocol string never passes for the journal protocol.
    rejects(
        "control protocol in the journal",
        &|v| v["protocol"] = json!("spc-lab-tx-control/v1"),
        "traffic_protocol_unsupported",
    );

    // Declared binding: regtest only, the lab key mode, derivation v2.
    rejects(
        "mainnet network",
        &|v| v["network"] = json!("mainnet"),
        "traffic_history_network_invalid",
    );
    rejects(
        "foreign key mode",
        &|v| v["key_mode"] = json!("hardware_wallet_v9"),
        "traffic_key_mode_unsupported",
    );
    rejects(
        "derivation version 1",
        &|v| v["derivation_version"] = json!(1),
        "traffic_key_mode_unsupported",
    );

    // Typed ids: the shared charset rules (at most 128 bytes, ASCII, no
    // control characters, no spaces).
    rejects(
        "invalid run id",
        &|v| v["run_id"] = json!("bad run"),
        "traffic_run_id_invalid",
    );
    rejects(
        "invalid driver id",
        &|v| v["driver_id"] = json!("bad driver"),
        "traffic_driver_id_invalid",
    );

    // Digests: exactly 64 lowercase hex - never uppercase, never short,
    // never non-hex.
    rejects(
        "uppercase seed fingerprint",
        &|v| v["run_seed_fingerprint"] = json!("AB".repeat(32)),
        "traffic_history_digest_invalid",
    );
    rejects(
        "short allocation digest",
        &|v| v["allocation_digest"] = json!("abcd"),
        "traffic_history_digest_invalid",
    );
    rejects(
        "non-hex allocation digest",
        &|v| v["allocation_digest"] = json!("zz".repeat(32)),
        "traffic_history_digest_invalid",
    );

    // Caps: out-of-range values reject, never clamp.
    rejects(
        "unknown_retry_sends 4",
        &|v| v["caps"]["unknown_retry_sends"] = json!(4),
        "traffic_caps_invalid",
    );
    rejects(
        "reorg_resubmits 0",
        &|v| v["caps"]["reorg_resubmits"] = json!(0),
        "traffic_caps_invalid",
    );

    // Strict parse: a missing field or an unknown extra field rejects
    // before validation even runs (deny_unknown_fields, no defaults).
    rejects(
        "missing field",
        &|v| {
            v.as_object_mut().unwrap().remove("allocation_digest");
        },
        "traffic_history_header_unparseable",
    );
    rejects(
        "unknown extra field",
        &|v| v["extra_field"] = json!(1),
        "traffic_history_header_unparseable",
    );
}

/// A DIFFERENT but intrinsically valid run id / driver id / digest
/// parses under the strict contract: [`TrafficHistoryHeader::validate`]
/// validates the contract, not the possession. Binding the header
/// against the expected run / driver / allocation is the consumer's
/// reopen concern - the T1a core enforces it on every reopen before
/// any mutation (any drift is a hard error, see `TrafficWalletSet::
/// reopen`), and the T0 consumer-side comparison is the same check.
/// This test pins that boundary honestly instead of claiming the parse
/// rejects it.
#[test]
fn a_valid_but_foreign_binding_parses_and_is_the_consumers_rejection() {
    let fixture = real_fixture("foreign-binding");
    let mut value: Value = serde_json::from_str(&fixture.line).unwrap();
    // Still-valid ids, still-64-hex digests, still-in-range caps: only
    // the BINDING changed, not the contract shape.
    value["run_id"] = json!("another-valid-run");
    value["driver_id"] = json!("another-valid-driver");
    value["allocation_digest"] = json!("cd".repeat(32));
    let header = TrafficHistoryHeader::from_journal_line(&value.to_string())
        .expect("an intrinsically valid history-3 header parses; binding is the consumer's check");
    assert_eq!(header.run_id.as_str(), "another-valid-run");

    // The consumer-side comparison the reopen path performs: the header
    // facts no longer match the fixture's own journal.
    let facts = fixture.set.journal().header().unwrap();
    assert_ne!(header.run_id.as_str(), facts.run_id);
    assert_ne!(header.driver_id.as_str(), facts.driver_id);
    assert_ne!(header.allocation_digest, facts.allocation_digest);
}

// ---------------------------------------------------------------------------
// The unknown-field boundary: T0 and history-3 core are strict
// ---------------------------------------------------------------------------

/// Both consumers reject unknown fields under the history-3 activation.
#[test]
fn unknown_field_rejection_is_required_of_the_t0_consumer() {
    let fixture = real_fixture("unknown-field");
    let mut with_extra: Value = serde_json::from_str(&fixture.line).unwrap();
    with_extra["extra_field"] = json!(1);
    let line_with_extra = with_extra.to_string();

    let core = serde_json::from_str::<spc_lab_txd::journal::JournalRecord>(&line_with_extra);
    assert!(core.is_err());

    // The T0 strict consumer refuses the same line.
    let err = TrafficHistoryHeader::from_journal_line(&line_with_extra).unwrap_err();
    assert_eq!(err.code(), "traffic_history_header_unparseable");
}
