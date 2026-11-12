# Ladder source/API handoff - candidate source, not live acceptance

Owners: QA collects/normalizes Python/SQL in its own tree; AP-3b owns wallet
scheduling. This packet changes only `lab-ladder-completion` source.

## Actual producer and schema

`bins/spc-pool/src/evidence.rs` emits private operator trace target
`spc_evidence`. WIRE §3.9 defines `spc.source-evidence/2` hashes/watermarks.
Collection must start before process startup and bind the independently captured
head for each process session. Missing prefix/head/event/reference is UNKNOWN.
The hash chain does not authenticate a head taken from the bundle itself.

Actual source call sites:

- `Front::refresh`: `distribution_frozen` on an issued-generation change, before
  moving its immutable job/distribution into front state. Includes raw job
  prefix/suffix, Merkle path, version/ntime/prevhash/bits, extranonce length,
  target/credit/profile/generation, raw non-coinbase transactions, witness
  root/reserved/commitment, exact selected slices/high-water/Q and full owed/key
  arrays. Large events emit `proof_unavailable`, not a completeness assertion.
- `evidence::job_issued`: actual ingress activation and complete emitted job
  bytes. `Published.job_id` remains in main's publication identity.
- `main::publish_events` -> `evidence::source_publication`: exact `raw_d_hex`,
  Redis stream/id, publication time/completeness, coinbase fingerprint and
  separately computed SQL snapshot key. The source sequence/hash supplies the
  retained publication boundary. This is a real XADD outcome, not inferred
  from a later SQL block. Join slice stream IDs here; a RAM-sealed slice can be
  visible at job build before its subsequent publication is acknowledged.
- `Front::on_share`: `accounted` only after successful slice push; adds actual
  `slice_id`, `credited_job_fees`, ingress `channel_id`/identity assignment,
  work, nonce, ntime, version, full extranonce, raw coinbase/header digest.
  `share_rejected` records actual stale-old-job rejection with active job ID.
- `Front::apply_bootstrap` and `apply_front_event`: actual bootstrap revision,
  pending checkpoint/applied-block list, and committed LedgerApplied deltas.
  Bootstrap is explicitly checkpoint-dependent, not fabricated genesis history.
- Migration 0008 `source_accounting_history` journals actual committed SQL
  transitions, including orphan DELETEs. Crypto destinations/proofs/IDs are
  redacted from this permanent accounting-only history; it remains SPc-private.

No secret scan/spend/master/a_send material is emitted. Trace attribution is
private operator evidence; none of it belongs in Ark streams or user-wide DTOs.

## Exact QA adapter mapping

Use `(source.session, front_id, job_id, generation)` to join trace records;
the SQL share leaf's job ID is `(height << 32) | int(job_id,16)`.

| QA `payout.py` field | Actual source |
|---|---|
| snapshot | `block_found` JOIN `distribution_snapshot` on stored fingerprint handle; SPCB/2 event fingerprint remains the separate coinbase fingerprint |
| entries[identity].p_x | `ds.entries` array, one logical row per sp_id, mandatory format_version=2, p_x arrays in k order |
| full owed | `entries[].owed_sats`; independently recompute from proofs/history, never accept it as authority |
| slices_newest_first | frozen high-water/Q selection; SQL slice + slice_credit + shares, joined with actual accounted records by slice_id and share leaf fields |
| jobs / source share job_key | distribution_frozen + job_issued for exact generation; raw prefix/suffix/Merkle/target/header fields, not inferred from the found block |
| job.prev_hash_le | trace prev_hash (internal raw bytes encoded hex) |
| job.merkle_path | trace merkle_branch (deepest first) |
| job.share_target_be | trace share_target (explicit big-endian integer) |
| share.share_hash_le | trace share_hash (raw SHA256d bytes, compared little-endian for PoW) |
| share.credited_work | trace d/m_issue; independently verify reference-target ratio |
| job.transactions | actual distribution_frozen.transactions raw hex; obtain each real prevout transaction through the owned node/broker RPC/chain collector |
| prior pending | replay complete committed source_accounting_history and raw canonical settlement/reorg ancestry; do not use current pending_balance as a prior default |
| ledger | exact ledger_history rows plus history tombstones/transaction membership; grouped coinbase row includes ark_due_sats remainder |
| delivery | frozen entry destination presence/digest plus committed registration source state; mutable current registration is not a past destination rewrite |

QA precision: `payout.py` currently compares sorted denomination multisets at
line 119. It must also compare each k-associated actual amount to the greedy
piece at that k; otherwise swapped amounts at valid keys pass. This tree does
not edit QA's files.

SQL collection uses one `BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY`:

```sql
SELECT pg_current_snapshot()::text, pg_current_wal_lsn()::text;
SELECT max(event_id) AS history_watermark FROM source_accounting_history;
SELECT * FROM source_accounting_history ORDER BY event_id;
SELECT bf.*, ds.* FROM block_found bf
LEFT JOIN distribution_snapshot ds ON ds.fingerprint = bf.fingerprint;
SELECT s.*, sc.sp_id, sc.diff_sum, sc.diff_fee_sum
FROM slice s LEFT JOIN slice_credit sc USING(slice_id)
ORDER BY s.slice_id DESC, sc.sp_id;
SELECT * FROM ledger_history ORDER BY block_hash, sp_id;
SELECT * FROM ark_payout ORDER BY block_hash, sp_id;
SELECT * FROM ark_aggregate ORDER BY batch_seq;
SELECT * FROM ark_aggregate_member ORDER BY order_id, ordinal;
COMMIT;
```

The collector must paginate/bound retained results under the SAME snapshot,
check `history_start.complete_from_empty` or validate an external checkpoint,
and require every referenced row/generation. SQL sequence gaps can be rollback
allocations: transaction_id and committed snapshot delimit completeness.
Namespace SQL goes through the owned storage broker; that collection adapter
belongs to QA. Tables/proof traces are never exposed to the Ark role.

Additional named columns are enumerated machine-readably in
`/tmp/opencode/ladder-completion-owner-api-r1.json`: `ark_aggregate`,
`ark_aggregate_member`, `ark_send_authorization`, `source_accounting_history`.
There is no fictitious provisional/publication table. Actual publication bytes
are retained in the private source trace and Redis; provisional tracking is a
known baseline implementation gap. `BlockFound` is currently ignored by front
and no runtime producer populates `provisional_paids`. Missing provisional
ancestry is UNKNOWN, never a zero balance. Parent clarification was requested
for consumed-prior-pending reservations versus full unbooked-paid subtraction.

## Runtime miner verifier entry point

`PoolJob::verify_source_bundle` in `bins/spc-agent/src/sv2_client.rs` consumes
the ACTUAL received SV2 job and negotiated extranonce context, plus wallet-local
scan key/spend public key and an independently sealed collector bundle/history
checkpoint. It calls `verify::verify_source_entitlement`, then
`spc_scan::source_audit::verify`, then `verify_entitlement` over outputs decoded
from those actual job bytes. It cannot replace source evidence with an owed scalar.

Typed schema is `spc.payout-proof/1` in
`crates/spc-scan/src/source_audit.rs`: raw job/prevout transactions, immutable jobs,
attributed accepted shares, sealed roots/work, frozen profile/high-water/Q,
complete-source watermark and separately verified pending-history checkpoint.
It independently checks raw headers/PoW, Merkle sets, fees, exact work ratios,
window arithmetic and full owed. Pool ingress attribution and collector history
validation are explicit trust boundaries; they are not invented identity proofs
in identity-free share leaves. TrustedSource roots must arrive from the protected
collector/chain verifier, not fields supplied by that same untrusted bundle.

`translator::bind_and_run_with_auditor` now invokes `SourceJobAuditor` before
EVERY downstream job publication, including the initial job. Its `SourceProofFeed`
supplies the bounded bundle plus independently captured source/history seals;
scan material stays wallet/agent-local. Refuse mode without that feed rejects
startup rather than silently mining unaudited work. AP-3b/QA connect their
protected collector output through this runtime trait. Missing
collector/watermark/history stays unknown; this packet does not claim the live
pre-mining feed or full funded acceptance has run.

## AP-3b Ark watch/status contract

Fresh single-use B_scan API auth protects:

- `GET /api/sp/me/ark-aggregates`: `spc.aggregate-status/1`, own positive
  obligations (including unbatched), complete frozen anchor/member lists,
  state/revision, loss risk and proof-derived expiry when available.
- `GET /api/sp/me/ark-aggregates/<order_id_hex>/receipt`: bounded binary SPRP/1
  containing complete stock VTXO proofs, including every chained-send VTXO.

Pool Sent/IDs wake immediate retrieval. Only wallet-verified owned VTXO/proof,
or explicit terminal reconciliation with no delivery owed, closes a watch.
`wallet_received` is null. Nonanchor obligations may follow the anchor after
freeze without disappearing. API outage/unknown/coinbase discovery/H+1014 never
close watches; keep fair bounded continued reads. No static registration replay
authenticates these private reads. AP-3b source was not edited.
