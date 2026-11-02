# 2026-11-02 - Native SV1, Nix-only builds, and the protocol baseline

**Trigger.** Two requirements added after the v0.1 spec was frozen: the pool
must serve SV1 and SV2 connections natively, and every build/test/CI/deploy
artifact must be a Nix flake output. Implementation also needed a concrete
baseline for the SV2 codec, because the spec pins upstream repositories but the
crate releases move independently.

## Decision

1. **Native SV1 in `spc-pool`'s `front` role.** A new listener accepts
   standard Stratum V1 (`mining.subscribe`, `mining.authorize`,
   `mining.set_difficulty`, `mining.notify`, `mining.submit`). The identity is
   the SV1 username with an optional `.worker` suffix stripped at the first
   `.`, parsed by the same `§2.1` rules and rejected fail-closed on any
   failure. One connection is one `sp_id`. SV1 has no Job Declaration: every
   SV1 share is pool-template mode, `§4.1`'s `delta = 0` case, and uses the
   `§2.4.1` scriptSig carrier. Shares enter the same validation, replay set,
   slice and distribution path as SV2 shares; there is no second accounting
   engine. The `spc-agent` translator role remains for miners that need local
   fan-out or own-payout verification, but it is no longer required to reach
   the pool.
2. **Protocol baseline.** The workspace uses `stratum-core` 0.6.0 from
   crates.io (re-exporting `binary_sv2`, `codec_sv2`, `framing_sv2`,
   `noise_sv2`, `mining_sv2`, `channels_sv2`, `sv1_api`). The project-defined
   `0x0003` (PR #203) and `0x5350` message structs live in `spc-ext`; no fork of
   `extensions_sv2` is required. Upstream SHAs in `UPSTREAM_DEPS.md` remain
   reference points for behaviour and are absorbed in-tree per
   `2026-10-14-upstream-own-suite.md`.
3. **Nix is the only build path.** `flake.nix` provides the dev shell, the
   three binaries, checks (fmt, clippy, tests, docs, e2e), NixOS modules and
   the CI entry point. A Cargo build still works for developers, but no
   release, CI job or deployment may depend on it.
4. **TDD is spec-derived.** Each work packet in
   `docs/IMPLEMENTATION-PLAN.md` names the spec sections that gate it. A packet
   is not done until those tests exist and pass. Ambiguity is resolved in a
   dated decision record, never by a silent implementation choice.

## Why

- **Reach.** Most installed ASIC firmware speaks SV1. Shipping a translator is
  not the same as supporting those miners: the pool's published policy
  (`§3.1`, `§11.1`) and payout verification (`§3.6`) are only enforceable at
  the pool if the pool sees the connection. Native SV1 also lets a miner build
  the coinbase (`coinb1`/`coinb2`, extranonce) and therefore see the SPc
  carrier, which a translated connection hides.
- **One identity rule.** Parsing SV1 usernames with the same fail-closed
  function eliminates the historical failure mode where an unrecognised
  username silently becomes the pool's own payout. A bad username is a JSON-RPC
  error and no job, with the connection left open to retry.
- **One accounting path.** SV1 is a pool-template client, so `delta = 0` and
  the fee term collapses. Reusing `spc-accounting` avoids a second, untested
  payout implementation.
- **Reproducibility.** The spec's pins are design references, but CI must be
  reproducible on any host. `flake.lock` plus `Cargo.lock` gives one command
  (`nix flake check`) that proves the tree, including the regtest harness.
- **Reviewability.** A crate-level baseline with our own extension codecs keeps
  the diff we own small and the upstream surface replaceable.

## Alternatives rejected

- **Translator-only SV1** (the original spec): rejected because it cannot
  enforce identity or deliver SV1-level coinbase visibility, and it makes the
  pool's deployment depend on a client binary.
- **Forking `sv2-apps` pool and JD client wholesale**: rejected as the first
  step because it imports a large upstream application structure whose pinned
  commits already lag the crates.io codec, and it would make the SPc-specific
  paths harder to isolate. The fork remains a reference; `stratum-core` is the
  dependency.
- **Git dependencies on the pinned SHA only**: rejected for the initial
  baseline because `warioishere/stratum@9be4023` changes an unreleased codec
  and pins by branch upstream. Behaviour is ported into `spc-ext` instead.
- **CMake/Make/Nix hybrid build**: rejected; it defeats the single-command
  reproducibility goal.

## Consequences

- `spc-pool` gains an SV1 module and config (`[sv1]`), and `front` must budget
  `extranonce1 + extranonce2` inside the `§2.4.1` scriptSig budget. Defaults:
  4 + 4 bytes, leaving 55 bytes of the 63-byte budget.
- SV1 clients cannot run JD or request the type-32 extension; the pool
  advertises that in its SV1 `mining.subscribe` response extensions and in
  `/api/pool`.
- The spec's "pool never speaks SV1" statements in `§1`, `§3`, `§9` and
  `ARCHITECTURE.md` are amended; the SV2-only claim in the README is amended.
- CI images are Nix-derived; no GitHub Action may run `cargo` outside
  `nix develop`.
- `UPSTREAM_DEPS.md` gains the crates.io baseline row; a future pin bump must
  update it, `Cargo.lock` and `flake.lock` together.

## Edits applied

- `docs/SPEC.md`: frontmatter changelog; `§1` components and process-boundary
  text; `§3.1` SV1 channel open; `§3.4` shared share path; `§6.3` `[sv1]`
  config; `§7.1` workspace; `§9` topology.
- `docs/ARCHITECTURE.md`: `§1`, `§2.1`, `§5.1`, `§10`.
- `docs/ROADMAP.md`: Decision 8 note and Phase 5 scope.
- `README.md`: status/component wording.
- `UPSTREAM_DEPS.md`: crates.io baseline.- `docs/IMPLEMENTATION-PLAN.md`, `docs/STATUS.md`.
