# Fable review - T0 traffic contract (schema freeze, protocol vocabulary, AP-1a traffic projection) (2026-11-08)

**Decision: REQUEST_CHANGES (bounded, T0-internal).** The frozen `TrafficSpec`
schema, the tx-control/history/lifecycle vocabulary and the command payloads
are a sound, additive foundation: every pre-traffic fixture still validates,
the baseline runner provably rejects a traffic-bearing scenario at parse, the
rejection codes are stable and tested, and nothing executes, signs, mines or
probes. What does not yet hold is the packet's own headline for the second
half of its scope - "scenario rejection before process start when traffic is
declared without the recorded projection" (plan Section 14, T0). As delivered
the gate is a library function nobody calls, the existing capability
mechanism it leans on is name-based and fail-open under a config-declared
name (reproduced below), the gate accepts the explicitly unapproved r1
lineage, the record tamper check re-validates key sets only (three forgeries
reproduced), and the single `satisfied` verdict cannot ever be true from a
read-only AP-1a report, which makes the live T3 path circular. All four are
small, T0-owned fixes; none require the T1 signer, the actor or a funded node.

This review approves nothing about AP-1a: contract hash `11bdb100...` remains
unapproved (and the live AP-1a tree already hashes differently, see F2), no
Core RPC-version compatibility is claimed, and no funded behavior is implied.

## 1. Scope and provenance

| item | result |
|---|---|
| Candidate | `/home/ethan/.local/share/opencode/worktree/ad8886/lab-traffic-contract`, git base `b0ab4ca`, uncommitted; untouched by me |
| Parent full manifest `/tmp/opencode/T0-review-manifest.json` | 377 rows; recipe sha256 of sorted-key compact JSON `{base, files}` reproduces `a9f841cb7236f21c4d4dca70cef5f8ee249f8b2eba51337eec468bc42cab353c`; recorded `code_snapshot_sha256` `1030eeef...` (report excluded) carried as stated |
| Delta vs frozen 370-file lab-final-integration baseline | exactly 4 modified (`crates/spc-lab/src/{engine,lib,manifest,scenario}.rs`) + 7 new (`crates/spc-lab/src/traffic/{mod,spec,protocol,commands,projection}.rs`, `crates/spc-lab/tests/traffic_contract.rs`, `docs/reviews/2026-11-08-traffic-contract-implementation.md`); all 11 on-disk sha256 values equal the manifest and the implementer's `/tmp/opencode/T0-delta-manifest.json` rows |
| Shared-file hunks | diffed against the frozen baseline: `lib.rs` module + re-export block; `scenario.rs` optional `traffic` field, one `validate_traffic` call, one method; `manifest.rs` optional `traffic_capability` field; `engine.rs` one `traffic_capability: None` line. Exactly as the report's Section 7 states; nothing else moved |
| Approved plan | `/tmp/opencode/traffic-plan-approved-r2/plan.md` sha256 `d64a4235...` and peer review `5cd97e19...` match the receipt; Sections 5, 6, 8, 11, 12, 14, 16 read in full |
| Tests reproduced | `cargo test -p spc-lab --test traffic_contract --test scenario_v1 --test preflight -j 2`: 34 + 8 + 3 passed. Implementer log `T0-test-spc-lab-full.log`: 32 binaries, 139 passed, 0 failed. `cargo fmt -p spc-lab -- --check` clean now (the saved `T0-fmt-check.log` is a pre-fix diff; the final tree is clean). Clippy log clean |
| ASCII | all 7 new files are pure ASCII |
| Own probing | one scratch crate at `/tmp/opencode/T0-fable-scratch` (own target dir `/tmp/opencode/T0-fable-target`, path-dependency on the candidate crate, nothing written into the candidate); it drives `RunEngine::prepare` with the fixture adapters and calls the projection/gate directly. Run dirs removed after each probe. No services, no nodes, no mining |
| AP-1a context | primary `docs/reviews/2026-11-08-ark-rpc-probe-fable.md` (v1 review + r1 re-review) read; r1 evidence `/tmp/opencode/T0-ap1a-r1-evidence/*` inspected; live AP-1a tree `worktree/ad8886/lab-ark-rpc-probe/testing/ark-rpc` queried read-only (`contracts.contract_hash()`, `canonical_contract_json()`, `report.schema.json`) |

I made no edits to the candidate or to any other primary file. The only file
written to the repository is this one.

## 2. Confirmed sound

- **Additivity (plan 11 (a)/(b))**: baseline `Scenario` is
  `deny_unknown_fields` with the exact 14 top-level fields the test's mirror
  struct lists (verified against `lab-final-integration/.../scenario.rs`);
  `parse_scenario` checks `scenario_version` then strict-deserializes, so an
  old runner fails `scenario_schema_invalid` / `unknown field 'traffic'`
  before validation. The report says plainly that no old binary was run;
  the property is source-determined and the mirror test is an honest proxy.
- **Schema vs plan Section 11**: every field, default (`native`,
  `fixed_jitter`, `reorg_resubmit_max = 3`), range and listed error code is
  present; `rpc_generate`, `burst`, `p2wpkh`, `bootstrap.enabled=false`,
  `registration_before_mining=false`, `excluded_from_statistics=false` all
  fail closed with distinct codes; integer extremes are range-rejected, not
  overflowed (`u64` accumulation for wallet totals); `f64` fields check
  `is_finite()` so Rust-constructed NaN/inf is rejected (JSON cannot express
  them). Fee rates and amounts are integer sats throughout; nothing goes
  through a float.
- **Protocol vocabulary**: name + major exactness, `v1.2`/`/1`/no-version
  forms rejected; unknown major fails at hello validation; accepted ack must
  carry history version 1; rejected ack must carry a reason; the v1
  capability registry is truthfully empty and `supports()` requires both
  registry knowledge and advertisement. Lifecycle vocabulary is the full
  Section 6.5 set; `rejected`/`conflicted` terminal, `confirmed` not
  (reorg reconcile) - matches the plan.
- **Command payloads**: `CommandLog::payload_hash` hashes
  `serde_json::to_vec(&command.kind)` (re-serialized struct, not the caller's
  bytes), so struct field order, not caller key order, governs idempotency;
  the "fixed field order" claim is therefore the right one and
  `Value`-typed `InjectTx.payload` re-serializes with sorted keys. Same id +
  different payload stays a conflict.
- **Projection honesty**: statuses map as-is; the availability axis rejects
  `not_run` exactly as `report.schema.json` v2 does; verdict enum
  `incompatible|surface_compatible|unproven` and the read-only invariants
  (`funded_state == "unproven"`, `full_acceptance == false`) match the live
  schema; the bark verdict is recorded, never consulted; `getnetworkinfo.gate.*`
  rows never touch the traffic rows; a method with no rows is `not_run`; the
  r1 `getmempoolinfo` `passed/failed` row is preserved as `failed` with no
  coercion, and the report correctly states that T0 introduces no
  `getmempoolinfo` consumer and leaves the `RpcNodeControl` decoder choice to
  T1 and the canonical AP-1a owner. The five foundational ids and the twelve
  method ids exist with those exact strings and methods in the live AP-1a
  `CHECKS` table (`protocol.id-echo` has `method: null`).
- **No second verdict, no probe**: the crate gains no RPC client, no
  `probe_mempool_rpc`, no process launch, no `CommandKind` variant, no
  workspace member. Root `Cargo.toml`/`Cargo.lock`/`flake.nix` untouched.
- **r1 AP-1a manifest recipe**: the implementer questioned `620e42...`; it
  reproduces exactly from `lab-ark-rpc-probe-r1-review-manifest.json` with
  `json.dumps({'base': m['base'], 'files': m['files']}, sort_keys=True,
  separators=(',',':'))`. `daf62d7e...` is the sha256 of the manifest file
  itself - a different object, not a dispute. The T0 report does not repeat
  the claim, so nothing to correct there.

## 3. Findings, ranked

Severity: **H** blocks the packet's stated purpose; **M** fix in this packet;
**L** advisory. Each H/M has a reproduced counterexample or a source line.

### F1 (H, gate not wired; existing mechanism is fail-open by name) - a traffic-bearing scenario runs today with traffic silently ignored

`check_traffic_capability` is never called. The only thing standing between
a traffic scenario and `RunEngine::prepare` is `config::preflight`
(`config.rs:219-231`), which requires each node adapter to *declare* every
name in `required_capabilities`. The real host runtime declares only
`NODE_CAPABILITIES` (`runtime.rs:55-59`), so that path fails
`capability_unavailable` - by coincidence of the name list, not by design,
and `config.rs:1-5` itself says a declared name is never evidence. Any
`LabConfig` from `--config`/`runtime.json` that lists the name passes.

Reproduced with the fixture adapters (scratch crate, two-pool fixture plus
the test's `traffic` section):

```
[baseline-caps]  PREPARE REJECTED: capability_unavailable / node node-a ... does not declare "spc.node.regtest.mempool_rpc.v1"
[name-declared]  PREPARE OK: run-manifest exists=true traffic_capability=false scenario.traffic declared=true
```

The second line is a run directory created, lock taken, authorities
generated and a manifest written for a scenario whose `traffic` section
nothing will honour - the plan's "rejected before any process starts" is
not met, and `start_run` would proceed with no driver, no bootstrap and
measured mining against a scenario that declares traffic. The report defers
"the engine preflight call" to T1 merge notes (Section 7), but plan Section
14 puts "scenario rejection before process start when traffic is declared
without the recorded projection" in **T0**, and the packet already ships an
`engine.rs` hunk.

Fix (small, fail-closed, no T1 dependency): in `RunEngine::prepare`, right
after `preflight(...)` and before `create_dir_all`, call
`check_traffic_capability(&scenario, None)` (the fresh manifest has no
record; this returns `traffic_capability_missing` for every traffic
scenario until T1 provides a way to record one); in the attach/adopt path
(`engine.rs:335`) call it with `manifest.traffic_capability.as_ref()`.
Add an engine test asserting `prepare` fails with
`traffic_capability_missing` and that the run directory contains no
`run-manifest.json` afterwards. Also add the capability name to the
real-runtime `NODE_CAPABILITIES` only when T1 actually records a
projection - never before.

### F2 (H, lineage) - the gate accepts the unapproved r1 lineage; the live AP-1a tree already disagrees with the constant

`check_traffic_capability` passes any record whose `contract_hash ==
AP1A_CONTRACT_SHA256_CANDIDATE` (`projection.rs:469`). The constant's
doc-comment says "DO NOT TREAT AS APPROVED", but the gate does exactly that:
my CE1 record (hash `11bdb100...`, fully passed rows) returns `Ok(())`.
Meanwhile the live AP-1a worktree's `contracts.contract_hash()` now returns
`f4fc3e043f420870675095ab24144ba14e0afba6cb9ad69d818d99b1bccb2f62` (r2 rework
in progress, also unapproved), so the pinned string is stale as well as
unapproved. The primary Fable r1 re-review lists C1/C2/D1-D3/D5 as required
before any hash approval; none of that is T0's fault, but T0 must not encode
an unapproved hash as an accepting lineage.

Fix: make the approved lineage an `Option<&'static str>` (or an empty
`&[&str]`) that is `None` until a recorded decision sets it; the gate
returns `traffic_capability_lineage_unapproved` while it is `None`. Keep
the r1 string only as a documented *recognized-but-unapproved* value for
tests. When the reviewed hash lands, update the constant **and** the
expected check-id table (F3) in the same decision record - the ids are
currently identical in r2, which I verified, but a hash is not a shape
check.

### F3 (H, record integrity) - projection and gate bind to self-declared strings only; forged reports and hand-built records pass

Three reproduced counterexamples against the candidate code (scratch
crate, `check_traffic_capability` with the test's traffic scenario):

```
CE1 one forged row per method ("<method>.whatever"), observations=null,
    target/profile foreign (remote-url, mainnet, genesis "00"):
    satisfied=true, gate Ok(())
CE2 row id "getblock.verbosity1" attributed to method "getrawtransaction":
    satisfied=true, getrawtransaction.check_ids=["getblock.verbosity1"]
CE3 hand-built record: every methods[*].check_ids = [],
    report_schema_version=99, report_sha256="", probe_version="", run_token="":
    gate Ok(())
```

Root causes: the projection groups by the row's `method` string and never
checks that the expected canonical ids for that method are present (the
report's "12 r1 check ids are stable contract identifiers" is prose, not
code); `observations` is accepted as any `Value` including `null`;
`target`/`profile` are parsed and dropped, so the record cannot be tied to
the run's node (network, expected genesis, node binary sha256 are all in
the report but absent from the record); the gate checks only the two key
sets and the `satisfied` recomputation, not `report_schema_version`,
`report_sha256` shape, non-empty `check_ids`, or that every `check_ids`
entry is a known id of that method. A manifest is a JSON file; the record
type is `Deserialize`; the gate is the only defense.

Fix (all inside `projection.rs` + tests): (a) freeze
`TRAFFIC_EXPECTED_CHECKS: &[(id, method)]` (the 12 method rows + 5
foundational rows) next to the lineage constant; a consulted row whose id is
not in the table, or whose `method` disagrees with the table, fails
`traffic_projection_report_invalid`; an expected id missing from the report
makes its method `not_run` (as the foundational path already does); (b)
require `observations` to be a JSON object and carry
`profile.expected_network`, `profile.expected_genesis`, `target` and
`node.binary_sha256` (when present) into the record so T1 can bind it to
the run's node and the manifest's `binary_hashes`; (c) in the gate
re-validate `report_schema_version == 2`, `report_sha256` is 64 hex,
non-empty `probe_version`/`run_token`/`generated_utc`, and each method's
`check_ids` is exactly the expected set for that method. Regenerate the
"satisfied" test fixtures from the canonical ids only (the current
`projection_method_extension_joins_and_fails_closed` test should become
"unknown id fails closed").

### F4 (H, design coherence for T3) - a single `satisfied` makes live traffic impossible by construction

`recompute_satisfied` requires behavior `passed` on every row of all five
methods. `sendrawtransaction.recognized` has coverage class `funded` in the
canonical contract and is `not_run` by design in read-only mode;
`getrawtransaction.*` is `unknown` on any fresh node (genesis coinbase -5,
Core-identical, `unknown_behavior_ok: true` in the canonical `CHECKS`). The
run's own node starts from a fresh datadir, so a per-run AP-1a report is
necessarily read-only at preflight time. Hence no traffic scenario can ever
pass this gate on the lab, and bootstrap funding (which would make a funded
probe possible) sits behind the gate: the preflight is circular. The report
acknowledges "a satisfied capability is currently unattainable from any
read-only AP-1a report" and calls it the plan 16 item 1 expectation, but
plan Section 8 says "Recognition is only preflight" and reserves funded
proof for T3 ("every T3 acceptance run must exercise and typed-parse all
five on real data"). Freezing a one-bit verdict now means T1 will either
never start or will loosen the rule ad hoc in a shared file.

Fix (T0-owned, small): split the record verdict into two frozen stages.
`recognized` (the preflight gate): availability `passed` on every consulted
row, no `failed` on either axis anywhere consulted, all foundational rows
fully `passed`, and behavior `unknown`/`not_run` tolerated only for ids the
frozen expected-check table marks `unknown_behavior_ok` / `funded`
(exactly the canonical contract's own flags; `getmempoolinfo` behavior
`failed` still blocks, which is the honest current outcome).
`behavior_proven` (never true from a read-only report): all behavior
`passed`; T3's acceptance consumes it, or more precisely consumes the T3
run's own exercised evidence. The gate requires `recognized`; nothing in T0
may claim `behavior_proven`. Record both bits and the list of
unproven ids so the manifest says what remains to be exercised.

### F5 (M, record shape vs plan Section 12/6.6) - `send_user_payment` has no destination; plan sections disagree and the packet chose silently

Plan 6.6 writes `send_user_payment {user, wallet, amount_sats, fee_rate}`
and 6.3 requires "manual payments verify the exact requested amount to the
selected destination"; plan 12 writes `{ user, amount_sats,
fee_rate_sats_vb }`. The candidate froze the Section 12 form with
`deny_unknown_fields`, so T1 cannot add a destination without breaking the
frozen payload. State the destination rule in the type (an explicit
destination wallet/address field, or a documented deterministic rule) and
record the choice; `amount_sats` exactness is otherwise correctly integer.

### F6 (M, typed ids) - actor and driver ids are bare `String`s

`TrafficActorSpec.id`, `TxControlHello.driver_id`, `PauseTxDriver.driver_id`
etc. are checked only for non-emptiness, while every other lab identifier
goes through `ids::validate_id` (128-byte, ASCII, no control characters;
`ActorId` already exists). These ids will be rendered into private file
names, evidence and process specs. Use `ActorId` (or a new `DriverId` via
`id_type!`) so length and charset are bounded at parse. Same for
`driver.history`: fine as a relative path check, but it is not restricted
to `traffic/` and can collide with existing private artifact names
(`scenario.json`, `node-rpc-*.cookie`); require the `traffic/` prefix the
plan names, or check against `ArtifactPaths`.

### F7 (M, process) - no decision record or changelog fragment

Sibling lab packets that added scenario vocabulary and manifest fields
shipped `docs/decisions/<date>-<slug>.md` plus a `docs/changelog/` fragment
(e.g. `2026-11-06-lab-convergence-v1`). T0 freezes a new scenario section, a
new manifest field, a protocol name/version and a lineage constant that its
own comments say "is updated only through a recorded decision", yet creates
no record. Add one decision record covering the frozen schema, the manifest
field, the two-stage capability verdict (F4) and the lineage-approval
procedure (F2), plus the changelog fragment. `docs/LAB-RUNTIME.md` should
gain the `traffic` section pointer once F1 lands.

### F8 (L, advisory)

- `validate_protocol` trims `protocol.control` while
  `TrafficHistoryHeader::validate` does not; pick one (exact, no trim).
- `traffic_capability_missing` is used both for an empty
  `required_capability` string (spec) and for "no record in manifest"
  (gate); a distinct code for the latter (`traffic_capability_unrecorded`)
  reads better in evidence.
- Rates/mixes as `f64` are fine as configuration, as the report argues; the
  planned-schedule domain the plan names (`run_seed, protocol_version,
  driver_id, sequence`, Section 6.3) is not frozen as a seed-domain
  constant alongside the wallet domains - add it so T1 does not invent the
  string.
- `Dropped`/`Expired` terminality is left undefined ("later profile");
  state that explicitly in the enum docs so T6 does not inherit an implicit
  non-terminal reading.
- `MANIFEST_SCHEMA_VERSION` stays 1 with an additive optional field; since
  `RunManifest` is `deny_unknown_fields`, an old runner adopting a manifest
  that carries `traffic_capability` fails closed - acceptable, worth one
  sentence in the record.

## 4. External dependencies, not T0 defects

- **AP-1a lineage**: hash approval is pending C1/C2/D1-D3/D5 of the primary
  re-review; the r2 contract already differs. T0's job is to fail closed
  until a decision (F2) and to pin ids, not to pick a hash.
- **`getmempoolinfo` decoder**: the `maxdatacarriersize: null` failure is
  real for the selected corepc v30 consumer and not a Bark requirement
  (bark `c419159` never calls the method); whether T1's `RpcNodeControl`
  route needs the full v30 superset or a traffic-subset decoder is the T1 +
  canonical-owner decision, and the node must not be patched to launder it.
  The candidate states this correctly. Until decided, F4's `recognized`
  stage will still block on that row - honestly.
- **No funded pass anywhere**: nothing in T0 can or should prove broadcast
  behavior; that is T3 against the run's own exercised data.

## 5. Required before approval

1. F1: wire `check_traffic_capability` into `RunEngine::prepare` (before any
   filesystem effect) and the adopt path; engine test proving a
   traffic-bearing scenario is rejected with no run directory artifacts.
2. F2: approved lineage becomes `None` until a decision; gate fails
   `traffic_capability_lineage_unapproved` on every record meanwhile.
3. F3: frozen expected `(id, method)` table; unknown/wrong-method ids and
   missing ids fail closed; `observations` must be an object; carry
   network/genesis/target/node binary into the record; gate re-validates
   `check_ids`, `report_schema_version`, `report_sha256` shape and the
   non-empty provenance strings. Regression tests for CE1-CE3.
4. F4: two-stage verdict (`recognized` for preflight, `behavior_proven`
   never from read-only) with the unproven id list recorded.
5. F5, F6, F7 as above.

F8 is recommended, not blocking. The schema, protocol and command modules
need no structural change for any of this; the work is in `projection.rs`,
one `engine.rs` hunk, id types, and documents.

## 6. Explicit non-approvals

- Contract hash `11bdb100...` is not approved, nor `f4fc3e04...`,
  `48894548...` or `905bf4b8...`; this review does not inspect the r2
  encoding fixes and makes no claim that a constant update alone would be
  correct.
- No node capability `spc.node.regtest.mempool_rpc.v1` exists on any node
  today; the r1 evidence yields an unsatisfied projection on both approved
  binaries, and nothing here changes that.
- No T1 runtime, signer, actor, bootstrap mining, RPC route or `CommandKind`
  variant is implied to exist; T1/T2-L/T3 are reviewed separately.
- No Core RPC-version compatibility, `bark-v31` profile, node patch or
  version override is approved or implied.

---
---

# Fable re-review r1 - T0 traffic contract after the F1-F8 rework (2026-11-08, later)

**Decision: APPROVE - bounded T0 exactly as frozen by
`/tmp/opencode/T0-fix-r1-review-manifest.json` (full snapshot
`3304dffa84c32d546ee783bd15beec09bbfe5fa6755a71c9c5721bc6241d41a8`).**
Every H/M finding of the first review is closed in code and tests, each of
my CE1-CE3 counterexamples now fails closed, the engine gate runs before any
effect on both the prepare and adopt paths, and the approved AP-1a r2
lineage is bound atomically with a field-by-field-verified 17-row table.
What is approved is the frozen schema/protocol/command vocabulary, the
manifest field, the projection + gate, and the safe-unavailable state in
which every traffic scenario is rejected at `prepare` until a T1 recording
path exists. Nothing here approves T1, a node capability, a node profile,
funded behavior or Ark readiness (Section R1-6).

## R1-1. Freeze and provenance

| item | result |
|---|---|
| Full manifest | 380 rows; sorted-key compact JSON `{base, files}` reproduces `3304dffa...`; `baseline_manifest_sha256` `8bbe097d...` unchanged; `supersedes` names the first-packet manifest, which is untouched |
| `packet_code_snapshot_sha256` `86d3de7f...` | reproduces as the same recipe over the 10 delta `.rs` rows only - a declared, non-standard scope, correctly labelled in the manifest's `recipe` string; not the usual full-tree-minus-report code hash |
| Delta vs frozen 370-file baseline | 5 modified (`engine.rs`, `lib.rs`, `manifest.rs`, `scenario.rs` unchanged from the first packet; `engine.rs` now carries the gate) + 10 new; all 15 on-disk sha256 values equal the manifest rows |
| `engine.rs` diff against baseline | import + gate call after `preflight` and before the run-manifest existence check / `create_dir_all`; adopt path: scenario/runtime load, preflight, gate, **then** `RunLock::acquire` (lock moved below the gate); read-only attach ungated. Nothing else moved |
| Tests reproduced | `cargo test -p spc-lab -j 2`: lib 36, `traffic_contract` 42, `scenario_v1` 8, `preflight` 3, `engine_restart` 3, `run_lock` 1, `supervisor_adoption` 7 - all green in the existing target dir. Implementer full log: 32 binaries, 154 passed, 0 failed. `cargo fmt -p spc-lab -- --check` clean; clippy log clean |
| AP-1a lineage source | live `lab-ark-rpc-probe` tree: `contracts.contract_hash()` = `f4fc3e04...`; primary `2026-11-08-ark-rpc-probe-fable.md` re-review r2 "Decision: APPROVE" for exactly that hash; r2 review manifest `f0aa504a...` reproduces from `/tmp/opencode/lab-ark-rpc-probe-r2-review-manifest.json` |
| Table atomicity | I compared all 17 `TRAFFIC_EXPECTED_CHECKS` rows against `canonical_contract_json()['checks']` at `f4fc3e04...` on `(id, method, coverage, unknown_behavior_ok)`: 0 mismatches. The in-crate test `frozen_expected_checks_table_is_the_approved_r2_contract` pins a **frozen copy** of those 17 rows and the one-entry allowlist; it does not read the live AP-1a path |
| ASCII | all new/changed Markdown is pure ASCII; one deliberate non-ASCII byte in a Rust test literal (`traffic_contract.rs:518`, an invalid-id input) - fine |
| Own probing | same scratch crate as before (`/tmp/opencode/T0-fable-scratch`, own target dir, path dependency, no writes into the candidate); run dirs removed; no services, nodes or mining |

I made no edits to the candidate. The only file written is this appended
section.

## R1-2. F1-F8 dispositions (verified, not accepted as stated)

| finding | verified status |
|---|---|
| F1 gate wiring | **Closed.** Reproduction: fixture config declaring `spc.node.regtest.mempool_rpc.v1` on the node adapter -> `prepare` now `traffic_capability_unrecorded`, **0 entries** in the run directory after rejection (first packet: run dir + manifest written). Baseline config still `capability_unavailable`, also 0 entries. No-traffic scenario prepares as before. Adopt path: engine test `engine_attach_gates_traffic_on_the_adopt_path` covers unrecorded / unknown-lineage / forged / approved / read-only; I confirmed the lock is acquired only after the gate and that everything read before the gate (manifest, private scenario, runtime) is read-only input binding with no write |
| F2 lineage | **Closed.** `AP1A_APPROVED_CONTRACT_SHA256S = ["f4fc3e04..."]` with the recorded decision `docs/decisions/2026-11-08-traffic-ap1a-lineage-r2-approved.md`; r1 `11bdb100...` -> `traffic_capability_lineage_unapproved`; unknown hash -> `traffic_capability_lineage_unknown`; empty allowlist (private `#[cfg(test)]`-reachable seam only, no runtime bypass) rejects every record. Decision text states the approval scope limits verbatim from R2-7 |
| F3 record integrity | **Closed.** CE1 (forged `<method>.whatever` rows) -> `traffic_projection_report_invalid`; CE1 with canonical rows but `observations: null` -> rejected; remote-url target or `mainnet` profile -> `traffic_capability_target_unbound`; CE2 (`getblock.verbosity1` under `getrawtransaction`) -> rejected at projection; CE3 variants (empty `check_ids`, empty `check_states`, `report_schema_version=99`, empty `report_sha256`, empty `probe_version`, short timestamp, bad `node_binary_sha256`, short `expected_genesis`, extra unknown `check_states` id, tampered `recognized`, tampered aggregates, `behavior_proven=true`) -> all `traffic_capability_record_invalid`. The gate recomputes method states, foundational states, `recognized` and the unproven list from the full per-check rows and rejects any disagreement |
| F4 two-stage verdict | **Closed.** `recognized` requires every expected id present with availability `passed`, no `failed`, and behavior `unknown`/`not_run` only where the frozen table says `unknown_behavior_ok` or coverage `funded` (exactly the 4 `getrawtransaction.*` rows and `sendrawtransaction.recognized`). A missing tolerated row still blocks (verified). An untolerated `unknown` (`getrawmempool`) blocks (verified). `behavior_proven` is false by construction and a `true` claim is rejected. The r1/r2 shape blocks on `getmempoolinfo=failed` alone: with that single row passed, the same report is `recognized` with the five tolerated ids listed as unproven - the non-circular preflight the plan intends |
| F5 destination | **Closed.** `SendUserPayment { user, destination_address, amount_sats, fee_rate_sats_vb }`, `deny_unknown_fields`; destination bounded 128 bytes, no control chars, `bitcoin::Address` parse, `require_network(Regtest)`, `is_p2pkh()`; mainnet and `bcrt1q...` p2wpkh rejected with distinct codes; the plan 6.6/12 disagreement is recorded as resolved by the parent |
| F6 typed ids / history path | **Closed.** `ActorId` on actor/driver fields with explicit re-validation at validate time (correct: the derived `Deserialize` does not validate); `history` must be `traffic/<file>` with no `.`/`..`/empty/backslash components and no surrounding whitespace |
| F7 records | **Closed.** Two decision records, a changelog fragment, a `docs/LAB-RUNTIME.md` pointer describing the safe-unavailable state; `scripts/check-lab-docs.py` still passes |
| F8 advisories | **Closed.** Exact (untrimmed) protocol string in the spec; `traffic_capability_unrecorded` distinct from the spec's `traffic_capability_missing`; `SEED_DOMAIN_PLANNED_SCHEDULE` frozen; `Dropped`/`Expired` explicitly undefined until T6; manifest-compat sentence recorded |

Real-evidence check: both r2 reports
(`reports/2026-11-08-r2-{l2-eb257af7,v09lab-bcb6d5ee}/report.json`) project
cleanly through the strict provenance checks (hash `f4fc3e04...`, owned
target, `regtest`, Core regtest genesis, node binary sha256 carried) and are
honestly **unrecognized** with `getmempoolinfo` the only blocking row -
exactly the current truth, no coercion, no funded claim.

## R1-3. T1a journal header: sequencing, not a type fork

The T1a core-journal rework (separate Fable review) requires the journal
header to carry network, key mode, run id, driver id, protocol and caps.
T0's frozen `TrafficHistoryHeader` v1 is `{history_version, run_id,
driver_id, protocol}` with `deny_unknown_fields`, `TRAFFIC_HISTORY_VERSION
= 1`, and the scenario validator rejects `protocol.history_version != 1`.
So a v1 header **cannot** carry the T1a-mandatory fields, and a T1 actor
cannot both satisfy T1a's core invariants and write a T0 v1 header. This
is not a contradiction inside T0 and does not block this approval, but
the T0 documents' phrasing ("requires a `history_version` bump through a
recorded decision", "T1b/T6 allocations") must be read as a **hard
precondition on the first T1 journal write**, not as permission to defer:
the `history_version` 2 header (and the matching `TRAFFIC_HISTORY_VERSION`
/ scenario `protocol.history_version` acceptance in `spec.rs` and
`protocol.rs`) must land by dated decision **before any T1 actor writes a
journal line**, and a v1 header lacking those fields may never be written.
`Invalidated` is already in the frozen vocabulary; the cascade semantics
and the global per-actor index ranges / single bootstrap owner are T1
allocation rules over T0's counts and domain strings - correctly left out
of T0. The lab key mode (public scenario seed bound to run id, no secrecy
claim) and the retry cap (one initial send plus at most N retries, outcomes
always recordable) as recorded in the T0 decision are consistent with the
T0 types.

## R1-4. Advisory notes (non-blocking; no hash change needed)

- `RunEngine::prepare` calls `check_traffic_capability(&scenario, None)`
  with a literal `None`. That is correct today (a fresh manifest has no
  record) but means the T1 recording path must change this call (e.g. an
  `EngineOptions` record input) - it should be named explicitly in the T1
  merge note rather than discovered.
- `expected_genesis` is shape-checked, not bound: a well-formed foreign
  genesis under an approved hash passes the T0 gate (reproduced,
  expected). The rbitcoin `regtest_pow_bits_overlay` does not alter the
  genesis hash, so the lab genesis is the Core regtest genesis; T1 should
  bind the record to the run's observed `getblockhash 0` and
  `binary_paths_and_hashes`, as the record fields now allow. The gate also
  binds network to the constant `regtest` rather than to the scenario's
  network spec; fine for a single-network lab.
- Adopt path reorder: scenario and runtime artifacts are now read before
  the owner lock. Both are write-once private artifacts, so there is no
  effect before the gate, but a concurrent adopter now fails `run_lock`
  after preflight instead of before - a benign ordering change worth one
  line in the T1 merge note.
- `TrafficMethodStatus::check_ids` now lists the *expected* ids for the
  method, not the ids actually present; the present rows live in
  `check_states`. Consistent and gate-checked, but the field doc should
  say "expected".
- The synthetic test genesis (`...433e0ad2`) is not the real regtest
  genesis (`...466e2206`); harmless because T0 does not bind it, but if
  binding lands in T1 the fixtures should use the real value.

## R1-5. Required before approval

None. R1-3 is a binding statement for T1's first journal write, not a T0
change; R1-4 items are recommendations.

## R1-6. Explicit non-approvals (unchanged in substance)

- No node today has `spc.node.regtest.mempool_rpc.v1`: both approved
  binaries project to `recognized = false` on the `getmempoolinfo` row.
  Whether T1's `RpcNodeControl` consumer needs the corepc v30 superset or
  a traffic-subset decoder remains the T1 + canonical-owner decision; the
  node is not to be patched and the row is not a universal traffic failure.
- No T1 actor, signer, wallet, bootstrap mining, RPC route or `CommandKind`
  variant exists or is implied; `prepare` rejects every traffic scenario by
  design until T1 records a capability.
- `behavior_proven` is false for every read-only projection; T3's funded
  proof is a separate gate.
- No Core RPC-version compatibility, `bark-v31` profile, node patch or
  version override; no Ark readiness. The AP-1a approval covers the probe
  contract only; this approval covers T0's code.
