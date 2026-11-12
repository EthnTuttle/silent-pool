# 2026-11-10 - Live source-feed activation and wallet grant fences

## Authority

Parent-directed QA producer gaps and the pool-wallet-control fresh-grant
contract against frozen control source
`55a488197a0e116ab4d229ba426a12acb551bf16f8313698590de8cf0a685678`.

## Decision

The actual translator runtime consumes a bounded private Unix feed with a
pool-signed trace head and an independently signed collector checkpoint.
Actual received job bytes, generation and bundle/history digests are bound to
a fresh exchange. Verified proof consumption is signed by B_scan, durably
ACKed by the collector, then activates work. New jobs invalidate old work and
cancel stale audits; no self-attested bundle root establishes completeness.
QA owns collection/replay and the collector server; the agent owns reception,
verification and activation. Frozen source evidence records the real pre-mining
open/finding slice exclusion boundary. WIRE §3.10 defines the frames/preimages.

The stock sender takes the wallet's single shared operation capacity before
requesting a fresh grant. A non-cloneable permit is checked against that exact
wallet; guarded send never locks twice. The same immutable grant is fenced
after lock acquisition, before input registration and at single-use cosign
escape. The existing durable grant/worker pin precede escape. Completion after
escape requires no replacement grant; ambiguity keeps the same input action
and reservations. Capacity waits are bounded at 30 seconds and do not affect
the independent payout authorization dispatcher or unrelated mining.

## Integration boundary

Current concrete handle is `BarkWalletClient`, passed directly to
`BarkTransport` in main. The frozen control packet and this follow-up must
share its `operation_lock`, not instantiate a second mutex. Control-owned
ticks that already hold it call the guarded operations directly. No Ark
message, server RPC or grant-wire change is introduced.

Source is authored with focused regression cases; execution awaits the
parent-assigned heavy slot. This record is not a funded acceptance receipt.
