# T1 traffic runtime + J1 activation: primary integration

Date: 2026-11-09
Owner: serial primary Rust/Cargo integration writer (sole writer for this
packet; other agents isolated in their own worktrees)
Status: complete; primary selected code frozen after the checks in section 5

Candidate authority: frozen manifest
`/tmp/opencode/traffic-runtime-j1-final-manifest-full.json`
(file SHA-256
`79c0c00ae9052a93a0ff3f03b04a4e5ae418b6d811ae251408ad4832f79bc503`,
706 records, aggregate snapshot
`e53118dd3869db3722b4a28bf0028103604dcb7d162320c07fcf9607579dfaaa`,
freeze status "J1 validated; four-case actual unfunded T1 profile pass;
final report-only update"). The candidate worktree
`/home/ethan/.local/share/opencode/worktree/ad8886/lab-traffic-runtime`
was contaminated by a concurrent session during the integration window;
the frozen manifest (plus the manifest-verified `.theirs` bytes in
`/tmp/opencode/t1-3way/`) is the sole packet authority, and no live
worktree bytes were used as a transfer source after contamination was
detected (section 6).

Delta classification authority: `/tmp/opencode/t1-final-delta.json`
(file SHA-256
`bd718a59b2c5a137c04de3ee30823f7b0aaa539b531347f1b1149d7b231a9949`),
42 added + 24 modified paths with a per-path baseline hash dict.

External manifest:
`/tmp/opencode/traffic-runtime-j1-primary-integration-manifest.json`.

This document is the sole unapproved evidence record of the transfer. It
is not a behavior change.

## 1. Pre-write verification (all before any write to primary)

- Manifest file re-hashed; 706 records; every packet row's sha256
  re-verified against frozen candidate bytes (the five rows needing merge
  recovered as `.theirs` files in `/tmp/opencode/t1-3way/`, each
  hash-equal to its manifest row).
- Primary pre-state: all 42 added paths absent; the 24 modified paths
  classified via the delta `baseline` dict (19 byte-exact candidates,
  5 requiring verified merge or proof of no-edit: `main.rs`, `engine.rs`,
  `runtime.rs`, `docs/SPEC.md`, `flake.nix`).
- Index hash before any edit: `git write-tree` =
  `44f21a926d4d065ac6937fb0f81b698c7b2d0627`, equal to `HEAD^{tree}`
  (nothing staged). Nothing was staged, reset, cleaned, stashed or
  committed at any point.
- 8 primary rows that differ from the J1 manifest but are NOT in the
  delta (`AGENTS.md`, `bins/spc-pool/src/main.rs`,
  `crates/spc-lab/src/inventory.rs`, `crates/spc-lab/src/lib.rs`,
  `crates/spc-lab/src/resource.rs`, `crates/spc-lab/src/scenario.rs`,
  `docs/MERGE-QUEUE.md`, `docs/REGTEST-LAB-STATUS.md`) were identified
  as parent-side state postdating the J1 snapshot (AP4-era
  ark_service/self-hosted-Ark work plus parent doc updates) and were left
  untouched. `crates/spc-lab/src/lib.rs`'s `pub mod ark_service;` is
  required by the merged `runtime.rs`; the pre-existing
  `ark_service.rs`/self-hosted-Ark primary work is load-bearing, not
  conflict.

## 2. Transfer (functions tools only, narrow hunks)

- **42 new files** written byte-exact with the write tool and
  hash-verified against the manifest (full table in the external
  manifest). Three JSON fixtures
  (`crates/spc-lab/tests/fixtures/j1/launch-profile-goldens.json`,
  `.../launch-profile-rpc-policy-goldens.json`,
  `.../traffic_selected_consumer.json`) required a sentinel-prefix
  workaround for the write tool's JSON-object rejection; final files
  verified sha256-exact with no sentinel bytes remaining.
- **19 modified rows applied byte-exact** via per-hunk edits, each ending
  hash-equal to its manifest row: `traffic/projection.rs` (27 hunks),
  `access.rs`, `command.rs`, `adapters.rs`, `manifest.rs`, `config.rs`,
  `http.rs`, `evidence.rs`, `namespace.rs`, `supervisor.rs`, the traffic
  module files, the `spc-lab-txd` files and both `Cargo.toml`s. This
  count includes `tests/traffic_contract.rs` (see below: its 3-way merge
  resolved to byte-equality with the candidate row).
- **3-way merges** (`git merge-file` computed in `/tmp/opencode/t1-3way/`,
  then the diff(ours, merged) applied as functions.edit hunks):
  - bases for `bins/spc-lab/src/main.rs`, `crates/spc-lab/src/engine.rs`,
    `crates/spc-lab/src/runtime.rs` from `/tmp/opencode/AP2-nix-src`
    (`a3848044...`, `2ccb4d1c...`, `24aa016a...`); ours = primary
    pre-state; theirs = manifest rows (`52997120...`, `ca787553...`,
    `502ac127...`);
  - base for `crates/spc-lab/tests/traffic_contract.rs` = the
    lab-traffic-journal-v3 worktree version (`6f59de10...`, ours
    `3a4177bf...` equals the journal-v3 integration output recorded in
    `/tmp/opencode/traffic-journal-v3-primary-integration-manifest.json`);
    all 33 hunks applied; the merge resolved to byte-equality with the
    candidate row `efcd2461...` (every ours-side change subsumed by the
    candidate);
  - final merged states, each verified equal to its frozen `.merged`
    target: `main.rs`
    `b5f70e9cc3f115cac3d5d9c46a92f11486aa3ed76aa6203092021576543be48d`,
    `engine.rs`
    `c27fb45c6b404a04d5668a00b0642825c3e7e34bfa9528f05b012f8f278e51b4`,
    `runtime.rs`
    `f55d62851443b621fb18df0532461a1164512b920dee3036341a705a415790ec`,
    `traffic_contract.rs`
    `efcd246126ccca2681dc58494da860c5910d007a426cade15722127f308705bf`.
    `runtime.rs` had one textual conflict resolved keeping both sides in
    order (the `ark_service::validate_runtime` line first, then the T1
    traffic block).
- **`docs/SPEC.md` - no edit required.** The J1 manifest SPEC row
  (`f27c647bab652ab74bb3c07e2f6defbe0e395141c1f81fc1d495d0a5c8602003`)
  equals the candidate worktree SPEC byte-for-byte, and primary SPEC
  (`bfb28139f811e8fb74095509a6819b8447d9c3ea98c2f66d3d2f73321d0dce35`)
  equals that candidate plus exactly 12 inserted AP4-era lines (1
  frontmatter changelog line, 11 body lines) with zero candidate-only
  lines. Re-verified mechanically post-transfer (difflib opcode proof:
  two insert blocks, 12 lines total, no delete/replace). The whole J1
  SPEC delta was therefore already present in primary; no splice, no
  frontmatter change, no changelog fragment touched or created.
- **`flake.nix` - exactly one added line** vs primary pre-state:
  `spc-lab-txd = mkBinary "spc-lab-txd" "";` (3-way: ours `31c65b92...`
  AP4-era primary, base `b4bf0503...`, theirs `aa7f91f5...` manifest
  row; merged `3e6ed332...` verified equal to the frozen `.merged`
  target). No pin bump and no `flake.lock` change is part of this delta.

## 3. Producer copy (frozen r4 authority)

20 `testing/ark-rpc` files were brought to the frozen r4 producer state
(`/tmp/opencode/traffic-probe-profile-r4-manifest-full.json`, file
SHA-256
`ce6bdee7d0bd0a4b0aacfcee7d76a93d9b2c8406bdc8f9fbc219bee16f2d4a27`,
617 records, snapshot `71b4ee6f...`, traffic-consumer contract
`754e1e61...`, RPC-policy golden `7d87ad97...`); the source worktree
`lab-traffic-probe-profile` was verified frozen-OK before use. 9 files
write-new, 11 edited, 58 already equal; all 20 hash-verified
post-transfer, no sentinel bytes remaining. The r4 producer state
supersedes the J1-era manifest rows for 11 of those paths (reflected in
the section 5 scan). One nodeproc.py docstring mis-edit (a diff-marker
misread) was caught and corrected within the same hunk; the final file
is hash-exact.

Fixture dependencies verified: the two frozen probe reports
(`testing/ark-rpc/reports/2026-11-08-r1-l2-eb257af7/report.json`
`e1151d3e...`, `.../r2-.../report.json` `e834cba4...`) and the three j1
fixtures (`launch-profile-goldens.json` `4613f329...`,
`launch-profile-rpc-policy-goldens.json` `7d87ad97...`,
`traffic_selected_consumer.json` `05222f30...`) all match the digests
asserted by the merged tests.

## 4. Verification invariants (merged tree)

- **Privacy**: zero `sp_id`/`B_scan`/`B_spend`/`P_i` occurrences in
  `crates/spc-lab/src/traffic/` and `crates/spc-lab-txd/`;
  `TrafficArkSpec` is pure policy (`enabled`,
  `registration_before_mining` (must be true in v1),
  `chain_effect_budget_tx_per_sec`) with no identity fields; no traffic
  references in `ark_service.rs`.
- **Version contracts**: control protocol major version 1 with
  `TX_CONTROL_CAPABILITIES_V1` (4 entries); journal history version 3
  (`SUPPORTED_HISTORY_VERSION`); traffic key derivation version 2.
- **Access guards**: `UNAVAILABLE_KINDS` (`send_user_payment`,
  `bootstrap_fund`) is answered pre-parse in `http.rs`
  (`unavailable_kind_outcome`) with a closed rejection and no journal
  entry, so the T0 HTTP layer stays closed for kinds with no runtime
  variant; T1 traffic commands flow through the engine's internal
  submit path; budget authority stays in the existing `CommandLog`
  (`command/traffic_budget.rs`), never a second ledger.
- **ChainPolicy is field-projected, never assumed**:
  `engine/traffic_runtime.rs` `traffic_chain_policy()` builds the
  `spc_lab_txd::observer::ChainPolicy` from the actual canonical node's
  `manifest.effective_profiles` entry: `coinbase_maturity_blocks` must
  be `Some(100)` (else `traffic_consensus_unsupported`);
  `pow_limit_bits`/`initial_target_bits` are parsed from the profile's
  hex strings; `no_pow_retargeting`, `subsidy_initial_sats` and
  `subsidy_halving_interval_blocks` are all required, with any missing
  field failing closed as `traffic_consensus_unavailable`; the genesis
  is bound to the recorded `traffic_capability.expected_genesis`; and
  `pow_limit()` is re-validated after construction. No constant
  initial-bits or maturity assumption exists anywhere in the T1 path.

## 5. Scoped merged checks (one nonblocking flock, existing primary target)

All checks ran under `flock -n -o /tmp/opencode/silent-pool-heavy-build.lock`
(never `-w`, no polling, no auto-retry; acquired at 77 GiB free, above
the 50 GiB policy floor) using the existing primary `target/` directory.
Results were captured inline in the integration session and are recorded
here; no separate log files were kept, and re-running suites solely to
capture logs was avoided per the no-repeat policy. The broad 903-test
storm was not repeated; the checks below are the new/merged surfaces
plus the adjacent regression files.

| Check | Result |
|---|---|
| `cargo test -p spc-lab-txd` (full crate) | every suite 0 failed, 2 ignored (pre-existing); per-suite: 29, 26+2 ignored, 12, 6, 5, 4, 3, 3, 3, 1, 1, 1, 1 passed across the lib and harness suites (actor_control, declared_admission, hash_consumption, recovery_snapshot, signing_independent, journal/conformance); doc-tests 0 |
| `cargo test -p spc-lab --test traffic_contract` | 49 passed / 0 failed, incl. the 9 new J1 tests (authorized-r4-contract frozen table, rpc-policy goldens, immutable r1/r2 history, selected read-only recognition, frozen-empty v1 capability registry, metadata-omission refusal, raw/verbose form distinction, selected-rows method gating, contract-history grammar refusal) |
| `--test traffic_completion` | 1 passed / 0 failed |
| `--test traffic_history_conformance` | 7 passed / 0 failed |
| `--test traffic_j1_actual` | 0 passed / 1 ignored (the J2 fresh-harness scope; section 7) |
| `--test traffic_rpc_transport` | 1 passed / 0 failed |
| `--test traffic_runtime` | 3 passed / 0 failed |
| `cargo test -p spc-lab --lib` | 58 passed / 0 failed |
| 25 non-traffic regression files | all ok: access_hub_routes 3, access_projection 6, access_scope 18, ark_infrastructure 4, engine_restart 3, http_protocol 7, preflight 3, run_lock 1, scenario_v1 8, supervisor_adoption 7, two_runs_isolation 1, operator_ui_inventory, command_concurrency, command_idempotency, driver_loss 1, lifecycle_stop 3, snapshot_resume 4, snapshot_stop_race 1, http_sse 3, http_subscriber_lag 1, lab_hub 6, manifest_credentials 1, artifacts_privacy 1, height_wait 1, evidence_states 5 (counts shown where captured; every suite exit 0) |
| `cargo fmt -p spc-lab -p spc-lab-txd -- --check` | clean |
| `cargo clippy -p spc-lab -p spc-lab-txd --all-targets` | no code findings; only pre-existing cargo-cache permission noise and the sqlx-postgres future-incompat notice |

Post-check all-706 manifest scan: **682 rows byte-equal** to the J1
manifest, **5 expected differences** (the 3 merged files plus SPEC.md and
flake.nix, each verified in section 2), **19 intentional differences**
(the 8 pre-existing parent rows plus the 11 producer paths now at the
frozen r4 hashes, each verified against the r4 manifest), **0 missing**,
**0 unexpected**.

Index hash before == after all edits and tests:
`44f21a926d4d065ac6937fb0f81b698c7b2d0627` (re-verified after every
batch, including after all tests).

## 6. Source provenance and frozen-source recovery

The candidate worktree `lab-traffic-runtime` was contaminated by a
concurrent session during the integration window; its live bytes were
never used as a transfer source after detection. The transfer used only:

- the frozen J1-final manifest and its
  `/tmp/opencode/traffic-runtime-j1-*` freeze chain (handoff, patches,
  changed-rows, before-state);
- the manifest-verified `.theirs`/`.merged` files in
  `/tmp/opencode/t1-3way/` (every `.theirs` byte hash-equal to its
  manifest row);
- `/tmp/opencode/AP2-nix-src` for the main/engine/runtime 3-way bases;
- the lab-traffic-journal-v3 worktree version of
  `tests/traffic_contract.rs` (base `6f59de10...`; the journal-v3
  integration output `3a4177bf...` is cross-recorded in
  `/tmp/opencode/traffic-journal-v3-primary-integration-manifest.json`).

The engine.rs candidate row `ca787553...` is additionally cross-bound by
two independent frozen worktrees (`lab-native-bootstrap` and
`lab-traffic-witness-destinations`, both hash-equal at that row). Every
transferred byte is accounted for by a manifest row, a frozen `.merged`
target, or a cross-bound frozen worktree.

## 7. Scope and limits

- **J2 fresh-harness honesty**: `traffic_j1_actual` carries 1 ignored
  test; the J2 actual-fresh-harness scope is NOT executed or claimed by
  this integration. The freeze record's "four-case actual unfunded T1
  profile pass" is candidate-side evidence from the frozen source, not
  re-proven here.
- **No funded proof**: this transfer does not establish funded pool
  operation, Ark or witness spends, miner funding, or any live run. No
  node, miner, Nix build or live service was started by this transfer,
  and no sealed live run (controller 199214) or parent inspection run
  (1441192) was touched.
- **Queued next (not integrated here)**: the combined
  T2/contextual/T1-DEST packet
  (`/tmp/opencode/T2-T1DEST-combined-from-J1.patch` against this J1
  final state), then AP3b fixed r2, the AP3 UI read-model provider and
  the live wallet UI packets. Semantic merge requires T1 deferred
  funding AND AP3 fresh registration both before miners.
- In the producer copy, the r4 contract (`754e1e61...`) is the
  traffic-consumer authority; T2 test sources there are private
  evidence artifacts (observation-only); budget authority stays in the
  existing CommandLog.

## 8. Freeze

The primary selected code (the 42 added + 24 modified rows of section 2,
plus the 20 producer files of section 3) is frozen as of the completion
of the section 5 checks. Receipts:
`/tmp/opencode/traffic-runtime-j1-primary-integration-manifest.json`,
the `/tmp/opencode/t1-3way/` workspace, and the frozen source manifests
cited above. Index unchanged:
`44f21a926d4d065ac6937fb0f81b698c7b2d0627`.
