# 2026-11-09 - Frozen aggregate anchor and fresh send authorization

## Authority and conflict

Parent/operator authorization fills AGGREGATEDEST in SPEC §11.3. Its existing
"on failed or no receipt ... re-order next block ... after retry_blocks ...
credit pending_balance" rule conflicts with the authorized uncertainty policy
for potentially sent aggregates and is superseded below. Time and transport
failure do not prove no effect. DESIGN derivation and stock server RPCs stand.

## Binding decision

- Same identity only. Canonical rows sort by stored block height, block hash,
  identity bytes. Freeze the complete bounded member prefix, member amounts,
  oldest anchor, its STORED destination, checked total and opaque order ID in
  one transaction before publication. No current-registration replacement,
  synthetic epoch, partial member, rounding or forfeiture. Actual discovered
  server/order caps bound the prefix; remaining rows stay queued. Threshold is
  10,000 sats or oldest due-height age 144 blocks at the selected canonical tip.
- Before EVERY send attempt, the Ark worker requests authorization with a fresh
  random nonce and frozen-order digest. Payout freshly checks ALL covered blocks,
  then durably records potential-send state before returning a signed grant
  bound to order ID/digest, nonce, revision and expiry. Only the currently
  correlated live response permits one call; cached TTL booleans do not.
  Expired/lost/unknown grant permits no replacement send. The external chain
  may race after the check, which is explicitly possibly-sent risk.
- Once publication/authorization may have escaped, never regroup or reassign
  members until definitive outcome. Reorg touching any authorized/unknown/sent
  member pins the WHOLE set, stops attempts and retains loss-risk/reservations.
  Definitive Sent applies all member allocations atomically and idempotently;
  orphan-member value is pool loss, never fictitious unsent reversal. Definitive
  NoEffect atomically cancels, reverses orphan members and requeues only canonical
  members. Tombstones remain. Partial/invalid receipts release nothing. Without
  supported wallet/server proof AND durable sender receipt, remain
  `blocked_unknown`; no imaginary order-ID lookup, timeout refund or custom RPC.
  A never-published/never-authorized set can cancel under row-lock proof.
- Client anchor mailbox watches persist fairly until the wallet retrieves and
  verifies its owned VTXO/proof, or explicit terminal reconciliation proves no
  delivery is owed. Pool Sent/opaque VTXO IDs wake immediate fetch; they do NOT
  prove wallet receipt and never close the watch. Status includes own unbatched
  positive Ark obligations before 144-block batch formation. Once frozen,
  nonanchor members can follow the anchor without losing watch coverage.
  Coinbase discovery, H+1014, API outage and guessed expiry never close them.
  Own anchor/covered membership is available only through the existing fresh,
  single-use B_scan challenge-authenticated API (`spc.aggregate-status/1`).
  Replayed SetArkIdentity/AlreadyApplied never authenticates private reads.
  Scan secrets remain wallet-local; no master or SP spend key is required.
- Mappings, identities/rank and block hashes remain SPc-side. Ark authorization
  messages carry only opaque order ID, frozen-order digest, fresh nonce,
  revision, expiry and signature. The Ark role/server sees no covered mapping.
  Redis ACLs admit only the opaque authorization/order/receipt channels.

## Scope

Migration 0007 adds SPc-side frozen-order/membership/authorization records;
0006 is untouched. WIRE-FORMATS §3.8 defines bounded authorization messages.
SPEC §11.3 and §11.5 are amended. No upstream interface or pin changes.
Authorization is implementation authority, not test or funded-readiness evidence.

**Order-ID precision.** The older SPEC tuple `(sp_id, oldest block hash)` alone
cannot distinguish a definitively cancelled tombstone from a new batch that
reuses a still-canonical anchor. The bounded amendment adds the transactionally
allocated `batch_seq` to the `SPc/ArkAggregateV1` hash preimage. Order IDs remain
opaque 16-byte hashes and restart/replay stable for each frozen record; cancelled
order IDs never name a different payment or membership.
