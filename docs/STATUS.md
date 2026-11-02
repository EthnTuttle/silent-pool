# Status

Live build board. `docs/IMPLEMENTATION-PLAN.md` defines the packets and gates.
A packet is Done only when its tests exist, `nix flake check` passes, both
reviews are recorded for risky packets, and the commit is pushed.

## Current wave and packet

Wave 5-7: JD publication, agent roles, Ark role and the regtest harness.

## Done

- 2026-11-02: implementation plan, decision record for native SV1/Nix/protocol
  baseline, SPEC/ARCHITECTURE/ROADMAP/README/UPSTREAM_DEPS updates.
- WP0: workspace, `flake.nix` (devShell, binaries, checks, NixOS modules), CI.
  Commit `0d2e333`.
- WP1: `spc-crypto` complete, BIP 352 reference address, frozen vectors,
  negative controls. Commit `8546cd0`; review fixes `c2d5b89`.
- WP2: `spc-scan` full-block and light-proof scanning, carriers, tweak export,
  CLI. Commit `be175c8`.
- WP3: `spc-accounting` slices, window, SLICE scores, tier routing, trim,
  autoscaler, distribution, O11 claim; SPEC §4.5 fixture and proptests.
  Commit `19598e5`.
- WP4: schema/migrations, binary stream codecs, bootstrap frame, Redis stream
  helpers, settlement/orphan/Ark-failure paths. Commits `f1ed26d`, `e5bb990`,
  `25c7b20`.
- WP5: native SV1 session + TCP listener; native SV2 Noise listener with
  SetupConnection, extended channel, jobs and shares; pool-template coinbase;
  block assembly; front state. Commits `9c6d2de`, `b508935`, `5953230`,
  `fb9c834`, `c3f5efa`.
- WP6 (partial): read-only API with `b_scan` challenge auth; payout role
  consuming `spc:slices`/`spc:blocks`/`spc:arkreg` and publishing
  `spc:front:events`; liability/settlement store. Commits `d3e0247`,
  `25c7b20`.
- WP7/WP8/WP9 (codec half): 0x0003 distribution/TLV/formula, 0x5350
  registration messages and error matrix, Ark order/receipt signing.
  Commit `b2ff4e2`.
- WP10 (partial): agent config validation and own-payout verifier.
  Commit `3cb96ae`.
- WP11 (partial): identity-free share leaf, merkle proofs, sampling verifier.
  Commit `4769802`.
- End-to-end tests: real SV1 socket finds a block; real SV2 Noise session
  finds a block. Commits `abf7102`, `c3f5efa`.
- `nix flake check` green after the SV1/SV2 and store additions.

Note: the background implementation sessions returned no files; all code above
was written and verified in the orchestration session plus local runs.

## In progress

- JDS publication/validation on top of `spc-ext::SetPayoutDistribution`.
- `spc-agent` translator/JDC roles and miner wallet.
- `ark` role using the signed order codec.
- `testing/regtest-e2e.sh` and the real-bitcoind path.

## Blocked

None. Remaining gaps are implementation, not decisions.

## Next dispatch

1. JDS/JDC: wire `SetPayoutDistribution`, `distribution_id` TLV, validation and
   same-height grace into `front` and `spc-agent`.
2. Ark role: order verification, float sends, receipts, board shortfall.
3. Regtest harness: bitcoind + pool + two SV1 miners + one SV2/JDC miner.
4. Type-32 server endpoints from the slice store and the seeded cheat test.
