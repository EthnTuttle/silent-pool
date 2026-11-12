# 2026-11-11 - Retain actual full-rail decision observations

## Authority

User instruction and QA `T3-fullrail-r19/PRODUCER-FIELDS-NEEDED.md`. This closes
historical observation gaps; it does not change entitlement, tiers, JD/O11,
membership, fee policy, the ten-second authorization lease or Ark server RPCs.

## Decision

Record actual complete settlement delivery reads in the same repeatable-read
transaction as their append-only receipt. Record queried identities and actual
delivery/registration/epoch rows, including explicit proven nulls. Failed reads
are unknown, never absence. Bind accepted reconstruction to its exact read and
link it immutably to economic acceptance. Existing blocks keep a null reference.

At front derivation retain public input availability, committed request/revision
metadata, actual mailbox selection, destination bytes or actual error stage and
reason. These are front-cache observations; they are not later settlement reads.
Persist new frozen-job observations before advertising that generation. Front
sends a private SPc-only observation to payout and awaits its digest-bound
receipt; payout remains the sole Postgres writer. Front SQL access stays read-only.
Failed unadvertised attempts retain partial/unknown status and no invented job ID.

At authorization retain the actual bracketed ChainView reads, complete queried
membership, issuance clock and signed bytes atomically with the existing SQL
checks. Full authorization-row history survives working-row deletion. An opaque
lease-consumption observation must be durably acknowledged before the stock
cosign edge under the same wallet permit; the same grant is revalidated afterward.
This adds no grant and extends no lease. Missing receipt or post-grant expiry stays
unknown/pinned. Consumption is not delivery, refund, cancellation or NoEffect.

All identity/chain/member evidence stays in SPc SQL/source records. Ark receives
only SPAC/1 and SPCA/1 opaque fields; its two additional ACL selectors are XADD
on the consumption stream and HGET on the receipt hash. No private scalar is
recorded, and no broader key/stream permission is granted.

## Schema and verification

`docs/FULLRAIL-OBSERVATIONS.md` specifies migration 0012, versioned bodies and
the local frames. New history begins at installation; old r5/r16 and approved
2314 receipts are never backfilled or relabelled. New PG, opaque-frame,
front-derivation, rollback/scrub, timeout and restricted-role tests are authored.
This source-only packet awaits its assigned build slot and independent review.
