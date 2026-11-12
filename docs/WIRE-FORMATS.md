# Wire Formats

Byte-level formats BESPOKE to this project. Never restate upstream SV2 content — reference it. Where this file and `SPEC.md` disagree, the spec wins and this file is the bug.

## 1. Scope and conventions

- **SV2 framing / standard messages** per `sv2-spec @ d8bf5b8` (pins in `UPSTREAM_DEPS.md`).
- **Integer endianness**: little-endian unless otherwise noted in SV2.
- **Type vocabulary** (the SV2 codec's, as used in SPEC §11.4; stated once here):
  - `U8`, `U16`, `U32`, `U64` — little-endian unsigned integers; `I64` — little-endian two's-complement signed; `U256` — 32-byte array.
  - `B0nn` — fixed-length byte array of `nn` bytes, no length prefix (`B004`, `B016`, `B032`, `B033`, `B064`).
  - `B0_nn` — variable-length byte array, 1-byte length prefix, at most `nn` bytes (`B0_32`, `B0_255`).
  - `STR0_255` — `B0_255` holding UTF-8.
  - `SEQ0_64K[T]` — `U16` element count, then the elements.
- **Redis stream entries.** Each `XADD` carries exactly one field, `d`, whose value is the binary payload defined below. Stream names and producers/consumers are in SPEC §5.2.
- **Identifiers.** `sp_id` is the 32-byte `SPc/Id` hash (SPEC §0, §2.1) and `block_hash` is in internal byte order. Public keys are `ser_P` (33-byte compressed) unless a field says `_x` (x-only, 32 bytes).

## 2. Specified elsewhere (reference anchors)

| Format | Anchor | Notes |
|---|---|---|
| `OP_RETURN` carrier payload (JD mode) | SPEC §2.4.2 | `0x6a 0x23 0x53 0x50 0x63 <ser_x(A_send)>` (`OP_RETURN`, push of 35 B = `"SPc"` ‖ 32 B x-only, even-Y) |
| `0x5350` messages (`SetArkIdentity` / `RegisterEpochMailboxes` / `ArkPayoutNotice`) | SPEC §11.4 | Project-defined extension, identifier `0x5350` ("SP"), negotiated via `0x0001` |
| `SetPayoutDistribution`, `distribution_id` TLV `0x0003`/`0x01` | SPEC §3.5 | PR #203 text pinned at `47dbedd`; `distribution_id` is the per-front scheme of §3.5; the `.Success`/`.Error` answers and their decline-code set are SPEC §3.5 "Declaration answers" |
| `MailboxAuthorization` 105-byte layout | `MAILBOX.md` §3 | `ser_P(M_i) ‖ le64(expiry_unix) ‖ sig64` |
| Type-32 share-accounting messages | `share-accounting-ext @ c661e93` + SPEC §3.7 deltas | Type 32 (`0x0020`), negotiated via `0x0001` |
| Postgres row schemas | SPEC §5.1 | Used by the bootstrap RPC (§3.5) |

## 3. New definitions (no upstream byte-level spec)

### 3.1 `spc:blocks` entry (front → payout)

```
B004   magic = "SPCB"
U8     format_version = 3     // Rule-B provenance; historical 2 has no economic suffix
U256   block_hash
U32    height
U8     front_id
B032   fingerprint            // SPEC §4.7; audit/dedup handle only
B032   a_send_x               // public x-only key; the secret is never stored
SEQ0_64K[ {U8 front_id, STR0_255 stream_id} ] high_water  // {front_id: last_slice_stream_id} (§4.3); Redis stream ids are "ms-seq" strings
U128   window_target          // rule 0: exact N * d; rule 1: exact Q(T) in v2 reference units (§4.3); settlement persists it (§7.4)
SEQ0_64K[ Entry ] entries     // the winning distribution's full entry list (SPEC §3.3 step 5)
U8     work_unit_version      // 1 legacy, 2 exact reference multiplier
B0_32  work_unit_fingerprint  // v1: 32-byte zero sentinel; v2: nonzero 32-byte reference hash
U8     window_rule            // 0 share_work_legacy (N * d), 1 network_block_work (Q(T))
B4     economic               // format 3 only: bounded UTF-8 FrozenJob JSON below
```

`Entry` mirrors SPEC §4.7's `entries` tuple:

```
B032   sp_id
U8     rail                   // 0 coinbase, 1 ark, 2 pending, 3 forfeit (the rail the builder expected)
SEQ0_64K[B032] p_x            // contiguous k order, NOT global TxOut script order
B2     dest_id                // U16 length + destination bytes; empty if not Ark-capable
U64    sats                   // owed - (owed mod 500k) for a kept id (SPEC §4.4 constant ladder), owed(id) otherwise
U64    owed_sats              // full frozen raw + prior pending, including remainder
```

The `SPCB`/2 envelope introduced the bounded `ladder-entry-v2` amendment
(decision 2026-11-09); checked production uses `SPCB`/3 with mandatory economic
provenance (decision 2026-11-10). Historical format 2 decodes with unavailable
economic provenance, never implicit zero pending. The old single-key Entry contradicted SPEC §5.1 and is
superseded, not extended in place. There is exactly one entry per identity;
key arrays contain valid distinct points and are nonempty iff rail is coinbase.
Total key count per entry and block is at most 2048. Amounts associate with k
in greedy-largest-first order before global output script sorting. Repeated
identities/keys, unknown version, oversized arrays and trailing bytes reject.
The destination's existing U16 length is explicit here (`B2`, maximum 65535).
JSON/SQL Entry requires `format_version: 2`, array-valued `p_x` and `owed_sats`;
missing versions reject. Front/payout and all entry readers deploy atomically.
Bootstrap currently carries no distribution entries; any future entry-bearing
bootstrap payload uses the same versioned JSON shape, without a legacy default.
The SQL snapshot key is SHA256(`SPc/DistributionSnapshotV2` ASCII || the complete
event encoding); `fingerprint` in this event still covers the coinbase outputs.

The format-3 suffix has a U32 little-endian byte length, at most 4 MiB, followed
by the UTF-8 JSON serialization of `FrozenJob` in this field order:
`version` (1), `revision` (u64), `parent_hash` (32-byte integer array),
`job_id` (u64), `generation` (u64), `balances` (array of objects with `sp_id`
as a 32-byte integer array and u64 `booked`, `reserved`, `available`), then
`slice_ids` (u64 array in descending selected-window order). JSON integers
MUST be parsed losslessly, including values above 2^53. Unknown fields,
duplicate identities, invalid B-R=A, incomplete entry coverage or absent
required fields reject. P is each identity's frozen `available`; booking
reuses its validated delta, never current pending in place of P. The event
snapshot digest includes this suffix. `slice_ids` fixes the exact visible
window, including legitimately empty windows; missing slices hold settlement.

`entries` are attribution data: they never leave the SPc side of the Redis ACL (SPEC §11.3) and are scrubbed per SPEC §5.1.

`work_unit_version` uses definite enum codes 1 and 2; `window_rule` uses 0 for
`share_work_legacy` and 1 for `network_block_work` (LP-1). These fields follow
the version-2 entries above, so old-format events fail
decode instead of acquiring a guessed profile. Front and payout deploy
atomically. In SQL the v1 fingerprint is NULL; on the wire it is the 32-byte
zero sentinel. A zero fingerprint with version 2 is invalid. A v1 distribution
stores `N * d`, never a converted Q; a v2 distribution stores `Q(T)` with rule 1.
Underfill and overshoot are derived from high_water and window_target, not
columns.

### 3.2 `spc:front:events` payloads (payout → all fronts)

Every payload starts with `U8 type` and `U64 rev` — `rev` is assigned by `payout` in commit order; fronts apply only newer revs (MULTI-REGION §6). Field sets follow SPEC §5.2.

**0x01 - `ArkRegApplied`**
```
U8     0x01
U64    rev                         // fresh event revision from payout, not the registration revision
B032   sp_id
U32    request_id
U64    registration_rev            // durable registration revision returned in Success
U8     state                       // 0 applied, 1 already_applied, 2 superseded
B033   b_scan
B033   b_spend
I64    identity_network            // 0 mainnet, 1 testnet4, 2 signet, 3 regtest
B004   ark_id
B033   b_ark
U8     mailbox_mode                // 0 none, 1 static, 2 epoch_derived, 3 epoch_registered
B0_33  mailbox_pubkey              // M; empty iff none
B0_33  mailbox_dh                  // D; empty iff none
U8     epoch_fallback              // 0 static, 1 hold
SEQ0_64K[ { U32 height, B033 mailbox_mi } ] epoch_mailboxes
```
This event remains inside the identity-bearing SPc-side Redis ACL. A
`superseded` event advances recovery but never reapplies old registration
fields. Other states install the full current identity and delivery data;
changed epoch mailboxes are merged by `(sp_id, height)`. Pre-AP-2 short
`ArkRegApplied` events do not decode and MUST NOT be replayed as registrations;
fronts bootstrap current full state across the upgrade. A malformed identity
key in an internal event is logged and skipped instead of panicking front.

**0x02 — `LedgerApplied`**
```
U8   0x02
U64  rev
U256 block_hash
U8   republish                // 0|1: force a SetPayoutDistribution republish and drop the grace slot (SPEC §3.5)
SEQ0_64K[ { B032 sp_id, I64 pending_delta_sats } ]   pending_deltas
```

**0x03 — `BlockFound`**
```
U8   0x03
U64  rev
U256 block_hash
U32  height
U8   front_id
SEQ0_64K[ { B032 sp_id, U64 sats } ]                 provisional_paids
SEQ0_64K[ { U8 front_id, STR0_255 stream_id } ]      high_water   // SPEC §4.3; Redis stream ids are "ms-seq" strings
```

**0x04 — `TipHeartbeat`**
```
U8   0x04
U64  rev
U256 tip_hash
U32  height
U64  unix_ms
```

**0x05 - `EconomicChanged`**
```
U8   0x05
U64  rev
U64  economic_revision
B032 anchor_hash
```
This is a reload hint, not economic authority. Before publishing descendant
work the front reloads a complete coherent economic snapshot and requires its
anchor to equal the actual template parent. `BlockFound.provisional_paids` is
retained only as a historical field; it MUST NOT be subtracted from fresh raw
or used as a Rule-B reservation. `LedgerApplied` also triggers a reload in
checked mode rather than applying a delta to the available-balance cache.

### 3.3 `spc:arkreg` entry (front -> payout)

Carries a validated `SetArkIdentity` plus the keys payout needs to upsert
`sp_identity`. The trailing `request_id` is an immutable signed correlation
field (SPEC section 11.4), not a new pool policy flag.

```
U8     kind = 0                    // SetArkIdentity
B032   sp_id                       // recomputed from b_scan/b_spend and checked
B033   b_scan
B033   b_spend
B004   ark_id
B033   b_ark
U8     mailbox_mode                // 0 none, 1 static, 2 epoch_derived, 3 epoch_registered
B0_33  mailbox_pubkey              // M; empty iff none
B0_33  mailbox_dh                  // D; empty iff none
U8     epoch_fallback              // 0 static, 1 hold
B064   sig
U32    request_id                  // appended v1 correlation, included in sig digest
B016   attempt                     // fresh per-handle nonce; not on client wire, not signed
```

`kind = 1` is variant-A `RegisterEpochMailboxes`:
```
U8     kind = 1
B032   sp_id
U16    count
B037   entries[count]              // ser32_be(height) || ser_P(M_height)
B064   sig
U32    request_id
B016   attempt                     // fresh per-handle nonce with the same cache-only semantics
```

`spc:arkreg:ack` is a separate SPc-side Redis hash, readable only by
front/payout (never the `ark` role ACL). Every handle gets a fresh attempt
nonce. Each field is `reg:<sp_id_hex>:<request_id>:<attempt_hex>` (16-byte hex
nonce), so concurrent identical requests cannot overwrite or steal each
other's replies. TTL is 3600 seconds; expiry is not a negative registration
verdict and never chooses outcome freshness. The attempt nonce has no client
wire field, signature-preimage slot or durable effect; it is strictly internal
per-handle correlation. JSON fields are:
```json
{
  "sp_id": "<64 lower-case hex characters>",
  "request_id": 1,
  "signed_fields_digest": "<64 lower-case hex characters>",
  "registration_rev": 12345,
  "state": "applied | already_applied | superseded | mismatch | stale | rejected | request_id_exhausted",
  "attempt": "<32 lower-case hex characters>",
  "error_code": null,
  "expires_at": 1794163200
}
```
The front requires matching `sp_id`, `request_id`, `signed_fields_digest`,
`attempt` and unexpired time. **Every handle triggers durable Postgres
resolution**, so a live cached `Applied` for an old counter cannot bypass
`superseded`. First valid semantic rejection uses state `rejected` with its
validated `error_code` instead of commit-unknown; matching same-ID retries
replay that terminal outcome. Mismatch/stale/superseded never restore old
fields. Unknown failures (database or Redis outages) stay unacknowledged.

**Journal/outcome policy.** `applied` is a real new registration effect,
`already_applied` is its current-ID duplicate, and `superseded` is a matching
older applied request. `stale` means a lower never-applied request (including
concurrent reorder loser/pruned old history), not an actual pruning schedule.
Before applying a row, `apply_registration` replays its `(sp_id, request_id)`
journal row under the existing identity lock (`FOR UPDATE`), before `is_new`.
Existing different digest is mismatch with no effect/insert. Equal digest
replays `rejected` with the stored terminal error, current applied as
`already_applied`, and older applied as `superseded`; no historical Applied
revives current delivery. No-journal-row requests alone proceed to counter
policy. `rejected` is a validly parsed/signed semantic rejection with
`bad-mailbox-fields`, `invalid-point`, `invalid-message`, `bad-signature`,
`wrong-ark-server` or another v1 terminal error. If an identity exists the
rejection journals atomically without changing current request state or
`ark_delivery`; if no identity exists no FK-safe row exists but the
deterministic terminal outcome still resolves the PEL entry. Unexpected DB
values and transport outages are not given a terminal semantic ack.

### 3.4 `spc:ark:orders` / `spc:ark:receipts` (payout ↔ ark)

```
ArkSendOrder:
B016    order_id              // random, opaque
STR0_255 ark_address          // bark address string, delivery per SPEC §11.8
U64     sats
U32     not_before_height     // H_f + confirmation_depth
B064    sig                   // BIP340 over hash_SPc/ArkSendOrder(the preceding fields exactly as serialised above)

ArkSendReceipt:
B016    order_id
U8      status                // 0 sent, 1 failed
B0_64   vtxo_id               // empty if failed
U32     expiry_height         // 0 if failed
```

The signing key is the order-signing key of SPEC §11.3 (secp256k1, BIP340; `order_pubkey` is the 32-byte x-only key). No `sp_id`, `B_scan`, `B_spend`, `P_i` or block hash appears in either message (SPEC §11.3, schema-tested in §11.9).

### 3.5 Bootstrap RPC (front → payout)Transport: length-prefixed (`U32 LE`) JSON over TCP to the `payout` role's internal listener, TLS with per-front credentials (the same trust level as the front's Redis ACL, MULTI-REGION §6). It returns attribution data, so it MUST NOT be exposed beyond the fronts. A front is not ready (no channel opens) until the RPC completes.

**Request**
```json
{
  "front_id": 1,
  "instance": "uuid-of-this-front-process",
  "cursors": { "spc:slices": "<last-seen stream id or null>", "spc:front:events": "<…>" }
}
```

**Response**
```json
{
  "rev": 12345,
  "pending_balance":   [ { "…": "rows per SPEC §5.1 pending_balance" } ],
  "economic": { "version": 1, "revision": 12340, "anchor_height": 799999, "anchor_hash": ["32 byte values"], "complete": true, "booked": [], "reservations": [] },
  "ark_delivery":      [ { "…": "rows per SPEC §5.1 ark_delivery, current as of rev" } ],
  "ark_identities":    [ { "…": "identity keys for restored registrations" } ],
  "ark_epoch_mailboxes": [ { "sp_id": "<hex>", "height": 800000, "mailbox_mi": "<66 hex characters>" } ],
  "window_tail":       [ { "…": "sealed slice aggregates for the live tail (SPEC §5.3 window_cache)" } ],
  "type32_blobs":      [ "<identity-free share blobs exported for every window_tail row (SPEC §3.7), base64>" ],
  "resume_cursors":    { "spc:slices": "<stream id>", "spc:front:events": "<stream id>" }
}
```

The response is a consistent snapshot as of `rev`; the front then subscribes from `resume_cursors`, so there is no gap between the read and the live tail. A front whose cursor has fallen behind the trim horizon (SPEC §5.2) repeats this call. In v2 the front MUST ignore `type32_blobs` and MUST NOT serve them; the payload MAY still carry identity-free blobs for v2 slices because `payout` exports every tail row's blob. Type-32 service remains v1-only: the v2 front never populates `Type32Store` from those blobs and refuses `0x0020` frames.

`economic.booked` contains `[sp_id-byte-array, sats]` pairs. Each reservation
contains `block_hash`, `parent_hash`, `height`, `payload_digest`, `job_id`,
`generation`, `lifecycle`, `transition`, `revision`, and `rows`. Each row has
`sp_id`, `prior`, `error_pending`, signed `delta`, and `consumed`. The active
set contains every `unbooked` and `loss_held` record (maximum 4096; overflow
fails closed); only `unbooked` consumption contributes to R. Missing/null
economic state, `complete=false`, or a parent mismatch means unavailable.
This data remains within the SPc-side ACL. SQL `economic_state`,
`economic_block`, and `economic_chain` plus committed accounting history are
the recovery authority (SPEC §4.6 and §5.1), not event arrival order.

### 3.6 `spc:slices` entry (front → payout)

Added 2026-11-02 to make the sealed-slice stream byte-exact. One `XADD` per
sealed slice, one field `d`:

```
U64    slice_id
U32    height
U256   prev_hash
U64    ref_job_id
U64    ref_fees
U32    n_shares
U128   difficulty                  // LE, 16 bytes; stored as NUMERIC(40,0)
B032   root                        // identity-free merkle root
B4     shares_blob                 // U32 length + the compact share blob (SPEC §4.2)
SEQ0_64K[ Credit ] credits         // per-identity aggregates; the scrub target
U8     work_unit_version            // 1 legacy, 2 exact reference multiplier
B0_32  work_unit_fingerprint        // v1: 32-byte zero sentinel; v2: nonzero 32-byte reference hash
```

A v2 slice never carries a zero fingerprint. `spc:slices` does not carry
`window_rule`; the distribution/snapshot and `spc:blocks` carry it. A reader
that sees a mixed or unknown profile MUST fail the window rather than infer a
rule.

`Credit`:

```
B032   sp_id
U128   diff_sum                    // Σ d
U128   diff_fee_sum                // Σ d·f
```

`U128` and `B4` are project extensions to the type vocabulary in §1: `U128` is
little-endian 16 bytes, `B4` is a `U32` little-endian length followed by that
many bytes. The share blob's internal layout belongs to `spc-accounting`; only
the front and payout read it.


### 3.7 `0x5350` v1 message payloads

All payloads start with `U8 version = 1`. Integers are little-endian; mailbox
entries embed their big-endian height as in MAILBOX.md. Decoders reject
trailing bytes. Message IDs are assigned by this table and are independent of
the Redis `kind` bytes.

| msg_type | Message | Direction |
|---:|---|---|
| `0x01` | SetArkIdentity | client -> front |
| `0x02` | SetArkIdentity.Success | front -> client |
| `0x03` | SetArkIdentity.Error | front -> client |
| `0x04` | RegisterEpochMailboxes | client -> front |
| `0x05` | RegisterEpochMailboxes.Success | front -> client |
| `0x06` | RegisterEpochMailboxes.Error | front -> client |
| `0x07` | ArkPayoutNotice | front -> client |

**SetArkIdentity (0x01)**
```
U8     version = 1
B004   ark_id
B033   b_ark
U8     mailbox_mode              // 0 none, 1 static, 2 epoch_derived, 3 epoch_registered
B0_33  mailbox_pubkey
B0_33  mailbox_dh
U8     epoch_fallback            // 0 static, 1 hold
B064   sig
U32    request_id                // per-sp_id monotonic, starts at 1
```

The BIP340 signature by `b_scan` covers
`hash_SPc/ArkIdentity(sp_id || version || ark_id || b_ark || mailbox_mode ||
mailbox_pubkey-with-length-prefix || mailbox_dh-with-length-prefix ||
epoch_fallback || request_id)`. The request ID follows the signature on the
wire but is signed (explicit AP-2 amendment): a legacy signature transplanted
to a new counter is invalid. The digest is also stored by payout for
idempotence/mismatch checks.

**Success (0x02 and 0x05)**
```
U8     version = 1
U32    request_id
U64    registration_rev
B004   ark_id
U8     state                     // 0 applied, 1 already_applied, 2 superseded
```

**Error (0x03 and 0x06)**
```
U8     version = 1
U32    request_id
U8     retryable                 // exactly 0 terminal or 1 retryable
B0_255 error_code                 // UTF-8
B0_255 detail                     // UTF-8
```

**RegisterEpochMailboxes (0x04)**
```
U8     version = 1
U16    count
B037   entries[count]
B064   sig
U32    request_id
```
Its `sig` covers `hash_SPc/EpochMailboxes(sp_id || version || count || entries ||
request_id)` with the same count and raw entries bytes, in the canonical order
part of the on-wire serialisation.

**ArkPayoutNotice (0x07)**
```
U8     version = 1
U256   block_hash
U64    sats
B0_64  vtxo_id
U32    expiry_height
```

### 3.8 Fresh Ark authorization and complete delivery receipt v1

SPc-side payout reads `spc:ark:authorization:requests`; Ark writes it. Payout
writes the `spc:ark:authorization:grants` reply hash; Ark reads it. These ACL
additions carry opaque delivery data only, never identity or member mapping.

Request (69 bytes): `SPAR` ASCII, U8 1, B016 order_id, B032 order_digest,
B016 fresh random nonce. `order_digest` is the BIP340 tagged hash
`SPc/FrozenArkOrderV1` over the complete existing ArkSendOrder encoding.

Grant (149 bytes): `SPAG`, U8 1, the same order_id/digest/nonce, U64 revision,
U64 expires_at (Unix seconds), B064 signature. BIP340 signs the tagged hash
`SPc/ArkAuthorizationV1` of every preceding grant byte. The request-specific
reply hash field is `<order_id_hex>:<nonce_hex>`; value is grant hex. Its 60 s
TTL is cleanup only. The live worker checks exact request correlation and
signature and permits one call only while `now < expires_at <= now + 10`.
Payout commits potential-send authorization before publishing the reply. Lost
grant, expiry and unknown transport outcome never authorize a replacement send.

Complete Sent receipt: `SPRP`, U8 1, the fixed 149-byte signed grant, U64 sats,
U16 proof_count (1..2048), then U32 length plus complete stock-protocol VTXO
bytes for each proof. Each proof is 1..1 MiB; the whole receipt is <=4 MiB.
Payout verifies grant binding, every VTXO signature/ancestry against its real
Bitcoin anchor transaction, destination, unique IDs and exact full sum before
atomic all-member application. First-ID legacy receipts are insufficient for
aggregates. Complete receipt bytes remain owner-fetchable; a chained send never
loses all but its first VTXO at the wallet interface.

`GET /api/sp/me/ark-aggregates` is `spc.aggregate-status/1`, through the existing
fresh one-time B_scan challenge auth. `obligations` includes own positive
unbatched rows; `orders` includes own anchor/full members/state/revision and
proof-derived expiry when known. `wallet_received` is null: pool Sent wakes
fetch-and-verify, never closes a watch. The same auth protects
`GET /api/sp/me/ark-aggregates/<order_id_hex>/receipt` (bounded binary SPRP/1).
An unknown/unavailable status or proof retains watches. Neither endpoint uses
registration replay as read authorization or exposes another identity's rows.

**Frozen signed order retrieval (owner API).** Before consuming a receipt,
an owner can fetch its frozen order using
`GET /api/sp/me/ark-aggregates/<32-hex-order-id>/order`, with the same fresh
single-use B_scan challenge as status/receipt. Success is HTTP 200,
`Content-Type: application/octet-stream`, `Cache-Control: no-store`; the body
is the stored canonical `ArkSendOrder` encoding of §3.4, including its 64-byte
signature, without a new envelope. The exact codec bound is **348 bytes**:
16-byte ID + U8 length + up to 255 UTF-8 address bytes + U64 sats + U32 height
+ 64-byte signature (fixed-field minimum 93 bytes). The client MUST enforce
the same bound, decode/roundtrip/verify, and match `order_digest` to the grant.

HTTP 401 means missing/invalid/expired/replayed authentication; 400 means an
invalid order-ID representation after authentication; 404 (`order_unknown`)
covers unknown and other-owner IDs alike; 503 (`order_unavailable`) covers
store/key unavailability or failed stored-byte integrity. Error JSON includes
`keep_watches:true` on authenticated reads. Readability includes frozen,
published, authorized, blocked_unknown, sent and cancelled states, without
changing their lifecycle. The API key is configured `[ark] order_pubkey`; it
is never inferred from a receipt. There is no caller-selected path or SQL.

**Local pre-request admission amendment (no wire change).** `ArkTransport`
and `BarkClient` expose typed `CapacityError` results and
`check_send_readiness(address/ArkPayment, amount, permit)` before nonce creation.
Closed pre-request deadline, wallet proof/policy and eligible-float shortages
return Held. Post-request transport errors retain their existing unknown/pin
semantics. No new Sent/Failed/NoEffect receipt status is introduced.
The Ark-side key `spc:ark:pending-cursor:<orders_stream>:<group>:<consumer>` stores
only the last pending stream ID for fair retries; exhaustion resets to `0`.
Held stream entries remain unacknowledged and frozen membership stays reserved.

### 3.9 Private source evidence v2 and accounting ancestry v1

The full-rail observation extension (migration 0012) is specified in
`FULLRAIL-OBSERVATIONS.md`. Its append-only SQL bodies remain SPc-side. Private
bootstrap delivery JSON adds optional `registration_commit` public provenance.
Ark-side SPAC/1 is exactly magic `SPAC` + version byte 1, the existing 149-byte
signed grant, and a little-endian u64 consumption clock in milliseconds (162
bytes total). SPCA/1 is magic `SPCA` + version byte 1, a 32-byte
`SPc/ArkConsumptionV1` tagged digest of the entire SPAC frame, a little-endian
u64 observation ID, and a little-endian u64 DB receipt clock (53 bytes total).
Neither frame adds identity, destination, block hash or member fields. These
are local observation messages, not an Ark-server RPC or a replacement grant.

Actual front trace target `spc_evidence` emits `distribution_frozen`,
`job_issued`, `accounted`, `share_rejected`, `bootstrap_applied`, and
`pending_applied`; payloads have `source.schema = spc.source-evidence/2`, a
random process `session` (16-byte hex), decimal-string `seq`, and previous/current
32-byte hash hex. Hash preimage: ASCII `SPc/SourceEvidenceV2`, session bytes,
LE64 sequence, previous hash bytes, LE32 event-name byte length, event-name UTF-8,
then recursively key-sorted compact JSON payload excluding `source`. First seq
is 1 with zero previous hash. Every event is bounded at 4 MiB; exceeding it
emits `proof_unavailable`, never a completeness claim. Missing prefix/root,
watermark, referenced generation, previous transaction or history is unknown.
This hash chain detects gaps/tampering against an independently captured head;
it does not authenticate a head supplied by the same untrusted bundle.

`distribution_frozen` captures actual immutable job prefix/suffix/Merkle path,
target/version/ntime/extranonce context, raw transaction set, witness plan,
format-2 logical key arrays/full owed, selected window/profile/target/high-water,
and the SPCB/3 `economic` frozen B/R/A/job/slice provenance. `accounted` supplies actual accepted channel assignment,
slice ID, credited work/fees, extranonce, nonce, header fields and raw digest.
Pool ingress attribution is explicitly attested by the pool; a verifier checks
actual headers/PoW, transaction/prevout fees and arithmetic, not claimed totals.
`finding_slice_id` is the actual open slice ID frozen before this generation
is mined; `finding_slice_excluded=true` declares its exclusion from the frozen
selected window. It is bound to the record's job ID and generation, not filled
in after discovering a winning share.

Migration 0008 journals committed ledger/pending/block/Ark state transitions in
`source_accounting_history` (SPc side only), including deletes on orphan reversal.
Rows share the real transaction_id and a monotone event_id; rolled-back SQL
transactions can consume sequence numbers, so sequence gaps alone are not proof
of lost committed history. `history_start.complete_from_empty` or an independently
verified external checkpoint is required. Keys, destinations and VTXO proof/ID
fields are excluded, preserving the accounting-history-only attribution surface.
Collectors use one read-only repeatable-read transaction and bind its snapshot,
maximum event ID, canonical raw chain and trace heads. The operator/private lab
broker collects full attribution; no source evidence is an Ark-role input.

Migration 0009 adds economic state/block/chain transitions to the same journal.
The collector MUST reconstruct the complete canonical unbooked set and frozen
P, replay additive booking and both orphan paths, and bind the actual job's
parent before sealing available A. `HistoryCheckpoint.pending` in the source
verifier is that independently validated available amount, not raw SQL booked
B. Its map/digest shape remains unchanged; a checkpoint made from B alone
does not satisfy Rule B. `provisional_candidate` and `economic_change_observed`
are evidence hints; neither grants completeness or substitutes for history.

### 3.10 Private live source feed and activation ACK

**Audited warmup amendment.** `spc.payout-proof/1` SourceBundle adds optional
`window = {facts:{window_target,actual_sum,shortfall,underfilled},
eligible_history_complete}`. The three work fields are canonical unsigned
decimal strings; underfilled/completeness are booleans. This annotation is
mandatory for empty/underfilled admission and must match recomputed facts.
Only an independently verified complete eligible census may assert completeness;
pool flags or a pruned/partial capture cannot do so. High water names every
participating front, zero for a known empty front. Source/history seals and
nonzero history watermarks remain mandatory. Legacy full-Q bundles may omit
the annotation. No new payout/coverage enum or SQL accounting column is added.

Optional `witness_reserved` maps combined job IDs (decimal JSON keys) to
32-byte integer arrays from immutable authenticated witness plans. It is
required for every current/referenced witness-commitment job. The verifier
checks current coinbase conservation/Merkle/witness even for empty history,
then the same full owed/k-output ownership audit. Empty raw follows the front's
raw=0/gross-K=T adapter and preserves prior pending. Net pay_P is determined
by actual full rung payments and signed E-P; K and the gross fee are not extra
pool-output floors. Actual payments above T still reject. No wire change is
needed for this conservation correction.

Activation transport is now **spc.source-feed/2**: Consumed mandatorily includes
verified WindowFacts and signs them under SourceConsumedV2. Version 1
activation frames fail closed; collector/wallet/public verifier deploy together.

The actual translator entry point consumes `[source_audit]` with `socket`
(absolute private Unix socket), `pool_pubkey`, `collector_pubkey` (distinct
32-byte x-only public keys as hex), and `scan_secret_key_file` (private
64-hex-character scan secret). Both files/socket and their containing
directories are owner-only; socket peer UID must match its owner. The collector
is QA/operator-owned and provisioned per agent, with that agent's public scan
key and identity binding. The pool front signs trace heads when launched with
`SPC_SOURCE_EVIDENCE_KEY_FILE=<private 64-hex source-signing-key file>`. The
signing key is loaded only in the front role; the Ark process never receives it.

Transport is one Unix connection per candidate, U32LE byte length followed by
strict UTF-8 JSON. Maximum frame: 9 MiB; embedded SourceBundle: 4 MiB encoded
as `bundle_hex`. Total connect/request/verify/ACK deadline: 10 seconds. Each
exchange uses a fresh random 16-byte attempt. Checkpoints must be unexpired
and at most 30 seconds into the future. Unknown/truncated/oversized frames,
missing source/history, bad signatures or mismatched correlation hold work.
Exact typed frame definitions and binary hash preimages are
`spc_scan::source_feed` (all byte arrays are JSON integer arrays; integers
require lossless parsing):

1. Agent `job`: `schema="spc.source-feed/2"`, `attempt`, `job:ReceivedJob`.
   ReceivedJob contains actual Noise-authenticated wire job ID, prev hash,
   version/bits/time, coinbase prefix/suffix, Merkle path, captured share target,
   pool extranonce and miner extranonce length.
2. Collector `proof`: same schema, `bundle_hex`, `checkpoint`. Checkpoint
   binds attempt, ReceivedJob digest, combined `(height<<32)|wire_job_id`,
   generation, source/history watermarks, bundle/history digests, expiry,
   completeness, pool trace seal, and collector signature.
3. Agent verifies both signatures, complete bundle/history seals, actual wire
   bytes and independent raw entitlement math, then sends `consumed`: same
   schema, `ack` with attempt, combined job ID, generation, bundle/history
   digests, checkpoint digest, verified `window:WindowFacts` and B_scan signature.
4. Collector durably records that ACK, then replies `committed`: same schema,
   attempt, ACK digest and collector signature. Only after this verifies can
   the translator publish the job to miners. `unavailable` carries schema and
   attempt and never constitutes an activation ACK.

Digests use BIP340 tagged SHA256, with these exact binary preimages:
- `SPc/ReceivedJobV1`: U32 job ID, B032 prev hash, U32 version/bits/time,
  U32-length prefix, U32-length suffix, U32 Merkle-count then B032 hashes,
  B032 big-endian target, U32-length pool extranonce, U32 miner length.
- `SPc/SourceTraceSealV1`: B016 session, U64 sequence, B032 trace digest.
  `source.pool_seal` adds this signature to the existing trace metadata.
- `SPc/SourceCheckpointV1`: B016 attempt, B032 wire-job digest; U64 combined
  job ID, generation, source watermark, history watermark, expiry; B032 bundle
  digest, B032 history digest, U8 complete, B032 pool-seal signing digest.
- `SPc/SourceConsumedV2`: B016 attempt, U64 combined job ID/generation,
  B032 bundle/history/checkpoint digests, U128LE window_target, U128LE
  actual_sum, U128LE shortfall, U8 underfilled. JSON work fields use decimal
  strings; the signing preimage uses fixed 16-byte little-endian values.
- `SPc/SourceCommittedV1`: B016 attempt, B032 consumed-ACK signing digest.

Signatures are 64-byte BIP340 values encoded as 128 hex characters. The
collector's separately pinned key, not a pool-supplied root, authenticates
complete source/history replay and the exact bundle. Source watermark equals
the supplied signed pool trace head sequence; the collector must have captured
and checked its complete prefix, source/process/config bindings and frozen
generation. History watermark denotes its independently replayed committed
SQL snapshot, including Rule B and the explicit initial seed (§3.11).

New wire jobs clear active work immediately and cancel older in-flight audit
exchanges. A dedicated reader avoids cancellation of partial Noise frames.
Only the newest successful candidate activates. `source_proof_consumed`,
`source_job_activated` and `translator_stale_submission` are actual runtime
observations. Missing feed in refuse mode fails startup; feed failure after
startup leaves no active work. This mechanism does not gate unrelated native
miners that have not been configured to use this translator.

**Loaded-wallet API and lifetime.** The public shared wallet adapter is
`UnixSourceAuditor::from_keys(socket: PathBuf, pool: XOnlyPublicKey,
collector: XOnlyPublicKey, scan: &SecretKey, spend: PublicKey,
address: &str, network: spc_crypto::Network) -> anyhow::Result<Self>`.
It validates the pinned SP address/network against both loaded public keys,
reads no secret file, and owns one Zeroizing scan-byte copy. Consuming
`verify_received(self, &ReceivedJob, attempt: [u8;16])` uses the actual native
outer attempt unchanged and returns `spc_scan::source_feed::VerifiedSourceEvidence`
with `schema`, `checkpoint`, `consumed` and `committed`. The result contains no
private source bundle, scan key or payout tweak. `verify_public_evidence` checks
all public signatures/correlation/expiry against externally pinned keys and
the native actor's current attempt/ReceivedJob digest. Run/connection/epoch
binding remains the native activation codec owner's responsibility.

The request-owned scan buffer is explicitly scrubbed on Drop; temporary secp
SecretKey/Keypair guards use supported best-effort non_secure_erase, not assumed
Copy-type Drop behavior. Configuration-file text is Zeroizing on every exit.
The 10-second exchange is cancellable, checks its deadline before ACK signing
and publication, and spawns no signer. Existing `from_config`/`JobAuditor`
wrappers delegate to the same verifier/exchange. The existing wallet service
passes its already-loaded authority; no additional scan-secret file/process is
required. Source-feed/2 is the only activation schema accepted by this API.

### 3.11 Economic initial checkpoint v1

Migration 0011 adds a private `economic_checkpoint` row with columns
`checkpoint_id=1`, `schema_version=1`, `migration_version=11`, `classification`,
`economic_revision`, `history_watermark`, `transaction_id`, `snapshot_boundary`,
`state_bytes`, `snapshot_bytes`, `snapshot_sha256`, `recorded_at`. `state_bytes`
is the exact UTF-8 PostgreSQL JSONB text of the actual singleton. Snapshot bytes
use `schema="spc.economic-checkpoint/1"` and include that state, initial
history_start, prior migration checksums and bounded monetary table snapshots.
Arrays cap at 1025 rows and economic-block entries contain provenance metadata;
raw event/proof/VTXO IDs are excluded. Snapshot byte bound is 8 MiB.

Capture holds the accounting lock and monetary table locks. Coordinates bind
the preceding max history event ID, actual economic revision and SQL snapshot.
`fresh_empty` requires the exact uninitialized singleton, empty monetary
tables, and only the valid complete-from-empty history_start. Otherwise the
classification is `unverified_existing`: bounded diagnostic snapshots are not
complete ancestry and existing amounts are preserved. No zero fallback or
automatic promotion exists. The real checkpoint INSERT is copied into the
append-only history; readback must match that copy and the byte digest.

Private collector API `Store::economic_checkpoint_record()` returns the row
as JSON (Postgres BYTEA fields use `\\x` hex). QA reads it, the journal and
the named monetary tables under one repeatable-read boundary. Bootstrap and
economic publication reject missing/altered/unverified seeds. A runtime receipt
must provide actual captured bytes; source-authored expected values are not
a substitute for the real PostgreSQL checkpoint.
## Private native wallet activation gate

The lab's direct native SV2 CPU path uses the private
[`spc.native-job-gate/1` wallet activation protocol](NATIVE-JOB-GATE.md)
before publishing candidate work. It carries public source-feed/2 evidence,
actual wallet/process/config lifecycle bindings and a wallet activation
signature. It is not an Ark-server RPC or a replacement for SV2 framing.
The exact bounds, binary preimages and required peer-registry ordering are
defined in that packet-owned wire document and shared public codec.

## 4. Where this file is linked from

- `docs/SPEC.md` §5.2 (stream table and delivery semantics).
- `docs/MULTI-REGION.md` §6 (bootstrap RPC and `spc:arkreg`).
