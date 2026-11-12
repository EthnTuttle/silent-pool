# EXP-OP operator runtime wiring - primary integration (2026-11-08)

Owner: serial primary Rust/Cargo integration writer (sole writer for this
packet; build-priority reservation held by this session,
`ses_e458119eeffe4Jl3MZjJ4J4HJR`, for the Hub -> history-v2 -> access ->
operator-runtime -> ark-registration serial window).

Approval: `docs/reviews/2026-11-08-operator-runtime-wiring-fable.md`,
**APPROVE bounded to full snapshot `fd198c44...` / code snapshot
`a9b6e4fc...`** (Round 3): the host `--operator-ui` runtime wiring -
off-by-default host-only flag, one leased loopback `front.<id>.operator`
listener per front inside the existing pool API process, rendered pool
TOML validated against the real pool parser, URL published only after an
HTTP 200 route-readiness proof recorded as evidence, typed
`operator_ui` interface in the hub inventory and JS guard - plus the D1-D3
merge conditions (documentation/report only). Explicitly NOT approved and
NOT done here: any OP module/static/SQL change, any new authentication on
the OP listener, optional H1/H2/H3 hardening, and any fresh manual host
run (the parent launches the owned manual run after the merged gate).

External manifest: `/tmp/opencode/EXP-OP-runtime-primary-integration-manifest.json`
(final sha256 below).

## 1. Pre-write verification (read-only, at transfer time)

- Candidate manifest `/tmp/opencode/EXP-OP-runtime-freeze-manifest.json`
  (604 rows) reproduced: full `fd198c44...` and code `a9b6e4fc...`
  recipes MATCH; the 5 new-path candidate rows clean
  (`EXP-OP-runtime-prewrite-reverify.log`).
- Primary pre-state: the 5 new paths absent; roots `Cargo.toml`
  `179312b2...`, `Cargo.lock` `55938583...` (history2 spc-lab-txd edge),
  `docs/SPEC.md` `5aca112c...` (49 entries), `flake.nix` `c9c477d1...`
  (vendor hash `ODJ+iUGF...`).
- 3-way bases re-derived against the post-ACCESS primary
  (`EXP-OP-3way-precompute-classify-r2.log`): the correct base is the
  **Hub-approved-as-landed** state (recovery bytes), not the pre-recovery
  Hub worktree snapshot. Classification: engine.rs 3-WAY; lib.rs and
  http.rs SKIP (candidate == Hub-as-landed; primary keeps the ACCESS
  content); runtime.rs, resource.rs, common/mod.rs, main.rs,
  inventory.rs, lab.js, lab_helpers.test.cjs BYTE-APPLY (primary ==
  OP baseline); traffic_contract.rs 3-WAY with base = pre-history2
  reconstructed by reversing the history2 amendment (verified equal to
  the OP baseline `c70afe57...`).
- Index `44f21a926d4d065ac6937fb0f81b698c7b2d0627` (== HEAD tree,
  nothing staged) before transfer.

## 2. Transfer (exact approved delta + merge-owner adaptation)

- **5 new files byte-exact** (candidate re-hash before copy, post-copy
  hash + mode verify; worktree 0664 modes normalized to 0644):
  `bins/spc-lab/tests/cli_operator_ui.rs` `32f56e73...`,
  `crates/spc-lab/tests/operator_ui_inventory.rs` `ba05d260...`,
  `docs/changelog/2026-11-08-operator-runtime-wiring.md` `3d4ede82...`,
  `docs/decisions/2026-11-08-operator-runtime-wiring.md` (pre-D1
  `1ef06c0d...`, see D1 below),
  `docs/reviews/2026-11-08-operator-runtime-wiring.md` `bcda0f30...`.
- **7 byte-apply files** (primary == OP baseline verified pre-copy, so
  candidate bytes apply as bytes): `runtime.rs` `24aa016a...`,
  `resource.rs` `7a4ebf39...`, `tests/common/mod.rs` `e84671d7...`,
  `bins/spc-lab/src/main.rs` `a3848044...`, `inventory.rs`
  `565ac045...`, `static/lab.js` `d7366d04...`,
  `tests/lab_helpers.test.cjs` `3aea2803...`. All 12 byte rows verified
  against the manifest after apply
  (`EXP-OP-runtime-transfer-bytes.log`).
- **2 semantic merges** (applied as reviewed `a/...`->`b/...` patches;
  results byte-equal to the staged 3-way outputs):
  - `engine.rs` -> `2ccb4d1c...`: base Hub-as-landed `06c2c179...`,
    ours post-ACCESS `64ecabc9...`, theirs candidate `fb2add58...`;
    clean merge (exit 0, 0 conflict markers), delta = the candidate's
    7-line change only (`operator_ui` field + `plan_resources` arg).
  - `tests/traffic_contract.rs` -> `76e1e664...`: base pre-history2
    `c70afe57...`, ours post-history2, theirs candidate; clean merge,
    delta = one `operator_ui: false,` constructor line.
- **lib.rs / http.rs SKIP** (verified candidate == Hub-as-landed, so the
  candidate carries no ACCESS-era content; primary keeps `5d6a7770...` /
  `7ffa90ed...` untouched by this packet).
- **4 EngineOptions constructor reconciliations** (merge-owner
  adaptation for ACCESS-era test files the candidate predates):
  `access_scope.rs` x2, `access_projection.rs` x1, `access_hub_routes.rs`
  x1, each gaining `operator_ui: false` with a comment noting the
  host-only opt-in flag.
- **Dependency edges (exactly three writes, verified against the
  candidate's approved roots)**: root `Cargo.toml`
  `179312b2...` -> `8530ce08...` adds `spc-pool = { path = "bins/spc-pool" }`
  to workspace dependencies (placement identical to the candidate; the
  `crates/spc-lab-txd` member is preserved - T1);
  `crates/spc-lab/Cargo.toml` `87f5ec27...` -> `1286e111...` adds
  `spc-pool.workspace = true` under `[dev-dependencies]` while keeping
  history2's `spc-lab-txd` dev-dep and its comment block;
  `Cargo.lock` `55938583...` -> `6ca42938...` inserts exactly one line,
  ` "spc-pool",`, in the **spc-lab** package dependencies block -
  verified identical to the candidate's approved lock edge modulo the
  preserved spc-lab-txd lines (candidate-vs-primary lock diff shows
  only the spc-lab-txd edge + package entry that primary correctly
  keeps). No pin moves.
- **D1 (fable merge condition, doc-only)**: the decision record item 9
  and LAB-RUNTIME section 4 carry the corrected sentence "The float fact
  reads `not_configured` until the AP-4 status source exists; the
  liability and payout facts are the pool store's actual values." The
  decision doc therefore deviates from the frozen candidate bytes by
  EXACTLY the 5-line D1 hunk (2 pre-D1 lines removed, 3 corrected lines
  added; `1ef06c0d...` -> `1d9d2bab...`, diff vs candidate verified to
  contain only that hunk). LAB-RUNTIME gains only the candidate's
  +28-line `--operator-ui` section with the D1-corrected sentence
  (`b3028acf...` -> `1934cbfe...`); the Hub section-5 hub paragraph, the
  recorded-run paragraph, the R1 wallet paragraph and the history2
  version-2 traffic paragraph are all preserved (the candidate's
  LAB-RUNTIME was built on the pre-recovery Hub worktree and would have
  reverted them).
- **D2 (fable merge condition, doc-only)**: `docs/LAB-ROLES.md` pool-front
  row gains exactly the prescribed clause ("With `spc-lab runtime
  --operator-ui` (host runs only) each front also serves the approved
  read-only operator page on a separate leased loopback listener, linked
  in `/lab` as the typed `operator_ui` interface after a start-time
  HTTP 200 route proof."); the Hub R5 sentence ("A missing observation
  is shown as unknown") is preserved (post `d5dd4c79...`).
- **SPEC frontmatter splice (parent-corrected workflow)**: the candidate
  itself provides only the changelog fragment (no direct SPEC edit) -
  which is what the Fable review's "docs/SPEC.md is untouched (correct,
  this is lab tooling)" line approves. Per the parent's explicit
  correction, the merge-owner convention (AGENTS.md working
  conventions, MERGE-QUEUE fragment procedure, parent OP-runtime
  handoff) REQUIRED consuming the fragment into the SPEC frontmatter at
  merge time; an earlier reading treating the review line as an
  exemption was wrong and is corrected in the prep note
  (`EXP-OP-runtime-integration-prep.md`, amendment appended). The
  splice ran as ONE doc-only edit AFTER the probe/validation chain and
  BEFORE the final snapshot: 49 -> 50 bullet entries
  (`5aca112c...` -> `21f47313...`), fragment bullet byte-exact (430
  bytes, no additions), reverse-splice reproduces the pre-state, all 49
  prior entries preserved (marker set: ACCESS, history2, hub, OP page,
  node-profile, T1a, T0, AP-3a, T4), SPEC body untouched. Script:
  `/tmp/opencode/EXP-OP-runtime-spec-splice.py` (`5534eb8c...`). The
  fragment stays archived at
  `docs/changelog/2026-11-08-operator-runtime-wiring.md` per the
  existing approved-packet receipt policy.
- No OP module, static asset or SQL change beyond the approved lab.js
  guard; no new authentication on the OP listener; no H1/H2/H3
  hardening. Status 210 -> 213 lines; index `44f21a92...` unchanged.

## 3. Vendor-hash consistency (new lock edge)

The spc-pool lock edge changes the `cargo-deps-vendor` fixed-output
derivation. Procedure reused unchanged from history2 (probe script
`/tmp/opencode/EXP-OP-runtime-vendor-probe.sh`, `768c3ec4...`):
throwaway copy of the git-selected source (648 files, rsync `--from0`),
flake vendor hash blanked to a valid same-length SRI dummy, `nix build
path:...#checks.x86_64-linux.test`. The FOD ran and reported
`hash mismatch in fixed-output derivation '...cargo-deps-vendor-staging.drv'`,
**got: `sha256-PvxybBOJ1f4X+nkf6DQCdHM6Le9Dr67v3nhoTdrLYBM=`**
(first-attempt success; no probe-script failures this round). Single
same-length flake edit on primary: `ODJ+iUGF...` -> `PvxybBOJ...`
(44-char body, one line); reverse-proof reproduces the recorded
pre-state `c9c477d1...`; post `b4bf0503...`; line count 161 unchanged.
The post-probe source edits (SPEC markdown, the flake hash line itself)
are not inputs to the vendor FOD (it filters to Cargo.toml/Cargo.lock),
so the reported hash binds the final state; the combined merged Nix
gate at queue end confirms.

**Frozen source snapshot** `/tmp/opencode/EXP-OP-runtime-nix-src`
(648 files, 10564867 bytes / 10.1 MiB - same git-selection as the
probe, frozen AFTER the splice and flake edit so the vendor hash binds
exactly this final source state), row manifest
`/tmp/opencode/EXP-OP-runtime-nix-src-manifest.json`
(`1da5955cf6f57aec1280a6be22e0f03a690ade88ba8c768561d7a7494521ec1f`).
No Nix gate run in this packet: the one combined merged gate runs after
the final queued root changes per the parent's batching instruction.

## 4. Validation (actual runs, pinned devshell, flock + >=50 GiB policy)

Runner `/tmp/opencode/EXP-OP-runtime-validation.sh` (sha256
`edc224f6f9834a5c090b001e9d2d7030c7eacdb0b442e64d6adf907528b5717c`),
under `nix develop` (cargo 1.93.0), session-bound ticket guard before and
inside the flock, disk 70-72 GiB free, existing primary `./target`, run
AFTER the probe (sequential lock handoff). **The validation ran on the
pre-splice source; the SPEC splice is doc-only (markdown, not a compile
input), so every result below applies unchanged to the final state.**

- **Lib unit tests** (`cargo test -p spc-lab --lib`): exit 0, **52
  passed, 0 failed** (42 post-ACCESS + 10 new runtime units, including
  the rendered-pool-TOML tests that exercise `spc_pool::config::Config`
  behind the new dev-dep edge - the real pool parser, never string
  matching).
- **operator_ui_inventory** (`-p spc-lab --test operator_ui_inventory`):
  exit 0, **5 passed, 0 failed**.
- **cli_operator_ui** (`-p spc-lab-cli --test cli_operator_ui`): exit 0,
  **3 passed, 0 failed** (flag off by default, `--topology` conflict,
  api-role fail-closed).
- **traffic_contract** (`-p spc-lab --test traffic_contract`): exit 0,
  **42 passed, 0 failed** (history2 amendment + OP merge intact).
- **Access suites + R4 hub routes** (`--test access_scope
  --test access_projection --test access_hub_routes`): exit 0, **27
  passed, 0 failed** (18 + 6 + 3; proves the 4 EngineOptions constructor
  reconciliations compile and pass).
- **Node JS guard** (`node --test bins/spc-lab/tests/*.test.cjs`,
  node v24.11.1): exit 0, **26 pass, 0 fail** (includes the new typed
  `operator_ui` link-rule test).
- **fmt** (`cargo fmt -p spc-lab -- --check` + `-p spc-lab-cli`): exit 0
  both. **clippy** (`cargo clippy -p spc-lab -p spc-lab-cli
  --all-targets -- -D warnings`): exit 0.
- **Merged spc-lab CLI binary** (`cargo build -p spc-lab-cli --bin
  spc-lab`): exit 0, `target/debug/spc-lab` sha256
  `0949b949b2344ef87b2f43ceb3f52ef63f6596c65e1ed211d03cd4626a9dd841`.
- **Production rpc spc-pool binary** (`cargo build -p spc-pool
  --features rpc`): exit 0, `target/debug/spc-pool` sha256
  `41f0dfbc8605928d2f9e34ab1aae5348ae20d88edc4c8e66a95b930c1742abe1`
  (feature set `rpc` - the production RPC profile; the `ark` feature is
  AP-2 territory and is not claimed here).
- **Aggregate**: SUCCESS, all 10 steps exit 0; **zero leaked
  primary-target fixtures** (post-run check clean; the runner kills only
  this target's orphans if any appear).

## 5. D3 report annotations (documented here, NOT edited into the frozen candidate report `bcda0f30...`)

1. The candidate report's full-snapshot value was stale (`5a84...`);
   the actual frozen full snapshot is `fd198c44...` (recipes re-verified
   at transfer, section 1).
2. The report's "Cargo.lock delta ... in the spc-lab-cli dependency
   list" is wrong: the edge is in the **spc-lab** package block
   (this receipt, section 2).
3. The report's "`expAccess` remains unapproved" was stale: EXP-ACCESS
   is approved (code snapshot `8085616e...`) and was merged into primary
   before this packet (receipt
   `docs/reviews/2026-11-08-expanded-access-primary-integration.md`).
4. The report's "compiled-asset proof ... occurs 6 times" understates
   the evidence: the full static assets are embedded byte-exact (all six
   assets verified byte-exact to the Hub-approved set in the ACCESS
   R4.2 gate; the CLI embeds them via `include_str!`).

## 6. Final state

- Index `44f21a926d4d065ac6937fb0f81b698c7b2d0627` (== HEAD tree;
  nothing staged, no commits made) - unchanged across the whole packet.
- Status 213 lines at receipt time.
- Roots final: `Cargo.toml` `8530ce08...`, `Cargo.lock` `6ca42938...`
  (spc-lab-txd + spc-pool edges), `docs/SPEC.md` `21f47313...`
  (50 entries), `flake.nix` `b4bf0503...` (vendor hash
  `sha256-PvxybBOJ...`).
- Queue after this receipt: AP-2 ark registration (Round 3 APPROVE;
  migration slot 0006 free at precheck, re-verified at transfer;
  post-OP classification: 9 added + 13 byte-apply + 11 three-way
  including Cargo.lock, which must additionally preserve this packet's
  spc-pool edge - `AP2-classify-post-OP.log`), then the one combined
  merged Nix gate. J1/history-3 remains unapproved and unmerged.

## 7. Not claimed here

- No OP module/static/SQL change, no new auth on the OP listener, no
  H1/H2/H3 hardening, no fresh manual host OP run (parent launches the
  owned run after the merged gate), no Ark server API change, no funded
  Ark or mining acceptance, no combined Nix gate (deferred per batching
  instruction), no candidate-binary reuse as merged proof.
