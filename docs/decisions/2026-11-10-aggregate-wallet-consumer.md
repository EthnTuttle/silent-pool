# Aggregate-aware wallet proof consumer

Date: 2026-11-10
Status: authored follow-up; compilation and funded acceptance pending

## Authority

Apply the assigned aggregate client contract and the ladder owner's binding
2026-11-09 aggregate-anchor/send-authorization decision to SPEC §11.3 and §11.5.
The approved wallet durability and UI-provider packets remain the base layers.
The dedicated changelog fragment is for serial SPEC integration by the owner.

## Decision

- Every private status or receipt read starts with a new single-use B_scan
  challenge. The SPc API origin and pool order-signing public key are explicit
  configuration; registration replies never authorize these reads.
- Positive unbatched obligations and complete frozen ordered member sets live
  in the existing private wallet SQLite database. Omission, API error, reorg,
  coinbase discovery and H+1014 cannot settle them. Cancelled tombstones allow
  authenticated regrouping but do not independently prove a member is paid or
  no longer owed. Status ingestion never credits money.
- Sent wakes receipt verification. Verify the exact signed frozen ArkSendOrder
  and its grant digest, derive the real stored anchor destination with the
  existing key algorithm, compare the entire Pubkey policy and full configured
  ArkInfo-validated server key, reject duplicate IDs and incomplete/excessive
  value, and validate every complete stock genesis/canonical funding proof.
  A grant's historical expiry is not a new send permission or a reason to
  discard an otherwise valid historical receipt.
- Owned mailbox proofs are durable candidates until the aggregate join passes.
  Pagination checkpoints follow durable candidate storage. They cannot credit
  an arbitrary subset before the full aggregate validates. One transaction in
  the existing ledger credits unique outputs, records receipt allocations,
  attaches every member and closes its covered watch. Replays cannot re-credit.
- Keep origin proofs for verified deliveries whose source blocks later orphan;
  Ark funding proofs still require canonical validation. Scanner observations
  retain all owned derivation-index/vout/tweak rows and restore canonical flags
  idempotently on a reorg back to an already observed block.
- Preserve bounded durable round-robin service and R1-R6 command/receipt
  protections. After mailbox exhaustion, an unknown obligation stays watched;
  the retry counter saturates at its last scheduled offset. The existing
  private UI provider observes only freshly revalidated ledger inputs.

## Explicit integration dependency

The parent assigned `/api/sp/me/ark-aggregates/<order_id_hex>/order`, authenticated
with a fresh single-use B_scan challenge and returning exact canonical stored
ArkSendOrder bytes including signature as `application/octet-stream`. The
client's fetch/storage path is bound to producer source
`0515a6a88686f75ccf4ed0ff768a0a0324c7a0b3ae4b95b5acd4a2c126a48a7d`
and schema `ae1eaed457e2d3373b3e4ffd032def20bdf10f806539769cbd5b64ccd901c466`.
Its named 93/348-byte codec bounds are selectively imported; success requires
HTTP 200 with the declared content type and `no-store`. Missing or invalid replies keep
`aggregate_frozen_order_unavailable`; all watches/candidates remain durable.
Explicit per-member no-delivery reconciliation is also not present; retain
those watches conservatively.

The runtime join derives a host-loopback origin only from the selected front's
leased API listener and verifies its owned process/socket before wallet launch.
Its order key comes from the actual public front/Ark-role configuration, bound
to the pool provisioning config hash. Persisted wallet bindings are revalidated
on the attach/UI and pre-miner paths without reprovisioning. The current
self-hosted Ark runtime rejects topology placement; this join also refuses it.

## Sections and evidence

- SPEC §11.3 and §11.5: dedicated changelog fragment; no wholesale SPEC import.
- `docs/reviews/2026-11-10-aggregate-wallet-consumer.md`: implementation and
  verification boundary, including disk refusal and serial runtime union.
