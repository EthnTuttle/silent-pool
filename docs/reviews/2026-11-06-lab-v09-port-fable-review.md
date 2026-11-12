# Lab L2 port to rbitcoin v0.9.0: Fable review

Date: 2026-11-06
Reviewer: Claude Fable 5.1 (subagent; read-only on the candidate)
Candidate: `/tmp/opencode/rbitcoin-v0.9-lab`, uncommitted on official
`v0.9.0` = `9614b27ecd832a04070a5226da26c8844284f2c4` (HEAD equals base).
Manifest: `/tmp/opencode/lab-v09-source-manifest.json`, 19 files,
`snapshot_sha256` `0e037d358c057d382adcca8d6c094bcf4a46eb1e02758777d20f5d70eb4ddcda`.
Binary: `target/dev/debug/rbitcoin-node` SHA256
`bcb6d5eefe0dd0bd96e2ac79e173b4b99a91f67f7717dae6620c30786378a171`.
Scope: source, port fidelity, and node gate only. Live compatibility,
retarget, mining, and namespace proof belong to the independent TFF
validator session and are not duplicated here. The namespace candidate
`553cd4e8` is a separate review; nothing here inherits to it.

## Verdict

**APPROVE** for source, port fidelity, and the node gate, tied to snapshot
`0e037d35` and binary `bcb6d5ee`. Approval is split explicitly:

| Claim | Status |
|---|---|
| Port is the approved L2 delta and nothing else | **Proven** (section 2) |
| Lab profile semantics preserved on v0.9.0 | **Proven by source and tests** (sections 3-4) |
| Default v0.9.0 behavior unchanged without the flag | **Proven by source** (section 3) |
| Workspace suite green | **Not established**; recorded limitation (section 5) |
| Live node behavior | **Pending** the independent validator; not claimed here |

Required code fixes: none. Two report corrections (section 6) and one
gate I ran because the report omitted it (ast-grep, pass).

## 1. Hash verification

All 19 per-file SHA256 values match the working tree. The aggregate
reproduces exactly from the recorded recipe (canonical sorted compact JSON
of `{base, files}`). `git status` shows 17 modified tracked files, 2
untracked source files, and `docs/reviews/` (excluded, mutable); no
`scripts/__pycache__`. The binary hash matches the parent's value.

## 2. Port fidelity (actual delta vs approved delta)

Method: regenerate the old approved delta from the frozen tree
(`git -C /tmp/opencode/rbitcoin-lab-retarget-profile diff e8cf9c9d`) and
the new delta (`git diff 9614b27e -- . ':!Cargo.lock'`), strip `index` and
`@@` lines, and compare.

- Both deltas are 797 lines; after stripping hunk headers they are
  **byte-identical**. `/tmp/opencode/v09-audit/l2-port.patch` equals the
  old delta the same way. The 16 tracked files changed by the port are the
  same 16 files with the same hunks; only line offsets moved.
- The two new files (`changelog.d/retarget-regtest.md`,
  `crates/rbitcoin-consensus/tests/header/regtest_overlay_tests.rs`) are
  `cmp`-identical to the frozen approved copies.
- For each touched production file, `diff old-candidate new-candidate`
  line count equals the upstream `e8cf9c9d..9614b27e` line count for that
  file (`header.rs` 0/0, `params.rs` 25/25, `config.rs` 0/0, `cli.rs`
  16/16, `methods/chain.rs` 0/0, `methods/mod.rs` 2/2). The port
  introduced nothing beyond the approved hunks, and upstream's changes in
  those files survive intact.
- Upstream content preserved at the overlaps: mainnet milestone
  `966_143` / testnet3 `5_128_859` and their hashes (`params.rs:517-544`),
  the v32.1 min-chain-work comments, `ERR_VERIFY_ALREADY_IN_UTXO_SET`
  (-27) in `methods/mod.rs`, and the `format!`-based usage text in
  `cli.rs` (the ported help block contains no braces, so the new
  `{main_height}` placeholders are unaffected; the node suite compiles and
  passes 120/0).
- No manual merge, no whole-file overwrite, no third-party dependency,
  pin, or `flake.lock` change. `UPSTREAM_DEPS.md` and the node checkout in
  silent-pool are not touched by this review.

### `Cargo.lock` (19th file)

The v0.9.0 tag's `Cargo.lock` is byte-identical to the old base's lock and
still lists workspace members at `0.8.99`, while `Cargo.toml`
`[workspace.package] version = "0.9.0"` with 16 member crates on
`version.workspace = true`. The first build regenerated the lock. Actual
change: **16** `-version = "0.8.99"` / `+version = "0.9.0"` pairs
(`rbitcoin-bench`, `-cli`, `-consensus`, `-electrum`, `-esplora`, `-ibd`,
`-log`, `-mempool`, `-net`, `-node`, `-primitives`, `-query`, `-rpc`,
`-store`, `-sv2`, `-test`), 32 lines, nothing else. No dependency entry,
checksum, or source changed. This is an upstream release-packaging defect
(`cargo --locked` on the tag would fail), not a port change; recording the
regenerated lock in the manifest is the right call because it is the
source the binary was built from.

## 3. Semantics preserved (re-read on the v0.9.0 tree, not inherited)

- Validation limit vs retarget floor: `pow_limit` stays `0x207fffff` for
  genesis and `pow_hash_meets_target`; `btc.max_attainable_target` is the
  floor `0x1e7fffff` only under the flag and only reaches the arithmetic
  through `from_next_work_required`'s `max_transition_threshold`.
- Bootstrap: height 1 computes `prev_bits = pow_limit`, min-difficulty off,
  overlay rewrites to the floor or `--regtest-pow-bits`; heights 2..23
  inherit; the first boundary at 24 uses genesis as period-first (Core's
  genesis-anchored first period, documented in `operations.md`: on a wall
  clock boundary 24 clamps slow, first responsive boundary is 48).
- Cross-network refusal, zero/negative/overflow bits, equal-to-floor
  accepted in production order, reverse order bounded by the attainable
  target: unchanged code, `tests/params.rs` and `tests/cli.rs` cover them.
- Introspection: `getblockchaininfo.consensus` is the identical object;
  `retarget_floor_*` equals `pow_limit_*` on networks without a floor.
- Fresh-datadir contract, not runtime rejection: the report states it
  correctly and adds the accurate note that upstream v0.9.0's wrong-genesis
  datadir refusal does not cover a default-regtest datadir (same genesis).
  No old-data rejection is claimed and none exists.
- Default and non-lab paths: `apply_regtest_pow_overlay` returns before
  any profile logic when the overlay is `None` or the network is not
  regtest; `apply_regtest_retarget` runs only under the flag. Without the
  flag the node is v0.9.0 upstream, including the new milestone defaults.
- Consumer set: the files and the count (28) of `no_pow_retargeting()`,
  `.pow_limit`, and `max_attainable_target` uses are identical between the
  approved tree and this tree; upstream added no new next-work path. All
  next-work paths still end in `next_work_bits` or
  `expected_bits_off_tip` -> `apply_regtest_pow_overlay`
  (`net/chain.rs:2152`, `ibd/header_walk.rs:3185`,
  `query/header_read.rs:189`, `confirm_run/phases.rs:540`).
- No second algorithm, no env knob, no schema change, no share-target
  input. Genuine PoW at every height.

## 4. Test evidence on this tree (logs read once; exit codes and names only)

| Gate | Result | Log |
|---|---|---|
| fmt | 0 | `/tmp/opencode/v09-fmt.log` |
| consensus suite | 0, 319/0 | `v09-suite-consensus.log` |
| node suite | 0, 120/0 | `v09-suite-node.log` |
| rpc suite (incl. three-period floor clamp) | 0, 217/0 | `v09-suite-rpc.log` |
| journey `retarget_regtest_node_journey` | 0, 1/0 | `v09-green-journey.log` |
| bench | 0, 49/0 | `v09-bench.log` |
| unwrap/hex ratchets + self-tests | 0 x4 | `v09-{unwrap,hex}*.log` |
| clippy workspace `-D warnings` | 0 | `v09-clippy-workspace.log` |
| cargo deny | 0 (advisories, bans, licenses, sources ok) | `v09-cargo-deny.log` |
| NixOS module eval | 0 | `v09-nixos-module-eval.log` |
| **ast-grep self-test + scan (run by me; missing from the report)** | **0, 0** (13 fixture checks; `ast-grep 0.42.1` in the pinned shell, CI pins 0.45.1) | `/tmp/opencode/Fable-v09-ast-grep.log` |

Red/green binding on the new tree: with `max_attainable_target = floor`
removed, `retarget_profile_boundary_takes_over_from_the_overlay` fails at
`tests/methods/mod.rs:6734` with `1f01ffff` vs `1e7fffff`
(`v09-red-floor-clamp.log`, exit 101); restored, 0
(`v09-green-boundary.log`). Same mechanism signature as the approved
proof. Nothing in the suite was weakened; no timeout changed.

## 5. Suite-green status (not established; recorded)

Facts from the logs, not the report:

- Standalone `rbitcoin-test` suite, one run, host load 2.0 -> 5.9 with an
  independent live validator active: 93/4. Failures:
  `cross_surface::sv2_tp_bootstrap` (`cross_surface.rs:3687`, fee
  visibility), `end_of_ibd::end_of_ibd_work_fork` (`end_of_ibd.rs:1026`,
  wall), `integration_multinode::mocktime_generate_keeps_ponging_peer`
  (`integration_multinode.rs:147`, mock step 8), and
  `integration_multinode::p2p_timeout_getaddr_and_keepalive_ping`
  (`integration_multinode.rs:821`, 20 s wall; a `P2PNode` journey with
  `worker_threads = 4`, first time this one appears in our runs).
- Workspace run minutes later, load 4.8 -> 8.5: `rbitcoin-test` 97/0
  (`v09-workspace-test.log:25292`); only `tip_accept::full_lane_parks_instead_of_spinning`
  failed (net 745/1), the test that failed every full-suite run on both
  old trees and on the old clean base.

Assessment: the four standalone failures are the same load-sensitive
family seen on the old candidate and on the old clean base, plus one new
member of the same shape (a wall timeout over a live P2P journey). The
port's code is identical to the approved delta, the delta has no path to
any of these tests (none runs the profile; none calls
`getblockchaininfo`), and that remains an argument from absence, not a
proof. There is no quiet A/B on v0.9.0 and no clean-v0.9.0 baseline run;
the one 97/0 pass under heavier load is consistent with, not proof of, an
upstream deflake. I did not run a baseline: it needs a second full build
and a quiet host, and the host is not quiet while the live validator runs.
The report says the same, which is honest.

This is recorded as a limitation on CI merge readiness, not on the lab
binary. It is the same limitation the L2 adjudication of 2026-11-06
recorded, with the same bounded diagnostic available if the parent wants
it later (three suite-alone runs with `--skip
scenarios::retarget_regtest_node_journey`, quiet window granted, not
assumed). Not required for lab use.

## 6. Report and manifest corrections

1. **Manifest provenance says "17 rbitcoin-* self-version entries"; the
   actual count is 16.** The report (`2026-11-06-v0.9-lab-port.md`) says
   16 and is right. Fix the `provenance` string in
   `lab-v09-source-manifest.json`; this changes the manifest file's own
   hash but not `snapshot_sha256`, which covers `base` and `files` only.
2. **ast-grep is a CI `qc` step (`ci.yml:58-64`) and was not in the
   report's gate table.** Add the row: self-test 13 passed, scan exit 0,
   `ast-grep 0.42.1` locally vs `0.45.1` in CI
   (`/tmp/opencode/Fable-v09-ast-grep.log`).

## 7. Nits (not gating)

- The report's binary row should say the dev-profile build is what the
  live validator is testing and that a release-profile build is a separate
  artifact with its own hash; the "Unresolved limits" section already says
  this, the handoff row should too.
- `COMPAT.md` and `operations.md` rows are the approved text verbatim;
  v0.9.0's own `COMPAT.md` rows above them changed, and the table still
  renders (checked the diff context). No action.

## What the parent needs for pin adoption

Per the user's rule, the developer pin is not adopted on this review
alone. This review clears source, port, and node gate for `0e037d35` /
`bcb6d5ee`. Adoption waits on the independent live validator's result on
the same binary hash and on the separate namespace review of `553cd4e8`.
