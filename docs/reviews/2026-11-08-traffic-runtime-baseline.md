# Traffic runtime source layer recipe

Recorded before implementation, 2026-11-08.

1. Start from the sole-owned `lab-traffic-runtime` worktree at `b0ab4ca`.
2. Verify all 658 rows of `/tmp/opencode/AP2-nix-src-manifest.json`
   (SHA-256 `9fc3c7e14997de3cc22bf7e534d08398f2967377e99eccf854c50df8731a9987`)
   against `/tmp/opencode/AP2-nix-src`, then copy only those indexed paths.
   Indexed source size: 10,828,681 bytes.
3. Verify all 15 rows of `/tmp/opencode/tjv3-changed-rows.json` against
   current primary before copying only those paths. These are the eight
   modified core/T0 rows, six new history-3 rows, and the merged SPEC row.
   `traffic_contract.rs` is the parent merge `3a4177bf739f26e05976df053fbeaad23e7e316bfd3a04fb0b72da9becbe395f`.
4. Preserve this combined source as the delta comparison manifest, outside
   the worktree. No .git, target, runtime datadir, or unindexed primary file
   is copied. The recorded recipe file itself is packet-owned documentation.

Implementation scope: traffic actor modules and existing-package binary;
typed controller RPC, traffic runtime/control, strict CommandLog bootstrap
reservation mode, T0 allocation/routing schema, focused engine/startup and
CLI integration, tests and dated decisions/changelog fragments. No changes
to audited journal/snapshot/rollback/signing/key code are planned. J1 probe
and AP4 server implementation remain separately owned.
