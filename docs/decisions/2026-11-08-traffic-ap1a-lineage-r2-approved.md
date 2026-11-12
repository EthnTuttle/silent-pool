# 2026-11-08 - AP-1a r2 contract lineage approved and bound

## Scope

Binds the Fable-approved AP-1a r2 canonical contract as the sole approved
traffic-capability lineage for the T0 projection gate
(`crates/spc-lab/src/traffic/projection.rs`). This is a lineage-binding
decision only: it approves nothing else in T0, T1 or any node profile.

## Provenance

- Approval: `/home/ethan/code/silent-pool/docs/reviews/2026-11-08-ark-rpc-probe-fable.md`,
  re-review r2 (2026-11-08, final), "Decision: APPROVE - the bounded
  read-only probe and the exact contract
  `f4fc3e043f420870675095ab24144ba14e0afba6cb9ad69d818d99b1bccb2f62`."
- Approved packet: 74 files frozen by
  `/tmp/opencode/lab-ark-rpc-probe-r2-review-manifest.json`
  (`f0aa504aa6f247474eb247b4c25d12f286bb3f88abb1d42e77c3ba643f65ce12`),
  source frozen in the `lab-ark-rpc-probe` worktree.
- Hash verification (this change): `canonical_contract_json()` of the frozen
  source re-hashed to `f4fc3e04...` and its `checks` table compared
  field-by-field against the frozen `TRAFFIC_EXPECTED_CHECKS` - all 17
  consulted rows (12 five-method + 5 foundational) match exactly on
  `(id, method, coverage, unknown_behavior_ok)`; the `funded` coverage class
  sits on `sendrawtransaction.recognized` alone among them. The integration
  test `frozen_expected_checks_table_is_the_approved_r2_contract` pins the
  17 rows and the allowlist as the atomicity guard.

## Decisions

1. `AP1A_APPROVED_CONTRACT_SHA256S` is exactly
   `["f4fc3e043f420870675095ab24144ba14e0afba6cb9ad69d818d99b1bccb2f62"]`.
   A record carrying that hash passes the lineage stage; every other hash
   fails closed.
2. The superseded r1 candidate `11bdb100...` stays in
   `AP1A_RECOGNIZED_UNAPPROVED_CANDIDATES` (diagnostics only, rejected with
   `traffic_capability_lineage_unapproved`). Unknown hashes are rejected
   with `traffic_capability_lineage_unknown`. The empty-allowlist path
   (the pre-approval safe state) is preserved and unit-tested through the
   private `check_traffic_capability_with_lineage` seam; no
   caller-configurable runtime approval bypass exists.
3. Atomicity rule (unchanged, now exercised): the approved hash list, the
   frozen `TRAFFIC_EXPECTED_CHECKS` table and any expected-profile mapping
   change together through a recorded decision - a hash is not a shape
   check, and a string-only substitution is never a valid approval.

## Approval scope limits (review R2-7, binding on this binding)

- The approval covers the read-only probe as frozen and the canonical
  contract JSON (contracts, encodings, check ids with coverage class /
  `unknown_behavior_ok` / funded flags, verdict rules). It does NOT approve
  T0 code, the traffic capability, any node profile, a Core RPC-version
  compatibility claim, a `bark-v31` named profile, a node patch or version
  override, bark acceptance, or any Ark or traffic runtime claim.
- The `getmempoolinfo` row stays `passed / failed`
  (`maxdatacarriersize: null`) on both approved binaries for the SELECTED
  MODELLING CONSUMER (corepc-types 0.13.0 `v30::GetMempoolInfo` via
  bitcoind-async-client `Reader::get_mempool_info`). T0 preserves it as a
  real `failed` row that blocks the `recognized` stage honestly. It is not
  reinterpreted, not coerced, not a universal traffic failure and not a
  Bark requirement (bark c419159 never calls the method). Whether T1's
  `RpcNodeControl` route requires the full v30 superset or a
  traffic-subset decoder is the explicit T1 + canonical-owner consumer
  decision, still pending. No Ark or Core version readiness claim is made
  or implied.
- Funded behavior, witness/preflight proofs (T3) and live bark remain
  separate gates; `funded_state: unproven` is permanent in read-only mode
  and `behavior_proven` is false by construction for every T0 projection.

## Consequences

The production gate now has an accept path: an approved-lineage record whose
rows recompute to `recognized` passes `check_traffic_capability` (and the
engine adopt path). Nothing can flow through `prepare` yet - a fresh
manifest never carries a record, so every traffic scenario still rejects
with `traffic_capability_unrecorded` until T1 provides the recording path.
Any change under `testing/ark-rpc/**` requires a new evidence run and a new
hash; when one lands, this decision is superseded by a new dated record
that updates the hash and the table atomically.
