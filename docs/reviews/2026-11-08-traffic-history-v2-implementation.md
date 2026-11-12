# 2026-11-08 - T0 traffic history-version-2 alignment: implementation report

Packet: `T0 traffic-history-v2` (bounded, mandated before the first T1
runtime journal write).
Worktree: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-traffic-history-v2`
(base commit `b0ab4ca34fda07c61a6335d05f598eaacc69678c`, nothing committed).
Decision record: `docs/decisions/2026-11-08-traffic-history-v2.md`.
Changelog fragment: `docs/changelog/2026-11-08-traffic-history-v2.md`.

## 1. What was implemented

The T0 history-header contract is aligned to the Fable-approved T1a v2
journal header (`crates/spc-lab-txd`, review Part 4 r3 APPROVE at
`docs/reviews/2026-11-08-traffic-wallet-fable.md`), before any runtime
ever writes a journal line:

- `TRAFFIC_HISTORY_VERSION = 2` everywhere a history version is
  validated (scenario spec, accepted hello-ack, header), with the
  never-written v1 shape refused explicitly and never migrated.
- `TrafficHistoryHeader` is redefined as the FULL 14-field v2 record
  mirror - `record`/`seq`/`format_magic`/`history_version`/`protocol`/
  `network`/`key_mode`/`derivation_version`/`run_id`/`driver_id`/
  `run_seed_fingerprint`/`allocation_digest`/`caps`/`created_unix_ms` -
  in the core's declaration order, `deny_unknown_fields`, no serde
  defaults. Re-serialization is byte-identical to the T1a first line
  (proven below against the real writer, not a replica).
- The JOURNAL protocol `spc-lab-tx-journal/2` is named separately from
  the unchanged CONTROL protocol `spc-lab-tx-control/v1` (new
  `TX_JOURNAL_*` constants); the journal protocol appears only in the
  journal header, never in a hello/ack exchange.
- `TrafficHistoryCaps` mirrors the core's `JournalCaps` policy
  (`unknown_retry_sends` 0..=3 retry SENDS - outcomes always
  recordable; `reorg_resubmits` 1..=3), the same numbers as the
  scenario `traffic.caps` fields.
- New strict consumer API `TrafficHistoryHeader::from_journal_line`
  (parse + intrinsic validation) with the documented binding boundary:
  `validate()` checks the contract, the consumer's reopen checks the
  possession (the T1a core enforces that on every reopen).
- New error codes `traffic_history_record_invalid`,
  `traffic_history_network_invalid`, `traffic_key_mode_unsupported`,
  `traffic_run_id_invalid`, `traffic_history_digest_invalid`,
  `traffic_caps_invalid`, `traffic_history_header_unparseable`;
  `traffic_protocol_unsupported` retained for version/protocol with
  explicit v1-refused wording.
- `allocation_digest` is defined as exactly the T1a
  `ProvisionConfig::validate_allocation` canonical digest - no second
  hash, no subset derivation in T0.

Not changed: the AP-1a gate (still fail-closed
`traffic_capability_unrecorded` / lineage-unknown / the r1
`getmempoolinfo` failure preserved - no premature traffic readiness),
the control protocol, the scenario schema beyond the history-version
value, every non-traffic contract, and the whole T1a core (byte-proof
below). This is not an EXP ACCESS command-hash change.

## 2. Layered source recipe and preservation proofs

The recipe was recorded BEFORE any edit at
`/tmp/opencode/T0-history-v2-recipe.json`:
git `b0ab4ca` + the EXP-OP baseline overlay (583 files, manifest
`/tmp/opencode/EXP-OP-baseline-manifest.json`, sha256 recorded in the
frozen full manifest) + ONLY the 15 `crates/spc-lab-txd/**` files of
the Fable-approved T1a r3 snapshot (manifest
`/tmp/opencode/T1a-fix-r3-manifest-full.json`, full snapshot
`65a16584...`, code snapshot `a8d28cea...`) + the owned edits below. No
new full baseline copy was made.

Preservation proofs (recomputed by the freeze script, fail-closed):

- all 15 T1a core files are byte-identical to the r3 manifest rows
  (the authoritative core is untouched);
- all 574 baseline files OUTSIDE the owned edit scope are
  byte-identical to the EXP-OP baseline layer (AP3 edges preserved);
- the full-tree delta vs the baseline layer is exactly the owned
  surface + the 15 T1a overlay files + this packet's 4 new files.

## 3. Owned edit surface

Modified (baseline-layer files):

- `crates/spc-lab/src/traffic/protocol.rs` - the contract (constants,
  `TrafficHistoryHeader`/`TrafficHistoryCaps`, `from_journal_line`,
  `validate`, hello-ack v1-refused wording).
- `crates/spc-lab/src/traffic/spec.rs` - `history_version` semantics +
  `validate_protocol` v1/other rejection.
- `crates/spc-lab/src/traffic/mod.rs` - re-exports only.
- `crates/spc-lab/src/traffic/projection.rs` - the cfg(test) fixture's
  `history_version` 1 -> 2 only.
- `crates/spc-lab/tests/traffic_contract.rs` - fixtures at history 2;
  `traffic_protocol_rejections`, `hello_ack_contract` and
  `history_header_contract` rewritten for the v2 type (every field and
  error code covered).
- `crates/spc-lab/Cargo.toml` - dev-dependencies section only
  (conformance-only `spc-lab-txd` dev-dep; no production coupling).
- `Cargo.toml` - the `crates/spc-lab-txd` member line only.
- `Cargo.lock` - the txd package block (byte-identical to the T1a
  worktree's block) + the single `"spc-lab-txd"` dev-dep edge under
  the `spc-lab` package, added by cargo on the first test run; the
  diff vs the baseline lock is exactly those two hunks.
- `docs/LAB-RUNTIME.md` - the traffic pointer paragraph only.

New files:

- `crates/spc-lab/tests/traffic_history_conformance.rs` - the
  conformance suite (below).
- `docs/decisions/2026-11-08-traffic-history-v2.md` (normative).
- `docs/changelog/2026-11-08-traffic-history-v2.md` (single-bullet
  fragment per `docs/changelog/README.md`).
- this report.

Deliberately NOT overlaid from T1a: the T1a docs (decision, fragment,
report) and T1a's `bins/`/`spc-lab`-owned files - the parent session
integrates the T1a packet separately; this packet only mirrors its
journal-header contract.

## 4. Conformance suite (real writer, not a replica)

`crates/spc-lab/tests/traffic_history_conformance.rs` provisions a REAL
journal with the frozen fixture (`ProvisionConfig::standard` with seed
`0x5452_4146_4649_4321`, run id `t1a-fixed-fixture`, driver
`t1a-conformance`, via `provision_for_conformance_tests`) and proves:

- `t0_constants_equal_the_t1a_core_constants`: `TRAFFIC_HISTORY_VERSION
  == SUPPORTED_HISTORY_VERSION`, `TX_JOURNAL_FORMAT_MAGIC ==
  FORMAT_MAGIC`, `TX_JOURNAL_PROTOCOL_STRING == PROTOCOL`,
  `TRAFFIC_KEY_MODE == KEY_MODE`, `TRAFFIC_KEY_DERIVATION_VERSION ==
  DERIVATION_VERSION`, and the default caps satisfy the T0 bounds.
- `real_t1a_first_line_roundtrips_through_the_t0_contract`: the actual
  first line of `private/traffic/history.jsonl` parses through the
  strict T0 consumer, re-serializes BYTE-IDENTICAL, carries exactly
  the 14 contract fields, and every typed fact equals the core's own
  `HeaderFacts` view, the live `validate_allocation` digest and the
  `TrafficPrivateKey::run_seed_fingerprint` computation.
- `golden_known_line_is_accepted_and_reproduced_byte_for_byte`: a
  fully hard-coded golden line (allocation digest
  `f804614a49714ccd348854b3b6d4261d518e011a3a8ce1d55255dbb123bf4978`,
  run-seed fingerprint
  `13e2ee5f45f3cb4fa7018afb0e6a11d7286288a93112d484728008ea712f96da`,
  fixed `created_unix_ms`) roundtrips losslessly, and the digests are
  cross-checked against the LIVE core computations so the constants
  cannot drift.
- `real_line_mutations_are_rejected_fail_closed`: 16 mutations of the
  real line (record kind, seq, magic, history 1 and 3, control
  protocol in the journal, network, key mode, derivation version, ids,
  uppercase/short/non-hex digests, both cap bounds, missing field,
  unknown field) each reject with the documented code.
- `a_valid_but_foreign_binding_parses_and_is_the_consumers_rejection`:
  pins the honest boundary - a valid-but-different run/driver/digest
  PARSES (the contract is shape, not possession); the reopen-side
  comparison is what rejects it, which the T1a core enforces.
- `unknown_field_rejection_is_required_of_the_t0_consumer`: the core's
  `JournalRecord` currently accepts an extra field (its chosen serde
  annotations do not declare `deny_unknown_fields` - a documented T1a
  design choice compensated by its reopen binding checks, not a serde
  impossibility), while the T0 strict consumer refuses the same line.
  Only the T0 rejection is asserted; the core's current acceptance is
  observed but not asserted, so a future hardened core cannot fail the
  suite.

## 5. Verification evidence

All commands ran in the worktree under the nonblocking heavy-build
lock with `CARGO_TARGET_DIR=/tmp/opencode/T1a-target` (reused, pinned
binaries untouched); 87-89 GiB free before each run (above the 50 GiB
floor). One earlier attempt was blocked by another session's live Nix
build holding the lock (reported once, work continued, no polling).

| Check | Command | Result |
|---|---|---|
| Contract tests | `cargo test -p spc-lab --test traffic_contract` | 42 passed, 0 failed (exit 0) |
| Conformance | `cargo test -p spc-lab --test traffic_history_conformance` | 6 passed, 0 failed (exit 0) |
| T1a core in this tree | `cargo test -p spc-lab-txd` | all suites passed (exit 0) |
| Format | `cargo fmt --check` | exit 0 (after rustfmt on the three owned files) |
| Lint | `cargo clippy -p spc-lab --all-targets` | exit 0, no warnings |

Logs: `/tmp/opencode/T0-history-v2-test-traffic.log`,
`/tmp/opencode/T0-history-v2-test-txd.log`,
`/tmp/opencode/T0-history-v2-fmt-check.log`,
`/tmp/opencode/T0-history-v2-clippy.log` (hashes printed by the freeze
into `/tmp/opencode/T0-history-v2-freeze.log`).

Two test-authoring fixes were needed during verification (both in
packet-owned test files, no production change): a 62-character hex
stand-in corrected to 64, and a field-set assertion corrected to
compare sorted sets (a `serde_json::Value` parse sorts keys; the field
ORDER is proven by the byte-identity assertion).

## 6. Frozen manifests

- `/tmp/opencode/T0-history-v2-manifest-full.json` - the full layered
  tree (SHA256 sorted-key compact JSON `{base, files}`, files sorted by
  path; same recipe as the T1a freezes), the baseline delta rows and
  the fail-closed preservation checks of section 2.
- `/tmp/opencode/T0-history-v2-manifest-code.json` - the code snapshot
  (excludes only this report).
- `/tmp/opencode/T0-history-v2-manifest-delta.json` - the authoritative
  baseline->packet hash delta (the baseline layer was never committed,
  so this - not a git patch - is the record of what changed over the
  layered input).
- `/tmp/opencode/T0-history-v2-recipe.json` - the pre-edit layering
  recipe.
- `/tmp/opencode/T0-history-v2-freeze.py` - the freeze script itself
  (rerunnable; fails closed on any surface or preservation drift).

## 7. Remaining decisions (T1b handoff, unchanged by this packet)

The actor/funding/mempool consumer decisions remain T1b's over the now
complete header contract: the adapter mapping of T0 scenario fields
onto `ProvisionConfig`, the deterministic disjoint per-actor allocation
over the frozen seed-domain strings, the funding observation flow, and
the mempool/chain consumer of the lifecycle vocabulary. The safe-
unavailable gate state is unchanged and intentional.

## 8. r1 revision (Fable F1: wording-only)

Fable's REQUEST_CHANGES review (primary repo,
`docs/reviews/2026-11-08-traffic-history-v2-fable.md`) approved all
code behavior unchanged and required one wording-only fix (F1): four
owned sites stated the T1 core's internally tagged `JournalRecord`
cannot carry `deny_unknown_fields` as a serde limitation. That claim
is false - serde 1.0.229 supports `deny_unknown_fields` on internally
tagged enums, and this crate's `CommandKind`
(`crates/spc-lab/src/command.rs`) uses it. The honest statement: the
T1 core's CURRENT chosen serde annotations accept unknown fields (a
documented T1a design choice, compensated by its reopen binding
checks); making the core stricter is permitted and would not change
the T0 contract.

Touched files (wording only - no code path, serde attribute, digest,
header contract or behavior change):

- `crates/spc-lab/src/traffic/protocol.rs` - two doc comments
  (`TRAFFIC_HISTORY_VERSION`, `TrafficHistoryHeader`).
- `docs/decisions/2026-11-08-traffic-history-v2.md` - item 9.
- `crates/spc-lab/tests/traffic_history_conformance.rs` - module doc;
  the boundary test renamed to
  `unknown_field_rejection_is_required_of_the_t0_consumer` with its
  doc and comment rewritten; and the core-acceptance `assert!`
  replaced by a non-asserting observation (`let _ = ...`), so only the
  T0 unknown-field rejection is required of the suite and a future
  hardened core cannot fail it. No import or behavior change.
- this report - the section 4 bullet corrected and this section added.

The changelog fragment needed no change (it contains no serde claim).
The same false comment in the T1a core files (`keys.rs:107`,
`journal.rs:307`, `adapter.rs`) is out of scope: those are the
Fable-approved r3 bytes, byte-preserved below.

No new test run: the r1 changes are comment/doc text plus replacing an
assertion with a non-asserting binding (which cannot fail and emits no
warning); the reviewer permits approval on the manifest alone.
Section 5's evidence is the historical pre-r1 run and is unchanged.

r1 manifests (same recipe; the r0 manifests of section 6 remain the
historical pre-r1 freeze, untouched):

- `/tmp/opencode/T0-history-v2-r1-manifest-full.json` - the full
  layered tree after r1.
- `/tmp/opencode/T0-history-v2-r1-manifest-code.json` - the code
  snapshot after r1 (excludes only this report).
- `/tmp/opencode/T0-history-v2-r1-manifest-delta.json` - the precise
  4-file delta vs the r0 full manifest (the r1 freeze fails closed if
  anything else moved).
- `/tmp/opencode/T0-history-v2-r1-freeze.py` - the r1 freeze script
  (rerunnable; re-proves the 15 T1a overlay files byte-identical to
  the r3 manifest and every out-of-scope baseline file preserved).
