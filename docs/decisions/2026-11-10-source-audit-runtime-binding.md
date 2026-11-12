# 2026-11-10 - Source-audit runtime binding and committed accounting ancestry

## Trigger and authority

Parent requested the real production adapter for QA's immutable-window oracle,
not a pool-provided owed scalar or fixture-only invocation. v2 does not publish
type-32 windows, and SPEC §4.2 deliberately omits identity from share leaves.
Identity attribution therefore remains explicitly pool-ingress-attested;
arithmetic, raw PoW/Merkle/prevout fees and coinbase ownership are independently
checked. No claimed independent identity proof is added to identity-free leaves.

## Decision

- Actual front generation/activation/share/bootstrap/ledger callbacks produce
  bounded private `spc.source-evidence/2` records (WIRE §3.9), binding actual
  prefix/suffix/Merkle/header/extranonce/target context, raw transactions,
  witness plan, frozen window/profile/Q/high-water, full owed/key arrays and
  source-attributed accepted share material. Missing event/reference/head or
  complete watermark is UNKNOWN. A hash chain needs independently captured
  heads; a bundle cannot authenticate its own replacement head.
- `SPc/JobWindowV1` commits the exact selected slice IDs/front IDs/roots/work
  sums/profile plus Q/rule. A changed observed window rolls job identity even
  if its relative scores leave every coinbase output and owed amount unchanged.
  Later SQL/RAM tails cannot silently replace an issued generation's window.
- Migration 0008 records actual committed accounting transitions, including
  orphan deletes, under transaction IDs and collection watermarks. An empty
  history-start proof or independently validated external checkpoint is required.
  Stored journal rows exclude destinations, order/proof bytes and VTXO IDs,
  retaining only the accounting-history attribution already permitted by §5.1.
  Full source collection is operator/private-broker-only, never an Ark input.
- `spc.payout-proof/1` carries raw immutable job/share/prevout/window evidence
  and separately verified pending-history checkpoint. Its Rust verifier checks
  actual issued bytes, raw share PoW, full transaction Merkle sets, exact work
  ratios, fees, frozen Q/profile/window and full owed. Collector/history seals
  are external trusted inputs; unverified pool totals are insufficient.
- `PoolJob::verify_source_bundle` binds the actual SV2 job reader to
  `verify_source_entitlement` and `verify_entitlement`. Translator
  `bind_and_run_with_auditor` runs the wallet-local `SourceJobAuditor` before
  every downstream job publication. Refuse mode without a complete proof feed
  fails closed. The default warn-mode translator is not described as audited.
  AP-3b supplies its provisioned scan material and protected collector feed;
  scan/master/spend secrets are never transferred into CPU-miner messages/config.
- Pool routing's denomination set is validated against the configured hard
  budget ceiling; the autoscaler floor cannot silently become a trim cap.
  Actual serialized coinbase/block weight and template reservation remain
  load-bearing. The default ceiling is the specified 1,000,000 WU.

## Interfaces and evidence

Exact source/schema/query/API mapping is
`docs/reviews/2026-11-09-ladder-source-api-handoff.md`. QA owns Python collection
and its namespace storage-broker adapter; AP-3b owns wallet watch scheduling.
Code hooks are not claims of an exercised collector feed, funded Ark lifecycle,
or full native-SV2 acceptance. Those require their own real runtime receipts.
