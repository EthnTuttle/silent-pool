# 2026-11-08 - Traffic history journal contract v2 (T0/T1a alignment)

## Scope

Bounded alignment of the T0 traffic history-header contract to the
Fable-approved T1a v2 journal header
(`crates/spc-lab-txd/src/journal.rs`, `JournalRecord::Header`; approval
`docs/reviews/2026-11-08-traffic-wallet-fable.md` Part 4, r3), landed
BEFORE the first T1 runtime journal write. This supersedes the
`TrafficHistoryHeader` bullet of the "T1b integration boundaries"
section of `docs/decisions/2026-11-08-lab-traffic-contract-t0.md` (the
v1 header could not carry network, key mode, derivation version, caps
or the allocation digest; that flag is now resolved by this record).
The control protocol `spc-lab-tx-control/v1`, the scenario schema, the
AP-1a gate and every non-traffic contract are unchanged. No T1 actor
behavior, no probe execution, no node change.

## Decisions

1. **History version 2 is the first version any runtime writes
   (`TRAFFIC_HISTORY_VERSION = 2`)**: the pre-rework v1 shape (a
   4-field `{history_version, run_id, driver_id, protocol}` object
   mislabeling the CONTROL protocol string) was Fable-reviewed, never
   approved for a journal write and never run, so no v1 private
   journal exists and there is nothing to migrate - version 1 and any
   later version are REFUSED explicitly, never migrated, everywhere a
   history version is validated (scenario `traffic.protocol.
   history_version`, accepted hello-ack, and the header itself), with
   wording that names the refusal rather than a generic mismatch.
2. **`TrafficHistoryHeader` is the full v2 record mirror, not a
   projection**: the T0 type now carries the exact 14 fields of the
   T1a first line - `record` ("header"), `seq` (0),
   `format_magic` ("SPC-LAB-TRAFFIC-JOURNAL"), `history_version` (2),
   `protocol` (`spc-lab-tx-journal/2`), `network` ("regtest"),
   `key_mode` ("lab_deterministic_run_seed_v1"), `derivation_version`
   (2), `run_id`, `driver_id`, `run_seed_fingerprint` (64-hex),
   `allocation_digest` (64-hex), `caps`, `created_unix_ms` (u64) - in
   the core's declaration order, so `serde_json::to_string` of the T0
   type is byte-identical to the core's line (the serde-transparent
   `RunId`/`ActorId` serialize as plain strings). The type is
   `deny_unknown_fields` with no serde defaults anywhere: a missing or
   unknown field rejects at parse time.
3. **Two named protocols, never conflated**: the control protocol
   `spc-lab-tx-control/v1` (hello/ack, capabilities) keeps major
   version 1 and is unchanged; the JOURNAL protocol
   `spc-lab-tx-journal/2` is named by the new `TX_JOURNAL_*`
   constants and appears only in the journal header, never in a
   hello/ack exchange. The T0 constants mirror the T1a core constants
   (`SUPPORTED_HISTORY_VERSION`, `FORMAT_MAGIC`, `PROTOCOL`,
   `KEY_MODE`, `DERIVATION_VERSION`) and the conformance suite asserts
   the equality so the crates cannot drift apart silently.
4. **Caps mirror the core's `JournalCaps`** (`TrafficHistoryCaps`):
   `unknown_retry_sends` 0..=3 (retry SENDS from `unknown`; outcomes
   are always recordable) and `reorg_resubmits` 1..=3, the same
   numeric policy as the scenario `traffic.caps` fields
   (`max_unknown_retries`, `reorg_resubmit_max`) - the scenario
   declares them, the header persists them, the journal revalidates
   them on every reopen. Out-of-range values reject, never clamp.
5. **`allocation_digest` is the real validated-allocation digest**:
   exactly the canonical SHA-256 over the sorted-key JSON of the
   complete validated wallet-set allocation as the T1a
   `ProvisionConfig::validate_allocation` computes it - no second
   hash, no subset derivation, no caller-supplied raw string in T0.
6. **Strict consumer API and the binding boundary**:
   `TrafficHistoryHeader::from_journal_line` is the strict consumer
   entry (parse with `deny_unknown_fields` + full intrinsic
   validation). `validate()` validates the CONTRACT (record kind, seq
   0, magic, versions, protocol strings, network, key mode, typed id
   rules, 64-lowercase-hex digests, cap bounds), not the POSSESSION:
   binding the header against an expected run id / driver id /
   allocation digest / caps is the consumer's reopen concern, which
   the T1a core enforces on every reopen before any mutation. The
   conformance suite pins this boundary explicitly (a valid-but-
   foreign binding parses; the consumer comparison rejects it).
7. **New error codes**: `traffic_history_record_invalid` (record
   kind/seq/magic), `traffic_history_network_invalid`,
   `traffic_key_mode_unsupported` (key mode and derivation version),
   `traffic_run_id_invalid`, `traffic_history_digest_invalid`
   (64-lowercase-hex check), `traffic_caps_invalid`,
   `traffic_history_header_unparseable` (strict parse). The existing
   `traffic_protocol_unsupported` is retained for history-version and
   protocol-string mismatches.
8. **Header semantics are context, not proof**: the fields are
   binding and provenance metadata - a valid header never proves the
   capability gate, the funded state, or any mempool/chain outcome,
   and the public-seed key mode makes no secrecy claim (mode-0600
   files protect nothing the scenario does not already reveal).
9. **Unknown-field boundary, stated honestly**: the T0 consumer is
   STRICTER than the core writer - the core's `JournalRecord` does not
   declare `deny_unknown_fields` (a documented T1a design choice
   compensated by its reopen binding checks), while the T0 struct
   rejects unknown fields. This is an annotation choice, not a serde
   impossibility: serde 1.0.229 supports `deny_unknown_fields` on
   internally tagged enums, and this crate's `CommandKind` uses it;
   making the core stricter is permitted and would not change this
   contract. The conformance suite requires only the T0 rejection (it
   observes, but does not assert, the core's current acceptance, so a
   future hardened core cannot fail the suite) instead of claiming the
   core rejects unknowns.
10. **Conformance, not coupling**: `spc-lab` gains a dev-only
    dependency on `spc-lab-txd` for the conformance suite (a real
    journal is provisioned with the frozen fixture - seed
    `0x5452_4146_4649_4321`, run id `t1a-fixed-fixture`, driver
    `t1a-conformance` - and the actual first line is roundtripped
    byte-for-byte through the T0 contract, compared field-by-field
    against the core's `HeaderFacts`, the live `validate_allocation`
    digest and the `run_seed_fingerprint` computation, plus a
    hard-coded golden line and a fail-closed mutation table). No
    production path of `spc-lab` depends on the signer or the journal.

## Sections edited

- `crates/spc-lab/src/traffic/protocol.rs`: journal-protocol constants,
  `TRAFFIC_HISTORY_VERSION = 2`, the full `TrafficHistoryHeader` /
  `TrafficHistoryCaps` types with `from_journal_line` / `validate`,
  explicit v1-refused wording in the hello-ack validation.
- `crates/spc-lab/src/traffic/spec.rs`: `history_version` documented
  as the journal-protocol version; `validate_protocol` rejects any
  version other than 2.
- `crates/spc-lab/src/traffic/mod.rs`: re-exports of the new types and
  constants.
- `crates/spc-lab/src/traffic/projection.rs`: the cfg(test) traffic
  fixture carries `history_version` 2.
- `crates/spc-lab/tests/traffic_contract.rs`: traffic fixtures at
  history 2; the hello-ack and history-header contract tests rewritten
  for the 14-field v2 type (valid at 2, explicit 1/3 rejections,
  golden mutations for every field and error code).
- `crates/spc-lab/tests/traffic_history_conformance.rs` (new): the
  real-T1a-writer conformance suite described above.
- `crates/spc-lab/Cargo.toml`: the conformance-only dev-dependency on
  `spc-lab-txd`.

## Consequences

The scenario `traffic.protocol.history_version` is now 2, and every
traffic fixture carries 2. The safe-unavailable state is unchanged: no
traffic run can start (`traffic_capability_unrecorded` until a T1
runtime records an approved-lineage, recognized AP-1a projection; the
r1 evidence shape still blocks on `getmempoolinfo`), so no v1-shaped
journal was ever written and the v2-first gate closes a hole rather
than migrating data. T1b receives a complete header contract: the
adapter maps the T0 scenario fields onto the core's
`ProvisionConfig`, and the actor/funding/mempool consumer decisions
remain T1b's, over the frozen vocabulary and the mirrored constants
recorded here. `DESIGN.md`/`docs/SPEC.md` authority is preserved; no
pool, payout, SV2, SPc or Ark semantics changed.
