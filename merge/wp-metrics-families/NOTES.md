# wp-metrics-families merge notes

Packet: role-sourced SPEC §9 metric families (STATUS "Remaining" item 4).

## Shared-file hunks (merge-session-owned surfaces)

- `docs/OPERATIONS.md`: apply `operations-section-7.patch` (moves the now
  implemented families from "Planned (not exported in this build)" to
  "Exported today", corrects `spc_distribution_slots` label classes to the
  SPEC §9 `{kept,ark,error}`, and leaves `spc_ark_surplus_sats`,
  `spc_outbox_*` and `spc_tip_divergence_seconds` as Planned with their
  blockers named). Verified with `git apply --check` against `3c2df0d`.
- `bins/spc-pool/src/main.rs` (committed on the branch, listed here because
  the file is merge-owned): four minimal hunks -
  1. api role: best-effort `Streams::connect` + `.with_streams(...)` on
     `StoreBackend` (role families absent without Redis);
  2. payout role: `.with_ark_retry_blocks(config.ark.retry_blocks)`;
  3. front role: three `CounterMap`s created and attached to
     `Sv1Server`/`Sv2Server`/`JdsServer` builders;
  4. front loop: a rate-limited `role_metrics::publish_front` at the top of
     the loop body (15 s interval, same `streams` handle).
- `bins/spc-pool/src/config.rs`: **no changes**.
- `Cargo.toml`/`Cargo.lock`/`flake.nix`/`config/*.example.toml`: **no
  changes** (no new dependencies; `cargo` re-normalizes two stray lockfile
  whitespace lines on any build in this tree - discard, not packet content).

## Changelog fragment

`docs/changelog/2026-11-17-metrics-families.md` (splice per the fragment
protocol, then delete).

## Pre-existing failure (not this packet)

`ark::tests::held_order_stays_unacked_until_due` fails at base `3c2df0d`
when `SPC_TEST_REDIS_URL` is set (the default gate never sets it, so CI is
green). Reproduced on the unmodified base in this worktree against a fresh
redis-server 8.10.3: after the tip reaches `not_before_height`, the single
`retry_pending()` pass emits no receipt - consistent with the durable fair
cursor having advanced past the held entry on the not-yet-due pass, needing
the wrap-to-zero pass the test does not perform. Needs its own diagnosis
packet.
