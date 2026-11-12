//! AP-1a traffic capability projection (traffic plan Section 8).
//!
//! There is one canonical probe executor - the Ark lane's AP-1a
//! (`ap1a-ark-rpc-probe`, schema v2 reports). This module runs no
//! competing probe and creates no second verdict: it owns the traffic
//! *projection* - the five-method traffic surface, the load-bearing
//! protocol/network/genesis rows, the mapping of the canonical report
//! into the run manifest, and scenario consumption.
//!
//! ## Frozen expected-check table
//!
//! [`TRAFFIC_EXPECTED_CHECKS`] pins the exact `(id, method)` binding,
//! coverage class and `unknown_behavior_ok` flag of every consulted
//! check (selected-consumer, chain-observation and foundational rows), exactly
//! as the canonical AP-1a `CHECKS` manifest declares them. A report row
//! whose id is unknown, or whose `method` disagrees with the table, is
//! rejected; a missing expected id makes its method `not_run` and blocks
//! the preflight stage. The table, the approved lineage list and any
//! expected-profile mapping change **atomically** through a recorded
//! decision - never a string-only substitution.
//!
//! ## Two-stage verdict
//!
//! - [`TrafficCapabilityRecord::recognized`] is the preflight gate
//!   verdict: availability `passed` on every consulted row, no `failed`
//!   on either axis, all foundational rows fully passed, and behavior
//!   `unknown`/`not_run` tolerated only for ids the table marks
//!   `unknown_behavior_ok` or coverage `funded`. The r1-evidence
//!   full-model `getmempoolinfo` shape failure remains real and is retained
//!   verbatim in `consumer_model_failures` and in `check_states`. The distinct
//!   size-only consumer gates recognition; no null-to-zero coercion occurs.
//!   There is no
//!   funded-broadcast requirement before bootstrap: the `funded` flag
//!   tolerates `sendrawtransaction.recognized` being `not_run` in
//!   read-only mode.
//! - [`TrafficCapabilityRecord::behavior_proven`] is FALSE by
//!   construction for every record this projection produces: a
//!   read-only report cannot prove funded behavior even if every row
//!   hypothetically passed. T3's acceptance consumes its own exercised
//!   funded evidence - never a read-only status loosen.
//!
//! ## Lineage
//!
//! [`AP1A_APPROVED_CONTRACT_SHA256S`] is the production approved-lineage
//! allowlist: exactly the parent/user-authorized canonical r4 contract
//! `754e1e61...`, activated with its 20-row table and metadata validation.
//! Historical Fable approval of the 17-row `f4fc3e04...` contract remains
//! historical evidence and cannot authorize this new table. This activation
//! does not claim a new Fable verdict; see the atomic J1 activation decision.
//! The approval covers the probe contract only - not T0 code, not the
//! traffic capability, not any node profile. Any other hash fails
//! closed: a recognized-but-unapproved candidate with
//! `traffic_capability_lineage_unapproved`, an unknown hash with
//! `traffic_capability_lineage_unknown`, and an empty allowlist (the
//! pre-approval safe state, exercised through the private test seam)
//! rejects every record. [`AP1A_RECOGNIZED_UNAPPROVED_CANDIDATES`]
//! names known unapproved hashes for diagnostics only - never
//! acceptance.
//!
//! The report's own verdict is the *bark* verdict and is recorded as-is:
//! it is never a traffic verdict, and the Bark version gate
//! (`getnetworkinfo.gate.*`) never affects this projection. Conversely,
//! the global protocol guards (id echo, envelope 2.0), network and
//! genesis rows stay load-bearing for traffic.
//!
//! ## Binding and honesty
//!
//! `report_sha256` is the sha256 of the exact report bytes and is a
//! self-reported integrity marker, NOT cryptographic authentication:
//! the record is only as trustworthy as the source that produced it.
//! Binding the record to the actual run inputs (the run's node binary
//! hash in `binary_paths_and_hashes`, the scenario's network and the
//! lab's genesis) is a T1 requirement recorded with the record fields
//! this projection carries for exactly that purpose
//! (`target_kind`, `expected_network`, `expected_genesis`,
//! `node_binary_sha256`). Until that binding exists the gate accepts
//! only `owned-fresh-regtest-node` targets on `regtest` - it never
//! claims to have checked an arbitrary remote.

use std::collections::BTreeSet as Set;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::scenario::{Scenario, ScenarioError};
use crate::traffic::spec::TRAFFIC_CAPABILITY;
use crate::traffic::TrafficError;

/// The only canonical probe executor this projection consumes.
pub const AP1A_PROBE_ID: &str = "ap1a-ark-rpc-probe";

/// The only AP-1a report schema version this projection consumes.
pub const AP1A_REPORT_SCHEMA_VERSION: u32 = 2;

/// Schema version of the projection record itself.
pub const TRAFFIC_CAPABILITY_RECORD_SCHEMA_VERSION: u32 = 1;

/// The production approved AP-1a contract-lineage allowlist.
///
/// Contains exactly the r4 contract activated by explicit parent/user
/// authorization together with its table and metadata contract. This is not
/// a claim of Fable approval. The old r2 approval and raw reports remain
/// immutable history, separate from the currently supported consumer schema.
///
/// While a hash is listed here the gate accepts its records' lineage;
/// every other hash fails closed (`traffic_capability_lineage_unknown`,
/// or `traffic_capability_lineage_unapproved` for a recognized
/// superseded candidate). Changing this list, the frozen
/// [`TRAFFIC_EXPECTED_CHECKS`] table or any expected-profile mapping
/// happens only atomically through a recorded decision: a hash is not a
/// shape check, and a string-only substitution is never a valid
/// approval.
pub const AP1A_TRAFFIC_CONTRACT_SHA256: &str =
    "754e1e6131955f9e5bbce7ef81b6f9aabaa2f1379ec6409f1c3a7a9e88be91f9";
pub const AP1A_APPROVED_CONTRACT_SHA256S: &[&str] = &[AP1A_TRAFFIC_CONTRACT_SHA256];
pub const AP1A_HISTORICAL_R2_CONTRACT_SHA256: &str =
    "f4fc3e043f420870675095ab24144ba14e0afba6cb9ad69d818d99b1bccb2f62";
pub const AP1A_PRE_RPC_POLICY_CONTRACT_SHA256: &str =
    "da87e2b665f4c3427098e308fd87d5946c8fc78d1ffa64c93a1ad0f84b3b5ca8";
pub const TRAFFIC_CONSUMER: &str = "spc-lab-tx-rpc-consumer/1";

/// AP-1a contract hashes that are recognized but explicitly NOT
/// approved, for diagnostics only - never acceptance:
///
/// - `11bdb100...` - the r1 candidate (2026-11-08); Fable review found
///   two blockers (HTTP 500/404 response-id mismatch propagation,
///   pinned request forms) and superseded it with the approved r2
///   contract.
///
/// The r2 contract `f4fc3e04...` retains its historical approval but is not a
/// currently supported contract. The never-approved v1-era
/// `48894548...` and pre-getmempoolinfo r1-draft `905bf4b8...` hashes
/// are not even recognized candidates.
pub const AP1A_RECOGNIZED_UNAPPROVED_CANDIDATES: &[&str] = &[
    "11bdb1007e130a749720f87482f287c1fb65d1e743c142fc4b1ca15a782d89e7",
    AP1A_PRE_RPC_POLICY_CONTRACT_SHA256,
];

/// The exactly-five traffic methods (traffic plan Section 8).
pub const TRAFFIC_METHODS: [&str; 5] = [
    "sendrawtransaction",
    "getrawmempool",
    "getrawtransaction",
    "getmempoolinfo",
    "getblock",
];

/// The load-bearing foundational check ids (canonical contract
/// identifiers): JSON-RPC protocol integrity (id echo and envelope 2.0
/// handling), network identity and regtest genesis consistency. A
/// failure or non-passed state on any of these fails the traffic
/// capability closed even when all five traffic methods pass, and
/// independently of the Bark version floor (which never affects this
/// projection).
pub const TRAFFIC_FOUNDATIONAL_CHECKS: [&str; 5] = [
    "protocol.id-echo",
    "protocol.envelope20.getnetworkinfo",
    "getblockchaininfo.network",
    "getblockhash.genesis",
    "getblockheader.verbose.genesis",
];

/// The lab's only network; the traffic capability is regtest by name
/// (`spc.node.regtest.mempool_rpc.v1`) and the gate binds the record's
/// `expected_network` to it.
pub const TRAFFIC_EXPECTED_NETWORK: &str = "regtest";

/// The only target kind that can satisfy a run's traffic capability:
/// the run's own freshly-launched regtest node. A `remote-url` target
/// says nothing about the run's nodes and is never accepted.
pub const TRAFFIC_TARGET_KIND_OWNED: &str = "owned-fresh-regtest-node";

/// The other target kind the AP-1a schema allows (legitimate for Bark
/// probes, never sufficient for a run's traffic capability).
pub const TRAFFIC_TARGET_KIND_REMOTE: &str = "remote-url";

/// One row of the frozen expected-check table (mirrors the canonical
/// AP-1a `CHECKS` manifest entry for a consulted check).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpectedCheck {
    /// The canonical check id.
    pub id: &'static str,
    /// The method the canonical contract binds to this id (`None` only
    /// for the protocol-level `protocol.id-echo` row). A report row
    /// carrying this id with any other method is rejected.
    pub method: Option<&'static str>,
    /// The canonical coverage class (`required`, `encoding`,
    /// `observational`, `protocol`, `traffic`, `funded`).
    pub coverage: &'static str,
    /// The canonical contract's own flag: the behavior axis may
    /// legitimately be `unknown` on a fresh node (genesis coinbase).
    pub unknown_behavior_ok: bool,
}

impl ExpectedCheck {
    /// Whether the recognized (preflight) stage tolerates a
    /// non-passed behavior axis for this id: only the canonical
    /// `unknown_behavior_ok` flag or the `funded` coverage class
    /// (recognition-only in read-only mode).
    pub fn behavior_tolerated(&self) -> bool {
        self.unknown_behavior_ok || self.coverage == "funded"
    }
}

/// The 20 selected-consumer, chain-observation and foundational rows, exactly
/// as the canonical r4 AP-1a `CHECKS` manifest
/// declares them (ids, methods, coverage classes, flags). This table -
/// not prose - is the contract: an unknown consulted id, a wrong
/// id-method binding or a missing expected id all fail closed. It
/// changes only atomically with the approved-lineage list through a
/// recorded decision.
pub const TRAFFIC_EXPECTED_CHECKS: &[ExpectedCheck] = &[
    // Five traffic methods.
    ExpectedCheck {
        id: "sendrawtransaction.recognized",
        method: Some("sendrawtransaction"),
        coverage: "funded",
        unknown_behavior_ok: false,
    },
    ExpectedCheck {
        id: "getrawmempool",
        method: Some("getrawmempool"),
        coverage: "required",
        unknown_behavior_ok: false,
    },
    ExpectedCheck {
        id: "getrawmempool.bool-form",
        method: Some("getrawmempool"),
        coverage: "encoding",
        unknown_behavior_ok: false,
    },
    ExpectedCheck {
        id: "getrawtransaction.verbosity0",
        method: Some("getrawtransaction"),
        coverage: "required",
        unknown_behavior_ok: true,
    },
    ExpectedCheck {
        id: "getrawtransaction.verbosity1",
        method: Some("getrawtransaction"),
        coverage: "required",
        unknown_behavior_ok: true,
    },
    ExpectedCheck {
        id: "getrawtransaction.bool-verbose",
        method: Some("getrawtransaction"),
        coverage: "encoding",
        unknown_behavior_ok: true,
    },
    ExpectedCheck {
        id: "getrawtransaction.bool-verbose.async",
        method: Some("getrawtransaction"),
        coverage: "encoding",
        unknown_behavior_ok: true,
    },
    ExpectedCheck {
        id: "getmempoolinfo.traffic-consumer",
        method: Some("getmempoolinfo"),
        coverage: "traffic",
        unknown_behavior_ok: false,
    },
    ExpectedCheck {
        id: "getblock.verbosity0",
        method: Some("getblock"),
        coverage: "required",
        unknown_behavior_ok: false,
    },
    ExpectedCheck {
        id: "getblock.verbosity0.rehash",
        method: Some("getblock"),
        coverage: "required",
        unknown_behavior_ok: false,
    },
    ExpectedCheck {
        id: "getblock.verbosity1",
        method: Some("getblock"),
        coverage: "required",
        unknown_behavior_ok: false,
    },
    ExpectedCheck {
        id: "getblock.singleparam",
        method: Some("getblock"),
        coverage: "encoding",
        unknown_behavior_ok: false,
    },
    // Foundational rows.
    ExpectedCheck {
        id: "getrawtransaction.blockhash.traffic-consumer",
        method: Some("getrawtransaction"),
        coverage: "traffic",
        unknown_behavior_ok: true,
    },
    ExpectedCheck {
        id: "getblockheader.traffic-consumer",
        method: Some("getblockheader"),
        coverage: "traffic",
        unknown_behavior_ok: false,
    },
    ExpectedCheck {
        id: "getbestblockhash",
        method: Some("getbestblockhash"),
        coverage: "observational",
        unknown_behavior_ok: false,
    },
    ExpectedCheck {
        id: "protocol.id-echo",
        method: None,
        coverage: "protocol",
        unknown_behavior_ok: false,
    },
    ExpectedCheck {
        id: "protocol.envelope20.getnetworkinfo",
        method: Some("getnetworkinfo"),
        coverage: "encoding",
        unknown_behavior_ok: false,
    },
    ExpectedCheck {
        id: "getblockchaininfo.network",
        method: Some("getblockchaininfo"),
        coverage: "required",
        unknown_behavior_ok: false,
    },
    ExpectedCheck {
        id: "getblockhash.genesis",
        method: Some("getblockhash"),
        coverage: "required",
        unknown_behavior_ok: false,
    },
    ExpectedCheck {
        id: "getblockheader.verbose.genesis",
        method: Some("getblockheader"),
        coverage: "required",
        unknown_behavior_ok: false,
    },
];

fn expected_check(id: &str) -> Option<&'static ExpectedCheck> {
    TRAFFIC_EXPECTED_CHECKS
        .iter()
        .find(|entry| entry.id == id)
        .or_else(|| (id == MODEL_CHECK.id).then_some(&MODEL_CHECK))
}

/// Full corepc-v30 modeling diagnostic, not a selected-consumer gating row.
const MODEL_CHECK: ExpectedCheck = ExpectedCheck {
    id: "getmempoolinfo",
    method: Some("getmempoolinfo"),
    coverage: "traffic",
    unknown_behavior_ok: false,
};

/// The expected check ids bound to one traffic method, in table order.
pub fn expected_ids_for_method(method: &str) -> Vec<&'static str> {
    TRAFFIC_EXPECTED_CHECKS
        .iter()
        .filter(|entry| entry.method == Some(method))
        .map(|entry| entry.id)
        .collect()
}

/// An AP-1a check status on either axis, mapped as-is. The availability
/// axis has no `not_run` value; the behavior axis does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ap1aCheckState {
    Passed,
    Failed,
    Unknown,
    NotRun,
}

impl Ap1aCheckState {
    /// Parses one axis value. `allow_not_run` is false for the
    /// availability axis (AP-1a schema v2).
    fn parse_axis(value: &str, allow_not_run: bool) -> Option<Self> {
        match value {
            "passed" => Some(Self::Passed),
            "failed" => Some(Self::Failed),
            "unknown" => Some(Self::Unknown),
            "not_run" if allow_not_run => Some(Self::NotRun),
            _ => None,
        }
    }

    /// Merge severity: `failed` > `unknown` > `not_run` > `passed`. A
    /// method (or foundational row) merges every check and both axes
    /// through this ordering, so any failure, unknown or not-run among
    /// the consulted rows fails the capability closed (traffic plan
    /// Section 16 item 1).
    fn severity(self) -> u8 {
        match self {
            Self::Failed => 3,
            Self::Unknown => 2,
            Self::NotRun => 1,
            Self::Passed => 0,
        }
    }

    fn merge(self, other: Self) -> Self {
        if self.severity() >= other.severity() {
            self
        } else {
            other
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
            Self::NotRun => "not_run",
        }
    }
}

/// One traffic method's merged status plus the check ids consulted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrafficMethodStatus {
    pub state: Ap1aCheckState,
    pub check_ids: Vec<String>,
}

/// Both axes of one consulted check, as the report recorded them. The
/// record carries the full per-check rows so the gate can recompute
/// both verdict stages and the unproven-id list - an aggregated state
/// alone cannot be tamper-checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrafficCheckStatus {
    pub availability: Ap1aCheckState,
    pub behavior: Ap1aCheckState,
}

/// The report's own (bark) verdict, recorded as-is. Never re-derived and
/// never a traffic verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ap1aVerdictSummary {
    pub status: String,
    pub funded_state: String,
    pub full_acceptance: bool,
}

/// The typed, fail-closed input adapter for the AP-1a schema v2 report.
///
/// Deliberately NOT `deny_unknown_fields`: the AP-1a schema allows
/// unknown properties (its own `schemadef.validate_report` governs the
/// full shape); this adapter types exactly the fields the projection
/// consumes and rejects a report missing any of them.
#[derive(Debug, Clone, Deserialize)]
pub struct Ap1aReportInput {
    pub probe: String,
    pub probe_version: String,
    pub schema_version: u32,
    pub generated_utc: String,
    pub run_token: String,
    pub target: Ap1aTargetInput,
    pub profile: Ap1aProfileInput,
    pub contract_hash: String,
    #[serde(default)]
    pub traffic_consumer: Option<String>,
    #[serde(default)]
    pub consumer_model_failures: Option<Vec<serde_json::Value>>,
    pub checks: Vec<Ap1aCheckInput>,
    pub observations: serde_json::Value,
    pub verdict: Ap1aVerdictInput,
    pub node: Option<Ap1aNodeInput>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Ap1aTargetInput {
    pub label: String,
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Ap1aProfileInput {
    pub name: String,
    pub server_min_rpc_version: u64,
    pub wallet_min_rpc_version: u64,
    pub expected_network: String,
    pub expected_genesis: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Ap1aNodeInput {
    #[serde(default)]
    pub binary_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ap1aCheckInput {
    pub id: String,
    /// `null` for protocol-level rows (e.g. `protocol.id-echo`).
    pub method: Option<String>,
    pub availability: String,
    pub behavior: String,
    pub detail: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Ap1aVerdictInput {
    pub status: String,
    pub funded_state: String,
    pub full_acceptance: bool,
}

/// The projection of one canonical AP-1a report into the run manifest.
///
/// Two frozen verdict stages (see the module docs): `recognized` is the
/// preflight gate verdict; `behavior_proven` is false by construction
/// for every read-only projection. `check_states` carries the full
/// per-check rows so the gate can recompute everything the record
/// claims; the merged `methods`/`foundational` views are derived and
/// re-checked, never trusted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrafficCapabilityRecord {
    pub schema_version: u32,
    pub probe: String,
    pub probe_version: String,
    pub report_schema_version: u32,
    /// sha256 of the exact report bytes this record was projected from.
    /// A self-reported integrity marker, NOT cryptographic
    /// authentication; run-input binding is the T1 requirement below.
    pub report_sha256: String,
    /// The canonical contract lineage of the producing probe.
    pub contract_hash: String,
    pub generated_utc: String,
    pub run_token: String,
    /// Target metadata, carried for run binding (T1 binds these to the
    /// actual run inputs; the T0 gate binds what is already known).
    pub target_label: String,
    /// `owned-fresh-regtest-node` or `remote-url`; only the former can
    /// satisfy a run's traffic capability.
    pub target_kind: String,
    /// The report profile's name (e.g. `bark-c419159`).
    pub profile_name: String,
    /// The profile's expected network; the gate requires `regtest`.
    pub expected_network: String,
    /// The profile's expected genesis (64-hex). Binding it to the run's
    /// actual node is a T1 requirement, recorded here for that purpose.
    pub expected_genesis: String,
    /// `node.binary_sha256` when the report carries node metadata;
    /// T1 binds it to the manifest's `binary_paths_and_hashes`.
    pub node_binary_sha256: Option<String>,
    pub traffic_consumer: Option<String>,
    /// Complete original modeling-failure rows; never used to coerce a pass.
    pub consumer_model_failures: Vec<serde_json::Value>,
    /// Exactly the five traffic methods (traffic plan Section 8).
    pub methods: BTreeMap<String, TrafficMethodStatus>,
    /// The load-bearing protocol/network/genesis rows, id -> merged state.
    pub foundational: BTreeMap<String, Ap1aCheckState>,
    /// Full per-check rows (both axes) for every consulted id the report
    /// carried, keyed by id. The gate recomputes both verdict stages,
    /// the merged views and the unproven list from these rows.
    pub check_states: BTreeMap<String, TrafficCheckStatus>,
    /// Relay/fee-policy facts observed by the probe (min relay fee,
    /// mempool size limits, ...), carried as-is so preset fee floors are
    /// grounded. Never re-derived. Always a JSON object.
    pub observations: serde_json::Value,
    /// The report's own bark verdict, recorded as-is.
    pub verdict: Ap1aVerdictSummary,
    /// Stage 1 - the preflight gate verdict (see module docs).
    pub recognized: bool,
    /// Stage 2 - always false from a read-only report, by construction.
    /// T3 consumes its own exercised funded evidence instead.
    pub behavior_proven: bool,
    /// Sorted ids whose behavior axis is not passed (missing rows
    /// included): what remains to be exercised before T3.
    pub unproven_check_ids: Vec<String>,
}

impl TrafficCapabilityRecord {
    /// Recomputes the merged per-method states from the full check
    /// rows. A method whose expected ids are all missing is `not_run` -
    /// never silently passed.
    pub fn recompute_method_states(&self) -> BTreeMap<String, Ap1aCheckState> {
        let mut methods = BTreeMap::new();
        for method in TRAFFIC_METHODS {
            let expected = expected_ids_for_method(method);
            let mut rows = Vec::new();
            for id in &expected {
                if let Some(status) = self.check_states.get(*id) {
                    rows.push(*status);
                }
            }
            let state = if rows.is_empty() {
                Ap1aCheckState::NotRun
            } else {
                rows.iter().fold(Ap1aCheckState::Passed, |merged, status| {
                    merged.merge(status.availability).merge(status.behavior)
                })
            };
            methods.insert(method.to_string(), state);
        }
        methods
    }

    /// Recomputes the merged foundational states; a missing row is
    /// `not_run`, never silently passed.
    pub fn recompute_foundational(&self) -> BTreeMap<String, Ap1aCheckState> {
        let mut foundational = BTreeMap::new();
        for id in TRAFFIC_FOUNDATIONAL_CHECKS {
            let state = match self.check_states.get(id) {
                Some(status) => status.availability.merge(status.behavior),
                None => Ap1aCheckState::NotRun,
            };
            foundational.insert(id.to_string(), state);
        }
        foundational
    }

    /// Recomputes stage 1 (the preflight gate verdict) from the full
    /// check rows: every expected id present, availability `passed` on
    /// every consulted row, no `failed` on either axis, and behavior
    /// `unknown`/`not_run` only on ids the frozen table flags
    /// (`unknown_behavior_ok` or `funded` coverage). Missing expected
    /// ids are never tolerated just because the id is flagged.
    pub fn recompute_recognized(&self) -> bool {
        TRAFFIC_EXPECTED_CHECKS.iter().all(|entry| {
            let Some(status) = self.check_states.get(entry.id) else {
                // A missing expected id blocks the preflight stage -
                // even when the id's flags would tolerate a non-passed
                // behavior axis.
                return false;
            };
            if status.availability != Ap1aCheckState::Passed {
                return false;
            }
            if status.behavior == Ap1aCheckState::Failed {
                return false;
            }
            status.behavior == Ap1aCheckState::Passed || entry.behavior_tolerated()
        })
    }

    /// Recomputes the sorted unproven-id list: every expected id whose
    /// behavior axis is not `passed` (missing rows included).
    pub fn recompute_unproven_ids(&self) -> Vec<String> {
        let mut ids: BTreeSet<String> = BTreeSet::new();
        for entry in TRAFFIC_EXPECTED_CHECKS {
            let proven = self
                .check_states
                .get(entry.id)
                .is_some_and(|status| status.behavior == Ap1aCheckState::Passed);
            if !proven {
                ids.insert(entry.id.to_string());
            }
        }
        ids.into_iter().collect()
    }
}

/// Strict provenance-string shape: non-empty, no surrounding
/// whitespace, no control characters.
fn strict_provenance_string(field: &str, value: &str) -> Result<(), TrafficError> {
    if value.is_empty() || value != value.trim() || value.bytes().any(|b| b.is_ascii_control()) {
        return Err(TrafficError::new(
            "traffic_projection_report_invalid",
            format!("report {field} {value:?} is not a strict non-empty provenance string"),
        ));
    }
    Ok(())
}

/// Strict `generated_utc` shape: exactly `YYYY-MM-DDTHH:MM:SSZ` (20
/// ASCII bytes). A shape check, not a semantic time validation.
fn is_strict_utc_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 20
        && bytes.iter().enumerate().all(|(index, b)| match index {
            4 | 7 => *b == b'-',
            10 => *b == b'T',
            13 | 16 => *b == b':',
            19 => *b == b'Z',
            _ => b.is_ascii_digit(),
        })
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Projects a canonical AP-1a report (exact bytes) into a
/// [`TrafficCapabilityRecord`].
///
/// Fail-closed on: unparseable or incomplete reports, wrong probe id,
/// wrong report schema version, duplicate check ids, malformed contract
/// hash, malformed provenance (empty/whitespace probe version, run
/// token, generated timestamp), a non-object `observations`, a
/// malformed target/profile/node shape, unknown status values on any
/// consulted row, an unknown consulted check id, and any consulted row
/// whose `method` disagrees with the frozen expected-check table. Never
/// coerces a state and never derives a traffic verdict from the bark
/// verdict. `behavior_proven` is false by construction.
pub fn project_traffic_capability(
    report_bytes: &[u8],
) -> Result<TrafficCapabilityRecord, TrafficError> {
    let report_sha256 = hex::encode(Sha256::digest(report_bytes));
    let report: Ap1aReportInput = serde_json::from_slice(report_bytes).map_err(|e| {
        TrafficError::new(
            "traffic_projection_report_invalid",
            format!("AP-1a report does not parse as a schema v2 report: {e}"),
        )
    })?;
    if report.probe != AP1A_PROBE_ID {
        return Err(TrafficError::new(
            "traffic_projection_provenance_invalid",
            format!(
                "report probe {:?} is not the canonical probe {AP1A_PROBE_ID}",
                report.probe
            ),
        ));
    }
    if report.schema_version != AP1A_REPORT_SCHEMA_VERSION {
        return Err(TrafficError::new(
            "traffic_projection_schema_unsupported",
            format!(
                "report schema_version {} is not {AP1A_REPORT_SCHEMA_VERSION}",
                report.schema_version
            ),
        ));
    }
    if !is_sha256_hex(&report.contract_hash) {
        return Err(TrafficError::new(
            "traffic_projection_report_invalid",
            format!(
                "report contract_hash {:?} is not a 64-character hex sha256",
                report.contract_hash
            ),
        ));
    }
    strict_provenance_string("probe_version", &report.probe_version)?;
    if report.contract_hash == AP1A_TRAFFIC_CONTRACT_SHA256 {
        if report.traffic_consumer.as_deref() != Some(TRAFFIC_CONSUMER) {
            return Err(TrafficError::new(
                "traffic_projection_report_invalid",
                "missing or mismatched traffic consumer",
            ));
        }
        let model_rows: Vec<_> = report
            .checks
            .iter()
            .filter(|c| c.id == MODEL_CHECK.id)
            .collect();
        if model_rows.len() != 1 {
            return Err(TrafficError::new(
                "traffic_projection_report_invalid",
                "missing full-model diagnostic row",
            ));
        }
        let failures: Vec<_> = model_rows
            .into_iter()
            .filter(|c| c.availability == "failed" || c.behavior == "failed")
            .map(|c| serde_json::to_value(c).expect("JSON report row serialization"))
            .collect();
        if report.consumer_model_failures.as_ref() != Some(&failures) {
            return Err(TrafficError::new(
                "traffic_projection_report_invalid",
                "full-model failure not preserved verbatim",
            ));
        }
    }
    strict_provenance_string("run_token", &report.run_token)?;
    if !is_strict_utc_timestamp(&report.generated_utc) {
        return Err(TrafficError::new(
            "traffic_projection_report_invalid",
            format!(
                "report generated_utc {:?} is not a strict YYYY-MM-DDTHH:MM:SSZ timestamp",
                report.generated_utc
            ),
        ));
    }
    if !report.observations.is_object() {
        return Err(TrafficError::new(
            "traffic_projection_report_invalid",
            "report observations must be a JSON object",
        ));
    }
    if !matches!(
        report.target.kind.as_str(),
        TRAFFIC_TARGET_KIND_OWNED | TRAFFIC_TARGET_KIND_REMOTE
    ) {
        return Err(TrafficError::new(
            "traffic_projection_report_invalid",
            format!(
                "report target.kind {:?} is not {:?} or {:?}",
                report.target.kind, TRAFFIC_TARGET_KIND_OWNED, TRAFFIC_TARGET_KIND_REMOTE
            ),
        ));
    }
    strict_provenance_string("target.label", &report.target.label)?;
    strict_provenance_string("profile.name", &report.profile.name)?;
    if !is_sha256_hex(&report.profile.expected_genesis) {
        return Err(TrafficError::new(
            "traffic_projection_report_invalid",
            format!(
                "report profile.expected_genesis {:?} is not a 64-character hex sha256",
                report.profile.expected_genesis
            ),
        ));
    }
    strict_provenance_string("profile.expected_network", &report.profile.expected_network)?;
    if let Some(node) = &report.node {
        if let Some(binary_sha256) = &node.binary_sha256 {
            if !is_sha256_hex(binary_sha256) {
                return Err(TrafficError::new(
                    "traffic_projection_report_invalid",
                    format!(
                        "report node.binary_sha256 {binary_sha256:?} is not a 64-character \
                         hex sha256"
                    ),
                ));
            }
        }
    }
    if !matches!(
        report.verdict.status.as_str(),
        "incompatible" | "surface_compatible" | "unproven"
    ) {
        return Err(TrafficError::new(
            "traffic_projection_report_invalid",
            format!(
                "report verdict.status {:?} is not an AP-1a schema v2 status",
                report.verdict.status
            ),
        ));
    }
    if report.verdict.funded_state != "unproven" || report.verdict.full_acceptance {
        return Err(TrafficError::new(
            "traffic_projection_report_invalid",
            "report verdict violates the read-only invariants (funded_state must be \
             \"unproven\" and full_acceptance false)",
        ));
    }

    // Consulted rows: exact id-method binding against the frozen table.
    let mut seen_ids = Set::new();
    let mut check_states: BTreeMap<String, TrafficCheckStatus> = BTreeMap::new();
    for check in &report.checks {
        if check.id.is_empty() {
            return Err(TrafficError::new(
                "traffic_projection_report_invalid",
                "report contains a check with an empty id",
            ));
        }
        if !seen_ids.insert(check.id.clone()) {
            return Err(TrafficError::new(
                "traffic_projection_duplicate_check",
                format!("check id {} appears more than once in the report", check.id),
            ));
        }
        let entry = expected_check(&check.id);
        let row_method_is_traffic = check
            .method
            .as_deref()
            .is_some_and(|method| TRAFFIC_METHODS.contains(&method));
        match entry {
            Some(entry) => {
                if check.method.as_deref() != entry.method {
                    return Err(TrafficError::new(
                        "traffic_projection_report_invalid",
                        format!(
                            "check {} declares method {:?} but the frozen expected-check table \
                             binds it to {:?}",
                            check.id, check.method, entry.method
                        ),
                    ));
                }
            }
            None => {
                if row_method_is_traffic {
                    return Err(TrafficError::new(
                        "traffic_projection_report_invalid",
                        format!(
                            "check id {} is not in the frozen expected-check table but claims \
                             traffic method {:?}",
                            check.id, check.method
                        ),
                    ));
                }
                // Bark-consumer rows (including the getnetworkinfo
                // version floor) never affect the traffic projection.
                continue;
            }
        }
        let availability =
            Ap1aCheckState::parse_axis(&check.availability, false).ok_or_else(|| {
                TrafficError::new(
                    "traffic_projection_state_invalid",
                    format!(
                        "check {} availability {:?} is not passed|failed|unknown",
                        check.id, check.availability
                    ),
                )
            })?;
        let behavior = Ap1aCheckState::parse_axis(&check.behavior, true).ok_or_else(|| {
            TrafficError::new(
                "traffic_projection_state_invalid",
                format!(
                    "check {} behavior {:?} is not passed|failed|unknown|not_run",
                    check.id, check.behavior
                ),
            )
        })?;
        check_states.insert(
            check.id.clone(),
            TrafficCheckStatus {
                availability,
                behavior,
            },
        );
    }

    // Derived views: merged per-method states over the expected ids
    // only; missing expected ids make the method not_run.
    let mut methods = BTreeMap::new();
    for method in TRAFFIC_METHODS {
        let expected = expected_ids_for_method(method);
        let mut ids = Vec::new();
        for id in &expected {
            if let Some(status) = check_states.get(*id) {
                ids.push(*status);
            }
        }
        let state = if ids.is_empty() {
            Ap1aCheckState::NotRun
        } else {
            ids.iter().fold(Ap1aCheckState::Passed, |merged, status| {
                merged.merge(status.availability).merge(status.behavior)
            })
        };
        methods.insert(
            method.to_string(),
            TrafficMethodStatus {
                state,
                check_ids: expected.iter().map(|id| id.to_string()).collect(),
            },
        );
    }
    let mut foundational = BTreeMap::new();
    for id in TRAFFIC_FOUNDATIONAL_CHECKS {
        let state = match check_states.get(id) {
            Some(status) => status.availability.merge(status.behavior),
            None => Ap1aCheckState::NotRun,
        };
        foundational.insert(id.to_string(), state);
    }

    let mut record = TrafficCapabilityRecord {
        schema_version: TRAFFIC_CAPABILITY_RECORD_SCHEMA_VERSION,
        probe: report.probe,
        probe_version: report.probe_version,
        report_schema_version: AP1A_REPORT_SCHEMA_VERSION,
        report_sha256,
        contract_hash: report.contract_hash,
        generated_utc: report.generated_utc,
        run_token: report.run_token,
        target_label: report.target.label,
        target_kind: report.target.kind,
        profile_name: report.profile.name,
        expected_network: report.profile.expected_network,
        expected_genesis: report.profile.expected_genesis,
        node_binary_sha256: report.node.and_then(|node| node.binary_sha256),
        traffic_consumer: report.traffic_consumer,
        consumer_model_failures: report.consumer_model_failures.unwrap_or_default(),
        methods,
        foundational,
        check_states,
        observations: report.observations,
        verdict: Ap1aVerdictSummary {
            status: report.verdict.status,
            funded_state: report.verdict.funded_state,
            full_acceptance: report.verdict.full_acceptance,
        },
        recognized: false,
        behavior_proven: false,
        unproven_check_ids: Vec::new(),
    };
    record.recognized = record.recompute_recognized();
    record.unproven_check_ids = record.recompute_unproven_ids();
    Ok(record)
}

/// Scenario-side preflight gate: a scenario declaring traffic requires
/// a recorded, lineage-approved, recognized traffic capability before
/// any process starts - a real projection, never a config-declared
/// capability name.
///
/// Returns `Ok(())` when the scenario declares no traffic. A record
/// whose lineage is not in [`AP1A_APPROVED_CONTRACT_SHA256S`] fails
/// closed (`traffic_capability_lineage_unknown`, or
/// `traffic_capability_lineage_unapproved` for a recognized superseded
/// candidate; an empty allowlist - the pre-approval safe state - rejects
/// every record). The engine calls this in `prepare` (before any
/// filesystem, lock or authority effect) and in the adopt path (before
/// the owner lock).
pub fn check_traffic_capability(
    scenario: &Scenario,
    record: Option<&TrafficCapabilityRecord>,
) -> Result<(), ScenarioError> {
    check_traffic_capability_with_lineage(scenario, record, AP1A_APPROVED_CONTRACT_SHA256S)
}

/// PRIVATE test seam: [`check_traffic_capability`] with an explicit
/// approved-lineage list, so in-crate unit tests can exercise the
/// accept path against a fixture-approved hash. Not visible outside the
/// crate: no caller-configured runtime approval bypass exists.
fn check_traffic_capability_with_lineage(
    scenario: &Scenario,
    record: Option<&TrafficCapabilityRecord>,
    approved_lineage: &[&str],
) -> Result<(), ScenarioError> {
    let Some(traffic) = scenario.traffic.as_ref() else {
        return Ok(());
    };
    // Scenario validation already pinned required_capability to
    // TRAFFIC_CAPABILITY; re-assert so the gate cannot be bypassed by a
    // hand-built scenario object.
    if traffic.required_capability != TRAFFIC_CAPABILITY {
        return Err(ScenarioError::new(
            "traffic_capability_unknown",
            format!(
                "scenario traffic requires unknown capability {:?} (v1 knows only \
                 {TRAFFIC_CAPABILITY})",
                traffic.required_capability
            ),
        ));
    }
    let Some(record) = record else {
        return Err(ScenarioError::new(
            "traffic_capability_unrecorded",
            format!(
                "scenario declares traffic but the run manifest records no traffic \
                 capability {TRAFFIC_CAPABILITY} (AP-1a projection); a config-declared \
                 capability name is never evidence"
            ),
        ));
    };
    // The manifest is a JSON file and the record type is Deserialize:
    // this gate is the only defense, so it re-validates everything the
    // projection produced - never trusting an aggregated claim.
    let invalid = |detail: String| ScenarioError::new("traffic_capability_record_invalid", detail);
    if record.schema_version != TRAFFIC_CAPABILITY_RECORD_SCHEMA_VERSION {
        return Err(invalid(format!(
            "traffic capability record schema_version {} is not \
             {TRAFFIC_CAPABILITY_RECORD_SCHEMA_VERSION}",
            record.schema_version
        )));
    }
    if record.probe != AP1A_PROBE_ID {
        return Err(invalid(format!(
            "traffic capability record probe {:?} is not the canonical probe {AP1A_PROBE_ID}",
            record.probe
        )));
    }
    if record.report_schema_version != AP1A_REPORT_SCHEMA_VERSION {
        return Err(invalid(format!(
            "traffic capability record report_schema_version {} is not \
             {AP1A_REPORT_SCHEMA_VERSION}",
            record.report_schema_version
        )));
    }
    if !is_sha256_hex(&record.report_sha256) {
        return Err(invalid(format!(
            "traffic capability record report_sha256 {:?} is not a 64-character hex sha256",
            record.report_sha256
        )));
    }
    if !is_sha256_hex(&record.contract_hash) {
        return Err(invalid(format!(
            "traffic capability record contract_hash {:?} is not a 64-character hex sha256",
            record.contract_hash
        )));
    }
    if record.contract_hash == AP1A_HISTORICAL_R2_CONTRACT_SHA256 {
        return Err(ScenarioError::new("traffic_capability_contract_historical",
            "the historically approved 17-row r2 contract is unsupported by the current 20-row selected consumer; historical approval is unchanged"));
    }
    if record.probe_version.is_empty() || record.run_token.is_empty() {
        return Err(invalid(
            "traffic capability record carries an empty probe_version or run_token".to_string(),
        ));
    }
    if !is_strict_utc_timestamp(&record.generated_utc) {
        return Err(invalid(format!(
            "traffic capability record generated_utc {:?} is not a strict \
             YYYY-MM-DDTHH:MM:SSZ timestamp",
            record.generated_utc
        )));
    }
    if !is_sha256_hex(&record.expected_genesis) {
        return Err(invalid(format!(
            "traffic capability record expected_genesis {:?} is not a 64-character hex sha256",
            record.expected_genesis
        )));
    }
    if let Some(binary_sha256) = &record.node_binary_sha256 {
        if !is_sha256_hex(binary_sha256) {
            return Err(invalid(format!(
                "traffic capability record node_binary_sha256 {binary_sha256:?} is not a \
                 64-character hex sha256"
            )));
        }
    }
    // Exact method/foundational key sets and per-method check-id sets.
    let expected_method_keys: Set<&str> = TRAFFIC_METHODS.iter().copied().collect();
    let method_keys: Set<&str> = record.methods.keys().map(String::as_str).collect();
    if method_keys != expected_method_keys {
        return Err(invalid(format!(
            "traffic capability record methods {method_keys:?} are not exactly the \
             five traffic methods {:?}",
            TRAFFIC_METHODS
        )));
    }
    for method in TRAFFIC_METHODS {
        let status = &record.methods[method];
        let expected_ids: Set<&str> = expected_ids_for_method(method).into_iter().collect();
        let recorded_ids: Set<&str> = status.check_ids.iter().map(String::as_str).collect();
        if recorded_ids.is_empty() || recorded_ids != expected_ids {
            return Err(invalid(format!(
                "traffic capability record method {method} check_ids {recorded_ids:?} are not \
                 exactly the frozen expected ids {expected_ids:?}"
            )));
        }
    }
    let expected_foundational_keys: Set<&str> =
        TRAFFIC_FOUNDATIONAL_CHECKS.iter().copied().collect();
    let foundational_keys: Set<&str> = record.foundational.keys().map(String::as_str).collect();
    if foundational_keys != expected_foundational_keys {
        return Err(invalid(format!(
            "traffic capability record foundational rows {foundational_keys:?} are not \
             exactly {:?}",
            TRAFFIC_FOUNDATIONAL_CHECKS
        )));
    }
    // Every recorded check-state id must be a frozen expected id.
    for id in record.check_states.keys() {
        if expected_check(id).is_none() {
            return Err(invalid(format!(
                "traffic capability record check_states carries unknown id {id:?}"
            )));
        }
    }
    // Aggregated views must match the full rows.
    let recomputed_methods = record.recompute_method_states();
    for (method, status) in &record.methods {
        if recomputed_methods.get(method) != Some(&status.state) {
            return Err(invalid(format!(
                "traffic capability record method {method} claims state {} but its rows \
                 recompute {}",
                status.state.as_str(),
                recomputed_methods
                    .get(method)
                    .map(|s| s.as_str())
                    .unwrap_or("missing")
            )));
        }
    }
    let recomputed_foundational = record.recompute_foundational();
    for (id, state) in &record.foundational {
        if recomputed_foundational.get(id) != Some(state) {
            return Err(invalid(format!(
                "traffic capability record foundational {id} claims {} but its rows recompute {}",
                state.as_str(),
                recomputed_foundational
                    .get(id)
                    .map(|s| s.as_str())
                    .unwrap_or("missing")
            )));
        }
    }
    let recomputed_recognized = record.recompute_recognized();
    if recomputed_recognized != record.recognized {
        return Err(invalid(format!(
            "traffic capability record claims recognized={} but its rows recompute \
             recognized={recomputed_recognized}",
            record.recognized
        )));
    }
    let recomputed_unproven = record.recompute_unproven_ids();
    if recomputed_unproven != record.unproven_check_ids {
        return Err(invalid(format!(
            "traffic capability record unproven_check_ids {:?} do not recompute to {:?}",
            record.unproven_check_ids, recomputed_unproven
        )));
    }
    if record.behavior_proven {
        return Err(invalid(
            "traffic capability record claims behavior_proven=true; no read-only projection \
             may claim funded behavior - T3 consumes its own exercised evidence"
                .to_string(),
        ));
    }
    if (approved_lineage.contains(&record.contract_hash.as_str())
        && record.contract_hash != AP1A_TRAFFIC_CONTRACT_SHA256)
        || record.traffic_consumer.as_deref() != Some(TRAFFIC_CONSUMER)
    {
        return Err(invalid(
            "traffic table/consumer lineage mismatch".to_string(),
        ));
    }
    let model = record
        .check_states
        .get(MODEL_CHECK.id)
        .ok_or_else(|| invalid("missing full-model diagnostic row".to_string()))?;
    let model_failed =
        model.availability == Ap1aCheckState::Failed || model.behavior == Ap1aCheckState::Failed;
    if record.consumer_model_failures.len() != usize::from(model_failed) {
        return Err(invalid(
            "full-model diagnostic failure count mismatch".to_string(),
        ));
    }
    if let Some(row) = record.consumer_model_failures.first() {
        if row.get("id").and_then(|v| v.as_str()) != Some(MODEL_CHECK.id)
            || row.get("method").and_then(|v| v.as_str()) != MODEL_CHECK.method
            || row.get("availability").and_then(|v| v.as_str()) != Some(model.availability.as_str())
            || row.get("behavior").and_then(|v| v.as_str()) != Some(model.behavior.as_str())
            || row.get("detail").and_then(|v| v.as_str()).is_none()
        {
            return Err(invalid("full-model diagnostic row mismatch".to_string()));
        }
    }
    // Run binding: only the run's own kind of target on the lab's only
    // network. A remote-url record never says anything about the run's
    // nodes; genesis and node-binary binding to the actual run inputs
    // is the recorded T1 requirement.
    if record.target_kind != TRAFFIC_TARGET_KIND_OWNED {
        return Err(ScenarioError::new(
            "traffic_capability_target_unbound",
            format!(
                "traffic capability record target_kind {:?} cannot be bound to this run's \
                 nodes (only {TRAFFIC_TARGET_KIND_OWNED} can)",
                record.target_kind
            ),
        ));
    }
    if record.expected_network != TRAFFIC_EXPECTED_NETWORK {
        return Err(ScenarioError::new(
            "traffic_capability_target_unbound",
            format!(
                "traffic capability record expected_network {:?} is not the lab's network \
                 {TRAFFIC_EXPECTED_NETWORK:?}",
                record.expected_network
            ),
        ));
    }
    // Lineage: the approved allowlist decides; recognized unapproved
    // candidates are diagnostics only.
    if !approved_lineage.contains(&record.contract_hash.as_str()) {
        if approved_lineage.is_empty() {
            return Err(ScenarioError::new(
                "traffic_capability_lineage_unapproved",
                format!(
                    "traffic capability record contract_hash {} is rejected: no AP-1a contract \
                     lineage is approved (the allowlist is empty; it is populated only through \
                     a recorded decision that updates the hash and the expected-check table \
                     atomically)",
                    record.contract_hash
                ),
            ));
        }
        if AP1A_RECOGNIZED_UNAPPROVED_CANDIDATES.contains(&record.contract_hash.as_str()) {
            return Err(ScenarioError::new(
                "traffic_capability_lineage_unapproved",
                format!(
                    "traffic capability record contract_hash {} is a recognized but unapproved \
                     AP-1a candidate (superseded or failed review) and is never accepted; the \
                     approved lineage changes only through a recorded decision that updates the \
                     hash and the expected-check table atomically",
                    record.contract_hash
                ),
            ));
        }
        return Err(ScenarioError::new(
            "traffic_capability_lineage_unknown",
            format!(
                "traffic capability record contract_hash {} is not in the approved AP-1a lineage \
                 list and is not a recognized candidate",
                record.contract_hash
            ),
        ));
    }
    // Stage 1 verdict.
    if !record.recognized {
        let detail = TRAFFIC_METHODS
            .iter()
            .map(|method| {
                format!(
                    "{method}={}",
                    record
                        .methods
                        .get(*method)
                        .map(|s| s.state.as_str())
                        .unwrap_or("missing")
                )
            })
            .chain(TRAFFIC_FOUNDATIONAL_CHECKS.iter().map(|id| {
                format!(
                    "{id}={}",
                    record
                        .foundational
                        .get(*id)
                        .map(|s| s.as_str())
                        .unwrap_or("missing")
                )
            }))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(ScenarioError::new(
            "traffic_capability_unrecognized",
            format!(
                "recorded traffic capability {TRAFFIC_CAPABILITY} is not recognized at the \
                 preflight stage: {detail}"
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn traffic_scenario() -> Scenario {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/scenario_v1_two_pools.json"
        ))
        .unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value["traffic"] = serde_json::json!({
            "version": 1,
            "required_capability": TRAFFIC_CAPABILITY,
            "protocol": {"control": "spc-lab-tx-control/v1", "history_version": 3},
            "bootstrap": {
                "enabled": true,
                "mining_mode": "native",
                "funding_blocks": 101,
                "distribution_outputs_per_wallet": 8,
                "excluded_from_statistics": true
            },
            "driver": {
                "actors": [{"id": "txd-0", "network_wallets": 8, "user_payment_wallets": 2, "hosts_bootstrap_funding": true}],
                "user_payment_wallets": 2,
                "history": "traffic/"
            },
            "rate": {"ordinary_tx_per_sec": 0.2, "pacing": "fixed_jitter"},
            "fees": {
                "rate_sats_vb": {"min": 1, "max": 20},
                "max_fee_sats_per_tx": 100000
            },
            "sizes": {
                "classes_vb": [
                    {"vb": 220, "mix": 0.70},
                    {"vb": 600, "mix": 0.25},
                    {"vb": 1500, "mix": 0.05}
                ],
                "max_vb": 100000
            },
            "spend_types": ["p2pkh"],
            "ark": {
                "enabled": true,
                "registration_before_mining": true,
                "chain_effect_budget_tx_per_sec": 0.05
            },
            "caps": {
                "max_inflight_broadcasts": 64,
                "max_unknown_retries": 3,
                "max_mempool_backlog_tx": 2000,
                "reorg_resubmit_max": 3
            },
            "pause_semantics": "no_broadcast_after_ack"
        });
        Scenario::from_json(&value.to_string()).unwrap()
    }

    fn check_row(
        id: &str,
        method: Option<&str>,
        availability: &str,
        behavior: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "method": method,
            "availability": availability,
            "behavior": behavior,
            "detail": "synthetic test row"
        })
    }

    fn all_passed_checks() -> Vec<serde_json::Value> {
        let mut checks: Vec<_> = TRAFFIC_EXPECTED_CHECKS
            .iter()
            .map(|entry| check_row(entry.id, entry.method, "passed", "passed"))
            .collect();
        checks.push(check_row(
            MODEL_CHECK.id,
            MODEL_CHECK.method,
            "passed",
            "passed",
        ));
        checks.push(check_row(
            "getnetworkinfo.gate.server",
            Some("getnetworkinfo"),
            "passed",
            "failed",
        ));
        checks
    }

    fn r1_shaped_checks() -> Vec<serde_json::Value> {
        let mut checks = vec![check_row(
            "getnetworkinfo.gate.server",
            Some("getnetworkinfo"),
            "passed",
            "failed",
        )];
        for entry in TRAFFIC_EXPECTED_CHECKS {
            let (availability, behavior) = match entry.id {
                "sendrawtransaction.recognized" => ("passed", "not_run"),
                "getrawtransaction.verbosity0"
                | "getrawtransaction.verbosity1"
                | "getrawtransaction.bool-verbose"
                | "getrawtransaction.bool-verbose.async" => ("passed", "unknown"),
                "getmempoolinfo.traffic-consumer" => ("passed", "failed"),
                _ => ("passed", "passed"),
            };
            checks.push(check_row(entry.id, entry.method, availability, behavior));
        }
        checks
    }

    fn report_fixture(
        mut checks: Vec<serde_json::Value>,
        contract_hash: &str,
    ) -> serde_json::Value {
        if !checks.iter().any(|c| c["id"] == MODEL_CHECK.id) {
            checks.push(check_row(
                MODEL_CHECK.id,
                MODEL_CHECK.method,
                "passed",
                "passed",
            ));
        }
        let failures: Vec<_> = checks
            .iter()
            .filter(|c| {
                c["id"] == MODEL_CHECK.id
                    && (c["availability"] == "failed" || c["behavior"] == "failed")
            })
            .cloned()
            .collect();
        serde_json::json!({
            "probe": AP1A_PROBE_ID,
            "probe_version": "1.0.0",
            "schema_version": 2,
            "generated_utc": "2026-11-08T00:00:00Z",
            "run_token": "AP1a-test",
            "target": {"label": "test-node", "kind": TRAFFIC_TARGET_KIND_OWNED},
            "profile": {
                "name": "test",
                "server_min_rpc_version": 310000,
                "wallet_min_rpc_version": 290000,
                "expected_network": TRAFFIC_EXPECTED_NETWORK,
                "expected_genesis":
                    "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11433e0ad2"
            },
            "contract_hash": contract_hash,
            "traffic_consumer": TRAFFIC_CONSUMER,
            "consumer_model_failures": failures,
            "checks": checks,
            "observations": {"mempool_size": 0},
            "verdict": {
                "status": "surface_compatible",
                "reasons": ["funded-unproven"],
                "summary": "synthetic test report",
                "funded_state": "unproven",
                "full_acceptance": false
            }
        })
    }

    fn project(checks: Vec<serde_json::Value>, contract_hash: &str) -> TrafficCapabilityRecord {
        project_traffic_capability(report_fixture(checks, contract_hash).to_string().as_bytes())
            .unwrap()
    }

    const TEST_APPROVED_HASH: &str = AP1A_TRAFFIC_CONTRACT_SHA256;

    #[test]
    fn selected_report_requires_probe_consumer_and_verbatim_diagnostics() {
        for field in ["probe", "traffic_consumer", "consumer_model_failures"] {
            let mut report = report_fixture(all_passed_checks(), TEST_APPROVED_HASH);
            report.as_object_mut().unwrap().remove(field);
            assert!(project_traffic_capability(report.to_string().as_bytes()).is_err());
        }
        let mut report = report_fixture(all_passed_checks(), TEST_APPROVED_HASH);
        report["traffic_consumer"] = serde_json::json!("another-consumer/1");
        assert!(project_traffic_capability(report.to_string().as_bytes()).is_err());
    }

    #[test]
    fn old_frozen_report_cannot_use_the_selected_table() {
        let bytes = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testing/ark-rpc/reports/2026-11-08-r2-l2-eb257af7/report.json"
        ));
        let record = project_traffic_capability(bytes).unwrap();
        assert!(!record.recognized);
        assert_eq!(
            record.check_states[MODEL_CHECK.id].behavior,
            Ap1aCheckState::Failed
        );
        assert_eq!(
            check_traffic_capability(&traffic_scenario(), Some(&record))
                .unwrap_err()
                .code(),
            "traffic_capability_contract_historical"
        );
    }

    #[test]
    fn gate_accepts_a_recognized_record_under_an_approved_lineage() {
        let record = project(all_passed_checks(), TEST_APPROVED_HASH);
        assert!(record.recognized);
        assert!(!record.behavior_proven);
        assert!(record.unproven_check_ids.is_empty());
        check_traffic_capability_with_lineage(
            &traffic_scenario(),
            Some(&record),
            &[TEST_APPROVED_HASH],
        )
        .unwrap();
    }

    /// The authorized atomic activation is exactly r4's table + metadata +
    /// hash. Historical approval cannot license these new rows.
    #[test]
    fn production_gate_binds_exactly_the_authorized_r4_lineage() {
        let scenario = traffic_scenario();
        assert_eq!(AP1A_APPROVED_CONTRACT_SHA256S.len(), 1);

        // The approved r2 contract + a fully passing, recognized record:
        // the production accept path.
        let approved = project(all_passed_checks(), AP1A_APPROVED_CONTRACT_SHA256S[0]);
        check_traffic_capability(&scenario, Some(&approved)).unwrap();
        let historical = project(all_passed_checks(), AP1A_HISTORICAL_R2_CONTRACT_SHA256);
        assert_eq!(
            check_traffic_capability(&scenario, Some(&historical))
                .unwrap_err()
                .code(),
            "traffic_capability_contract_historical"
        );

        // The approved lineage does not rescue an unrecognized record:
        // the r1 evidence shape (getmempoolinfo failed) still blocks at
        // the recognized stage.
        let r1_shaped = project(r1_shaped_checks(), AP1A_APPROVED_CONTRACT_SHA256S[0]);
        assert!(!r1_shaped.recognized);
        let err = check_traffic_capability(&scenario, Some(&r1_shaped)).unwrap_err();
        assert_eq!(err.code(), "traffic_capability_unrecognized");

        // The superseded r1 candidate hash is named and never accepted.
        let candidate = project(
            all_passed_checks(),
            AP1A_RECOGNIZED_UNAPPROVED_CANDIDATES[0],
        );
        let err = check_traffic_capability(&scenario, Some(&candidate)).unwrap_err();
        assert_eq!(err.code(), "traffic_capability_lineage_unapproved");
        assert!(err.detail().contains("recognized but unapproved"));

        // An unknown hash is not in the approved lineage.
        let unknown = project(
            all_passed_checks(),
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        );
        let err = check_traffic_capability(&scenario, Some(&unknown)).unwrap_err();
        assert_eq!(err.code(), "traffic_capability_lineage_unknown");
    }

    #[test]
    fn gate_rejects_the_r1_shape_at_the_recognized_stage() {
        let record = project(r1_shaped_checks(), TEST_APPROVED_HASH);
        assert!(!record.recognized);
        // getmempoolinfo's real shape failure blocks recognition; the
        // funded sendrawtransaction row and the genesis unknowns do not.
        let err = check_traffic_capability_with_lineage(
            &traffic_scenario(),
            Some(&record),
            &[TEST_APPROVED_HASH],
        )
        .unwrap_err();
        assert_eq!(err.code(), "traffic_capability_unrecognized");
        assert!(err.detail().contains("getmempoolinfo=failed"));
    }

    #[test]
    fn selected_consumer_retains_full_model_failure_without_gating() {
        let mut checks = all_passed_checks();
        let model = checks
            .iter_mut()
            .find(|c| c["id"] == MODEL_CHECK.id)
            .unwrap();
        model["behavior"] = serde_json::json!("failed");
        model["detail"] =
            serde_json::json!("shape: maxdatacarriersize: expected JSON integer, got null");
        let record = project(checks, TEST_APPROVED_HASH);
        assert!(record.recognized);
        assert!(!record.behavior_proven);
        assert_eq!(
            record.check_states[MODEL_CHECK.id].behavior,
            Ap1aCheckState::Failed
        );
        assert_eq!(
            record.consumer_model_failures[0]["detail"],
            "shape: maxdatacarriersize: expected JSON integer, got null"
        );
        check_traffic_capability_with_lineage(
            &traffic_scenario(),
            Some(&record),
            &[TEST_APPROVED_HASH],
        )
        .unwrap();
        // Activation-specific change from the frozen producer's candidate test.
        check_traffic_capability(&traffic_scenario(), Some(&record)).unwrap();
    }

    #[test]
    fn missing_expected_id_blocks_even_when_its_flag_would_tolerate_it() {
        let mut checks = all_passed_checks();
        checks.retain(|row| row["id"].as_str() != Some("sendrawtransaction.recognized"));
        let record = project(checks, TEST_APPROVED_HASH);
        assert!(!record.recognized);
        assert!(record
            .unproven_check_ids
            .contains(&"sendrawtransaction.recognized".to_string()));
        let err = check_traffic_capability_with_lineage(
            &traffic_scenario(),
            Some(&record),
            &[TEST_APPROVED_HASH],
        )
        .unwrap_err();
        assert_eq!(err.code(), "traffic_capability_unrecognized");
    }

    #[test]
    fn behavior_proven_is_false_by_construction_even_with_all_rows_passed() {
        let record = project(all_passed_checks(), TEST_APPROVED_HASH);
        assert!(record.recognized);
        assert!(!record.behavior_proven);
        // A tampered claim is rejected by the gate.
        let mut tampered = record.clone();
        tampered.behavior_proven = true;
        let err = check_traffic_capability_with_lineage(
            &traffic_scenario(),
            Some(&tampered),
            &[TEST_APPROVED_HASH],
        )
        .unwrap_err();
        assert_eq!(err.code(), "traffic_capability_record_invalid");
    }

    #[test]
    fn gate_rejects_unapproved_candidates_and_unknown_lineages() {
        let scenario = traffic_scenario();
        // The pre-approval safe state (exercised through the seam): an
        // empty allowlist rejects even a fully passing record.
        let passing = project(all_passed_checks(), TEST_APPROVED_HASH);
        let err =
            check_traffic_capability_with_lineage(&scenario, Some(&passing), &[]).unwrap_err();
        assert_eq!(err.code(), "traffic_capability_lineage_unapproved");

        // A recognized unapproved candidate is still unapproved.
        let candidate = project(
            all_passed_checks(),
            AP1A_RECOGNIZED_UNAPPROVED_CANDIDATES[0],
        );
        let err = check_traffic_capability_with_lineage(
            &scenario,
            Some(&candidate),
            &[TEST_APPROVED_HASH],
        )
        .unwrap_err();
        assert_eq!(err.code(), "traffic_capability_lineage_unapproved");

        // A completely unknown lineage with a non-empty allowlist.
        let unknown = project(
            all_passed_checks(),
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        );
        let err =
            check_traffic_capability_with_lineage(&scenario, Some(&unknown), &[TEST_APPROVED_HASH])
                .unwrap_err();
        assert_eq!(err.code(), "traffic_capability_lineage_unknown");
    }

    #[test]
    fn gate_rejects_unbound_targets() {
        let scenario = traffic_scenario();
        let mut record = project(all_passed_checks(), TEST_APPROVED_HASH);
        record.target_kind = TRAFFIC_TARGET_KIND_REMOTE.to_string();
        let err =
            check_traffic_capability_with_lineage(&scenario, Some(&record), &[TEST_APPROVED_HASH])
                .unwrap_err();
        assert_eq!(err.code(), "traffic_capability_target_unbound");

        let mut record = project(all_passed_checks(), TEST_APPROVED_HASH);
        record.expected_network = "mainnet".to_string();
        let err =
            check_traffic_capability_with_lineage(&scenario, Some(&record), &[TEST_APPROVED_HASH])
                .unwrap_err();
        assert_eq!(err.code(), "traffic_capability_target_unbound");
    }
}
