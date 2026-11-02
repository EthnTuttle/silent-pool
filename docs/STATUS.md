# Status

Live build board. `docs/IMPLEMENTATION-PLAN.md` defines the packets and gates.
A packet is Done only when its tests exist, `nix flake check` passes, both
reviews are recorded for risky packets, and the commit is pushed.

## Current state

The pool runs end to end on regtest. Native SV1 and SV2 listeners build the
same SPc coinbase, the front assembles and submits a block, and `spc-scan`
recovers the miner's output. The remaining work is the live JDS/JDC loop, the
stock-bark `ark` adapter, and the type-32 network server.

## Done

- Plan, decision records and spec amendments for native SV1, Nix-only builds
  and the `stratum-core` 0.6.0 baseline.
- `spc-crypto`: identity, BIP 352 address, ECDH, tier tweaks, P/U, mailbox
  blinding, carriers, zeroising `SendKey`; frozen vectors and BIP 352
  reference address.
- `spc-scan`: full-block and light-proof scanning, both carriers, H/H-1,
  tweak export, `gen-address`, file and RPC sources.
- `spc-accounting`: slices, sealing, window, SLICE scores, tier routing,
  pending, trim, autoscaler, distribution, JD weights, O11 claim. SPEC §4.5
  fixture and proptests.
- `spc-store`: SPEC §5.1 schema, migrations, settlement/orphan/Ark-failure
  paths, binary stream codecs, bootstrap frame, Redis stream helpers, store
  queries for the API.
- `spc-pool`: native SV1 session + TCP listener; native SV2 Noise listener
  (SetupConnection, extended channel, jobs, extended shares); pool-template
  coinbase; block assembly; front state; payout role; read-only API with
  `b_scan` auth; JD distribution publication/validation state; Ark order
  verification; identity-free share blobs and the API audit view.
- `spc-ext`: 0x0003 distribution/TLV/formula and 0x5350 registration messages,
  BIP340 signing, Ark order/receipt codecs.
- `spc-share-ext`: identity-free leaf, merkle proofs, sampling verifier.
- `spc-agent`: SV2 client, translator role, own-payout verifier, config.
- End-to-end: mock SV1, mock SV2, SV1 -> translator -> SV2, and a real
  bitcoind regtest run that submits two blocks and scans the miner output.
- `nix flake check` green.

## Remaining

1. Wire the JD network loop: JDP listener in `front`, `SetPayoutDistribution`
   publication, `distribution_id` TLV extraction from declarations, and the
   JDC role in `spc-agent` (state machine and codecs are done).
2. Replace the mock Ark transport with a stock bark wallet adapter, and emit
   `spc:ark:orders` from `payout` at `H_f + 6`.
3. Serve type-32 messages over SV2 from the stored slices and add the seeded
   cheat-slice test.
4. Add the Postgres/Redis services and the payout/API roles to the regtest
   harness (the front path already passes without them).
5. Run the plan's normative and security reviews over the integrated tree.

## Blocked

None. The remaining items are implementation.

## Next dispatch

One session per item above, disjoint paths, with the existing tests as the
acceptance gate. Every change ends with `nix flake check` and a why-first
commit.
