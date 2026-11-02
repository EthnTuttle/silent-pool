# Status

Live build board. `docs/IMPLEMENTATION-PLAN.md` defines the packets and gates.
A packet is Done only when its tests exist, `nix flake check` passes, both
reviews are recorded for risky packets, and the commit is pushed.

## Current wave and packet

Wave 0 - WP0 (workspace, Nix, CI, protocol baseline).

## Done

- 2026-11-02: `docs/IMPLEMENTATION-PLAN.md` written (packets WP0-WP13, session
  roster, test matrix, Nix/CI design, waves).
- 2026-11-02: decision record for native SV1, Nix-only builds and the
  `stratum-core` baseline; SPEC/ARCHITECTURE/ROADMAP/README/UPSTREAM_DEPS
  amended.

## In progress

- WP0: workspace skeleton, `flake.nix`, CI.

## Blocked

None.

## Next dispatch

WP0 to Nix/CI engineer. Then WP1 to Core implementer A in parallel with WP0's
review.
