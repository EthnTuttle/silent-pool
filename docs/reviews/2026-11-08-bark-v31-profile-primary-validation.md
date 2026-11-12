# AP-1b lab bark-v31 profile - primary validation (2026-11-08)

Date: 2026-11-08
Author: TFF lab-integration subagent (validation only; no code changes)
Status: **VALIDATION PASSED.** Every required gate of decision
`docs/decisions/2026-11-08-lab-bark-v31-profile.md` item 6 passed on the
integrated binary built from `/tmp/opencode/AP1b-lab-integration`. The
bark-v31 profile is now a **validated opt-in artifact** - still not a
replacement for the default L2 node, and not full Ark end-to-end ready.
External receipt:
`/tmp/opencode/AP1b-lab-integration-validation.json`; all raw receipts
under `/tmp/opencode/AP1b-lab-integration-validation/`.

## 1. Scope and build-lock discipline

- Validation only, after the primary six-check Nix gate completed
  (receipt `/tmp/opencode/primary-four-packet-nix-gate.json`, exit 0).
  No preparation step was repeated; the 82 primary records were not
  re-transferred; no frozen tree, binary or manifest was written.
- Both heavy phases (gates, then probes) ran under nonblocking
  `flock /tmp/opencode/silent-pool-heavy-build.lock`, one at a time, per
  the AGENTS.md build disk budget. Disk was checked **after** lock
  acquisition: **90 GiB free** at gates start (>= 50 GiB rule); recorded
  estimate ~4-8 GiB peak growth would leave >= 84 GiB (>= 20 GiB
  reserve). Actual measured growth: the target directory is **1.3G**
  after everything; 90G free at probe start, 89G free at close (exact
  `df` receipts in section 9).
- One integrated-node target directory, first and only:
  `/tmp/opencode/AP1b-lab-integration-target`. The approved candidate
  (`/tmp/opencode/AP1b-bark-profile/target`) and L2
  (`/tmp/opencode/rbitcoin-lab-retarget-profile/target`) targets were
  never written. No whole-workspace build: only `-p rbitcoin-node -p
  rbitcoin-rpc`. Existing dependency cache reused; no cache cleanup.
- No code changes, no commits, no `git add`, no subagents, no
  `UPSTREAM_DEPS.md`/pin/default-L2/default-runtime-binary changes. Two
  pre-existing unrelated `rbitcoin-node` processes (ports 39001, 48332)
  were left alone throughout.

## 2. Required gates (actual commands, first attempt, no reruns)

All commands ran inside `nix-shell` from the integrated tree with
`CARGO_TARGET_DIR=/tmp/opencode/AP1b-lab-integration-target`,
`CARGO_BUILD_JOBS=2`, `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0`
(see `run-gates.sh`, sha256 `5befca82...`, and `gates-console.log`).
Toolchain pinned by the tree's flake.lock: **rustc 1.95.0
(59807616e 2026-04-14)**, **cargo 1.95.0 (f2d3ce0bd 2026-03-21)**.

| Gate | Command | Exit | Result |
|---|---|---|---|
| fmt | `cargo fmt --all -- --check` | 0 | clean |
| clippy | `cargo clippy --locked -p rbitcoin-node -p rbitcoin-rpc --all-targets` | 0 | clean |
| node suite | `cargo test --locked -p rbitcoin-node --test suite -- --test-threads=1` | 0 | **118 passed; 0 failed** (3.33s) |
| rpc suite | `cargo test --locked -p rbitcoin-rpc --test suite -- --test-threads=1` | 0 | **214 passed; 0 failed** (148.97s) |
| build | `cargo build --locked -p rbitcoin-node` | 0 | binary produced |

- Test threads serialized (`--test-threads=1`) and `-j2` per the known
  unrelated load flakes; the rpc suite passed **214/214 on the first
  attempt** (the r0 flaky-rerun history did not recur). **No failed
  attempts and no quiet reruns**: each row above is the one and only
  invocation; per-step `.cmd`/`.exit`/`.log` receipts are in the
  validation directory.
- Node suite 118/118 vs the r0 reference 117/117; the run includes the
  patch's profile tests, among them
  `cli::rpc_compat_profile_env_is_not_a_knob` and
  `cli::rpc_compat_profile_is_bark_v31_regtest_cli_only`.
- The only clippy-log warnings are cargo's shared-cache write-permission
  notices (`failed to write cache ... Permission denied` for the reused
  read-only registry cache) - not lint findings; clippy itself is clean.
- Gates ran 14:33:32Z-14:40:14Z UTC under the held lock.

## 3. Integrated binary provenance (identity is provenance, not inequality)

Per the parent correction applied **before** probe execution, the
integrated binary is identified by explicit path, build receipt and
source index - never by requiring a hash difference from the candidate.

- Path: `/tmp/opencode/AP1b-lab-integration-target/debug/rbitcoin-node`;
  regular file, not a symlink (inode 48398870, device 66306); resolved
  path is in no candidate/L2 tree; not the same inode as the candidate
  or L2 binary files.
- Build receipt: `build.cmd`/`build.exit` (exit 0)/`binary-sha.txt`/
  `toolchain.txt` bound to source tree `/tmp/opencode/AP1b-lab-integration`.
- Hard precondition re-verified before any node launch: all **2102**
  source-index files match; canonical index aggregate
  `da21f56dd6ab581e0d8353fe7431cc44efa9eb29bffb998459cbd7ac29105a9e`
  reproduced.
- Actual sha256:
  `b75dad84bdd6663673a29c07352d10c4f78d2dcd14e885f62ad3209d214443e4` -
  **byte-identical to the approved candidate binary.** This is the
  legitimate case the correction anticipated: the integrated tree is
  byte-identical to the candidate outside `target/`, and the pinned
  toolchain reproduces identical bytes. Recorded as provenance, not
  gated; the default-vs-profile counterfactuals below separately prove
  the patched code is what runs. Distinct from the default L2 binary
  `eb257af7...` (no match, no alias).

## 4. Frozen approved probe - devcheck gate FIRST

The frozen probe's own golden-vector gate ran first and unmodified
(`devcheck/run.sh` in the frozen `lab-ark-rpc-probe` worktree): exit 0,
**12/12 vectors** (receipt `devcheck-receipt.json`). No probe evidence
was written before it passed.

## 5. Frozen approved probe - default run (profile OFF)

Frozen probe contract
`f4fc3e043f420870675095ab24144ba14e0afba6cb9ad69d818d99b1bccb2f62`,
external mode (`probe.py probe --rpc-url ... --cookie-file ...`) against
an owned integrated node launched with explicit CLI argv, **no**
`--rpc-compat-profile` flag: fresh private regtest datadir, private
one-run 0600 cookie (contents never recorded), `--no-listen
--no-seeds --no-discover`, loopback RPC only. Node ready in 0.509s.

- `getnetworkinfo.version` **190000** (default unchanged); subversion
  the real `/rbitcoin:0.8.99/`; no `rbitcoin_compat` object; no profile
  startup banner.
- Read-only dialect counterfactual (Fable r1 method: two unrelated
  1-in-1-out txs, fabricated prevouts): rejected `-1 mempool relay
  disabled (still in IBD or tip not ready)` - no topology gate, exactly
  the default behavior; `getrawmempool` empty.
- Verdict: `incompatible [fail-version-gate, funded-unproven]`;
  `full_acceptance` false. Failing rows are the two honest version-gate
  rows plus the known `getmempoolinfo` shape failure (section 8).
- Probe exit 0; owned node reaped (SIGTERM, clean exit) and scratch
  destroyed in a `finally` block on every path; cleanup verified in the
  binding.

## 6. Frozen approved probe - profile-on run (the counterfactual that proves the patch)

Same frozen probe, same isolation, launched with explicit CLI
`--rpc-compat-profile bark-v31`. **Clean environment: every
`RBITCOIN_*` variable stripped from the parent environment before
launch** (none were set; nothing injected) - in particular
`RBITCOIN_RPC_PACKAGE_DIALECT` was **UNSET**, so the profile ALONE had
to select the Core package dialect. The harness refuses to run if the
removed `RBITCOIN_RPC_COMPAT_PROFILE` env knob is set.

- `getnetworkinfo.version` **310000** as the compatibility claim;
  subversion still the real `/rbitcoin:0.8.99/`; `protocolversion`
  70016 unchanged; `rbitcoin_compat` object present
  (`claimed_version 310000`, `package_dialect
  core-is-child-with-parents`, `profile bark-v31`); startup banner
  `rpc_compat_profile=bark-v31 claimed_version=310000
  software=rbitcoin software_version=0.8.99`.
- Read-only dialect counterfactual with the dialect env UNSET:
  rejected **`-25 package topology disallowed. not child-with-parents
  or parents depend on each other.`** and `getrawmempool` stays empty -
  the profile alone enforces Core `IsChildWithParents` on the
  integrated binary.
- Verdict: `surface_compatible [funded-unproven]`; `full_acceptance`
  false; 11 unproven checks (funded-method rows; read-only mode can
  never prove them).
- Probe exit 0; node reaped and scratch destroyed in `finally`; cleanup
  verified.

## 7. Fail-closed small checks on the integrated binary

- `dialect-off-conflict`: `RBITCOIN_RPC_PACKAGE_DIALECT=0` (explicitly
  off) plus `--rpc-compat-profile bark-v31` -> **exit 2**, message
  `RBITCOIN_RPC_PACKAGE_DIALECT is explicitly off` present, **datadir
  never created** (refused before effects).
- `non-regtest`: `--network signet` plus the profile -> **exit 1**,
  `requires network=regtest` present, **datadir never created**.

Both PASS (receipts `small-checks/small-check-*.json`).

## 8. Known limitations (unchanged, deliberately not laundered)

- The `getmempoolinfo` check row **failed in both runs**
  (`maxdatacarriersize: expected JSON integer, got null` against the
  selected v30 modelling consumer) - the known, deliberately unpatched
  traffic limitation carried from the reviews; this validation does not
  fix or hide it.
- Funded broadcast/mempool behavior remains `not_proven`
  (`funded-unproven` in both verdicts; `full_acceptance` false; 11
  unproven checks). No funded transactions, no `generate*`, no Bark
  service, no live PoW acceptance were exercised.
- No Core v31 parity is claimed; the claim covers the tested read-only
  RPC surface only. Funded/witness/real-Bark gates and native
  WITNESS/T4b acceptance remain separate later work.
- The removed env knob `RBITCOIN_RPC_COMPAT_PROFILE` stays unapproved
  and inert (CLI/conf-only profile; the node suite's
  `rpc_compat_profile_env_is_not_a_knob` test passes).
- Not full Ark end-to-end ready.

## 9. Exact disk measurements

- After lock, at gates start: **90 GiB free** (gates-console.log).
- Target directory actual size after all gates+build: **1.3G**
  (estimate had been ~4-8 GiB; measured, not assumed).
- At probe start (`df-at-probes.txt`): 938G total, 801G used, **90G
  avail** (90%).
- After probes (`df-after-probes.txt`): 938G total, 801G used, **90G
  avail**.
- At receipt close: 938G total, 802G used, **89G avail** (91%).

Every measurement stays far above the 50 GiB build floor and the 20 GiB
post-growth reserve.

## 10. Post-validation boundary verification (all unchanged)

- Integrated source tree: all **2102** files still match the source
  index; aggregate `da21f56d...` reproduced after validation.
- Approved L2 tree: all 18 manifest records match; L2 binary still
  `eb257af7...`.
- Candidate tree `/tmp/opencode/AP1b-bark-profile`: all 25 records
  match; aggregate `f8a51111...` intact.
- Frozen packet worktree `lab-node-compat-profile`: all 82 records
  match; aggregate `9375653c...` intact.
- Primary `testing/rbitcoin-bark-v31/**` + two implementation reports:
  all 82 records match byte-exact.
- Frozen probe worktree (contract `f4fc3e04...`): all 74 freeze records
  match - the probe ran unmodified.

## 11. Deliverables and pending items

- External receipt `/tmp/opencode/AP1b-lab-integration-validation.json`
  (assembled by `build-validation-json.py`, sha256
  `42660657...`, extended with the primary-documents section after the
  report and decision note were written; final file hash in the parent
  receipt), plus all raw
  gate/probe/small-check/binary-provenance receipts under
  `/tmp/opencode/AP1b-lab-integration-validation/`. Temporary
  validation harnesses (outside the frozen packet):
  `run-gates.sh` `5befca82...`, `run-probes.sh` `bde02c3e...` (parent
  correction applied before probe execution), `ap1b-integrated-external-run.py`
  `d87c804f...` (N5 cleanup contained: owned proc reaped in `finally`,
  scratch destroyed on every path).
- The decision record's item 6 pre-use gate is satisfied; the
  append-only gate-completion note has been added to
  `docs/decisions/2026-11-08-lab-bark-v31-profile.md`.
- The changelog fragment `docs/changelog/2026-11-08-lab-bark-v31-profile.md`
  **remains pending splice** into the `SPEC.md` frontmatter: per
  `docs/MERGE-QUEUE.md`, AP-1b's SPEC fragment must not race the T1a
  primary writer; the parent/merge session coordinates the slot. No
  `SPEC.md` edit was made by this step.

## 12. Non-claims

This validation makes the bark-v31 profile a **validated opt-in
artifact only**. The default lab node remains the approved L2 binary
`eb257af7...` with the standard config; no pin, `UPSTREAM_DEPS.md`,
v0.9-adoption or runtime-config change; no Core v31 parity; no funded
or Ark end-to-end acceptance. Nothing here weakens the privacy
constraints or the fail-closed rules; no identity data crosses the ark
role. Fable's bounded provider approval remains what it was; nothing
here claims Fable approved a runtime replacement.
