# LP-hub read-only live lab hub - primary integration (2026-11-08)

Owner: serial primary Rust/Cargo integration writer (sole writer for this
packet; the build-priority reservation was transferred to this session,
`ses_e458119eeffe4Jl3MZjJ4J4HJR`, by the parent for the Hub -> history-v2
-> access serial window; other agents isolated in their worktrees).

Approval: `docs/reviews/2026-11-08-live-lab-hub-fable.md`, **APPROVE
bounded to code snapshot `712fa8cb...` / full snapshot `3896b132...`**:
the lab hub pages (`/lab`, `/lab.js`, `/lab.css`), the closed
`spc.lab.inventory.v1` projection (`inventory.rs`), the three hub routes on
the existing loopback HTTP server, `lab_hub` Rust tests,
`lab_helpers.test.cjs` Node tests, `LAB-ROLES.md`, the merged hunks in
main/app/index/http/http_sse/engine/lib/README/LAB-RUNTIME/ARCHITECTURE,
and merge conditions R1 (wording), R2 (decision + changelog + SPEC
splice). Explicitly NOT approved (and NOT done here): operator/agent UI
work, Ark end-to-end claims, a new auth platform, a fresh native live
experiment, and the optional R3/R4/R6/R7 code nits (deferred to the next
hub packet).

External manifest: `/tmp/opencode/LP-hub-primary-integration-manifest.json`
(final sha256 below).

## 1. Pre-write verification (read-only, before any transfer)

- Candidate manifest `/tmp/opencode/LP-hub-verified-review-manifest.json`
  reproduced: full snapshot `3896b132...`, code snapshot `712fa8cb...`;
  all 378 records re-hashed and mode-checked against the candidate
  worktree (read-only): 0 mismatches; the 7 recovery rows carry their
  after-hashes in the manifest.
- Primary pre-state: the 6 modified files byte-equal to the shared
  baseline; the 7 new paths and the report absent; the 4 semantic-merge
  files (README, engine.rs, lib.rs, LAB-RUNTIME) diverged from baseline as
  expected (T0/AP/OP content present).
- Primary pre-state roots: `Cargo.toml` `179312b2...`, `Cargo.lock`
  `7ec525f8...`, `docs/SPEC.md` `81de2be8...` (46 entries), `flake.nix`
  `07c0a887...`, index `44f21a926d4d065ac6937fb0f81b698c7b2d0627` (==
  HEAD tree, nothing staged), status 195 lines.

## 2. Transfer (exact approved delta only; pre-state re-verified per row)

- **13 files byte-exact** (source re-hash + post-copy hash+mode verify,
  all 100644): 6 modified-as-bytes (main.rs `7d235082...`, app.js
  `1f0b6a86...`, index.html `0e4335cb...`, http.rs `0ba5102b...`,
  http_sse.rs `93d739a4...`, ARCHITECTURE.md `e7dc2ecb...`), 7 new at
  recovery bytes (lab.css `ccb82914...`, lab.html `b452cd04...`, lab.js
  `3c6f7b54...`, lab_helpers.test.cjs `99fe62f2...`, inventory.rs
  `00c57593...`, lab_hub.rs `962601e0...`, LAB-ROLES.md `61471eea...`)
  plus the implementation report `1c07603b...`.
- **4 semantic merges via edit-tool hunks** (never whole-file copies),
  preserving all primary-side T0/AP/OP content verbatim (verified by
  marker checks and residual diffs): README.md (2 hunks, post
  `098f0165...`), engine.rs (2 hunks, post `06c2c179...`; T0 traffic gate
  code untouched), lib.rs (1 hunk, post `7bc62c3d...`; traffic module +
  re-exports preserved), LAB-RUNTIME.md (2 hunks, post `ab1ea107...`;
  T0 traffic paragraph + decision links preserved).
- **R1 applied byte-exact**: the blanket wallet sentence replaced by the
  exact contiguous sentence "The standard scenario launches no
  `spc-agent` wallet role and wires no wallet desktop/mobile UI; backend
  wallet foundation on primary is described in `LAB-ROLES.md` section 3
  and is not launched by this run."
- **R5 doc nit applied with the approval's leave**: LAB-ROLES.md single
  sentence "No observation is shown as unknown." -> "A missing observation
  is shown as unknown." (diff vs recovery bytes is exactly 2 lines; post
  `53dba50d...`).
- **R2 authored**: decision record
  `docs/decisions/2026-11-08-lab-live-hub.md` + 324-byte one-line
  changelog fragment `docs/changelog/2026-11-08-lab-live-hub.md`; SPEC
  splice `81de2be8...` -> `81412842...` (46 -> 47 entries; independent
  reverse-splice reproduces the pre-state; all prior bullets preserved).
- No Cargo/flake carry: `Cargo.toml`, `Cargo.lock`, `flake.nix` unchanged
  (the 17-file delta contains no such paths); status 195 -> 200; index
  `44f21a92...` unchanged.
- Full transfer bindings, hunk details and reverse proofs:
  `/tmp/opencode/LP-hub-transfer-state.json`
  (updated with validation results, sha256 below); verification logs
  `LP-hub-verify-candidates.log`, `LP-hub-transfer-copy.log`,
  `LP-hub-transfer-splice.log`, `LP-hub-transfer-post-verify2.log`.

## 3. Validation (actual runs, pinned devshell, flock + >=50 GiB policy)

Runner `/tmp/opencode/LP-hub-validation.sh` (final version sha256
`6a3c94692386d8474834cd4a653288e09fbb2b58958abcc841a2fec4efd2934a`,
executed under `nix develop`: cargo 1.93.0, node v24.21.0;
session-bound ticket guard before and inside the flock; disk 75-86 GiB
free across invocations; existing primary `./target`; no new targets).
Aggregate exit logic verified by isolated stub test (4/4) before use.

- **Node suite** (`node --test bins/spc-lab/tests/*.test.cjs`): exit 0,
  **24 tests, 24 pass, 0 fail** (log `node-tests.log`).
- **Rust suites** (`cargo test -j2 -p spc-lab -p spc-lab-cli`): exit 0,
  **160 passed, 0 failed, 16 ignored** over 34 result lines (log
  `rust-tests.log`).
- **fmt** (`cargo fmt -p spc-lab -p spc-lab-cli -- --check`): exit 0.
- **clippy** (`cargo clippy -j2 --all-targets -- -D warnings`): exit 0.
- **Merged CLI build** (`cargo build -j2 -p spc-lab-cli`): exit 0;
  merged binary `target/debug/spc-lab` sha256
  `dec073c2e09ed0b444d658a48a591b1c9df7260e2a300c23b7f59cc2331405fd`.
  Per the approval's provenance note this is the merged tree's own
  identity; the candidate's sealed binary `a577aea7...` (preserved at
  `/tmp/opencode/LP-hub-verify-target/debug/spc-lab`) is NOT the merged
  binary and was not used as proof.
- **Embedded assets**: all 6 static assets (lab.html, lab.js, lab.css,
  index.html, app.js, style.css) embedded exactly once each, byte-equal
  to the merged source (log `embedded-assets.log`).
- **Read-only recorded serve smoke** on the stopped
  `/tmp/opencode/LP-hub-run-1` (run_id `lab-runtime-v2-two-pools`, state
  stopped): merged binary, no adopt, server spawned with fd 9 closed (no
  lock inheritance).
  - Original run: 10 passed / 2 failed - the two failures are a recorded
    **probe artifact, not a product defect**: the script asserted the
    gallery port 18481 was "free", but 18481 carries the parent's
    EXISTING gallery listener (python3, pid 1799953), which must remain
    untouched, not absent. All ten actual behavior checks passed even in
    that run, including run-dir immutability.
  - Corrected recheck (same binary `dec073c2...`, stronger assertion:
    the 18481 listener identity must be byte-identical before and after):
    **11 passed, 0 failed** (log `serve-smoke-r2.log`). Verified:
    read-only serve up (loopback 38271); `GET /lab` 200 text/html,
    `GET /lab.js` 200 text/javascript, `GET /lab.css` 200 text/css;
    inventory DTO `schema_version` = `spc.lab.inventory.v1`, closed
    shape, `view` = `recorded_read_only`, no identity fields/paths
    leaked; root page carries the recorded banner and links `/lab`;
    typed StopRun POST (new command id) answered
    `rejected_precondition` ("attached run is read-only; use the live run
    or attach for adoption to control it") with the journal
    byte-unchanged; full run-dir tree hash identical before/after
    (`9fbc9f9a...`); gallery listener unchanged (no bind, release or
    restart by this smoke); no adopt, no live mining, no gallery
    restart.
- No broad suites re-run: earlier packet gates stand on their frozen
  snapshots; the only primary change is this packet's delta, covered by
  the suites above.

## 4. Final state and concurrent-work record

- Index `44f21a926d4d065ac6937fb0f81b698c7b2d0627` (== HEAD tree;
  nothing staged, no commits made) - unchanged across the whole packet.
- Status 201 lines at receipt time: 200 post-transfer + 1 concurrent
  foreign review doc
  (`docs/reviews/2026-11-08-operator-runtime-wiring-fable.md`, added by
  the parent with the OP-runtime packet approval; not absorbed, not
  reverted). Status captures:
  `LP-hub-integration-gitstatus-{before,after}.txt`,
  `LP-hub-receipt-gitstatus.txt`.
- Shared roots final: `Cargo.toml` `179312b2...` (unchanged),
  `Cargo.lock` `7ec525f8...` (unchanged), `docs/SPEC.md` `81412842...`
  (47 entries), `flake.nix` `07c0a887...` (unchanged).
- Queue after this receipt (all under this session's reserved serial
  window): history-v2 integration, then access integration, then
  operator-runtime-wiring integration (all Fable-approved, receipts
  pending). AP2 R2.1 and J1/history-3 remain unapproved and unmerged.

## 5. Not claimed here

- No operator/agent UI work, no Ark end-to-end claim, no fresh native
  live experiment, no new auth surface, no browser-suite re-run (the
  candidate's 84/84 sealed browser proof stands on its own binary).
- The optional R3/R4/R6/R7 code nits remain deferred to the next hub
  packet by the approval's exact-binary scope.

External manifest: `/tmp/opencode/LP-hub-primary-integration-manifest.json`.
