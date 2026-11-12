# Self-hosted Nix Ark infrastructure - primary integration (2026-11-08)

Owner: serial primary Rust/Cargo integration writer (sole writer for this
packet). Authorization: the user's "ungate everything" grant - workflow
approval holds REMOVED by the user; the parent cleared the priority ticket.
No priority file was active; single-build/disk/nonblocking rules remained in
force. **No Fable review was performed for this packet**; an async source
audit may run in parallel and is not a ceremonial start gate - actionable
findings would require fixes before affected runtime use. This receipt makes
no approval claim beyond the parent's verified tested-source results.

Packet authority: `/tmp/opencode/self-hosted-ark-packet/` - `manifest.json`
(22 rows), `manifest-full.json` (671 rows, full aggregate
`6529a8f22e808469889a422a2ab51e2950cc308ef520d2e9636245374d989460`),
`package-bindings.json`, `launch-bindings.nix`, `changes.patch`
(`97a370c2c80ff21854b40cf36e09260157bbff0c8845ff88d2f5d0dae1a2e866`).
Candidate worktree: `lab-self-hosted-ark` (671/671 rows clean at reverify).
Baseline: AP2-nix-src 658 files, manifest
`9fc3c7e14997de3cc22bf7e534d08398f2967377e99eccf854c50df8731a9987`.
Report owner (transferred as a packet row, byte-exact):
`docs/reviews/2026-11-08-self-hosted-ark-infrastructure.md` (`87683d7d...`).

External manifest:
`/tmp/opencode/self-hosted-ark-primary-integration-manifest.json`.

## 1. Pre-write verification (read-only, at transfer time)

- Full aggregate reproduced with the declared recipe
  (`canonical{base_manifest_sha256, files}`): MATCH `6529a8f2...`; patch
  SHA256: MATCH `97a370c2...`; candidate worktree rows 671/671 clean.
- Classification of the 22 rows against the AP2 baseline: **all 22
  BYTE-APPLY** (primary == baseline on every row; tjv3 touched none of
  them). 9 modified + 13 new (new paths absent).
- Primary-vs-candidate divergences fully explained: 22 unapplied ark rows +
  9 retained tjv3 rows + 3 primary-side doc evolutions RETAINED (AGENTS.md
  self-host bullet, MERGE-QUEUE window-complete status, REGTEST-LAB-STATUS
  inspection-run status). No unexpected divergence.
- Roots at expected pre-state: `Cargo.toml` `8530ce08...`, `Cargo.lock`
  `6ca42938...` (both unchanged by this packet - no dependency delta),
  `docs/SPEC.md` `f27c647b...` (55 entries, post-tjv3), `flake.nix`
  `b4bf0503...`; index `44f21a926d4d065ac6937fb0f81b698c7b2d0627`.
- Packet patch dry-run (`git apply --check`) clean before any write.

## 2. Transfer (bounded hunks only - no whole-file copies of old lab source)

- Applied the packet's authority `changes.patch` via `git apply` after the
  guards above; then set modes per manifest (0644 modified / 0664 new).
  **All 22 rows post-hash == manifest** (`ark-transfer.log`):
  - 9 modified: `bins/spc-lab/src/main.rs` `3305e13e...`,
    `bins/spc-pool/src/main.rs` `4e0fdf73...`, `engine.rs` `7173aa68...`,
    `inventory.rs` `440efaaf...`, `lib.rs` `c22e964c...`,
    `resource.rs` `f339253f...`, `runtime.rs` `73ac5b35...`,
    `scenario.rs` `1c59e479...`, `flake.nix` `31c65b92...`
    (b4bf0503 -> 31c65b92, the only root change).
  - 13 new: `crates/spc-lab/scenarios/lab-ark-infrastructure.json`
    `f801f1fb...`, `crates/spc-lab/src/ark_service.rs` `6310bc97...`,
    `crates/spc-lab/tests/ark_infrastructure.rs` `b2486118...`,
    `docs/changelog/2026-11-08-self-hosted-lab-ark.md` `4a387b8a...`,
    `docs/decisions/2026-11-08-self-hosted-lab-ark.md` `9eee2999...`,
    `docs/reviews/2026-11-08-self-hosted-ark-infrastructure.md`
    `87683d7d...`, `nix/lab/ark-node.nix` `35939d1d...`,
    `nix/lab/ark-probe.nix` `2d35b0b0...`, `nix/lab/ark-runtime.nix`
    `ec62d21a...`, `nix/lab/ark.nix` `e0139e38...`,
    `testing/lab-ark/README.md` `1b90c63c...`,
    `testing/lab-ark/infrastructure.py` `8fa9de02...`,
    `testing/lab-ark/release.py` `e6c0761e...`.
- Retained untouched (verified byte-identical pre/post): the 15 tjv3 rows,
  AGENTS.md, docs/MERGE-QUEUE.md, docs/REGTEST-LAB-STATUS.md.
- **SPEC splice (merge-owner edit, from the transferred fragment)**:
  frontmatter bullet 55 -> **56** (`f27c647b...` ->
  `bfb28139f811e8fb74095509a6819b8447d9c3ea98c2f66d3d2f73321d0dce35`) -
  bullet byte-exact from the fragment's yaml block, inserted at the top of
  the changelog list, reverse-splice proof, all 55 prior bullets preserved
  (marker set verified, including the tjv3 (e)/(f) amendments, AP-2, R2.1,
  OP, hub, ACCESS, T1a, T0, AP-3a, T4); PLUS the fragment's section-9 body
  append after the "v2 is a lab profile" paragraph (byte-exact, 781 bytes,
  reverse-splice proof, `**Rollout.**`/`**Monitoring**` neighbors intact).
  The body is lab-deployment-only: no payout-logic change (verified - no
  payout-policy or distribution language in the appended text). SPEC now
  has 56 changelog entries.
- Cargo.toml/Cargo.lock byte-unchanged; no pins bumped; no server code
  patch; no external hosted-service fallback anywhere in the delta.
- Status 234 -> 239 lines; index `44f21a92...` unchanged throughout. No
  commits, no subagents, no cache cleanup, no GC, no new target dirs.

## 3. Boundary and invariant verification (source-level)

- **Role privacy (§11.3)**: `ark_service.rs` and the infrastructure
  scenario contain no `sp_id`/`B_scan`/`B_spend`/`P_i`/`user_identity`
  identity fields; the scenario has zero user-wallet and zero miner actors.
- **No pool Ark-role SPc secret load**: `bins/spc-pool/src/main.rs` loads
  the order key only under `Role::Payout`; the ark-only process makes zero
  attempts on the SPc signing key path (parent's strace trace on the
  tested source confirmed zero opens; the conditional is the source
  binding). `provision.json` written 0o600 with `"funded_e2e": false`.
- **RuntimeOptions**: new `ark: Option<ArkBinaries>` field; the two
  in-tree test constructors carry literal `ark: None` (count verified: 2).
- **Engine guards**: `local_ark_hook_missing` (built-in runtime required
  for local Ark), `launch_ark` skips local backends (hook-provisioned
  before fronts), `ark_agent_bridge_unavailable` refusal for miners in
  this profile (registered-agent bridge is future work, not bypassed).
- **Node binding**: `--rpc-compat-profile bark-v31` added only when a
  local Ark backend exists; `nix/lab/ark-node.nix` binds the FINAL node
  source (metadata full manifest `f1592c93...`, NAR
  `sha256-t6LQYTA738Uzstg5AsakWCJS4yGanDBTn4FSpUmhiIU=`) - not the r0/r1
  failed NARs; tested release binary `bba47522...`. Captaind built from
  stock Bark `c419159` with no source patch (upstream-supported env
  inputs only).
- **Cross-refs**: all repo-relative references in the new docs resolve;
  the one `crates/rbitcoin-rpc/...` citation is a node-source-tree
  reference (exists in the frozen
  `/tmp/opencode/rbitcoin-lab-profile-facts-src` the report names).
- Prior-packet content preserved: tjv3 history-3 (SUPPORTED_HISTORY_
  VERSION = 3 spot-checked), ACCESS inventory operator-only surface, OP
  flags, history3 projections - all in the retained untouched rows.

## 4. Validation - PASSED on the merged tree (2026-11-08 22:05-22:10)

All under one nonblocking `flock -n -o` grant each (runner scripts
`/tmp/opencode/ark-validation-inner.sh` +
`/tmp/opencode/ark-nix-eval-inner.sh`), existing primary `./target`,
disk >= 50 GiB inside, no priority file:

1. **Focused lab tests** (the Nix checkPhase suite):
   `cargo test -p spc-lab --test ark_infrastructure --test lifecycle_stop
   --test two_runs_isolation --test supervisor_adoption
   --test operator_ui_inventory -j2`: **exit 0 - 20 passed / 0 failed**
   (4 + 3 + 1 + 7 + 5; exactly the manifest's `lab_tools_tests_passed`).
2. **Core3 conformance**: `cargo test -p spc-lab
   --test traffic_history_conformance -j2`: **exit 0 - 7 passed /
   0 failed** (tjv3's history-3 contract still green with the ark delta
   present).
3. `cargo fmt --check`: **exit 0**.
4. `cargo clippy -p spc-lab -p spc-pool --all-targets -j2 --
   -D warnings`: **exit 0** (affected pool/lab compile clean, including
   the two `ark: None` test literals and the pool main ark-provision
   block).
5. **Nix eval hash-binding (output reuse, no build)**: the git-tree flake
   eval fails BY DESIGN for the untracked `nix/lab/*.nix` files - the
   primary index must stay unchanged (nothing staged), and Nix git flakes
   see only tracked files. This is recorded honestly, not worked around by
   staging. Take 2 replicated the packet owner's own methodology (a clean
   source snapshot, `launch-bindings.nix` style, not a git flake):
   `/tmp/opencode/ark-primary-eval-src`, 11.1 MiB, exclusions per AGENTS.md
   (.git, target, result*, __pycache__, .cache, node_modules,
   spc_ark_client/test, spc_ark_e2e/test), size recorded before ingestion.
   Pure evaluation, zero builds. Results:
   - `captaind` ->
     `/nix/store/4mnskprw4qigq8i8xqf4pps7iak1bnly-captaind-0.7.1-c419159`
   - `rbitcoin-node-ark-lab` ->
     `/nix/store/lbpv9zs0mdp36f912k63a31pg9qrzi54-rbitcoin-node-ark-lab-provenance`
   - `rbitcoin-node-ark-lab.passthru.sourceBuild` (tested release) ->
     `/nix/store/03pw2rf9wrxxnd9rl0l8ln0mnw9gcpx2-rbitcoin-node-ark-lab-0.8.99-profile-facts`
   All three **rebound unchanged** to the parent-verified candidate
   outputs - the primary source binds the same captaind (stock c419159)
   and node (FINAL f159/t6LQ source, `bba47522...` release) closures.
   No cold rebuild of any suite; captaind/node/tools outputs untouched.
   **Primary-binary honesty**: the `spc-lab-ark-tools` / launcher
   outPaths were deliberately NOT compared to the candidate closure -
   the primary tree differs (tjv3 rows are in its src), so its tools
   derivation is a different (unbuilt) store path. The existing Nix tools
   output `kkd5y9jy...` proves the candidate API; the current primary
   binary is NOT claimed identical to the candidate closure (it was not
   built here).

Aggregate: all Rust steps exit 0 (**27 passed / 0 failed** across test
steps); Nix hash-binding verified with full output reuse. No leaked
primary-target fixtures. Full logs: `/tmp/opencode/ark-validation-run.log`
(`fb33ce15...`), per-step in `/tmp/opencode/ark-integration-logs/`
(`nix-eval.log` = the honest git-flake failure record;
`nix-eval-snapshot.log` `cfdcf403...` = the successful snapshot eval).

## 5. J2 boundary honesty (unchanged from the packet - CRITICAL)

`actual_engine_launch_profile_supported = false` in the final manifest:
the stock engine's ownership token contains `:`, rejected by the canonical
r4 normalizer. The full unfunded infra test records the null comparison +
the exact error (`traffic_launch_profile_unsupported: missing or invalid
ownership token`) + the observed raw ENV hash - it does NOT pass the
engine J2 gate. The full-Nix-launcher verification
(`full_nix_launcher_verified = true`, closure
`/nix/store/sslr7nzw0gp3qzb68is0p44aiqpn7a74-spc-lab-self-hosted-ark`)
is an unfunded infrastructure proof only - it is NOT funded traffic proof
and NOT a J2 engine proof. The T1 owner is fixing the traffic-only
token/helper flow separately; no probe grammar bypass and no observed
token rewrites exist in this delta. No fake floats, no funded board, no
lifecycle receipts are claimed by this integration.

## 6. Untouched (binding)

- The sealed LIVE user lab run: controller pid 199214,
  `/tmp/opencode/live-lab-20261108T231027Z/bin/*` - never stopped,
  reaped or mutated; it uses sealed binaries, not the primary target.
- The parent's fresh UNFUNDED inspection run:
  `/tmp/opencode/ark-inspection-20261109T025651Z/run` (controller
  1441192, API 33579, operator 41225/37983) - never touched; the
  operator claim-file token was not published.
- All packaged Nix outputs (9 outputs + 53 executable hashes), other
  owners' processes/targets/locks, the shared build lock between grants.

## 7. Final state

- Index `44f21a926d4d065ac6937fb0f81b698c7b2d0627` (== HEAD tree;
  nothing staged, no commits) - unchanged across the packet.
- Status 239 lines at receipt time.
- Roots: `Cargo.toml` `8530ce08...`, `Cargo.lock` `6ca42938...`
  (unchanged); `flake.nix` `31c65b92...`; `docs/SPEC.md` `bfb28139...`
  (56 entries).
- Changed rows with git modes: 23 rows (22 packet + SPEC) in
  `/tmp/opencode/ark-changed-rows.json` and the external manifest.
- Logs: `ark-transfer.log`, `ark-spec-splice.log`, `ark-source-verify.log`,
  `ark-crossrefs.log`, `ark-validation-run.log`, `ark-nix-eval-snapshot.log`,
  `ark-integration-logs/*`.
- Final HTML deliverable for the parent:
  `docs/lab-build-report.html` - actual component changes, APIs, evidence
  and open gates across the integrated packets, citing these receipts.
- Method note (per the current developer + handoff requirement): the
  verified 22-row transfer above was applied with `git apply` of the
  packet's authority patch (recorded here honestly, not redone); all
  future file edits use `functions.patch`.

## 8. Not claimed here

- No funded end-to-end (`funded_e2e = false`): native funding, the
  registered-agent/miner bridge, board/receive/refresh/offboard funded
  acceptance, and the four-user acceptance profile remain open (T1/T2/
  agent-bridge gates).
- No engine J2 gate (section 5); no complete mining-to-Ark verification.
- No Fable/independent review approval of this packet (user workflow
  waiver; async audit may flag findings - actionable ones get fixed
  before affected runtime use).
- No 903-test broad gate repeat, no full Nix suite rebuild, no GC, no
  commits, no staging (hence the honest git-flake eval note in section 4).
