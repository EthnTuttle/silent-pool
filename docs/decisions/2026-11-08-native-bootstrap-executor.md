# 2026-11-08 - Native legacy bootstrap executor (T2-L)

## Scope

Implements approved traffic-runtime plan §4 and the §3/§4 reservation contract,
SHA256 `44daaa2baa41a4b93eb46a9e69dfdcf2bba1e0ce73367e27e83a87ab3a6b3307`.
Lab-only implementation decision; no pool payout, SPc, Ark, or witness-spend
semantics change. Source implementation is not a funded acceptance result.

## Decisions

1. Install `NativeBootstrapExecutor` after the selected host actor is launched
   with the bound effective chain policy and validated complete allocation.
   An explicitly injected executor is preserved. The existing long-operation
   controller, actor RPC allowlist, signer and independent completion verifier
   remain the authority for their respective operations.
2. Use the public `spc-cpu-miner` header/merkle/hash and budget/pacing library.
   Reserve at most 16 attempts before each batch; validate operation, actual
   actor incarnation, batch identity, count and cumulative debit. Both actor
   and executor consume authorizations at most once. A failed/lost ack grants
   no work. Reconcile actual attempts separately; no reservation is refunded.
   One pacer and one finite budget span all templates and blocks. The Linux
   boot id and absolute monotonic deadline come from the controller budget.
3. Accept strict integer GBT metadata and byte-roundtripping raw transactions.
   Reject witness transactions without changing selection or fees. Validate
   compact target sign/zero/overflow/pow limit, separate GBT target equality,
   and effective initial target equality. This implementation supports the
   bound no-retarget schedule; a retargeting profile rejects because the
   existing `ChainPolicy` omits the additional schedule inputs it would need.
   Coinbase subsidy is independently halved at the bound interval and added
   to exact integer included fees. GBT coinbasevalue must equal that result.
4. Construct BIP34/null-prevout legacy coinbases paying the exact P2PKH script
   returned by the same actor's funding-address operation. Verify actual raw
   block bytes, txid Merkle root, header PoW and output on every actual node,
   plus common canonical presence, before asking the actor to observe it.
   Actor observation admits the real tip before funding. Maturity is per
   outpoint; 101 blocks mature only the oldest coinbase.
5. Distribution is a deterministic bounded sequence through the existing
   multi-destination signer. Before any send, budget the whole allocation from
   one conservatively selected mature coinbase amount. Reserve the declared
   maximum fee per part and a dust-sized funding remainder, divide the rest
   equally by the exact global output count, and leave integer rounding and
   unused fee reserve in host change. Output amount, count, part, txids and
   resulting canonical evidence are recorded. Every part is confirmed before
   the next. The common raw chain must prove each output's exact script/value,
   fee and native mature-input ancestry before completion is returned.
6. Parent interpretation approved on 2026-11-08: raw attempted headers,
   blocks, authorizations and measured attempts belong in create-new mode-0600
   `native-bootstrap.jsonl` under the validated, owned, non-symlink mode-0700
   driver root. It is observation-only, with no budget, signing or replay
   authority. Write, flush or sync failure stops the operation. The durable
   controller CommandLog remains the sole hash-reservation authority even
   if this artifact is lost or truncated; the original core funding journal
   remains the sole money authority. Existing-artifact restart refuses rather
   than deleting the artifact or duplicating execution.
   Record the artifact path and source/hash provenance through private
   evidence or the appropriate existing manifest channel. Never publish raw
   records through user/public `TrafficObservation`; no second ledger or new
   public schema is required. The finite operation budget bounds artifact
   growth. `source=bootstrap` and exclusion flags are claims for the oracle
   to check against the actual measured boundary, not sufficient proof by
   themselves. This is an approved interpretation, not a pending workflow
   hold; runtime conformance remains an implementation acceptance check.
7. Restart/re-entry never redeems an old permission. An outstanding debit or
   existing evidence artifact rejects automatic re-execution. There is no
   controller-resume promise in this source packet. Cancellation and pause
   are cooperative between short batches and bounded T1 requests; deadline
   time continues while paused. Any uncertain submission stays uncertain
   unless actual canonical raw evidence resolves it.

## Sections edited

- `crates/spc-lab-txd/src/native_bootstrap.rs` and its offline tests.
- `crates/spc-lab-txd/src/lib.rs`, package Cargo manifest and root lock entry.
- `crates/spc-lab/src/engine/traffic_runtime.rs`: narrow host installer.
- SPEC frontmatter changelog only; normative payout sections are unaffected.

## Acceptance boundaries

2026-11-09 implementation/verification clarification: private evidence has
an explicit 256 MiB byte ceiling in addition to finite attempt/deadline
limits. Root ownership/type/0700 are rechecked before file creation. The
existing private artifact channel records the artifact path, compile-time
executor source hash, executing controller hash and manifest source claims.
The compiled executor preserves rejected GBT metadata privately. Fifteen
native tests and affected checks pass, but both authorized real-node builds
return first-height GBT bits 207fffff against the bound fixed 200fffff policy.
Real funding therefore fails before any hash authorization. No fallback
target, mock time or generated funding is permitted by this result. See the
implementation report's current verification section for exact receipts.

The earlier parent-interpretation update was documentation-only. Its prior freeze,
decision and report bytes are preserved at
`/tmp/opencode/T2-native-r9-preserved-9c39eca8/`; source was unchanged at that checkpoint.
Private source/artifact-location provenance is recorded in
`/tmp/opencode/T2-native-private-artifact-provenance.json`. A source receipt
is not evidence that a runtime artifact or a compiled executable exists.

J1 positive selected-consumer metadata and approved lineage, J2 exact live
launch-profile/binary/genesis/effective-policy equality, compiled T1b/core
integration and a fresh native node chain plus real signed distribution are
required before funded T2/T3 readiness. Offline fixture tests prove no node
compatibility or wallet credit. T1-DEST remains a separately owned output
extension; T4 remains the witness-spend gate. No Ark funding is claimed.
