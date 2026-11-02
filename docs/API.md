# HTTP API reference

The `api` role's read-only JSON interface, as specified in `docs/SPEC.md`
§6.1 and implemented by `spc-pool` (`bins/spc-pool/src/api.rs`, backed by
`api_backend.rs`). Listen address: `[api] listen` (default `127.0.0.1:8080`).
All bodies are JSON unless noted. This is the only HTTP surface the pool
exposes; there are no write endpoints.

## 1. Public vs authenticated data

Public endpoints expose **pool-wide** data only. Anything per-identity
requires a proof of `b_scan`; without that proof the API would be an
attribution oracle for anyone holding a miner's public silent-payment
address (SPEC §6.1). Authenticated responses never include scripts or keys -
the miner derives outputs themselves with `spc-scan`.

## 2. Endpoints

| Method and path | Auth | Purpose |
|---|---|---|
| `GET /api/pool` | none | pool-wide parameters and policy |
| `GET /api/pool/window` | none | current accounting window (no identities) |
| `GET /api/pool/blocks?limit=N` | none | recent found blocks |
| `GET /api/pool/liability` | none | everything the pool holds for miners (§5) |
| `GET /api/pool/capacity` | none | payout weight budget and slot usage |
| `POST /api/sp/challenge` | none | mint an auth nonce |
| `GET /api/sp/me` | SPc (§3) | the caller's account: pending balance and payout history |
| `GET /api/sp/me/audit/<block_hash>` | SPc (§3) | the type-32 share-window audit view for one block |
| `GET /healthz` | none | liveness |
| `GET /metrics` | none | Prometheus (see `docs/OPERATIONS.md` §7) |

`<block_hash>` is the block hash as 64 hex characters (SPEC §6.1 writes the
path parameter as `:block_hash`; the router spells it `{block_hash}`).

### GET /api/pool

Specified fields (SPEC §6.1): hashrate estimate, connected channels, `N`
(window size), `D`, current `H`, `A_send_x`, `coinbase_threshold_sats`, the
configured Ark `ark_id`, the forfeiture policy (§11.1: unregistered below
`C` is forfeited), mode flags, and the extensions advertised.

Current build (a subset; config-sourced):

```json
{
  "network": "regtest",
  "window_n": 8,
  "coinbase_threshold_sats": 330,
  "extensions": ["0x0001", "0x0003", "0x0020", "0x5350"]
}
```

### GET /api/pool/window

The current window per slice - no identities. Specified: `t`, `Sum d`,
`Sum d*f` per slice. Current build returns the slice tail:

```json
{ "slices": [ { "slice_id": 0, "height": 102, "difficulty": 1.0, "n_shares": 2 } ] }
```

### GET /api/pool/blocks?limit=N

`block_found` rows, newest first. `limit` defaults to 50 and is capped at
500. Specified fields (SPEC §6.1): hash, height, status, output count,
`pay_P`. Current build:

```json
{ "blocks": [ { "block_hash": "...", "height": 102, "status": "confirmed",
                "settled_at": "2026-11-02T00:00:00Z" } ] }
```

### GET /api/pool/liability

Returns `{ "sats": <u64> }`. The definition is normative (§5).

### GET /api/pool/capacity

Weight budget, utilisation, slots used/max, next autoscale direction
(SPEC §6.1). Current build (config-sourced subset):

```json
{ "budget_wu": 50000, "ceiling_wu": 400000, "max_payout_slots": 256 }
```

### POST /api/sp/challenge

Mints a nonce. Response:

```json
{ "nonce": "00112233445566778899aabbccddeeff", "expires_in": 60 }
```

`nonce` is 32 hex characters (16 random bytes, hex-encoded). Single-use,
60 s TTL.

### GET /api/sp/me (authenticated)

Specified payload (SPEC §6.1): pending balance; scheduled and sent Ark
payouts `{block_hash, sats, due_height, status, vtxo_id}` (`vtxo_id` is the
delivery path in `none` mailbox mode, §11.8); accepted difficulty over the
last 24 h; the last 50 per-block entries `{block_hash, height, rail,
paid_sats, forfeited_sats}`; no scripts or keys. Current build returns the
pending balance and the ledger tail:

```json
{
  "pending_sats": 0,
  "ledger": [ { "block_hash": "...", "height": 102, "rail": "coinbase",
                "paid_sats": 5000000000, "ark_due_sats": 0,
                "forfeited_sats": 0, "pending_delta": 0 } ]
}
```

### GET /api/sp/me/audit/<block_hash> (authenticated)

The type-32 window for that block as JSON - the same data the share
accounting extension serves (SPEC §6.1), i.e. what `spc-scan audit`
verifies. Current build:

```json
{ "height": 102, "slices": [ { "slice_id": 0, "difficulty": 1.0,
                               "root": "...", "shares": [ ... ] } ] }
```

### GET /healthz, GET /metrics

`/healthz` returns `{ "ok": true }`. `/metrics` serves Prometheus text; the
exporter wiring is pending (`docs/OPERATIONS.md` §7).

## 3. The SPc challenge flow

Authentication proves possession of `b_scan` - the scan key, not the spend
key - mirroring BIP 352's own key split and keeping `b_spend` cold
(SPEC §6.1).

1. **Mint a nonce.**

   ```sh
   curl -s -X POST https://pool.example:8080/api/sp/challenge
   ```

   Use the `nonce` string exactly as returned, within `expires_in` (60 s).
   Each nonce is single-use: the first request that spends it consumes it.

2. **Sign.** Compute the BIP 340 (Schnorr) message

   ```
   msg = hash_SPc/Api(nonce bytes)
   ```

   where `hash_SPc/Api` is the SHA-256 tagged hash with tag `"SPc/Api"`
   (tag string hashed per BIP 340 convention), and `nonce bytes` are the
   **ASCII bytes of the nonce hex string itself** - the 32 characters
   returned by the challenge endpoint, not the 16 decoded bytes. Sign `msg`
   with `b_scan` (the x-only key of `B_scan`), hex-encode the 64-byte
   signature.

   Test vector (frozen in `bins/spc-pool/src/api.rs`):

   ```
   nonce  = "00112233445566778899aabbccddeeff"
   msg    = 59ffe830c5807ba676a5971c3abb2d78149ed04e9712b97d519a037229020dd7
   ```

3. **Call an authenticated endpoint** with the header:

   ```
   Authorization: SPc <addr> <nonce> <sig>
   ```

   - `<addr>`: the caller's silent-payment address (`sp1q...` / `tsp1q...`),
     which must parse for the network the pool is running.
   - `<nonce>`: the minted nonce.
   - `<sig>`: the signature as 128 hex characters.

   ```sh
   curl -s https://pool.example:8080/api/sp/me \
     -H "Authorization: SPc tsp1q... 00112233445566778899aabbccddeeff 0123..."
   ```

Server side, in order: the header is parsed (exactly four whitespace-
separated parts, scheme `SPc`); the address is parsed fail-closed for the
pool's network and reduced to `sp_id`; the nonce is checked for existence,
non-use and TTL, then marked used; the signature is verified against
`B_scan`. One nonce authenticates exactly one request - mint a fresh one per
request.

## 4. Error statuses

Every error body is `{"error": "<message>"}`.

| Status | When |
|---|---|
| `401` | `missing or malformed Authorization header` - absent, not four `SPc`-scheme parts, or trailing junk |
| `401` | `unknown or expired challenge` - wrong nonce, already-used nonce, or older than 60 s |
| `401` | `invalid identity` - the address does not parse for the pool's network |
| `401` | `bad signature` - not 64 bytes of valid hex, or the BIP 340 verification fails |
| `404` | `{"error":"unknown identity"}` on `/api/sp/me` - the address parses and authenticates, but the pool has no account rows for it |
| `404` | `{"error":"no such block"}` on `/api/sp/me/audit/<block_hash>` - not 64 hex chars, or not a settled pool block |

A `401` never reveals whether the nonce or the identity was at fault beyond
the message above; challenges are rate-limited only by their single-use,
60 s nature in the current build.

## 5. The liability definition

`GET /api/pool/liability` returns, in sats (SPEC §6.1):

```
Sum(pending_balance.sats) + Sum(ark_payout.sats WHERE status = 'scheduled')
```

This is **everything the pool holds for miners** (SPEC §0, §4.6): trimmed
balances waiting for a later coinbase, sub-threshold balances inside the
6-block confirmation window before the Ark send, and failed Ark sends
awaiting retry. It deliberately excludes:

- coinbase-tier payouts - non-custodial; they are the miner's the moment the
  block is mined, already on-chain;
- forfeited sub-threshold earnings of miners with no Ark delivery
  registered - not held; kept by the pool under the published policy
  (SPEC §11.1), tracked separately as `spc_forfeited_sats_total`;
- Ark-tier payouts already `sent` - from then on they are the recipient's
  VTXOs (trustoidal until refreshed, SPEC §11.7).

The same number is exported as `spc_pending_liability_sats`
(`docs/OPERATIONS.md` §7) for reconciliation.
