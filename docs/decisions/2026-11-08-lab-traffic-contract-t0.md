# 2026-11-08 - Lab traffic contract T0 rework (F1-F8)

## Scope

Bounded rework of the T0 traffic-contract packet
(`crates/spc-lab/src/traffic/`, the manifest field, the engine gate wiring)
in response to the Fable review
`docs/reviews/2026-11-08-traffic-contract-fable.md` (REQUEST_CHANGES,
findings F1-F8, counterexamples CE1-CE3). No T1 actor implementation, no
probe execution, no node change. `DESIGN.md` and `docs/SPEC.md` remain
authoritative; this record changes lab-tool vocabulary only. The approved
AP-1a lineage binding is recorded separately in
`docs/decisions/2026-11-08-traffic-ap1a-lineage-r2-approved.md` and landed
in the same change.

## Decisions

1. **Engine gate wiring (F1)**: `check_traffic_capability` runs in
   `RunEngine::prepare` immediately after `preflight(...)` and BEFORE any
   filesystem, lock or authority effect (before the run-manifest check and
   `create_dir_all`), and on the attach/adopt path after preflight and
   BEFORE `RunLock::acquire`. A traffic-bearing scenario with no recorded
   capability rejects with `traffic_capability_unrecorded` even when the
   node adapter config declares the capability NAME - a config-declared
   name is never evidence. Read-only attach never runs the gate. No
   `NODE_CAPABILITIES` claim is added until real T1 recording exists.
2. **Approved lineage (F2)**: production lineage is the Fable-approved r2
   contract hash (see the lineage decision). The approval procedure is
   frozen: hash + expected-check table + any profile mapping change
   atomically through a dated decision; recognized-but-unapproved hashes
   (the r1 candidate) are named in diagnostics and never accepted;
   unknown hashes and the empty-allowlist state fail closed. No
   caller-configurable runtime approval bypass exists (the seam is
   `#[cfg]`-private to in-crate tests).
3. **Frozen expected-check table and record shape (F3)**:
   `TRAFFIC_EXPECTED_CHECKS` pins the exact `(id, method, coverage,
   unknown_behavior_ok)` of all 17 consulted rows (12 five-method + 5
   foundational) as the approved canonical CHECKS declare them. Consultation
   is by id-in-table (method must match exactly) - an unknown consulted id
   or a wrong id-method binding rejects with
   `traffic_projection_report_invalid`; a missing expected id makes its
   method `not_run` and blocks preflight even when the id's flags would
   tolerate a non-passed behavior axis. Reports must carry an object
   `observations` and strict provenance (non-empty, untrimmed
   `probe_version`/`run_token`, exact `YYYY-MM-DDTHH:MM:SSZ`
   `generated_utc`, 64-hex `contract_hash`/`expected_genesis`/
   `node.binary_sha256`, schema v2, canonical probe id). The record carries
   `target_label`/`target_kind`/`profile_name`/`expected_network`/
   `expected_genesis`/`node_binary_sha256` for T1 run binding, plus FULL
   per-check rows (`check_states`, both axes) so the gate recomputes
   everything - an aggregated state alone cannot be tamper-checked
   (CE1-CE3 regressions). The gate re-validates the record completely, not
   key sets only. `report_sha256` is a self-reported integrity marker, NOT
   cryptographic authentication.
4. **Two-stage verdict (F4)**: `recognized` is the preflight verdict -
   availability `passed` on every consulted row, no `failed` on either
   axis, all foundational rows fully passed, behavior `unknown`/`not_run`
   tolerated only on the canonical `unknown_behavior_ok`/`funded` flags
   (missing expected ids are never tolerated via those flags).
   `behavior_proven` is FALSE by construction for every read-only
   projection, even with hypothetically all-passed rows; T3 consumes its
   own exercised funded evidence. The gate requires `recognized` and
   asserts `!behavior_proven`. The r1-evidence `getmempoolinfo` shape
   failure blocks `recognized` honestly (selected-consumer result,
   preserved as-is, never reinterpreted, never a universal gate); there is
   no funded-broadcast requirement before bootstrap and no second
   probe/verdict.
5. **`send_user_payment` destination (F5)**: resolves the plan Section
   6.6/6.3 vs Section 12 ambiguity in favor of an explicit
   `destination_address: String` field on `SendUserPayment` (source of
   funds stays the named user and their per-identity payment wallet).
   Mandatory, non-empty, bounded (`DESTINATION_ADDRESS_MAX_LEN = 128`),
   fail-closed validated with the shared bitcoin types: parses as an
   address, valid for regtest (the lab's only network,
   `traffic_payment_destination_network_invalid`), and p2pkh (the frozen
   v1 spend-type gate, `traffic_payment_destination_script_unsupported`;
   other classes arrive with T4). No freeform endpoint, no default, no
   silent ignore. The remaining T1-before-signing checks (destination
   wallet/UTXO admission, exact output construction) are the signer's
   domain and are recorded as such in the type docs - the T1 signer
   handoff.
6. **Typed actor ids and history confinement (F6)**:
   `TrafficActorSpec.id`, `TxControlHello.driver_id`,
   `TrafficHistoryHeader.driver_id` and the command `driver_id` fields use
   the existing `ActorId` (128-byte ASCII charset, re-validated in
   `validate()` since serde-transparent deserialization does not validate).
   `driver.history` is confined to the `traffic/` subdirectory under the
   run private artifacts: first path component exactly `traffic`, at least
   two components, no `.`/`..`/backslash/absolute/whitespace - no
   collision with existing private artifact names, no arbitrary workspace
   paths.
7. **Protocol exactness and vocabulary (F8)**: `protocol.control` is an
   exact byte-for-byte string (no trim - surrounding whitespace rejects).
   The missing-record gate code is `traffic_capability_unrecorded`
   (distinct from the spec's `traffic_capability_missing`).
   `SEED_DOMAIN_PLANNED_SCHEDULE = "spc-lab/traffic/planned-schedule"` is
   frozen over `(run_seed, protocol_version, driver_id, sequence)` per plan
   Section 6.3. `TxLifecycleState::Dropped`/`Expired` terminality is
   explicitly UNDEFINED until T6 - the vocabulary slot is frozen, the
   semantics are not.
8. **Manifest field and old runners (F8)**: `MANIFEST_SCHEMA_VERSION`
   stays 1 with the additive optional `traffic_capability` field.
   `RunManifest` is `deny_unknown_fields`, so an old runner adopting a
   manifest that carries `traffic_capability` fails closed - accepted
   deliberately.
9. **T1b handoff clarifications (this change, doc-level only)**: the
   unknown-outcome retry cap is specified as one initial send plus at most
   N retry sends, with outcomes always recordable after the cap (the cap
   bounds sends, never evidence); bounds 0..=3 reject out-of-range values,
   never clamping. The lab deterministic key mode derives traffic wallet
   keys from the PUBLIC scenario seed bound to the run id - publicly
   reproducible lab keys, no secrecy claim; the exact per-domain input
   tuple is T1b's deterministic disjoint allocation over the frozen domain
   strings.

## T1b integration boundaries (flagged, not forked)

T0 owns the control/schema vocabulary; T1b maps counts to its deterministic
disjoint allocation and validates core-complete assignment against it. No
shared type fork is created and no unfinished core crate is imported. Exact
fields T1b will need to extend through a future recorded decision:

- `TrafficHistoryHeader` (v1: `history_version`, `run_id`, `driver_id`,
  `protocol`) does not yet carry `network`, `key_mode` or journal caps. A
  T1b journal header carrying those requires a `history_version` bump and
  a dated decision - not a silent field addition and not a forked type.
- `Invalidated` cascade semantics are T1b/T6 to specify over the frozen
  state vocabulary; `Dropped`/`Expired` remain undefined until T6.
- Explicit global per-actor index ranges and the exactly-one bootstrap
  owner are T1b allocation concerns over the T0 counts
  (`network_wallets`, `user_payment_wallets`, the seed-domain index
  functions); T0 freezes the counts and the domain strings only.

## Consequences

Traffic scenarios remain unstartable through `prepare` until T1 provides
the recording path (a fresh manifest never carries a record) - the
safe-unavailable state, documented in `docs/LAB-RUNTIME.md`. The production
gate's accept path exists exactly for approved-lineage, recognized records
on the adopt path. `DESIGN.md`/`docs/SPEC.md` authority is preserved; no
pool, payout, SV2, SPc or Ark semantics changed.
