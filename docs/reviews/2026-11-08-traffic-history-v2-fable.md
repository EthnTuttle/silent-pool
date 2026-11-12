# Fable review - T0 traffic history journal contract v2 alignment (2026-11-08)

**Decision: REQUEST_CHANGES (minimal, wording-only; all code behavior is
correct and would be approved unchanged).** The v2 alignment does what the
T0 re-review (R1-3) required before the first T1 journal write: the T0
`TrafficHistoryHeader` is now the full 14-field mirror of the
Fable-approved T1a `JournalRecord::Header` line, byte-identical on
re-serialization, strict on parse, with every constant, id, digest and cap
bound re-validated; version 1 is refused everywhere a history version is
checked; the control protocol `spc-lab-tx-control/v1` is untouched and
never conflated with the journal protocol `spc-lab-tx-journal/2`; the
AP-1a gate, the scenario schema (beyond the version value) and the 15
overlaid T1a core files are byte-preserved; the conformance suite proves
the contract against a journal the real T1a writer produced.

One defect stops approval of this exact snapshot: the packet codifies, in
a binding decision record, in the T0 source comments and in a test's
rationale, the statement that internally tagged serde enums "cannot carry
`deny_unknown_fields` (a serde limitation)". That statement is false (F1
below, verified against the pinned `serde_derive` source and against this
very codebase), and a binding record must not teach T1/T1b that the core
cannot be made strict. The fix is prose in four places and one test
message; no code path, constant, digest or test outcome changes.

Nothing here activates an actor, approves T1b/T2/T3 or funded traffic,
decides the `getmempoolinfo` consumer, provides the T1 recording path, or
reopens the T1a core approval.

## 1. Scope and provenance

| item | result |
|---|---|
| Candidate | `/home/ethan/.local/share/opencode/worktree/ad8886/lab-traffic-history-v2`, git base `b0ab4ca`; untouched by me |
| Full manifest `/tmp/opencode/T0-history-v2-manifest-full.json` | 602 rows; sorted-key compact JSON `{base, files}` reproduces `dc2c865854d899a6fe013aa69c1a88b418798b650f8ad1ab4d60abf0019736a7`; all 602 on-disk sha256 values match (0 missing, 0 mismatched) |
| Code manifest | 601 rows = full minus `docs/reviews/2026-11-08-traffic-history-v2-implementation.md`; same recipe reproduces `df8cedb6cbd47cff3632d0774d85144467389d3071f28d2e9af1be316c5ed4b8` |
| Layering (`T0-history-v2-recipe.json`) | EXP-OP baseline manifest `ab174afe...` (583 files) + 15 `crates/spc-lab-txd/**` files from the T1a r3 manifest `6f3bb870...` (snapshot `65a16584...`, code `a8d28cea...` = the Part 4 APPROVE hashes in the primary `2026-11-08-traffic-wallet-fable.md`) + owned edits |
| Baseline preservation | 574 of 583 baseline files byte-identical; the 9 modified are exactly the recipe's owned scope: `Cargo.lock`, `Cargo.toml`, `crates/spc-lab/Cargo.toml`, `traffic/{mod,projection,protocol,spec}.rs`, `tests/traffic_contract.rs`, `docs/LAB-RUNTIME.md`. Their "old" hashes are the approved T0 r1 hashes from my previous re-review |
| T1a overlay | all 15 `spc-lab-txd` files byte-identical to the r3 manifest - reviewed as approved context, not as new code |
| Owned hunks verified by diff | root `Cargo.toml`: one member line; `Cargo.lock`: the `spc-lab-txd` package block plus one `spc-lab` dev-dep edge; `crates/spc-lab/Cargo.toml`: `[dev-dependencies] spc-lab-txd` only - no production dependency path from `spc-lab` to the signer/journal; `projection.rs`: the `cfg(test)` fixture `history_version` 1 -> 2 only (the AP-1a table, allowlist and gate are untouched); `mod.rs`: re-exports only; `LAB-RUNTIME.md`: one pointer sentence; `spec.rs`: doc comment + refusal wording (the check itself was already `!= TRAFFIC_HISTORY_VERSION`) |
| Logs | `/tmp/opencode/T0-history-v2-*`: `traffic_contract` 42/0, `traffic_history_conformance` 6/0, `spc-lab-txd` suites all green, fmt exit 0, clippy clean; log mtimes (10:01:49-10:02:02) post-date the last source edit (10:01:44, the rustfmt pass) and precede the freeze (10:03); the freeze log records the log hashes |
| Disk policy | 86 GiB free on `/`. My one attempted Cargo invocation (a 20-line serde probe into the shared `/tmp/opencode/T1a-target`) found `/tmp/opencode/silent-pool-heavy-build.lock` held by another session (`flock -n` failed); reported here once, not retried, no polling. I did not run the candidate's tests myself; the verification below is source-level plus the implementer's source-bound logs |
| ASCII | all new/changed Markdown and Rust pure ASCII |

I made no edits to the candidate, built nothing, started nothing. The only
file written is this one.

## 2. Confirmed sound

- **Mirror fidelity (source-verified)**: `JournalRecord::Header` in the
  overlaid `journal.rs:312-333` declares, under `#[serde(tag = "record",
  rename_all = "snake_case")]`, exactly `seq, format_magic,
  history_version, protocol, network, key_mode, derivation_version,
  run_id, driver_id, run_seed_fingerprint, allocation_digest, caps,
  created_unix_ms`; `JournalCaps` is `{unknown_retry_sends,
  reorg_resubmits}`. The T0 struct declares `record` first then the same
  13 in the same order with the same JSON types (`RunId`/`ActorId` are
  transparent strings); `TrafficHistoryCaps` matches. The constants
  `SUPPORTED_HISTORY_VERSION = 2`, `FORMAT_MAGIC`, `PROTOCOL =
  "spc-lab-tx-journal/2"`, `KEY_MODE = "lab_deterministic_run_seed_v1"`,
  `DERIVATION_VERSION = 2` equal the T0 constants, and
  `t0_constants_equal_the_t1a_core_constants` pins the equality.
- **Strictness**: `deny_unknown_fields`, no serde defaults on any header
  field, `from_journal_line` -> `validate()`; record kind `"header"`, `seq
  == 0`, magic, version 2, journal protocol string (the control string is
  an explicit rejection), `regtest` only, key mode and derivation version,
  `RunId::new`/`ActorId::new` re-validation (the derived `Deserialize`
  does not validate - correctly handled), 64 **lowercase** hex digests
  (uppercase/short/non-hex/padded all rejected), caps 0..=3 / 1..=3 with
  no clamping. The core's `validate_run_id` (`keys.rs:90-102`) uses the
  same 128-byte / same-charset rule as `ids::validate_id`, so no header
  the core can write is rejected by T0's id rules.
- **v1 refusal, no migration**: scenario `traffic.protocol.history_version`,
  accepted hello-ack and header all accept exactly 2; tests cover 1 and 3
  at each site; the refusal wording names the retired shape. No v1
  journal was ever written (the T0 gate still rejects every traffic
  scenario at `prepare`), so "refuse, never migrate" is the honest state.
- **Protocol separation**: `TX_CONTROL_*` unchanged (`spc-lab-tx-control`,
  major 1); `TX_JOURNAL_*` new and used only by the header; the
  conformance mutation "control protocol in the journal" rejects.
- **Conformance against the real writer**: `real_fixture` provisions a
  real `TrafficWalletSet` (synthetic fixed seed, empty journal, no
  funding - correctly not claimed as funded proof), reads the actual
  first line, parses it through the strict T0 consumer, re-serializes
  and asserts byte identity; compares all 14 facts against the core's
  `HeaderFacts`; recomputes `allocation_digest` via the live
  `ProvisionConfig::validate_allocation` and `run_seed_fingerprint` via
  `TrafficPrivateKey::run_seed_fingerprint`; the hard-coded golden
  digests `f804614a...`/`13e2ee5f...` are asserted equal to those live
  computations (no silent drift); 16 real-line mutations reject with the
  documented codes; the foreign-but-well-formed binding test states the
  shape-vs-possession boundary honestly (decision item 6) and shows the
  reopen-side comparison that rejects it - no producer string is blessed,
  and nothing claims authentication.
- **No EXP ACCESS confusion**: nothing touches command payload hashing or
  the `CommandLog`; the `allocation_digest` is the core's own canonical
  digest, not a T0 derivation.
- **Process**: one-bullet changelog fragment, a dated decision that
  supersedes the T0 "T1b boundaries" header bullet, a `LAB-RUNTIME.md`
  pointer; no SPEC body edits; `DESIGN.md`/`SPEC.md` authority unchanged.

## 3. Findings

### F1 (M, binding-record accuracy; wording only) - "internally tagged enums cannot carry `deny_unknown_fields`" is false and must not be codified

Where it appears: `crates/spc-lab/src/traffic/protocol.rs` module doc
(line 51, "strictness the core's internally-tagged enum cannot express")
and `TrafficHistoryHeader` doc (line 357, "cannot carry
`deny_unknown_fields` - a serde limitation the core documents");
`docs/decisions/2026-11-08-traffic-history-v2.md` item 9 ("a serde
limitation the core documents"); the implementation report Section 4;
`tests/traffic_history_conformance.rs` module doc (lines 22-26), the test
doc (lines 500-506) and the assertion message (line 519, "documented serde
limitation"; comment line 514, "lenient by serde construction, not by
choice").

Why it is false:

- `serde_derive 1.0.229` (the version in the candidate's `Cargo.lock`):
  `src/de/enum_internally.rs:98-103` dispatches a struct variant of an
  internally tagged enum to `struct_::deserialize(params, fields, cattrs,
  StructForm::InternallyTagged(..))`, and `src/de/struct_.rs:580-586`
  emits **no** ignore arm when `cattrs.deny_unknown_fields()` is set, so
  an unknown key becomes an `unknown_field` error for internally tagged
  variants exactly as for plain structs. The only documented
  incompatibility is `flatten`, which the core does not use.
- This codebase already relies on it: `crates/spc-lab/src/command.rs:53`
  declares `#[serde(tag = "type", rename_all = "snake_case",
  deny_unknown_fields)] pub enum CommandKind` - the very type the core's
  comment cites as sharing the supposed constraint.

The T1a core's own comment (`journal.rs:307-308`, `keys.rs:107-109`) is
the origin of the error. That comment is inside the approved r3 bytes and
is **out of scope here**: this packet does not need, and must not make, a
core change, and the core's leniency is a documented, compensated design
choice in an approved packet. What this packet must not do is re-state
the choice as an impossibility in a **binding** decision record and in a
regression test's rationale, because T1b would then believe the core
cannot be hardened.

Required fix (prose only; no behavior change, no new digests, no re-run
of the T1a suites needed):

1. `protocol.rs` lines 51 and 357: replace "cannot express / cannot carry
   ... a serde limitation" with "the core's `JournalRecord` does not
   declare `deny_unknown_fields` (a documented T1a design choice,
   compensated by its reopen binding checks); the T0 struct does".
2. Decision item 9: same correction; add one sentence that making the
   core stricter is permitted and would not change this contract.
3. Conformance test: keep the mandatory half (T0 refuses the extra
   field). For the core half, either drop the `core.is_ok()` assertion or
   relabel it as "observed behavior of the approved T1a r3 core; a
   stricter core is allowed and this assertion is to be removed when it
   lands" - a T0 test must not fail when T1 hardens the core.
4. Report Section 4: same wording.

### F2 (L, advisory) - reopen-time binding helper

`validate()` deliberately checks shape, not possession, and the decision
says binding is the consumer's reopen concern enforced by the core. That
is correct for T0. When T1 wires the controller side, it should compare
the header against the run's own `run_id`, the scenario's driver id, the
scenario-derived caps and the expected allocation digest **before** any
use of the journal, as the core does; a small `TrafficHistoryHeader::
bind(expected: &ExpectedBinding)` helper in T0 would give T1 one
fail-closed function instead of five ad hoc comparisons. Not required.

### F3 (L, advisory) - test-only coupling direction

`spc-lab` -> `spc-lab-txd` is a dev-dependency only (verified: production
`[dependencies]` unchanged, no `use spc_lab_txd` outside `tests/`). Note
for the integration owner: the root `Cargo.toml` member line and the
`Cargo.lock` block are byte-identical to the T1a r3 hunks, so the T1a
primary integration (already landed per the parent) and this packet
converge on the same lock content; only the `spc-lab` dev-dep edge is new.

## 4. Required before approval

F1 items 1-4 (prose and one test message/assertion). Everything else in
the snapshot is approved as-is: a resubmission that changes only those
texts, keeps all 42 + 6 tests green and preserves the 15 T1a bytes and the
574 baseline files is approvable on the manifest alone.

## 5. Explicit non-approvals

- No T1 actor activation, no T1 recording path, no T1b/T2/T3, no funded
  traffic, no bootstrap mining: `prepare` still rejects every traffic
  scenario with `traffic_capability_unrecorded`, and the r2 AP-1a
  evidence still blocks at `getmempoolinfo` for the selected modelling
  consumer - the T1 + canonical-owner decision remains open.
- The T1a core approval (snapshot `65a16584...`, code `a8d28cea...`) is
  not reopened; its leniency comment is noted as the source of F1 only.
- The conformance fixtures use a synthetic fixed seed and an empty
  journal; they prove the header contract, not funding, signing or any
  chain outcome.
- No node version/profile, Core compatibility or Ark readiness claim.

---
---

# Fable re-check r1 - F1 wording correction (manifest-only recheck, 2026-11-08, later)

**Decision: APPROVE - the bounded history-v2 alignment exactly as frozen by
`/tmp/opencode/T0-history-v2-r1-manifest-full.json` (full snapshot
`e2dc40c7d78527536998e7f9bd793273bbef4698fa700300e2d15f1c4c211859`, code
`4c1bce8c0d12193ae1243c1166e13812798906c2f17e6748aa0b9842993ffb84`).** F1
is closed; nothing else changed.

## R1-1. Freeze

| item | result |
|---|---|
| Full manifest | 602 rows; sorted-key compact `{base, files}` reproduces `e2dc40c7...`; code recipe (full minus the implementation report) reproduces `4c1bce8c...`; all 602 on-disk sha256 values match |
| Delta vs r0 (`dc2c8658...`) | no rows added or removed; exactly 4 changed: `crates/spc-lab/src/traffic/protocol.rs`, `crates/spc-lab/tests/traffic_history_conformance.rs`, `docs/decisions/2026-11-08-traffic-history-v2.md`, `docs/reviews/2026-11-08-traffic-history-v2-implementation.md` |
| T1a overlay | all 15 `crates/spc-lab-txd/**` files still byte-identical to the r3 manifest; changelog fragment, `LAB-RUNTIME.md`, `spec.rs`, `mod.rs`, `projection.rs`, `traffic_contract.rs`, root/crate `Cargo.toml`, `Cargo.lock` unchanged from r0 |
| Code-vs-comment check | comment-stripped diff of `protocol.rs` against the EXP-OP baseline removes/adds exactly the same code lines as the r0 diff did (the retired v1 fields and `validate` arms out; the v2 constants, `TrafficHistoryCaps`, `from_journal_line`, `validate`, `validate_digest_hex64` in) - the r1 change is doc comments only. No type, constant, digest, serialization or validation token moved |
| Builds/tests | none run by me and none claimed by the packet for r1 (the r0 logs stand as historical evidence for the unchanged code); no lock taken, nothing written into the candidate |
| ASCII | the four changed files are pure ASCII |

## R1-2. F1 disposition (verified text)

- `protocol.rs` module doc and `TrafficHistoryHeader` doc now say the
  core's `JournalRecord` "does not declare `deny_unknown_fields` (a
  documented T1a design choice, compensated by its reopen binding checks -
  not a serde impossibility: serde 1.0.229 supports `deny_unknown_fields`
  on internally tagged enums, and this crate's `CommandKind` uses it)".
  Accurate against `serde_derive 1.0.229` (`enum_internally.rs:98` ->
  `struct_.rs:580`) and `command.rs:53`.
- Decision item 9: same correction, plus "making the core stricter is
  permitted and would not change this contract" and that the suite only
  observes the core's current acceptance.
- Conformance test renamed `unknown_field_rejection_is_required_of_the_t0_consumer`;
  the core parse result is bound with `let _ = core;` (observed, not
  asserted); the T0 `traffic_history_header_unparseable` rejection remains
  mandatory. A future hardened core cannot fail this suite.
- Report Section 8 records the r1 revision honestly, including that the
  original claim was false.
- No occurrence of "serde limitation", "cannot carry" or "cannot express"
  remains in the owned files except the report's own description of the
  corrected error.

## R1-3. Scope reminder (unchanged)

Approved: the T0 history-v2 header contract as the full mirror of the
approved T1a header, the v1 refusal at scenario/ack/header, the separate
journal protocol constants, the caps mirror, the strict consumer API, the
conformance suite against the real writer, the dev-only dependency, and the
recorded decision/changelog/pointer. Not approved or implied: T1 actor
activation or recording path, T1b/T2/T3, funded traffic, the
`getmempoolinfo` consumer decision, any node profile or Ark readiness, or a
reopening of the T1a core approval (`65a16584...` / `a8d28cea...`).
