# Status

Live build board. `docs/IMPLEMENTATION-PLAN.md` defines the packets and gates.
A packet is Done only when its tests exist, `nix flake check` passes, both
reviews are recorded for risky packets, and the commit is pushed.

## Active sessions


The integrated review returned REQUEST CHANGES (11 blockers); the fix session above owns its files. This thread only dispatches, verifies commits and updates this board.

## Current state

The pool runs end to end on regtest. Native SV1 and SV2 listeners build the
same SPc coinbase, the front assembles and submits a block, and `spc-scan`
recovers the miner's output. The remaining work is the live JDS/JDC loop, the
stock-bark `ark` adapter, and the type-32 network server.

## Done

- Bark adapter decision record (commit `25f0a01`): stock `bark-wallet` pool
  transport, in-tree agent keystore, O13 resolved; follow-up doc edits applied
  (UPSTREAM_DEPS bark pin, SPEC §10 O13 row, ROADMAP Phase 7 note).
- `docs/OPERATIONS.md` and `docs/API.md` (commit `7fcbd64`).
- Protocol fixes: SV2 one-identity-per-connection, JD-only carrier,
  own-payout verification, `OP_RETURN` value guard, build-time budget check
  (commit `76df4f8`).
- Front accounting fixes: network-difficulty share `d`, template `job_fees`,
  slice `ref_job_id`/`ref_fees`, fee-band tests (commit `3b79e76`).
- Settlement/Ark fixes: chain-gated booking, null-fingerprint pending,
  settlement-time forfeiture, Redis-backed Ark dedupe, scrub excludes
  `failed_to_pending` (commit `af8a818`).
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
- End-to-end: mock SV1, mock SV2, SV1 -> translator -> SV2, a real bitcoind
  regtest run, and a full-stack run with Postgres/Redis + `front,payout,api`
  that settles block 102 and serves the API.
- Live JDS/JDC: token allocation, `DeclareMiningJob` with the distribution
  TLV, and JDS recompute-and-compare of the declared coinbase.
- Type-32 windows and share proofs served over SV2; `spc-scan audit` verifies
  sampled shares against a live server.
- `payout` emits signed, capped `ArkSendOrder`s for Ark-rail entries; the `ark`
  role verifies them.
- `nix flake check` green.

## Active sessions (continued)

- `ses_e6244a39bffe4tzubdnupgzJ90` — merge-gate fixes: slice-credit FK placeholder, identity upsert, harness confirmation depth.

## Review

`docs/reviews/2026-11-02-integrated-review.md` — REQUEST CHANGES. The blockers
are being fixed by the sessions above; re-review follows.

## Remaining

1. Replace the mock Ark transport with a stock bark wallet adapter. Order
   emission and verification are done; only the wallet/server call is missing.
   This needs a pinned bark checkout and a stock `captaind`.
2. JD fallback rules (JDS declines, 10 s silence) and the full-template rebuild
   path in the JDC.
3. The seeded `--features cheat-slice-difficulty` statistical test.
4. Normative and security review over the integrated tree.
5. Testnet soak and third-party `spc-scan` recovery.

## Blocked

None. The remaining items are implementation.

## Next dispatch

One session per item above, disjoint paths, with the existing tests as the
acceptance gate. Every change ends with `nix flake check` and a why-first
commit.
