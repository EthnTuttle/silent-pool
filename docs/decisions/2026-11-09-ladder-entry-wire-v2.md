# 2026-11-09 - Bounded ladder entry wire v2 and full-entitlement binding

## Trigger

The binding 2026-11-03 decision and SPEC §4.7/§5.1 require one logical
identity row carrying an array of output keys. WIRE-FORMATS §3.1 instead
still encoded one `B0_32 p_x`. This is a lower-document inconsistency, not
permission to duplicate identities or reinterpret the ladder. The old entry
also cannot bind a kept miner's exact remainder: `sats` alone records only
the shaped payment. Different full entitlements can have identical outputs.

## Decision: named bounded wire amendment `ladder-entry-v2`

- Internal `spc:blocks` starts `SPCB` (four ASCII bytes), `U8 2`. This version
  is independent of work-unit versions 1/2 and `window_rule`.
- Each entry has exactly one identity, rail, `U16 key_count`, fixed 32-byte
  x-only keys in contiguous k order, `U16 dest_length` and destination bytes,
  `U64 sats`, then mandatory `U64 owed_sats`. Keys bind greedy-largest-first
  amounts before global script sorting. `owed_sats` freezes the full
  `raw + pending` entitlement, including the remainder. JD keeps exact weights.
- Key count is bounded at 2048 per identity and per block. The normal slot
  policy remains 1024; 2048 is documented fee-spike headroom. Unknown versions,
  invalid points, duplicate identities/keys, non-coinbase keys, empty coinbase
  key arrays, excessive counts and trailing bytes fail closed. This bound is
  a representation limit; actual serialized block/coinbase weight also binds.
- JSON/SQL entries require `format_version: 2`, `p_x` as an array of 32-byte
  arrays, and `owed_sats`. Old JSON/stream records are not silently converted.
  Deploy front/payout/readers together. Historical pending work requires an
  explicit version-bound recovery path; this amendment does not invent one.
- Pool-template settlement compares full reconstructed owed to `owed_sats`,
  count and every k-associated amount to the canonical decomposition, and the
  full sum to `sats`. No alternative high-water slice walk is attempted.
- `SPc/JobEntriesV2` commits identity, rail, key count/keys, sats, owed and
  destination length/bytes. Rail, destination and remainder-only changes roll
  job identity even when coinbase outputs are identical. `Published.job_id`
  continues participating in ingress publication identity.
- Snapshot primary-key handle is SHA256 of ASCII
  `SPc/DistributionSnapshotV2` followed by the complete version-2 block event.
  Its historical SQL column name remains `fingerprint`; the event's coinbase
  distribution fingerprint remains unchanged as an audit field. Thus different
  frozen windows or destination-only generations cannot alias one SQL row.
  All of this remains SPc-side attribution data, never Ark stream data.
- Recovery APIs observe all owned outputs without inferring payout policy from
  amounts. Audit requires an explicit `FixedLadderV1`, `JdExactV1` or
  `LegacyExactK0` source binding plus independently reconstructed entitlement.
  A bounded scan detects duplicate/gapped output sets. Coinbase discovery never
  implies an Ark epoch is fully paid; kept-row remainders still require receipt.

## Unresolved policy questions (not decisions)

1. **AGGREGATEDEST:** SPEC §11.3 names aggregation/order membership but never
   chooses the key/mailbox across per-block destinations. Minimal proposal for
   the parent: use the oldest covered row's stored destination; freeze the
   covered set, destination, amount and order ID before publication; retries
   preserve all four. Revalidate every covered block before first publication.
   A published/uncertain set cannot be silently regrouped after a reorg or
   registration change. Clients retain the anchor epoch and reconcile one
   VTXO against the authenticated complete covered set; membership never goes
   to Ark. The exact cancellation/recovery policy for uncertain reorged sends
   must be included before activating aggregation.
2. **Error-held entitlement exceeds revenue:** the coinbase has only its
   null-prevout input. If full canonical payouts cannot fit while retaining
   the reserved pool fee, construction fails closed. A funding mechanism for
   larger error-held claims needs a separate decision; partial caps, donation
   or denomination changes are not resolutions.

## Sections changed

SPEC §2.5, §4.7, §5.1 (entry binding), §11.5; WIRE-FORMATS §3.1.
DESIGN and the fixed-rung policy are unchanged. This record authorizes no Ark
server modification and makes no funded/native-lab acceptance claim.
