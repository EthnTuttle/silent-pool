# T1 -> live-wallet UI provider contract v1

The authenticated existing endpoint is:

`GET /api/lab/v1/runs/<run_id>/projection`

A user receives one `spc-user-projection/v1` object; the operator receives
an array of those objects, one per declared user claim. Existing worker fields
are preserved. The new `payment_wallet` field is exactly:

```json
{
  "schema": "spc-t1-payment-wallet/v1",
  "user_id": "id-alice",
  "state": "available",
  "source": "T1 test-payment funds",
  "stage": "active",
  "spendable_sats": "9007199254741115",
  "unconfirmed_sats": null,
  "pending_sats": null,
  "receive_address": "<actual current regtest P2PKH receive address>",
  "source_observed_at_ms": 1794280000000,
  "source_age_ms": 25,
  "reason": null
}
```

The amount above is an exact-string example, not a funding claim. No numeric
conversion through JavaScript Number is permitted. `state` is `available`,
`not_available` or `stale`; stage is `active` or `not_active`. Missing funds or
address are null, never invented zeros. A fresh fully proven empty balance may
be `"0"`. This provider always emits null pending/unconfirmed amounts. Unknown
outgoing reservations remain excluded from spendable confirmed funds.

Source times are from the last successful actor canonical observation, not the
HTTP poll time. The controller rechecks the monotonic age and boot. A failed
observer, stale/future source, busy or missing actor, no native bootstrap
provider, or incomplete traffic stage cannot enable the payment button.
Receive address derivation uses the current index without advancing it.

## Exact UI-owner join

Examined `lab-live-wallet-ui/bins/spc-lab/static/app.js:463-487` reads
`walletRows[i].payment_wallet`; `pollUser` already fetches `/projection` and
`/wallets` together. Normalize projection to an array and join by
`projection.user_id == wallet.identity_id`, assigning **this exact object** to
the row's `payment_wallet`. If a declared projection has no SP/Ark agent row,
the UI may create a T1-only row with that identity and null SP/Ark status/inputs.
Do not require an Ark agent to display T1 funds. Never merge either balance
into the other account. The existing button predicate (state available plus
positive exact-string spendable funds plus the authenticated command grant)
continues to apply. Display `receive_address` and source freshness separately.
Also expire cached observations at render/click time: require a non-future
`source_observed_at_ms` no more than 15000 ms old (or conservatively mark them
unavailable). A stalled browser poll must not retain an enabled button forever.

Alternatively, the UI backend may populate its rows by calling
`RunEngine::t1_payment_wallet(&AuthContext, declared_identity)` per row. That
method rejects cross-user or undeclared targets before actor I/O. An operator
may explicitly request each declared identity. No caller-supplied driver,
wallet index, path or arbitrary RPC is accepted. `/projection` query selectors
are rejected before projection I/O. No new HTTP route or stock wallet RPC exists.

The authenticated operator `/catalog` additionally carries:

```json
{"traffic_controls":{"available":true,"drivers":[{"driver_id":"txd-0"}]}}
```

False availability yields an empty driver list. The gate requires active
verified traffic, a native executor, validated allocation and actual live
actors advertising `payment_wallet_read_v1`. Process-running flags alone are
insufficient. User catalog access remains forbidden.

## Private channel

Control-v1 operation `payment_wallet` has no durable command id and carries
only `{user_id,wallet_index,allocation_digest}`, selected by the controller's
validated allocation. Its closed private reply binds driver, global index,
allocation digest, observation boot/monotonic time and journal revision around
the public view. These private binding fields and recovery maps never enter
the browser object. Actor reads are multiplexed on the existing pipe and use
the same single-writer serialization as snapshots. They never append to the
wallet journal or allocate receive/change indexes.

This is a separately hash-bound layer on the passing acceptance-adapter
packet. UI asset wiring stays with its owner; live funded full T3/Ark/HTML
acceptance is not established by source, unit or API-fixture checks.
