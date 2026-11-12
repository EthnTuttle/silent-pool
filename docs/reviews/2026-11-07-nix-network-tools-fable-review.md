# Fable review: Nix network tools candidate (ad03 / 3b4f66)

Date: 2026-11-08 (review of the 2026-11-07 candidate)
Reviewer: Fable (final gate), read-only; no candidate edit, commit, mining run,
review-server restart or subagent.
Candidate: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-nix-network-tools`
Manifest: `/tmp/opencode/Nix-net-tools-final-manifest.json`
(sha256 `264e4820ef202109fe7da4405c34f33924f658d5eaaefce79843bedeb34d5b89`)
Code aggregate: `ad03c7e0d23ba83b5d82448fcf5a60754583ad83c5033569b2f3cd309692d5c3`
Full aggregate: `3b4f66c24058f1828b972573f313951ed40cd6230da909ba1ee2a4ee7b6926ab`
Candidate report: `docs/reviews/2026-11-07-nix-network-tools-implementation.md`
(sha256 `ea00a0dbb4d1e0f4ac7309a9d2380afc97ea3dca64043cbff889b6ec3495ef86`)

## 1. Verdict

**REQUEST_CHANGES - bounded to one file.**

The Nix packaging packet itself (`nix/lab-network-tools.nix`,
`scripts/lab/check-nix-network-tools.sh`, the `flake.nix` wiring, the two docs
sections, the realized closure and the recorded namespace proofs) is sound and
is **approved as-is** at the file hashes recorded in the manifest. The cold
`--offline` limitation is genuinely resolved: it was an incomplete store with
substituters disabled, not the pin, and no pin, trust, signature or config
change is present.

The blocker is the production delta in `crates/spc-lab/src/supervisor.rs`
(manifest record `b0feca68aff9bf29825e6c707645c4f209107488146dbd7083f696f538fd2502`,
from approved `a73892d0...`). `coreutils_applet_argument()` fixes the one
alias case the T4 topology test needs, but I verified against the real pinned
coreutils 9.12 binary that it **breaks every direct launch of a Nix coreutils
applet through the supervisor** (a case the approved `30fed` supervisor
handled correctly), and its only test asserts an argument the real binary
rejects. Details and the exact fix/test required are in section 5. Nothing
else needs to change; a corrected or removed `supervisor.rs` re-freezes a new
`ad03` and this review can be closed on that one file.

## 2. What I independently verified (manifest and aggregates)

- Manifest sha256 reproduces (`264e48...`). 373 records, path-sorted, no
  symlinks, no `.git`/`target`/`result` leakage, zero group-write bits.
- Re-walked the candidate tree from disk: 373/373 files, zero hash or mode
  mismatches, no missing or extra files. Recomputing the recipe
  `sha256(json.dumps({'base','files'}, sort_keys, separators=(',',':')))`
  from disk gives `ad03...` (all files except the mutable report) and
  `3b4f66...` (all 373 files). Both match.
- Approved `LP-final-runtime-manifest.json` (`8bbe09...`) reproduces its own
  `30fed...` code and `9ffff6...` full aggregates; the frozen approved source
  copy `/tmp/opencode/LP-final-runtime-src-v4` still matches it 370/370.
- **Like-for-like per-file delta** approved -> candidate: 365 approved code
  files byte-identical + the frozen 2026-11-06 report byte-identical (so 366
  preserved, as claimed); added `nix/lab-network-tools.nix`,
  `scripts/lab/check-nix-network-tools.sh`, this candidate's report;
  modified exactly `crates/spc-lab/src/supervisor.rs`, `docs/LAB-RUNTIME.md`,
  `docs/OPERATIONS.md`, `flake.nix` with the from/to hashes in the manifest's
  `bootstrap.modified_approved_paths`. No mode changes. The report's scope
  caveat (the `ad03` aggregate includes the 11-06 report that `30fed`
  excluded) is correct; the naive `30fed` vs `ad03` comparison is indeed not
  like-for-like and I did not use it.
- `git diff` in the worktree is not the candidate delta (the approved tree is
  itself dirty against `b0ab4ca`); I diffed against the frozen approved copy.

## 3. Nix closure: purity, provenance, completeness

Verified on the realized store, not from the report:

- **Tool output** `7rrzlf5d67m0k5ggg9wfs5yqnfvmljqz-spc-lab-network-tools`:
  342 `bin` entries, 60 declared tools; every declared tool resolves into
  `/nix/store`; `package-outputs` lists exactly the 15 pinned package outputs.
  Closure 116 paths, `closureSize` 478,069,488 bytes, list identical to
  `/tmp/opencode/Nix-net-tools-closure-paths.txt`.
- **coreutils copy / sleep hard link**: `bin/coreutils` and `bin/sleep` share
  one inode in the output (link count 2) and are byte-identical to the pinned
  `cb987zkxvsv0490753ilrvn5yk7ylgxi-coreutils-9.12/bin/coreutils`
  (sha256 `cffdaa7584377d94bc53b152826287d33123d747407cceb39b329181ce223675`
  on all three). No host binary is laundered. Colliding `kill`/`uptime`
  resolve deterministically to coreutils as the Nix file says.
- **Provenance signatures** (`nix path-info --sigs -r`): 114 of 116 closure
  paths carry `cache.nixos.org-1`; the 2 unsigned are the bespoke tool output
  itself (expected, local build) and `n98x77rw...-libmnl-1.0.5`, which was
  **built locally** at 2026-11-07 15:31:07Z (registration time), not
  substituted. I fetched the cache's narinfo for that path: its `NarHash`
  `1a6v9w92k1w4lm1q3jkfcv3f9v1d7r5q5d0hasm7pcrgyfk67lm6` equals the local
  NAR hash, so the local build is bit-identical to the signed upstream
  artifact. Report accuracy note only: "upstream dependencies realized as
  signed substitutes" is true for 113 of 114 upstream paths; one is a
  reproducible local build. No trust implication.
- **Strict launcher**: `env -i` with `PATH=<tools>/bin` only; the check
  script hard-fails (exit 2) when `PATH != $SPC_LAB_NIX_TOOLS/bin`. The env
  only sets HOME/TMPDIR/USER/LOGNAME/LANG and the three `SPC_LAB_NIX_*`
  store paths; no secret is read or printed, and `summary.json`/stdout carry
  only store paths. `sh` -> bash-interactive `bin/sh` exists; `python` ->
  python3 exists.
- **unsafeDiscardStringContext** is used only for the `provenance` JSON
  (`toFile`). The `env` derivation interpolates the packages with context, and
  the realized output's `references` list includes all 15 package outputs
  plus the RPATH-scanned glibc/acl/attr/gmp. The env/check runners reference
  env + scripts + provenance. No required closure reachability is lost.
- **Trivial derivations**: plain `derivation {}` with `pkgs.bash` as builder,
  coreutils/findutils/grep/sed on PATH; no `__noChroot`, no
  `allowSubstitutes`, no impure flags, no new substituter or key. The
  `flake.nix` delta against the approved tree is purely additive (the
  `labNetwork` import, four packages, one app, one check, one devShell); the
  pre-existing `cleanSourceWith` and `fetchCargoVendor` lines are part of the
  approved `30fed` tree, not this candidate.
- **Scripts output** `ahd5yra043pclpaanw6by2zydr072d2r`: 30 files, identical
  file set to source `scripts/` + `testing/`; lines 2..EOF byte-identical in
  every file; exactly 26 shebangs rewritten (15 `#!/usr/bin/env bash`, 11
  `#!/usr/bin/env python3`) to the pinned bash-interactive/python3; executable
  bits preserved. `w6vrs1fb...` (used by native-r1c) is content-identical
  (`diff -r` clean, same narHash `2I4eZP...`). Rebuilding from the current
  tree yields a third path `za2wqrr...` with the same narHash, so the
  normalization is deterministic. No namespace policy was weakened.
- **Dependency completeness**: the fetch report added `which`; it is never
  invoked by the packaged scripts (only `command -v` in bash and
  `shutil.which` in Python), so omitting it from the 60-tool list is correct.
  No other external command used by `scripts/lab/**`, `testing/lab-isolation`
  or `testing/lab-topology` is outside the list.
- **Fail-closed check script**: unknown arguments exit 2; closure failure
  skips host proofs and exits 1; exit 10/20/21 are classified only when the
  preflight artifact agrees, otherwise `inconsistent_*` -> exit 1; missing
  `nft` negative control must exit non-zero with no probe artifact and no
  `/usr/bin` or `/bin` in the constructed PATH. There is no path to status
  `passed` through an unknown command or an error.

## 4. Receipts and recorded tests

- **Seven-check realization receipt** (`afdb53...`): I re-evaluated the
  current tree offline (`nix flake check --offline --no-build`, then
  `nix-store -q --outputs` + `nix path-info` with no build). All seven
  outputs in the receipt exist with matching derivers. Scope nuance that the
  report states correctly and I confirm from the store: the receipt's `src`
  input `74v24zvq...-source` equals the final tree **minus** the candidate
  report (372/372 files match the manifest), because `src` includes
  `docs/reviews/`. The current full tree (`3b4f66`) evaluates to new drv
  paths for the four src-dependent checks; they are not realized yet and will
  rebuild deterministically after integration. The receipts bind `ad03`
  (code), not `3b4f66` (full), exactly as stated.
- **Offline exit receipt**: exit 0, 00:22:42Z-00:22:44Z, stdout empty,
  stderr sha256 `8417da...` reproduces; the stderr is the full evaluation
  log ending in `running 7 flake checks...` with no build lines (warm store).
- **Unit test in the `test` check**: `nix log` of
  `62il56dl...-spc-workspace-tests-0.1.0.drv` shows
  `supervisor::tests::coreutils_applet_is_preserved_across_symlinks_and_hardlinks ... ok`
  among 30 spc-lab unit tests. So the delta compiles and its unit test passes
  in the sandbox, as the report says.
- **Topology r4 (14/14)**: the test binary (`4e0ce0...`) was built at 15:00
  local from `/tmp/opencode/Nix-net-tools-rust-src-r1`, whose
  `supervisor.rs` is `657449...`, not the final `b0feca...`. I diffed them:
  the **only** difference is the appended `#[cfg(test)] mod tests` block; the
  `coreutils_applet_argument` function body and its call site are
  byte-identical. The report therefore slightly under-claims: the helper's
  logic *was* exercised at runtime by T4's `sleep-symlink` alias path
  (compiled from identical function code) against tools iteration
  `5a74j1...`, whose `bin` layout is identical to the final `7rrzlf...`
  (the only `diff -r` differences are the added `lib/` and
  `share/postgresql` symlinks). That does not rescue the delta, see section 5.
- **Isolation final r1**: summary status `passed`, 60/60 tools ok, all
  realpaths in store, `bash`/`python3` `/proc/self/exe` in store, `sleep`
  recorded as `same-output-hardlink` with equal digests, preflight backend
  `netns` with netem, isolation 28 passed / 0 failed, negative `nft`
  exit 1 with `host_fallback: false`.
- **Native r1c**: `dirty_tree_hash` `30fed`, project binaries
  `aac127.../443de4.../f8000b...` from approved Nix builds, postgres/redis
  from the final tools output, node the recorded external exception, scripts
  `w6vrs1fb` (content-identical to final). I confirm it must **not** be read
  as a run of the `ad03` candidate. Additional bounded observation: the run
  manifest does not record the hash of the `spc-lab` controller binary
  itself (pre-existing manifest schema, not this packet), so which supervisor
  build drove native-r1c is unrecorded; the report's "approved binaries"
  claim is supported for every recorded process and is the only claim made.
- **Cold/warm offline claims** are accurate and consistent with the fetch
  evidence (`53a61a...`): cold `--offline` on an incomplete store fails at the
  stage0 bootstrap source (no fetch URL), online realization is fetch-only
  from `cache.nixos.org` with signatures required, warm `--offline` passes.
  Both docs sections state this correctly.
- **My own light re-run** (allowed scope): `nix run --offline
  path:$PWD#lab-network-check -- --closure-only --artifacts
  /tmp/opencode/fable-nix-review-closure` -> exit 0, 60/60, negative control
  fail-closed, PATH exactly `7rrzlf.../bin`, process exe the pinned bash. It
  rebuilt only the src-dependent `scripts` and `check` runner derivations.
  Evidence files are kept under `/tmp/opencode/fable-nix-review-*`; probe
  fixtures were private and removed.

## 5. The blocker: `crates/spc-lab/src/supervisor.rs` (`b0feca68...`)

`coreutils_applet_argument()` (candidate lines 265-301) returns
`--coreutils-prog=<name>` whenever the canonical target is named `coreutils`
or is hard-linked to a sibling `coreutils`, and `launch_with_pipes()` pushes
it as the first program argument (lines 367-369). `<name>` is the program's
symlink-target basename if that is not `coreutils`, otherwise the program's
own basename.

GNU coreutils only consults `--coreutils-prog=` when `argv[0]`'s basename is
**not** already a known applet. Empirical results against the pinned 9.12
binary (`env -i PATH=<tools>/bin`, no host tools):

| Case | Program as the supervisor would exec it | Injected argument | Result |
|---|---|---|---|
| A | `alias/sleep-alias -> <tools>/bin/sleep` | none (approved behaviour) | `coreutils: unknown program 'sleep-alias'`, exit 1 |
| B | same | `--coreutils-prog=sleep` | exit 0 (the case the helper fixes) |
| C | `<tools>/bin/sleep` (hard link) | none (approved behaviour) | exit 0 |
| D | `<tools>/bin/sleep` | `--coreutils-prog=sleep` (what the helper injects: own name, hard link matches) | `unrecognized option '--coreutils-prog=sleep'`, exit 1 |
| E | `<coreutils>/bin/sleep -> coreutils` | none | exit 0 |
| F | `<coreutils>/bin/sleep` | `--coreutils-prog=sleep` (helper injects: resolved name is `coreutils`) | `unrecognized option`, exit 1 |
| G | `<tools>/bin/cat -> <coreutils>/bin/cat` | `--coreutils-prog=cat` | `unrecognized option`, exit 1 |
| I | hard link named `copied-sleep` | `--coreutils-prog=copied-sleep` (the unit test's asserted value) | `coreutils: unknown program 'copied-sleep'`, exit 1 |

So: the delta converts cases C, E, G (every direct Nix coreutils applet,
including the `sleep` the lab's own tools output provides, and any NixOS host
where `/run/current-system/sw/bin/<applet>` resolves into a coreutils store
path) from working to failing, in exchange for fixing case B. The unit test
builds its fixture from empty files and asserts an argument (case I) that the
real binary rejects, so it validates the string logic, not the behaviour.
This is a regression in a production crate relative to the approved `30fed`
supervisor, not covered by any recorded runtime test:

- T4 only launches the alias (case B) and a direct `sleep` whose `nsenter`
  into `/proc/1/ns/net` fails before exec, so the injected argument never
  reaches coreutils.
- native-r1c launched no coreutils applet and (per the report) not this
  supervisor.
- No current adapter config declares a coreutils applet, so the approved lab
  scenarios are not affected today; that bounds the blast radius but does not
  make an unverified production change approvable under the repo's
  fail-closed convention.

### Required change (either option closes this finding)

**Option 1 - remove the production delta (preferred, minimal).** Restore
`crates/spc-lab/src/supervisor.rs` to the approved `a73892d0...` and make the
T4 fixture a symlink whose basename is the applet (e.g.
`<dir>/sleep -> <tools>/bin/sleep` instead of `sleep-symlink`), which the
approved supervisor already handles (case C/E path) and which still proves
"the exec handshake survives a symlink" and records the canonical executable.
Required proof: re-run only
`topology_namespace::the_exec_handshake_survives_a_symlink_and_reports_a_failed_launch`
inside `nix develop path:$PWD#lab-network` against the final tools output
`7rrzlf...`, with stdout/stderr and the test binary hash recorded.

**Option 2 - keep the helper, fix it, prove it on the real binary.** Inject
only when the program's own basename differs from the resolved applet name
(i.e. only for alias symlinks: `link_name` present, not `coreutils`, and
`!= own_name`; never for a program whose own basename is the applet, and
never for the bare multicall). Then:

1. Replace the empty-file fixture assertion `--coreutils-prog=copied-sleep`
   with assertions that match real behaviour: alias -> `Some(sleep)`; direct
   applet symlink, direct hard link named `sleep`, and bare `coreutils` ->
   `None`.
2. Add one integration test (may be `#[ignore]`-gated like the other netns
   tests) that launches, via `Supervisor::launch`, both `<tools>/bin/sleep`
   directly **and** an alias to it, with args `["0"]`, against the real
   pinned coreutils, and asserts both exit 0. Run it in
   `nix develop path:$PWD#lab-network` and record the output.
3. Re-run T4 as in option 1.

Either way: re-freeze the manifest (new `ad03`/`3b4f66`), and add the
realized `test` check log line for the updated unit test. No other file in
the packet needs to move; re-review will be limited to the one changed
record and the new proof.

## 6. Non-blocking notes for the author (no change required for approval)

1. Report section 4 says upstream dependencies were all signed substitutes;
   `libmnl-1.0.5` is a local build, bit-identical to the cache NAR
   (section 3 above). A one-line correction would make the report exact.
2. Report limitation 1 can be stated more precisely: the helper's function
   body ran at runtime in topology r4 (identical code, alias path only); what
   is unproven is the direct-applet path, which is the one that fails.
3. Because `src` includes `docs/reviews/`, every report edit re-derives the
   `test`/`clippy`/`doc`/`fmt`/`ui-node`/`test-ark`/`lab-network-tools`
   checks and the scripts/check-runner outputs. Expected and deterministic
   (scripts narHash stayed `2I4eZP...` across three src inputs), but
   reviewers should expect a rebuild, not a warm `--offline` pass, on the
   final full tree.
4. The run-manifest schema does not record the controller binary hash; a
   future packet could add it so a "which supervisor ran this" question is
   answerable from evidence alone.
5. The external rbitcoin node stays the recorded non-Nix exception; it is
   out of this packet's scope and was not evaluated here.

## 7. Scope statement

Reviewed: manifest integrity and recipe, like-for-like approved-tree delta,
Nix expression purity and references, realized closure provenance and
signatures, scripts normalization, check-script fail-closed semantics,
seven-check receipts, offline receipt, isolation/topology/native evidence
scope, docs accuracy, and the supervisor delta by diff plus direct
execution of the pinned coreutils binary. Not done: full suite or mining
rerun, candidate edits, commits, review-server interaction, subagents.
Verdict applies to manifest `264e48...` / code `ad03...` / full `3b4f66...`;
any re-freeze supersedes it for the changed record only.
