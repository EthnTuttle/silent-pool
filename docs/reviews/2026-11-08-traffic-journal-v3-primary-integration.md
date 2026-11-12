# Traffic journal history 3 (core prerequisite) - primary integration (2026-11-08)

Owner: serial primary Rust/Cargo integration writer (sole writer for this
packet). Authorization: the user's "ungate everything" grant - workflow
approval holds REMOVED by the user; the parent cleared the priority
ticket. Single-build/disk/nonblocking rules remained in force throughout.

**Approval record**: the asynchronous code audit returned **APPROVE
(bounded, exact `ff6353c2...`/`df0b0545...`)** -
`docs/reviews/2026-11-08-traffic-journal-v3-review.md`
(`2efba991d98a2b21c7c6b185a0f06c65672029559811ef45289964b59d916309`,
Fable code audit, placed in primary by the auditor 2026-11-08 18:25;
only that file written). No concrete bug found; the candidate's original
136+child receipts were verified and no additional execution is required
by the verdict; four low observations (L1-L4) and controller integration
requirements are recorded as follow-ups outside this merge scope.
*History, retained dated*: at transfer time (pre-18:25) the audit was
still pending and this receipt carried a "not-a-Fable-approval, audit
pending" note; the audit APPROVE arrived post-transfer and is now the
approval of record. The audit was not a ceremonial pre-integration hold;
any actionable bug findings would still require fixes before affected
runtime use (none were found).

Tested source: worktree
`/home/ethan/.local/share/opencode/worktree/ad8886/lab-traffic-journal-v3`,
frozen as `/tmp/opencode/traffic-journal-v3-tested-r1-manifest-full.json`
(608 files), full
`ff6353c276373e38737e10d1659c1f57206f2cbf83de79242b3b5bc9c4a57746`, code
`df0b05458a6744ed435f094568cb570edc583adcb7d5b75ae959be19369a735a`,
against the 602-file history2-r1 baseline (`e2dc40c7...` full /
`4c1bce8c...` code), which primary had already integrated. The parent
independently verified all hashes/modes/log+exit bindings (136 top-level
pass, 0 fail, 2 ignored, + fault-child 1; fmt/clippy 0) on the tested
source. Rust is exactly the tested bytes; only the decision-doc workflow
wording and the report were edited after that gate (2 rows, recorded in
`traffic-journal-v3-tested-r1-source-before-after.json`; the original
`1d84f881...`/`d106e200...` freeze is preserved).

External manifest: `/tmp/opencode/traffic-journal-v3-primary-integration-manifest.json`.

## 1. Pre-write verification (read-only, at transfer time)

- Aggregates reproduced with the declared recipe (`canonical(base+rows)`):
  full MATCH `ff6353c2...`, code MATCH `df0b0545...`; candidate worktree
  rows 608/608 clean (`tjv3-prewrite-reverify.log`).
- The 6 added paths absent from primary.
- Classification of the 8 modified rows against the history2 baseline:
  7 BYTE-APPLY (primary == baseline; later packets did not touch them),
  1 THREE-WAY - `crates/spc-lab/tests/traffic_contract.rs` (primary moved
  past baseline via the OP `operator_ui: false` constructor).
- Roots at expected pre-state: `Cargo.toml` `8530ce08...`, `Cargo.lock`
  `6ca42938...`, `docs/SPEC.md` `bf222a83...` (53 entries), `flake.nix`
  `b4bf0503...`; index `44f21a926d4d065ac6937fb0f81b698c7b2d0627`.

## 2. Transfer

- **6 new files byte-exact** (post-copy hash + mode verify, 0644):
  `crates/spc-lab-txd/tests/history3.rs` `71770920...`,
  `docs/changelog/2026-11-08-traffic-observer-snapshot.md` `47b9499a...`,
  `docs/changelog/2026-11-08-traffic-rollback-history3.md` `2f1d0b0b...`,
  `docs/decisions/2026-11-08-traffic-observer-snapshot.md` `74f9feed...`,
  `docs/decisions/2026-11-08-traffic-rollback-history3.md` `ab4d1147...`,
  `docs/reviews/2026-11-08-traffic-journal-v3-implementation.md`
  `592e0801...` (the tested report, transferred unmodified; integration
  notes are recorded HERE, not edited into the tested docs).
- **7 byte-apply files via generated `a/...`->`b/...` patches** (primary ==
  baseline verified pre-apply; post-apply hashes == candidate):
  `journal.rs` `ae2a8775...` (829-line patch), `lib.rs` `e3d5087b...`,
  `wallet.rs` `114c3ecd...`, `projection.rs` `804dceff...`,
  `protocol.rs` `cb22a61f...`, `spec.rs` `c5f5f318...`,
  `traffic_history_conformance.rs` `1874cef4...`.
- **1 three-way merge** (`crates/spc-lab/tests/traffic_contract.rs` ->
  `3a4177bf739f...`): base = history2 baseline `f412337d...` (the OP
  workspace's preserved `ours`), ours = post-OP primary `76e1e664...`,
  theirs = candidate `6f59de10...`. **Clean merge, zero conflicts.**
  The post-OP `operator_ui: false` constructor survives byte-exact
  (verified: count 1, line present); ours->merged delta is exactly
  +8/-8 lines, all the candidate's history-2->3 contract updates
  (version constants, retired/unsupported expectations), no foreign
  lines.
- **Projection shared hunk**: exactly the one `cfg(test)` fixture line
  `"history_version": 2 -> 3` (inside `mod tests` at projection.rs:1224);
  patch is 1 removed + 1 added line, nothing else.
- **Cargo/lock/flake: NO delta** - `Cargo.toml`, `Cargo.lock`,
  `flake.nix` are byte-unchanged by this packet (verified post-transfer).
  No vendor-hash probe, no Nix rebuild (no dependencies changed).
- **SPEC frontmatter splice (merge-owner convention)**: the two
  changelog fragments are single-line bullets in frontmatter form;
  consumed as ONE doc-only edit after the transfer: 53 -> **55**
  entries (`bf222a83...` -> `f27c647b...`), both bullets byte-exact,
  reverse-splice proof, all 53 prior entries preserved (marker set:
  ACCESS, history2, hub, OP runtime, node-profile, T1a, T0, AP-3a, T4,
  AP-2 registration), SPEC body untouched. Both fragments stay archived
  in `docs/changelog/`.
- Status 227 -> 232 lines; index `44f21a92...` unchanged throughout. No
  commits, no subagents, no cache cleanup.

## 3. Boundary and invariant verification

- **Version constants**: `DERIVATION_VERSION = 2` (keys.rs:83, KEY
  derivation unchanged), `TX_CONTROL_PROTOCOL_STRING =
  "spc-lab-tx-control/v1"` (CONTROL v1), `PROTOCOL =
  "spc-lab-tx-journal/3"` and `SUPPORTED_HISTORY_VERSION = 3`
  (journal.rs:102/112).
- **History-2 reopen fails closed**: `wallet.rs:649-655` returns
  `UnsupportedHistoryVersion` before any mutation. Nothing existing was
  migrated (by design).
- **No scope creep**: no J1 projection table/lineage imported (the only
  projection change is the cfg(test) version line); no T1
  actor/admit/bootstrap scope; snapshot status source/math/key fixtures
  unchanged.
- **Core proof semantics** (recorded, not extended): the versioned
  raw-header proof atomically admits lower canonical tips; the observer
  still trusts the selected node's canonical facts. Controller-side J2
  (pre-forward guard + local rollback serialization, raw index
  discovery) is future work NOT claimed here.
- **Cross-refs**: all references in the 5 new docs resolve; the two
  `docs/plans/reviews/...` citations are qualified planning-tree
  references (verified present in the `lab-experiment-expansion`
  planning tree; the wiring plan hash `44daaa2b...` verified at
  `/tmp/opencode/traffic-runtime-plan-review-v5/`), not primary links.
- **Prior-packet content preserved**: OP `operator_ui` constructor,
  ACCESS-era test content, AP-2 registration module untouched
  (`544decbad25b...`); all prior owned hunks in the merged
  traffic_contract.rs.

## 4. Validation - PASSED on the merged tree (2026-11-08 22:03-22:04)

The scoped gate ran on the merged primary tree under one nonblocking
`flock -n -o` grant (runner `/tmp/opencode/tjv3-validation-inner.sh`,
`d1660f93...`), using the candidate's exact tested commands:

1. core `cargo test -p spc-lab-txd -j2`: **exit 0** - 81 passed, 0
   failed, 2 ignored, 0 measured (includes the new `tests/history3.rs`
   and the journal/fault units; full per-suite counts in
   `tjv3-integration-logs/core.log`).
2. t0 `cargo test -p spc-lab --test traffic_contract
   --test traffic_history_conformance -j2`: **exit 0** - 49 passed
   (42 + 7), 0 failed, 0 ignored - the OP-era constructors and the
   candidate's version-3 contract expectations compile and pass
   together on the actual target.
3. projection `cargo test -p spc-lab --lib
   traffic::projection::tests -j2`: **exit 0** - 7 passed, 0 failed
   (45 filtered out) - the single cfg(test) history3 fixture line.
4. `cargo fmt --check`: **exit 0**.
5. `cargo clippy -p spc-lab-txd -p spc-lab --all-targets -j2 --
   -D warnings`: **exit 0**.

Aggregate: **all 5 steps exit 0; 137 passed / 0 failed / 2 ignored**
across the test steps. No leaked primary-target fixtures after the run
(sealed LIVE run untouched by construction - separate bin dir). Full
log: `/tmp/opencode/tjv3-validation-run.log`
(`2d7bed00525edb0df4cf744886a239147e1c999bc7fb8abe71152244cf77737a`),
per-step logs/exits in `/tmp/opencode/tjv3-integration-logs/`.

*History, retained dated*: the first two invocation attempts were
blocked by busy lock holders (distinct owners, each reported once, not
touched); the second attempt additionally used a blocking `flock -o -w
1800` wait - a resource-policy deviation corrected on parent notice
(owned wait inspected still-waiting and cancelled cleanly via SIGTERM,
PIDs 256157/256158; holder 235239 untouched; `-w` removed from all
future commands). The successful run above was a fresh single
nonblocking attempt at the next granted window. Earlier blocked-run
records are preserved in the pre-completion revisions of this file and
the superseded `244b888a...` log.

The sealed LIVE user lab (controller pid 199214,
`/tmp/opencode/live-lab-20261108T231027Z/bin/*`) was never touched and
uses sealed binaries, not the primary target; it has no tx journal and
nothing existing was migrated for it.

## 5. Final state

- Index `44f21a926d4d065ac6937fb0f81b698c7b2d0627` (== HEAD tree;
  nothing staged, no commits) - unchanged across the packet.
- Status 232 lines at receipt time.
- Roots: `Cargo.toml` `8530ce08...`, `Cargo.lock` `6ca42938...`,
  `flake.nix` `b4bf0503...` (all unchanged - no dependency delta);
  `docs/SPEC.md` `f27c647b...` (55 entries).
- Changed rows with git modes: 15 rows (14 packet rows + SPEC) captured
  in `/tmp/opencode/tjv3-changed-rows.json` and the external manifest;
  all mode 100644.
- Logs: `/tmp/opencode/tjv3-prewrite-reverify.log`,
  `tjv3-transfer.log`, `tjv3-post-verify.log`, `tjv3-crossrefs.log`,
  `tjv3-validation-run.log` (`2d7bed00...`, successful gate) and
  `/tmp/opencode/tjv3-integration-logs/` (per-step logs + exits).

## 6. Not claimed here

- No J1 projection/lineage, no T1 scope, no controller J2 serialization,
  no migration of any existing journal, no Ark server work (all future
  Ark servers self-hosted from Nix stock source per the new AGENTS.md
  requirement; no remote endpoint or server fork), no commits.
- The audit's L1-L4 low observations and controller integration
  requirements are follow-ups outside this merge scope.
