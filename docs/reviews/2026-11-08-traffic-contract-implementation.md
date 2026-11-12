# T0 traffic contract implementation review (2026-11-08)

> **2026-11-08, later - r2 rework applied.** Sections 1-9 below describe the
> original T0 packet. The Fable review
> `docs/reviews/2026-11-08-traffic-contract-fable.md` (REQUEST_CHANGES,
> F1-F8) was answered in this worktree; Section 10 records the dispositions,
> the dated corrections supersede the affected statements below, and the
> verification numbers in Section 9 are superseded by Section 10.6. The
> AP-1a r2 contract is now Fable-APPROVED and bound (Section 10.2).

Worktree: `lab-traffic-contract` (base `b0ab4ca`, frozen 196-file lab-final-integration
runtime delta copied in and verified; all changes uncommitted by design).
Scope owned: packet T0 of the lab transaction traffic plan - the frozen traffic
schema/validation/versioned contract and the AP-1a traffic capability projection ONLY.

Plan baseline: `docs/plans/2026-11-07-lab-transaction-traffic.md` frozen copy at
`/tmp/opencode/traffic-plan-approved-r2/plan.md`, sha256
`d64a423570509f0974f3853453942e530ec0e21e0c5c3d1904d4d1427022ad18`
(parent receipt: APPROVE-plan under peer R1/R2 correction condition; review
`5cd97e198672864eafde95d45c31ed48e0f5716b199a2e58d48937a94a0067e7`).
Sections 5, 6, 8, 11, 12 and 14 (T0) are normative for this packet.

## 1. What was implemented

New packet-owned module `crates/spc-lab/src/traffic/` (nothing else in the crate
layout changed ownership):

- `spec.rs` - the complete frozen `TrafficSpec` scenario schema (plan Section 11):
  structs, serde attributes, defaults, ranges, cross-field validation, error codes.
- `protocol.rs` - the `spc-lab-tx-control/v1` hello/hello-ack contract, the
  versioned private history record contract (`history_version` 1), the transaction
  lifecycle state vocabulary (plan Section 6.5), and the deterministic seed
  derivation domains (plan Sections 5 and 6.2).
- `commands.rs` - typed durable command payloads for the plan Section 12
  `CommandKind` additions (T1 wires them into `command.rs`).
- `projection.rs` - the AP-1a traffic capability projection (plan Section 8) and
  the scenario-side preflight gate `check_traffic_capability`.
- `mod.rs` - module registry, re-exports and the `TrafficError` type.

New integration test suite `crates/spc-lab/tests/traffic_contract.rs` (34 tests).

NOT implemented (deliberately, per plan Section 14 T0): the tx driver actor, the
wallet/signer, the reservation ledger, bootstrap mining (T2-L), any process
launch, any RPC route addition, the engine preflight call, the `CommandKind`
enum additions, UI work, and any probe execution. No commits were made.

## 2. The frozen `TrafficSpec` schema

Additive at `scenario_version` 1: `Scenario` gains `traffic:
Option<TrafficSpec>` (serde default, `deny_unknown_fields` inside). Existing
scenarios parse and validate unchanged (regression suite green). An old runner
rejects a traffic-bearing scenario at parse (see Section 6).

Fields, defaults and ranges (all validated before any process starts; bounds
are exported constants in `spec.rs`):

| field | rule |
|---|---|
| `version` | exactly 1 (`traffic_version_unsupported`) |
| `required_capability` | must equal `spc.node.regtest.mempool_rpc.v1` (`traffic_capability_missing` / `traffic_capability_unknown`) |
| `protocol.control` | exactly `spc-lab-tx-control/v1` (`traffic_protocol_unsupported`; unknown major fails pre-broadcast) |
| `protocol.history_version` | exactly 1 (`traffic_protocol_unsupported`) |
| `bootstrap.enabled` | must be true in v1 (`traffic_bootstrap_required`; no alternative funding source is declared) |
| `bootstrap.mining_mode` | default `native`; `rpc_generate` rejected (`traffic_bootstrap_mining_mode_invalid`; no proof record mechanism exists in v1) |
| `bootstrap.funding_blocks` | 101..=1000 (`bootstrap_maturity_insufficient` below 101; `traffic_bootstrap_funding_blocks_out_of_range` above) |
| `bootstrap.distribution_outputs_per_wallet` | 1..=64 (`traffic_bootstrap_distribution_invalid`) |
| `bootstrap.excluded_from_statistics` | must be true (`traffic_bootstrap_statistics_not_excluded`) |
| `driver.actors` | 1..=2 (`traffic_actor_count_invalid`); ids non-empty and distinct |
| `driver.actors[].network_wallets` | 1..=256 per actor and in total (`traffic_actor_wallet_overflow`; defensive ceiling above the stress preset's 100) |
| `driver.user_payment_wallets` | >= 1 and equal to the scenario identity count (`traffic_user_payment_wallets_invalid` / `traffic_user_payment_wallets_mismatch`) |
| `driver.history` | non-empty relative path, no `.`, `..`, empty or backslash components (`traffic_history_path_invalid`) |
| `rate.ordinary_tx_per_sec` | 0.01..=20.0 (`traffic_rate_bounds_invalid` / `traffic_rate_exceeds_preset_cap`) |
| `rate.pacing` | default `fixed_jitter`; `burst` is T6-gated (`traffic_pacing_unsupported`) |
| `fees.rate_sats_vb` | 1..=100 with min <= max (`traffic_fee_bounds_invalid`) |
| `fees.max_fee_sats_per_tx` | 1..=100000 (`traffic_fee_bounds_invalid`) |
| `sizes.classes_vb` | 1..=64 classes, each `vb` in 1..=max_vb and `mix` in (0,1], `sum(mix) == 1` within 1e-9 (`traffic_size_mix_invalid` / `traffic_size_cap_invalid`) |
| `sizes.max_vb` | 1..=100000 (`traffic_size_cap_invalid`) |
| `spend_types` | non-empty, no duplicates; v1 accepts only `p2pkh` (`traffic_spend_type_missing` / `traffic_spend_type_duplicate` / `traffic_spend_type_gated` for `p2wpkh` until T4) |
| `ark.registration_before_mining` | must be true (`traffic_ark_registration_invalid`) |
| `ark.chain_effect_budget_tx_per_sec` | 0 when disabled, (0, 1.0] when enabled (`traffic_ark_budget_invalid`) |
| admission | `ordinary + ark budget <= 20.0` tx/s (`traffic_admission_overlapping_streams`; user payments are manual and counted at runtime) |
| `caps.*` | inflight 1..=64, unknown retries 0..=3, backlog 1..=2000, reorg resubmit 1..=3 with declared default 3 (`traffic_caps_invalid`) |
| `pause_semantics` | only `no_broadcast_after_ack` (serde-enforced single variant) |

Decimal fractions (`mix` sums, admission sums) compare as f64 with a 1e-9
tolerance, documented in `FRACTION_EPSILON`: these are configuration, not
money; the plan's exact-decimal requirement governs BTC amounts in T1.

On successful validation the traffic capability
`spc.node.regtest.mempool_rpc.v1` is inserted into
`ValidationReport.required_capabilities` (same discipline as
`spc.node.regtest.pow_baseline`).

## 3. Protocol, history, lifecycle, seed domains

- `TX_CONTROL_PROTOCOL_NAME = "spc-lab-tx-control"`, major 1 (minor 0). The
  scenario string form is exactly `spc-lab-tx-control/v1`; `parse_control_protocol`
  accepts name + `/v<major>` only. An unknown major version fails the hello
  exchange before any broadcast.
- `TxControlHello` (protocol name/major/minor, driver id, network and
  user-payment wallet counts, advertised capabilities) and `TxControlHelloAck`
  (accepted + history_version, or rejected + reason) are fully typed with
  `deny_unknown_fields` and `validate()`.
- **The v1 capability registry is frozen EMPTY** (`TX_CONTROL_CAPABILITIES_V1`):
  a driver advertising a capability the registry does not know never makes the
  controller rely on it (`supports()` requires both registry knowledge and
  advertisement), so no capability is falsely advertised. T1a extends the
  registry alongside its implementation.
- `TrafficHistoryHeader` carries `history_version` 1 (the private journal
  contract); `TRAFFIC_HISTORY_VERSION` is the only accepted value.
- `TxLifecycleState` is the full Section 6.5 vocabulary (created,
  broadcast_sent, accepted_by_rpc, observed_in_mempool, confirmed, rejected,
  conflicted, unknown, reorged, invalidated, dropped, expired) with snake_case
  serde mapping and terminality: `rejected` and `conflicted` are terminal;
  `confirmed` is not (canonical displacement reconciles it).
- Seed derivation domains (exact strings, plan Sections 5/6.2):
  `spc-lab/traffic/bootstrap-funding`,
  `spc-lab/traffic/network-wallet/<i>`,
  `spc-lab/traffic/user-payment-wallet/<i>`.

## 4. Durable command payloads (typed, behavior-free)

`PauseTxDriver`, `ResumeTxDriver`, `SetTxRate`, `SendUserPayment`,
`BootstrapFund`, `RpcSignedTxPayload` (the `inject_tx` adapter `"rpc-signed"`
payload). Each is `deny_unknown_fields` with a fixed field order (so the
existing `CommandLog` payload-hash idempotency/conflict semantics apply
unchanged) and carries a `validate()` with stable codes. Bounds mirror the
frozen spec (rate cap, fee band, non-empty ids, even-length hex). Full
consensus decode of the injected transaction is T1's domain; here it is a
fail-closed hex shape check only.

## 5. AP-1a traffic capability projection (the mapping boundary)

One canonical probe executor: `ap1a-ark-rpc-probe`, report schema v2. T0 runs
no competing probe and creates no second verdict. The projection
(`project_traffic_capability`) maps exact report bytes into a
`TrafficCapabilityRecord` stored in the run manifest (new optional
`traffic_capability` field); `check_traffic_capability` gates scenarios
(scenario declaring traffic without a recorded, lineage-consistent, satisfied
capability is rejected before any process starts).

> **2026-11-08, later - superseded by the r2 rework (Section 10).** The
> single `satisfied` verdict no longer exists: the record now carries the
> two-stage `recognized` / `behavior_proven` verdict with full per-check
> rows, and the gate is wired into the engine (Section 10.1, 10.3). The
> rules below that survive unchanged are the as-is status mapping, the
> foundational load-bearing rows, the Bark-version-floor isolation and the
> observations carry-through.

Rules, all tested:

- The five traffic methods (plan Section 8) map from their check rows in
  `report.checks` by method; the 12 r1 check ids are stable contract
  identifiers (sendrawtransaction.recognized; getrawmempool +
  bool-form; getrawtransaction.verbosity0/verbosity1/bool-verbose/
  bool-verbose.async; getmempoolinfo; getblock.verbosity0/
  verbosity0.rehash/verbosity1/singleparam).
- Load-bearing foundational rows (by id): `protocol.id-echo`,
  `protocol.envelope20.getnetworkinfo`, `getblockchaininfo.network`,
  `getblockhash.genesis`, `getblockheader.verbose.genesis`. Any non-passed
  state on these fails the capability closed even when all five traffic
  methods pass, and independently of the Bark version floor.
- The Bark version floor (`getnetworkinfo.gate.*`) never affects the
  projection: an honest `fail-version-gate` coexists with a usable traffic
  surface.
- Statuses map as-is (`passed|failed|unknown|not_run` on the two axes).
  Projected availability != `passed` is blocking; behavior `unknown`/`not_run`
  can never produce a funded pass. Per-method/per-row merge severity:
  failed > unknown > not_run > passed; a method with no rows is `not_run`,
  never silently passed. A future check on a traffic method joins the
  projection and fails closed on disagreement.
- The report's own verdict is the BARK verdict: recorded as-is
  (`status`/`funded_state`/`full_acceptance`), never re-derived, never a
  traffic verdict.
- Relay/fee-policy facts (`report.observations`, e.g. `mempoolinfo.*`,
  `mempool_size`) are carried as-is so preset fee floors are grounded.
- Fail-closed lineage: wrong probe id, wrong report schema version, duplicate
  check ids, malformed contract hash, unknown status values on consulted
  rows, missing required report fields, and read-only verdict invariant
  violations (`funded_state`/`full_acceptance`) are all hard errors.
- The record itself is tamper-checked at gate time: schema version, probe id,
  exact method/foundational key sets, and `satisfied` must match a
  recomputation from its rows.

### 5.1 Candidate contract hash - gated, NOT approved

> **2026-11-08, later - superseded: the r2 contract is APPROVED and bound.**
> The Fable re-review r2 approved the exact contract
> `f4fc3e043f420870675095ab24144ba14e0afba6cb9ad69d818d99b1bccb2f62`; the
> production allowlist now holds exactly that hash, the r1 candidate stays
> recognized-unapproved diagnostics, and the binding is recorded in
> `docs/decisions/2026-11-08-traffic-ap1a-lineage-r2-approved.md`
> (Section 10.2). The paragraphs below are the pre-approval state.

`AP1A_CONTRACT_SHA256_CANDIDATE` is the r1 candidate
`11bdb1007e130a749720f87482f287c1fb65d1e743c142fc4b1ca15a782d89e7`.
The Fable review has NOT approved it: two blockers (HTTP 500/404 response-id
mismatch propagation; pinned request forms - trailing nulls, active Bark
package async 1.0 forms) are being corrected in r2, and the contract mapping
must await the new reviewed hash. The projection therefore fail-closes on any
report lineage that disagrees with the constant, and the constant is updated
only through a recorded decision when the reviewed r2 hash lands. The
v1-era `48894548...` and r1-draft `905bf4b8...` hashes are never treated as
approved. (The Fable review primary, `docs/reviews/2026-11-08-ark-rpc-probe-fable.md`
in the AP-1a worktree, was not present in that tree at T0 time - consistent
with the r1 doc's own provenance disclosure - so this packet works from the
parent-relayed findings.)

### 5.2 getmempoolinfo `maxdatacarriersize` - preserved failure, pending decision

The r1 evidence records `getmempoolinfo` as availability `passed` / behavior
`failed` (shape) on both approved binaries: `maxdatacarriersize` is JSON
`null` where the selected corepc v30 `GetMempoolInfo` deserializer (13
required fields, no Option) requires u64. Fable confirms the failure is REAL
for that selected consumer but NOT a universal traffic failure and NOT a Bark
requirement (bark c419159 never calls the method). This projection preserves
the failure as-is - there is no null-to-zero coercion anywhere - so the
traffic capability is honestly unsatisfied on those binaries today. Combined
with `sendrawtransaction.recognized` behavior `not_run` (read-only mode,
invalid sample proves recognition only) and the getrawtransaction
genesis-unknowns, a satisfied capability is currently unattainable from any
read-only AP-1a report - exactly the plan Section 16 item 1 expectation.

**RpcNodeControl consumer statement (requested by the parent)**: T0 itself
introduces no getmempoolinfo consumer - the projection consumes check states
and the observations object only, and needs no v30 decoder. Whether the T1
runtime's controller-mediated `RpcNodeControl` route needs the full corepc
v30 13-field superset, or a deliberately reduced traffic-subset decoder, is a
pending decision for T1 and the canonical owner: a source-grounded
traffic-subset extension must stay with the AP-1a canonical owner (Fable
rework), never a second probe in spc-lab, and the node must not be patched to
launder the failure. T0 records the decision as pending; it does not make
irrelevant Bark/Core policy fields traffic requirements.

## 6. Old-runner boundary (honest report)

The frozen baseline `Scenario` (pre-T0) carries `#[serde(deny_unknown_fields)]`
with no `traffic` field, and `parse_scenario` checks `scenario_version` before
strict deserialization. A traffic-bearing scenario therefore fails at parse
with `scenario_schema_invalid` / "unknown field `traffic`" on any old runner -
before validation and before any process starts. The test suite proves this
mechanically with a local mirror struct replicating the pre-T0 top-level field
set (deny_unknown_fields, nested fields as values). **No actual old-binary run
was performed** (it would require a parallel baseline build); the property is
fully determined by the baseline source, which the mirror struct reproduces.

## 7. Merge notes (serial integration owner)

Applied hunks against the frozen lab-final-integration baseline (all minimal;
the complete worktree delta vs the frozen 370-file manifest is exactly these
four files plus the new packet-owned paths):

- `crates/spc-lab/src/lib.rs`: `pub mod traffic;` (module list) + a re-export
  block for `check_traffic_capability`, `project_traffic_capability`,
  `TrafficCapabilityRecord`, `TrafficSpec`.
- `crates/spc-lab/src/scenario.rs`: optional `traffic` field on `Scenario`
  (serde default, doc comment); `self.validate_traffic(&mut report)?;` call in
  `validate()` between `validate_statistics` and `validate_observers`; the
  `validate_traffic` method (delegates to `TrafficSpec::validate(identities_len, report)`).
- `crates/spc-lab/src/manifest.rs`: optional `traffic_capability:
  Option<TrafficCapabilityRecord>` field on `RunManifest` (serde default;
  additive, so existing manifests parse unchanged).
- `crates/spc-lab/src/engine.rs`: one line - `traffic_capability: None,` in
  the `RunManifest` construction at run creation.

Deliberately NOT wired in T0 (T1 merge notes): the engine preflight call to
`check_traffic_capability(&scenario, manifest.traffic_capability.as_ref())`
before process start; the `CommandKind` variants for the plan Section 12
commands; `TxDriverControl` in adapters; the `spc-lab-txd` workspace member
and its runtime plumbing; the evidence-catalog entries for `tx_traffic`
events. Root `Cargo.toml`/`Cargo.lock`/`flake.nix` are untouched.

> **2026-11-08, later - the gate call is wired (F1, Section 10.1).** The
> engine now calls `check_traffic_capability` in `prepare` (before any
> filesystem, lock or authority effect) and on the adopt path (before the
> owner lock). The other T1 items remain unwired.

## 8. Non-claims

- No probe execution, no second verdict, no `probe_mempool_rpc`; AP-1a
  lineage remains the Fable-rework dependency and the r1 hash is not approved.
- No signer, wallet, broadcast, process launch, mining, UTXO journal, worker
  budget scheduler, new RPC route, or UI work.
- No funded pass is fabricated anywhere; `sendrawtransaction` recognition
  remains recognition-only in read-only mode.
- The `getmempoolinfo` shape failure is reported, not fixed, coerced or
  reinterpreted; the node is not patched.
- No commit was made; nothing outside this worktree was modified.

## 9. Verification (actual results, this worktree)

- `cargo test -p spc-lab` (full package, 32 test binaries): **139 passed,
  0 failed** - includes the new `traffic_contract` suite (34 tests: valid
  traffic scenario; existing fixtures unchanged; old-runner mirror rejection;
  every rejection code above; defaults; integer extremes (u32::MAX/u64::MAX
  rejected by range, no panic); mix sums; wallet-count cross-field; admission
  budget; caps bounds; serialization roundtrips; command serde determinism;
  projection as-is mapping, r1 evidence shape preservation, version-gate
  non-propagation, foundational load-bearing failures and missing rows,
  availability-blocking, method-extension fail-closed, duplicate/lineage/
  state/shape rejections, verdict invariants, record tampering, manifest-field
  roundtrip) and the pre-existing suites (scenario_v1 8, engine, manifest,
  command idempotency/concurrency, http, evidence, privacy, ...).
- `cargo fmt -p spc-lab -- --check`: clean (formatting touched only the new
  packet-owned files).
- `cargo clippy -p spc-lab --all-targets`: zero warnings.
- All commands run with `-j 2`; logs under `/tmp/opencode/T0-*`.
- Worktree delta vs the frozen 370-file runtime manifest: exactly
  `crates/spc-lab/src/{lib,scenario,manifest,engine}.rs` modified (hunks in
  Section 7) plus new packet-owned paths
  `crates/spc-lab/src/traffic/{mod,spec,protocol,commands,projection}.rs`,
  `crates/spc-lab/tests/traffic_contract.rs` and this document. Frozen inputs,
  the primary checkout and other worktrees untouched.

## 10. r2 rework - F1-F8 dispositions (2026-11-08, later)

Answers `docs/reviews/2026-11-08-traffic-contract-fable.md` finding by
finding. Decisions of record:
`docs/decisions/2026-11-08-lab-traffic-contract-t0.md` (rework) and
`docs/decisions/2026-11-08-traffic-ap1a-lineage-r2-approved.md` (lineage).

### 10.1 F1 - gate wiring

`check_traffic_capability` is called in `RunEngine::prepare` immediately
after `preflight(...)` and BEFORE the run-manifest existence check and
`create_dir_all` (no filesystem, lock or authority effect can precede a
rejection), and in `attach_internal` on the adopt path after preflight and
BEFORE `RunLock::acquire`. Read-only attach never runs the gate. A missing
record rejects with `traffic_capability_unrecorded` even when the node
adapter config declares the capability name - the fail-open reproduction
(name-declaring config) is a regression test
(`engine_prepare_rejects_traffic_before_any_run_artifact` asserts an empty
run directory after the rejection). No-traffic scenarios prepare unchanged.
No `NODE_CAPABILITIES` claim was added.

### 10.2 F2 - approved lineage bound

The Fable re-review r2 approved the exact contract
`f4fc3e043f420870675095ab24144ba14e0afba6cb9ad69d818d99b1bccb2f62`
(packet manifest `f0aa504a...`, 74 files, source frozen in
`lab-ark-rpc-probe`). `AP1A_APPROVED_CONTRACT_SHA256S` holds exactly that
hash; the table was verified field-by-field against the frozen source's
`canonical_contract_json()` before binding, and the integration test
`frozen_expected_checks_table_is_the_approved_r2_contract` pins the 17
rows plus the allowlist as the atomicity guard (a hash-string-only
substitution cannot drift past the table). The r1 candidate
`11bdb100...` remains a recognized-unapproved diagnostic
(`traffic_capability_lineage_unapproved`); unknown hashes reject with
`traffic_capability_lineage_unknown`; the empty-allowlist safe state is
preserved and unit-tested through the crate-private seam. No
caller-configurable runtime approval bypass exists. The approval scope is
the probe contract ONLY - no T0/traffic/node-profile approval, no Core
version-compatibility or `bark-v31` claim, no Ark or traffic runtime
claim; the explicit T1 `RpcNodeControl` consumer choice remains the next
integration contract decision via the canonical owner.

### 10.3 F3 - frozen table, record shape, strictness

`TRAFFIC_EXPECTED_CHECKS` (17 rows: 12 five-method + 5 foundational) with
exact id-method binding; unknown consulted ids and wrong id-method
bindings reject (`traffic_projection_report_invalid`); missing expected
ids are `not_run` and block preflight even when the id's flags would
tolerate a non-passed behavior axis. `observations` must be an object;
provenance is strict (untrimmed non-empty `probe_version`/`run_token`,
exact `YYYY-MM-DDTHH:MM:SSZ` `generated_utc`, 64-hex hashes, schema v2,
canonical probe id); target/profile/node metadata
(`target_label`/`target_kind`/`profile_name`/`expected_network`/
`expected_genesis`/`node_binary_sha256`) is carried for T1 run binding.
The record carries FULL per-check rows (`check_states`, both axes) and
the gate recomputes the merged views, both verdict stages and the
unproven-id list from them - CE1 (forged per-method ids, null
observations, foreign target), CE2 (id attributed to the wrong method)
and CE3 (hand-built record, empty check_ids, schema 99, empty provenance)
are exact regression tests, plus 12 further tamper cases. Run binding
today: only `owned-fresh-regtest-node` targets on `regtest`
(`traffic_capability_target_unbound` otherwise); genesis/node-binary
binding to actual run inputs is the recorded T1 requirement.
`report_sha256` is documented as a self-reported integrity marker, not
authentication.

### 10.4 F4 - two-stage verdict

`recognized` (preflight): availability `passed` on every consulted row,
no `failed` on either axis, foundational fully passed, behavior
`unknown`/`not_run` only on the canonical `unknown_behavior_ok`/`funded`
flags. `behavior_proven`: false by construction for every read-only
projection (tampering it true rejects); T3 consumes its own exercised
funded evidence. The gate requires `recognized` and asserts
`!behavior_proven`. The r1-evidence `getmempoolinfo` shape failure blocks
`recognized` honestly - the selected-consumer result is preserved as-is,
never reinterpreted, never a universal policy gate; there is no
funded-broadcast-before-bootstrap circularity (the `funded` flag
tolerates `sendrawtransaction.recognized` being `not_run`) and no second
probe/verdict.

### 10.5 F5-F8

- **F5**: `SendUserPayment` gained the explicit mandatory
  `destination_address` (bounded 128, fail-closed: parse, regtest,
  p2pkh via the shared bitcoin types; codes
  `traffic_payment_destination_{invalid,network_invalid,script_unsupported}`).
  Resolves the plan 6.6/6.3 vs 12 ambiguity; the paying user/wallet stays
  the source; T1-before-signing checks are recorded in the type docs as
  the signer's domain. No freeform endpoint, no default, no silent
  ignore, no wired duplication.
- **F6**: `ActorId` typing on `TrafficActorSpec.id`,
  `TxControlHello.driver_id`, `TrafficHistoryHeader.driver_id` and the
  command `driver_id` fields (re-validated in `validate()`); the history
  journal is confined to `traffic/` (first component exactly `traffic`,
  no parent/backslash/whitespace/artifact-collision paths).
- **F7**: the two decision records, the changelog fragment and the
  `docs/LAB-RUNTIME.md` traffic pointer (safe-unavailable state) ship in
  this change.
- **F8**: exact protocol string (no trim); `traffic_capability_unrecorded`
  distinct code; `SEED_DOMAIN_PLANNED_SCHEDULE` frozen over
  `(run_seed, protocol_version, driver_id, sequence)`;
  `Dropped`/`Expired` terminality explicitly undefined until T6; the
  old-runner deny-unknown manifest behavior is recorded as intended
  fail-closed (decision record item 8).

### 10.6 Verification (actual results, this worktree, after the rework)

- `cargo test -p spc-lab --test traffic_contract -j 2`: **42 passed,
  0 failed** (34 prior + the reworked projection/gate suites + the
  engine prepare/adopt gate tests + the frozen-table atomicity guard).
- `cargo test -p spc-lab --lib -j 2`: **36 passed, 0 failed** (includes
  the projection module unit tests).
- `cargo test -p spc-lab -j 2` (full package): **32 test binaries, all
  ok, 0 failed**.
- `cargo fmt -p spc-lab -- --check`: clean. `cargo clippy -p spc-lab
  --all-targets -j 2`: zero warnings.
- Logs: `/tmp/opencode/T0-fix-r1-{test-traffic-contract,test-lib,
  test-spc-lab-full,fmt-check,clippy}.log` (one full-suite attempt hit a
  crates.io-index network timeout - environmental, re-run clean; the
  registry-cache permission warnings under concurrent agents are
  pre-existing and harmless).

### 10.7 Provenance correction (r1-era note)

The r1 AP-1a review-manifest snapshot `620e42e3...` disputed during the
first packet reproduces exactly as the sha256 of the sorted-key compact
JSON `{"base": ..., "files": ...}` built from the r1 manifest's
`(path, sha256, mode)` entries (sorted by path, `sort_keys=True`,
separators `(',', ':')`) - it is NOT the sha256 of the manifest file
itself (`daf62d7e...`). The r2 approval's `f0aa504a...` uses the same
recipe.

### 10.8 T1a/T1b handoff (informational, not a type fork)

The T1a core-wallet candidate (`lab-traffic-wallet`, Fable
REQUEST_CHANGES on core money invariants, author fixing in a separate
crate) is NOT imported and the T0 normative contract was NOT converged to
its defaults. For coordination, the exact T0 changes T1a/T1b must map
against: `SendUserPayment { user, destination_address, amount_sats,
fee_rate_sats_vb }` (destination validated regtest p2pkh, T1 signer owns
wallet/UTXO admission and output construction); `ActorId` typing on all
driver refs; gate codes `traffic_capability_{unrecorded,unknown,
record_invalid,target_unbound,lineage_unapproved,lineage_unknown,
unrecognized}`; the record shape (full `check_states`, two-stage verdict,
binding metadata fields). T1b boundaries flagged in the decision record:
the journal header extension (`network`, `key_mode`, journal caps)
requires a `history_version` bump through a recorded decision;
`Invalidated` cascade semantics and global per-actor index ranges are
T1b/T6 allocations over the frozen vocabulary; retry-cap semantics are
specified (initial send + at most N retries, outcomes recordable after
the cap, 0..=3 rejects out-of-range); the lab key mode is public-seed +
run-id bound (publicly reproducible, no secrecy claim), with the
per-domain input tuple T1b's to define over the frozen domain strings.
No SPc/Ark key change is implied.

### 10.9 Remaining dependencies

- **AP-1a**: RESOLVED - the approved r2 contract is bound (10.2). Any
  drift under `testing/ark-rpc/**` requires a new hash and a new dated
  decision.
- **T1 recording path**: no runtime records a capability yet, so
  `prepare` rejects every traffic scenario
  (`traffic_capability_unrecorded`) - the intended safe-unavailable
  state.
- **T1 consumer decision**: whether `RpcNodeControl` needs the full
  corepc v30 `getmempoolinfo` superset or a traffic-subset decoder
  remains the explicit T1 + canonical-owner decision; the
  `maxdatacarriersize: null` failure is preserved, not reinterpreted.
- **T3**: exercised funded evidence; `behavior_proven` stays false from
  read-only reports by construction.
