# 2026-11-10 - Full atomic recanonicalization of loss-held blocks

## Authority

Parent-directed Rule-B/loss-held review: no partial rebooking to pass tests;
preserve UNKNOWN when irreversible provenance is unresolved and account for
confirmed recovery of an orphaned payout block accurately.

## Transition

Payout freshly verifies raw canonical ancestry and depth, then invokes
`economic_restore_loss_held`. The store checks the exact event digest and
frozen ledger, parent anchor, retained monetary rows and reserved/Sent Ark
provenance under the accounting transaction lock. It restores every missing
original row and its additive pending/Ark effect; retained rows are verified,
never applied again. Complete ledger coverage and B>=R are mandatory before
commit. The transition advances to booked once, without provisional partial
booking. A replay makes no monetary change.

Confirmed recanonical membership clears its orphan flag and recomputes only
the affected Sent aggregates' pool loss from remaining orphaned members.
Existing possibly-sent orders stay pinned; no replacement grant or delivery
is created by this transition. Raw-event loss, stale chain evidence, shallow
depth, differing effects/due policy or insufficient available pending holds
UNKNOWN. The event/history transaction is the recovery evidence.

## Checks

Authored real-PG cases cover positive/negative restored effects, an earlier
canonical unbooked reservation, retained Sent, shallow rejection, replay and
pool-loss correction. Native and funded tests remain separate and unrun on
this source until the parent assigns the heavy slot.
