# 2026-11-09 - Contextual native regtest difficulty consumer

## Decision

The node's no-periodic-retarget profile retains its independent minimum-
difficulty flag and the regtest overlay's gap/boundary/inheritance exceptions.
Initial-target metadata is not a constant future-header target. The previous
T2/observer interpretation lost that policy fact; its preserved refusals are
consumer failures, not node-consensus failures.

ChainPolicy now requires `allow_min_difficulty_blocks`, `pow_target_spacing`,
`difficulty_adjustment_interval` and `regtest_pow_bits_overlay`. The controller
projects actual bound NodeEffectiveProfile facts and refuses absence. No
default spacing/interval or inferred min-difficulty flag is permitted. The
supported lane requires a reported regtest overlay, positive non-overflowing
spacing/interval and no periodic retargeting. Unsupported periodic retargeting
fails before hash authorization. Control v1, journal 3 and key derivation 2
stay unchanged; old private actor configurations missing facts refuse load.

VerifiedHeaders owns a complete genesis-inclusive sequence whose hashes,
parent links, PoW and expected bits have been checked. Candidate height and
parent hash must identify its exact tip. Both GBT admission and canonical raw
observer/completion validation use the same pure contextual calculation:

1. A no-retarget adjustment boundary inherits parent bits.
2. Otherwise, with min difficulty enabled, time strictly greater than parent
   time plus twice the bound spacing requires pow-limit bits. Without that
   gap, walk back across pow-limit parents until a non-minimum header or the
   previous adjustment boundary. With min difficulty disabled, inherit parent.
3. Apply the regtest overlay only to a resulting pow-limit compact outside an
   adjustment boundary and outside an enabled minimum-difficulty gap. Harder
   inherited bits are preserved. Checked arithmetic refuses unsupported range.

This is the exact order in the source-bound node's
`rbitcoin-consensus/header.rs:217-365` and `query/header_read.rs:173-202`, not
a generic Core rule substituted for that node's overlay semantics. Source
trace and unchanged-node proof: `/tmp/opencode/node-target-semantics-r3/REPORT.md`.
The node remains authoritative. GBT target must match valid compact bits,
raw parent context is independently verified, and each fresh template's
timestamp/parent is checked before reservation. No historical timestamp is
fabricated. Contextual `207fffff` is honored with the same declared 40 H/s and
finite crash-safe attempt/deadline budget; it is not an arbitrary fallback.

## Evidence and acceptance

Fresh native T2 acceptance passed at
`/tmp/opencode/T2-native-funded-20261110-context-v1`: two actual nodes,
105 native blocks, 80 outputs across two actual wallet actors, four confirmed
same-host signed transactions, 1,632 reserved/reconciled measured attempts
under the unchanged 40 H/s budget, and successful independent completion plus
cleanup. Other roles were explicitly inert fixtures; no full T3/Ark claim.
Exact receipts and retained earlier failures are in the implementation report.

Earlier zero-hash refusal receipts remain unchanged. Tests cover fresh long
gap, exact two-spacing boundary, short-gap recovery, enabled/disabled min
difficulty, no-retarget boundaries, harder inheritance, multi-minimum
backtracking, missing/overflowing facts, sparse ancestry, equal-height forks,
and agreement between real hashed raw headers and observer validation.
The independent live oracle derives these rules separately from raw blocks
and live node facts. Real funding acceptance is recorded only after native
blocks, per-outpoint maturity and the same-wallet signed distribution are
verified canonically on every actual node.

## Sections and interfaces

- `contextual_pow.rs` and its tests: pure shared verified ancestry/rules.
- `observer.rs`: required policy fields and per-header contextual validation.
- `native_bootstrap.rs`: raw authority context before GBT hash authorization.
- `engine/traffic_runtime.rs`: actual bound profile projection.
- Actor/completion fixtures carry explicit policy facts.
- SPEC frontmatter records this lab-only amendment. Pool payout/Ark policy,
  node policy, control messages, journal bytes and signer logic are unchanged.
