# T0 traffic contract: primary integration

Date: 2026-11-08
Owner: serial primary code integration owner (approved T0 fix-r1 packet only)
Status: complete; transferred packet frozen after the checks in section 5
Candidate: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-traffic-contract`
(read-only throughout), r1 review manifest
`/tmp/opencode/T0-fix-r1-review-manifest.json`
(sha256 `12c4e23d156914b05961c8ea3489c856a0070d709db264800ea3e913a29a0b78`),
380 rows, base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`, 15 delta rows
vs the approved 370-file LP-final baseline
(`/tmp/opencode/LP-final-runtime-manifest.json`,
sha256 `8bbe097de563600ce8bf0b8cb9693b082a9cff9bb51580d96afd89d2c0c3021f`).
Approval: Fable re-review r1 (latest section of
`docs/reviews/2026-11-08-traffic-contract-fable.md`): **APPROVE - bounded
T0 exactly as frozen by the manifest (full snapshot
`3304dffa84c32d546ee783bd15beec09bbfe5fa6755a71c9c5721bc6241d41a8`)** -
the frozen schema/protocol/command vocabulary, the manifest field, the
projection + gate, and the safe-unavailable state.

**Hash authority note:** the packet's `packet_code_snapshot_sha256`
`86d3de7f10b9e9eedf8c4c34987662bcab9de313c68fdc946b6871d8eb7f27f7` is
ONLY the same recipe over the delta `.rs` files (a build recipe
documented in the manifest); the full approval authority is the
whole-tree snapshot `3304dffa...`, which is what this transfer was
verified against (all 380 rows re-hashed, 0 mismatches, aggregate recipe
reproduced).

## 1. Pre-edit verification (all before any write to primary)

- Manifest recipe reproduced from its own rows: full snapshot
  `3304dffa...` and packet recipe `86d3de7f...` both match; all 380
  records re-hashed in the candidate worktree (hash and mode):
  0 mismatches.
- Primary pre-state for all 15 delta rows: the 5 modified paths
  (`crates/spc-lab/src/{engine,lib,manifest,scenario}.rs`,
  `docs/LAB-RUNTIME.md`) were byte-equal to the frozen 370-file baseline
  (hash and mode) - i.e. no T4a/AP-3a or other later-approved edits live
  in them (T4a touched spc-pool, AP-3a touched spc-agent; spc-lab was
  untouched), so the candidate bytes are exactly the approved delta with
  nothing to rebase; the 10 new paths were all absent.
- Primary index hash before any edit: `git write-tree` =
  `44f21a926d4d065ac6937fb0f81b698c7b2d0627` (nothing staged; unchanged
  throughout - section 6).
- Pre-state re-verified again immediately before each individual write
  (section 2), per the reverify-before-edit rule.

## 2. Transfer (exact bytes, 15 delta rows)

Each row was transferred with, immediately before the write, a repeat of
the pre-state check (baseline-equal for modified rows, absent for new
rows), a source re-hash against the manifest row, and after the write a
re-hash and mode check against the same row. All 15 rows verify
`(path, sha256, mode)`-exact against the frozen manifest (per-file list
in `/tmp/opencode/T0-integration-transfer.log`):

| kind | path | sha256 |
|---|---|---|
| modified | `crates/spc-lab/src/engine.rs` | `bf221dd393e5171577872fbd5800deb0bec9b5ac0267ff4206cd32afb20842fa` |
| modified | `crates/spc-lab/src/lib.rs` | `712ee4d0d36f86d2ddf16042cdc67a0f306e1c2f39fa1ba20cd7a40cec97edcc` |
| modified | `crates/spc-lab/src/manifest.rs` | `e61fe892567d81c6b4cb1aaf9093e55ed45b556634cc6463f58457ece7db05d9` |
| modified | `crates/spc-lab/src/scenario.rs` | `b1fec4af4bdbcb0229f55222e0a26bbef534394eb5075fb9d8e710deebf7379b` |
| new | `crates/spc-lab/src/traffic/commands.rs` | `601761efab149e2733633ec1f5994050ac5d47ffb66fbe85dbbf5c35aea5ba60` |
| new | `crates/spc-lab/src/traffic/mod.rs` | `d05aae2bc04802c36f8dbc5abfa61e068dfd0f0cf3943eb6c2c63a0895cb42ad` |
| new | `crates/spc-lab/src/traffic/projection.rs` | `a9009d594d9933d1ed2985dfe109524534eec6a3a4be50d5a8366292c51b3d92` |
| new | `crates/spc-lab/src/traffic/protocol.rs` | `640041491f4fa0a0f8b186d4e7ebb7c67b2e672820f1c62e8df07a850e5fa360` |
| new | `crates/spc-lab/src/traffic/spec.rs` | `f0788f0e40d14ae991ba93d223c73c1607c3273473593a97d0a27d1e3e422ca5` |
| new | `crates/spc-lab/tests/traffic_contract.rs` | `c70afe57067b8a27fa9bd31e3caedd8b4676d31ec40c9990016b781078ea1c7f` |
| modified | `docs/LAB-RUNTIME.md` | `322ec481d8f968e1cb6a04c20b65639aa5885e9af31bb65393735f74b8dddf5c` |
| new | `docs/changelog/2026-11-08-lab-traffic-contract-t0.md` | `776c2c07da70b99061b8b7c9e2eaba32e2a5b20f878ebb7187240904a0b677ab` |
| new | `docs/decisions/2026-11-08-lab-traffic-contract-t0.md` | `c5a802baf033344c142094fefdfac108dd74e88116b494f48f9cff6afffb4306` |
| new | `docs/decisions/2026-11-08-traffic-ap1a-lineage-r2-approved.md` | `3388326890666ec5d8db3d5a4b181a4e1549bffdb3b72b32cb4306e468de6510` |
| new | `docs/reviews/2026-11-08-traffic-contract-implementation.md` | `1f879ca9c7fdb8baf897fd82010458880db8c44693e2aa04412864ff05ffc9f3` |

Shared-root files: T0 touches no `Cargo.toml`, `Cargo.lock` or
`flake.nix` row (verified against the delta list; the candidate's merge
notes record the same). No pin moved. The AP-1a lineage binding
(contract `f4fc3e04...`, allowlist entry and the field-by-field-verified
17-row expected-checks table, asserted atomic by the
`frozen_expected_checks_table_is_the_approved_r2_contract` test) lands
in this change via `crates/spc-lab/src/traffic/projection.rs` plus the
recorded decision `docs/decisions/2026-11-08-traffic-ap1a-lineage-r2-approved.md`,
exactly as approved - the r1 candidate hash `11bdb100...` stays
recognized-unapproved diagnostics and unknown hashes fail closed.

## 3. SPEC frontmatter (the one non-byte-exact element - flagged)

T0 has **no SPEC body hunks**. The approved changelog fragment
`docs/changelog/2026-11-08-lab-traffic-contract-t0.md` is a 24-line
titled document, which does not match the single-bullet-line format the
`docs/changelog/README.md` requires for the frontmatter `changelog:`
list, so its bytes cannot appear verbatim inside the YAML. Disposition:

- The fragment **file** is transferred byte-exact as one of the 15 delta
  rows (above), preserving the approved bytes in-tree - matching the
  primary's standing precedent (the 2026-11-05/06 fragments exist
  alongside their spliced bullets).
- The frontmatter entry spliced at the head of the `changelog:` list is
  **mechanically derived** from that file: the title line supplies the
  conventional `2026-11-08: ` prefix, and the body's hard-wrapped lines
  are joined with single spaces (pure reflow - every word, its spelling
  and its order are the fragment's; no word added, removed or changed).
  Derived line: 1500 bytes **without** trailing newline, sha256
  `9b3abff5f1837fa142318aa0661bfe5e8c1262433fb08b18201f67ef91ff2e9a`
  (this is the exact byte string inserted into `docs/SPEC.md`); the
  reference copy saved at `/tmp/opencode/T0-integration-spliced-bullet.txt`
  is 1501 bytes - the same bullet plus one trailing newline - sha256
  `ed9325a424075025b347686307bac6470fceba498fe485c124007d0445c05bb2`.

Mechanical verification of the splice
(`/tmp/opencode/T0-integration-splice.log`): the pre-splice SPEC was the
post-AP-3a sha `8f3117f778a45e641e3412f48dfa1a2c4ed93de0bdb798372f6e20f6fd2b9b54`;
the derived line is single-line and contains no embedded double quote or
backslash (YAML double-quoted scalar safety); after the splice,
deleting exactly the inserted line reproduces the pre-splice sha
byte-for-byte (nothing else moved); the list went from 42 to 43 entries;
the T4a and AP-3a entries are retained immediately below the new entry;
`updated:` stays `2026-11-08` (today's date - no bump needed). Final
SPEC sha256:
`3f66994703d024dd764cbfd1ac0b45e25b153aad2296d9e9119ed473b76ae161`.

This derived line is the only content in this transfer whose exact bytes
were not frozen in the approved manifest; the parent may replace or
re-splice it without affecting any code transfer above.

## 4. Non-interference

- Index hash before == after:
  `44f21a926d4d065ac6937fb0f81b698c7b2d0627`.
- `git status` delta vs the pre-transfer snapshot
  (`/tmp/opencode/T0-integration-gitstatus-{before,after}.txt`) is
  exactly 4 added `??` lines: the changelog fragment, the two decision
  records and the implementation report. The 5 modified paths live under
  the already-untracked `crates/spc-lab/` / already-modified entries, and
  the new traffic module and test fold into the untracked
  `?? crates/spc-lab/` directory line - nothing removed or modified
  elsewhere.
- No node, service, mining or UI was started; no cache cleaned; no
  reset/clean/stash/commit/stage performed; the candidate worktree was
  not written.

## 5. Checks (merged primary tree, `-j2`, existing `target/`)

| Command | Exit | Result | Log (sha256) |
|---|---|---|---|
| `cargo fmt -p spc-lab -- --check` | 0 | clean | `19eaf43821a7660ec323a87c8457bf74823beb296c39f5e01aa8a683aa50f061` |
| `cargo test -j2 -p spc-lab` | 0 | 32 test binaries, **154 passed, 0 failed, 16 ignored** (netns/node-gated, unchanged); lib 36, traffic_contract 42, engine_restart 3, run_lock 1, supervisor_adoption 7, scenario_v1 8, preflight 3 | `2cedddd3e9bffb702f0086a4cb79c3536b765fa5d9b167410ebd79e8cce44416` |
| `cargo clippy -j2 -p spc-lab --all-targets -- -D warnings` | 0 | zero warnings | `77ea35cf72a655e06330c6c0bb7f02079edc456030b747c1b5e819bc0a709577` |

These match the candidate's fix-r1 receipts exactly (traffic_contract
42, lib 36, full package all ok, fmt/clippy clean). Logs:
`/tmp/opencode/T0-integration-logs/`. The cargo registry-cache permission
warnings in the log are the known pre-existing environmental noise under
concurrent agents (also recorded in the candidate's report); no cache
was touched.

Nix gate scope: the AP-3a receipt's `test-ark` Nix gate ran on the
pre-T0/AP-1a merged tree and covers spc-crypto/spc-pool/spc-agent; T0
touches only spc-lab, which `test-ark` does not build. Per the batching
instruction's already-running branch, T0's gate is the scoped spc-lab
cargo suite above; no second Nix run was started.

## 6. Scope and limits (what this transfer does NOT establish)

- **Safe-unavailable state**: every traffic scenario is rejected at
  `RunEngine::prepare` (and on the adopt path before the owner lock) with
  `traffic_capability_unrecorded` until a T1 recording path exists - even
  if the node config declares the capability name. `prepare` intentionally
  rejects traffic until T1; T1 must change the
  `check_traffic_capability(&scenario, None)` call in `prepare`.
- No driver, signing, or UI: T0 freezes the schema/protocol/command
  vocabulary, the projection and the gate only. Nothing executes, signs,
  mines or probes; no T1 recording path, node capability, node profile,
  funded behavior or Ark readiness.
- The actual `getmempoolinfo` selected-consumer failure (corepc-types
  0.13.0 `v30::GetMempoolInfo` shape mismatch for the approved
  binaries' modelling consumer) is recorded as a passed/failed row in the
  frozen 17-row table - not fixed here, and not a universal Core
  compatibility verdict.
- No commit was made; the transferred work sits in the working tree
  alongside the pre-existing primary user work.

## 7. Freeze

The transferred packet (15 delta rows + the spliced SPEC frontmatter
entry) is frozen as of the completion of the section 5 checks. Receipts:
`/tmp/opencode/T0-primary-integration-manifest.json` and the
`/tmp/opencode/T0-integration-*` files (transfer log, splice log and
derived bullet, check logs, git-status snapshots).
