# Traffic history v2 (T1a core journal mirror) - primary integration (2026-11-08)

Owner: serial primary Rust/Cargo integration writer (sole writer for this
packet; build-priority reservation held by this session,
`ses_e458119eeffe4Jl3MZjJ4J4HJR`, for the Hub -> history-v2 -> access ->
operator-runtime -> ark-registration serial window).

Approval: `docs/reviews/2026-11-08-traffic-history-v2-fable.md`, **APPROVE
bounded to code snapshot `4c1bce8c...` / full snapshot `f4ec0662...`**: the
version-2 private history journal contract (the full record mirror of the
Fable-approved T1a core header) in `crates/spc-lab/src/traffic/`
(mod/spec/protocol/projection), proven against the real writer by the new
conformance suite, the amended `traffic_contract.rs`, the spc-lab dev-dep
edge for `spc-lab-txd`, the LAB-RUNTIME paragraph, decision record,
changelog fragment and implementation report. Explicitly NOT approved (and
NOT done here): any actor/mempool readiness work, J1/history-3
(`lab-traffic-probe-profile`, `lab-traffic-journal-v3`), or the
traffic-runtime plan v5 (coordination record only).

External manifest: `/tmp/opencode/T0-history-v2-primary-integration-manifest.json`
(final sha256 below).

## 1. Pre-write verification (read-only, before any transfer)

- Candidate manifest `/tmp/opencode/T0-history-v2-r1-manifest-full.json`
  reproduced: full snapshot `f4ec0662...` and code snapshot `4c1bce8c...`
  recipes re-computed MATCH; the 4 new-path candidate rows and the 6
  amendment candidate rows re-hashed byte-equal in the read-only worktree
  (`/tmp/opencode/T0-history-v2-prewrite-reverify.log`).
- Primary pre-state: the 4 new paths absent; roots
  `Cargo.lock` `7ec525f8...`, `docs/SPEC.md` `81412842...` (47 entries),
  `Cargo.toml` `179312b2...`, `flake.nix` `07c0a887...`;
  `docs/LAB-RUNTIME.md` `ab1ea107...` with the paragraph anchor unique;
  engine.rs `06c2c179...` / lib.rs `7bc62c3d...` (Hub content present,
  untouched by this packet).
- Index `44f21a926d4d065ac6937fb0f81b698c7b2d0627` (== HEAD tree, nothing
  staged) before transfer.

## 2. Transfer (exact approved delta only)

- **4 new files copied byte-exact** (source re-hash + post-copy hash
  verify, all 100644): `crates/spc-lab/tests/traffic_history_conformance.rs`
  `151ca57c...`, `docs/changelog/2026-11-08-traffic-history-v2.md`
  `dbac237a...`, `docs/decisions/2026-11-08-traffic-history-v2.md`
  `fb8161f3...`, `docs/reviews/2026-11-08-traffic-history-v2-implementation.md`
  `8e60fab9...`.
- **6 code files via amendment patch** (generated patch of the candidate's
  amendment hunks, never whole-file copies): traffic `mod.rs` `cb8199a8...`,
  `spec.rs` `03832bc5...`, `protocol.rs` `e5a39c74...`, `projection.rs`
  `207a9540...`, `tests/traffic_contract.rs` `f412337d...`,
  `crates/spc-lab/Cargo.toml` `87f5ec27...`. All 10 rows verified
  byte-exact to the manifest after apply
  (`/tmp/opencode/T0-history-v2-transfer-post-verify.log`).
- **LAB-RUNTIME paragraph hunk** via edit tool (single hunk, Hub content
  preserved: section-5 hub paragraph, R1 wallet sentence, T0 traffic gate
  doc all verified present; post `b3028acf...`).
- **Cargo.lock: exactly one line added** - ` "spc-lab-txd",` in the
  spc-lab dependencies block (the dev-dep edge). Pre `7ec525f8...` ->
  post `55938583...`; reverse diff proves the only change is that one
  line. No pin moves.
- **SPEC.md changelog splice**: 47 -> 48 bullet entries, pre
  `81412842...` -> post `9932149d...`; independent reverse-splice
  reproduces the pre-state; all prior bullets preserved.
- No actor/mempool readiness content transferred; no J1/history-3 file
  touched; status 201 -> 205 lines; index `44f21a92...` unchanged.

## 3. Vendor-hash consistency (lock edge)

The new lock edge changes the `cargo-deps-vendor` fixed-output derivation,
so `flake.nix`'s vendor hash had to move. Procedure (read-only probe, then
single edit; PRIMARY probed via throwaway copy only):

- Probe copy: git-selected source set (634 files, NUL list + rsync
  `--from0`; excludes `.git`, targets, runtime datadirs by construction),
  flake vendor hash blanked to a syntactically valid same-length SRI dummy.
- Probe run 3 (`/tmp/opencode/T0-history-v2-integration-logs/vendor-hash-probe.log`):
  nix reached the FOD and reported
  `hash mismatch in fixed-output derivation '...cargo-deps-vendor-staging.drv'`,
  **got: `sha256-ODJ+iUGFIuolQVbe60p4EqL+S46ADnIDwolbGIONUdw=`**.
  (Runs 1-2 were probe-script bugs, not product issues: run 1 copied 0
  files - rsync `--files-from` without `--from0`; run 2 blanked the hash
  to an invalid SRI (`44 zeros`, no `=` padding) so nix rejected it at
  parse time and never ran the FOD. Both documented in
  `vendor-hash-probe-run.log` / `-run2.log` /
  `vendor-hash-probe-r2-invalid-sri.log`; probe script final sha256
  `e4307034ca6b83659cef86223fa4227a8c9a4af938bd7243f80dd842a4a49b38`.)
- **Single same-length flake.nix edit on primary**:
  `sha256-2aQzyvfi...` -> `sha256-ODJ+iUGF...` (44-char body, one line);
  reverse-proof (replace back) reproduces the recorded pre-state
  `07c0a887...` exactly; post `c9c477d1...`; line count unchanged (161).
- **Frozen source snapshot** `/tmp/opencode/T0-history-v2-nix-src`
  (634 files, 10223578 bytes / 9.7 MiB - same git-selection as the probe,
  so the vendor hash binds to exactly this source state), row manifest
  `/tmp/opencode/T0-history-v2-nix-src-manifest.json`
  (`ebb18289a09b175ee1be00fcc59ba80961d2064b3427b2b8c2212d8ccad56cdf`).
  No Nix gate run in this packet: per the parent's batching instruction
  the one combined merged Nix gate runs after the final queued root
  changes, on frozen per-packet bindings.

## 4. Validation (actual runs, pinned devshell, flock + >=50 GiB policy)

Runner `/tmp/opencode/T0-history-v2-validation.sh` (final sha256
`179fb5faa06a3968209e65a151e32faeaf8127194895b4a8729733b8bb93f21b`),
executed under `nix develop` (cargo 1.93.0): session-bound ticket guard
before and inside the flock; disk 74 GiB free; existing primary
`./target`; no new targets; no GC.

- **Traffic contract suite** (`cargo test -j2 -p spc-lab --test
  traffic_contract`): exit 0, **42 passed, 0 failed**.
- **History conformance suite** (`cargo test -j2 -p spc-lab --test
  traffic_history_conformance`): exit 0, **6 passed, 0 failed** (proves
  the v2 mirror contract against the real T1a writer).
- **fmt** (`cargo fmt -p spc-lab -- --check`): exit 0.
- **clippy** (`cargo clippy -j2 -p spc-lab --all-targets -- -D warnings`):
  exit 0.
- **Aggregate exception (runner bug, documented)**: the first aggregate
  verdict printed FAILURE because the runner called the shared summary
  helper without the packet's step list, so it checked the Hub's 7 step
  names and found the history-v2 exit files "missing". All four per-step
  exits above were 0 in that same run (no test was re-run). The helper
  was parameterized (steps as `$2`, default = the Hub's seven; new sha256
  `93491db4...`; stub tests 4/4 on both paths) and the corrected
  aggregate re-ran green: **SUCCESS, all 4 steps exit 0**
  (`aggregate-r2.log`). Same class of documented exception as the Hub
  serve-smoke probe artifact: runner-tooling defect, not a product
  defect; per-step actual results stand.

## 5. Final state

- Index `44f21a926d4d065ac6937fb0f81b698c7b2d0627` (== HEAD tree; nothing
  staged, no commits made) - unchanged across the whole packet.
- Status 205 lines at receipt time (201 pre-transfer + 4 new files of this
  packet's delta; the concurrent foreign review docs are already among
  the prior lines and were not touched).
- Roots final: `Cargo.toml` `179312b2...` (unchanged), `Cargo.lock`
  `55938583...` (one spc-lab-txd edge), `docs/SPEC.md` `9932149d...`
  (48 entries), `flake.nix` `c9c477d1...` (vendor hash
  `sha256-ODJ+iUGF...`).
- Queue after this receipt (under this session's reserved serial window):
  EXP-ACCESS integration (prep verified; staged 3-way merges re-verified
  non-stale against this state - `/tmp/opencode/EXP-ACCESS-3way/
  staleness-reverify-2026-11-08.log`), then operator-runtime wiring, then
  AP-2 ark registration. AP-2 R2.1 is approved Round 3 and queued;
  J1/history-3 remains unapproved and unmerged.

## 6. Not claimed here

- No actor/mempool readiness, no traffic-runtime v5 plan implementation,
  no J1/history-3 content, no fresh mining, no combined Nix gate (deferred
  per batching instruction), no candidate-binary reuse as merged proof.
