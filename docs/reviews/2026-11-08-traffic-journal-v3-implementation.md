# Traffic journal history 3 - implementation handoff (2026-11-08)

**Status: scoped core/T0 tests, fmt and clippy PASSED on the first run.
Tested freeze ready for parent review/integration. Implementation review
may proceed asynchronously under the owner's execution grant.**

Worktree: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-traffic-journal-v3`.
Git base: `b0ab4ca34fda07c61a6335d05f598eaacc69678c`. No commit made.

## Approved layer binding

Before copying, verified all 602 paths, regular-file modes and SHA256s
against `/tmp/opencode/T0-history-v2-r1-manifest-full.json`; copied ONLY
those rows from its frozen `lab-traffic-history-v2` source. No full source
snapshot was made in `/tmp`. The new worktree was clean at the named base.

- Manifest-file SHA256:
  `c3dd3a57f73edd2fc302360c38a6783059326c38094160bbb891c8e3bc8ad381`.
- Approved full snapshot:
  `e2dc40c7d78527536998e7f9bd793273bbef4698fa700300e2d15f1c4c211859`.
- Approved code snapshot:
  `4c1bce8c0d12193ae1243c1166e13812798906c2f17e6748aa0b9842993ffb84`.
- T1a r3 included by that layer: full `65a16584...`, code `a8d28cea...`;
  primary Fable review Part 4 APPROVE read alongside the history-v2-r1
  Fable APPROVE. Historical baseline reports remain preserved bytes.
- Approved runtime plan:
  `/tmp/opencode/traffic-runtime-plan-review-v5/2026-11-08-traffic-runtime-wiring.md`,
  SHA256 `44daaa2baa41a4b93eb46a9e69dfdcf2bba1e0ce73367e27e83a87ab3a6b3307`;
  same-reviewer Round 5 APPROVE in the planning tree
  `lab-experiment-expansion/docs/plans/reviews/2026-11-08-traffic-runtime-peer-review.md`.

All layer/plan hashes were recomputed. The complete primary history-v2-r1
integration prerequisite is now COMPLETE: read
`/home/ethan/code/silent-pool/docs/reviews/2026-11-08-traffic-history-v2-primary-integration.md`;
the parent also reports its separately verified combined Nix gate (903
passed, 0 failed, 18 ignored), not rerun or claimed as this packet's result.
This report's delta is against the original
602-file layer, NOT the much larger git diff against b0ab4ca.

## Implementation and exact owned surface

Modified:

1. `crates/spc-lab-txd/src/journal.rs`: history-3 writer/reopen, closed
   rollback evidence and one atomic record, coherent observer DTOs,
   iterative send-ancestry maturity guard, opaque send-intent identity,
   before-forward revalidation, deferred tail repair/binding checks.
2. `crates/spc-lab-txd/src/lib.rs`: re-exports only.
3. `crates/spc-lab-txd/src/wallet.rs`: internal bound reopen passes
   run/seed/network before repair or SessionStarted; current API docs.
4. `crates/spc-lab/src/traffic/protocol.rs`: history/journal constants
   and current-format documentation/refusal wording; header shape retained.
5. `crates/spc-lab/src/traffic/spec.rs`: history version documentation
   and refusal wording (validator consumes the shared version constant).
6. **SHARED HUNK** `crates/spc-lab/src/traffic/projection.rs`: exactly the
   cfg(test) fixture's history_version 2 -> 3 line. Apply this one hunk
   serially with the J1 owner, never transfer the full projection file.
7. `crates/spc-lab/tests/traffic_contract.rs`: version-3 fixture and
   retired/unsupported-version expectations.
8. `crates/spc-lab/tests/traffic_history_conformance.rs`: actual writer-3
   roundtrip/all fields/digest, explicit new golden, preserved v2 golden,
   mutation-free v2 refusal and strict unknown-field checks.

Added: `crates/spc-lab-txd/tests/history3.rs`; paired decisions and
single-bullet fragments `2026-11-08-traffic-observer-snapshot.md` and
`2026-11-08-traffic-rollback-history3.md`; this report. SPEC frontmatter
is supplied through those fragments for the merge owner.

The baseline manifests bound every other file. In particular the key
derivation implementation and frozen signature/crypto fixtures remain
byte-identical. The historical adapter.rs text describes its approved v2
packet; the new decisions supersede its history-version statements.

## APIs and proof policy

- `observer_snapshot() -> ObserverSnapshot`: one mutex-held committed
  copy, no I/O. Deterministic outpoint map with identity/value/locus/script,
  source/context, actual current status and direct coinbase maturity; tip
  hash/height and committed sequence. Old snapshots retain old values.
- `record_chain_rollback(RollbackEvidence)`: proof version 1 under
  history 3; required old/new/common anchors, both raw-header branches
  tip-to-ancestor inclusive, exact old-minus-common displacement vector.
  One validated durable record, one sequence, shared displacement cascade.
- Proof checks: exact current old anchor, strictly lower real new height,
  branch-length/height arithmetic, <=4096 headers/branch, 80-byte lowercase
  hex, raw hashes and prev links, unique branch hashes, valid nonzero
  nonnegative/nonoverflow compact targets <=regtest maximum, header PoW,
  identical common header, no earlier overlap, exact displacement order.
  Replay checks the same contract and bounds encoded proof records to
  2 MiB before enum-content expansion. No proof implies no lower-tip path.
- Ordinary nondecreasing `record_chain_observed` and S3 fresh-chain
  same-block re-inclusion keep their existing path.
- `BroadcastSent` and `ReorgResubmit` recheck direct and OwnTransaction
  ancestry maturity at every new intent. Retained immature coinbases keep
  their reservation/status. Checked height arithmetic never makes overflow
  mature. Unknown/retry/resubmit/ever-accepted retention semantics remain.
- `pending_send_intent(command_id) -> PendingSendIntent` captures an opaque
  exact durable-send identity. `check_send_before_forward(&intent,
  expected_committed_seq)` requires current binding/sequence/send identity,
  unpaused/nonpoisoned BroadcastSent state and current maturity/ancestry.

The core verifies evidence linkage, not node consensus or canonical
selection. The observer still provides validated facts from the one
persisted selected canonical node, including the exact bound consensus
target schedule and raw-block inclusion. The regtest maximum here is a
safety ceiling, not a bootstrap target. Nothing infers a longest chain.

## Required controller integration

The actor/controller MUST consume history 3 coherently before a writer
starts, rescan inclusively from genesis, and use current snapshot statuses
for external-funding dedup/re-observation versus own-creator restoration.

The controller MUST serialize journal-intent capture/revalidation, local
rollback application and actual forwarding, with once-only request/send
correlation. The guard's mutex ends on return; it alone cannot close that
gap. Refresh the expected committed sequence, recheck, and withhold if
immature or stale. A journaled-but-withheld send retains Unknown/retry
discipline. Already-forwarded calls keep honest responses/Unknown. There
is no remote-chain atomicity claim and no controller implementation here.

## Test coverage executed

`history3.rs` covers: detached snapshots and stale sequence values;
concurrent tip/sequence coherence; valid atomic proof and partial-write
rollback; reopened tip/outpoint/transaction cascade; same-block reorg-back;
old/new/common/height/header/prevlink/displacement/PoW/target tampering;
missing/unsupported/unknown/duplicate proof fields and vector bounds;
complete malformed tail refusal with/without newline; recognizable
truncated-proof refusal; safe recovery of an unidentifiable incomplete
prefix; signed and reserved retained coinbase maturity regression;
Unknown same-bytes retry; stale queued permit; foreign handle refusal;
paused/immature resubmit without cap consumption; overflow remains
immature; external canonical re-observation versus own-change restoration;
own-change ancestry checks; and run/seed/network/caps/allocation/history-2
refusal before repair/session mutation.

The ancestry-only test deliberately injects an adversarial early
confirmation through the existing observer-trusting API to isolate the
ancestor guard from displacement. It is not a valid Bitcoin block claim.
All header/funding tests are offline conformance fixtures, not real native
funding or live-chain acceptance evidence. The preserved v2 line is the
approved source golden, not a newly captured historical node artifact.

## Actual checks and source binding

Actual source checks are logged in
`/tmp/opencode/traffic-journal-v3-source-check.log`: approved hash/mode
binding, owned-surface/preserved-vector checks, standalone scoped rustfmt
parse/format check and `git diff --check`. Their hashes and exact source
rows are recorded by the freeze recipe/manifests below.

The parent explicitly granted session `ses_e4386c6b5ffevCnI2y8xQOxPuJ`
the cleared shared build slot. Ran the exact prepared recipe below under
one nonblocking lifetime flock, using only `/tmp/opencode/T1a-target` and
the pinned `1.93.0-x86_64-unknown-linux-gnu` toolchain (rustup verified its
worktree `rust-toolchain.toml` override).

| Check | Result | Log |
|---|---|---|
| Core unit tests | 28 passed | `traffic-journal-v3-core.log` |
| Core Fable regressions | 26 passed, 2 ignored; separately spawned R1 child 1 passed | same |
| New history-3 tests | 12 passed | same |
| Journal durability | 6 passed | same |
| Lifecycle/reorg | 5 passed | same |
| Independent signing | 3 passed | same |
| Core doc tests | 0 tests, exit 0 | same |
| T0 traffic contract | 42 passed | `traffic-journal-v3-t0.log` |
| T0 real-writer history conformance | 7 passed | same |
| Projection fixture tests | 7 passed, 29 filtered | `traffic-journal-v3-projection-fixture.log` |
| Cargo fmt | exit 0 | `traffic-journal-v3-fmt.log` |
| Scoped all-target clippy, `-D warnings` | exit 0 | `traffic-journal-v3-clippy.log` |

Totals: **136 passing top-level harness executions, 0 failed, 2 ignored**,
plus the separately reported passing R1 child. The shared RIPEMD vector
runs in multiple existing integration binaries, so this is not a count
of distinct test functions. All five step exit files are 0.

Available space inside the lock before core/T0/projection/fmt/clippy was,
respectively, 70,878,363,648 / 70,461,407,232 / 70,079,475,712 /
69,916,864,512 / 69,916,676,096 bytes. Every step cleared the >=50 GiB
floor and the conservative 10 GiB growth estimate leaving >=20 GiB.
After the run, target-filesystem free space was 69,753,274,368 bytes.
No new target, cleanup, GC, server build or service was needed. Cargo
reported its existing global-cache permission diagnostics; all scoped
commands still exited 0. No manual cache changes were made.

**No compile/test defects or counterexample fixes were needed.** Before
the run, all 608 files matched full snapshot `1d84f881...`, code
`d106e200...`; immediately after all five steps, all 608 hashes still
matched. Only this report and the owned decision's workflow-hold wording
were updated afterwards. Their prior bytes were archived before editing.
All Rust files, lockfiles, frozen key/signature vectors and protocol
fixtures in the final receipt are exactly the tested bytes. No passing
build/test was repeated. Review remains asynchronous; controller runtime
integration and its live proofs remain separate work.

Executed recipe (retained for reproduction in a coordinated build slot):

```sh
TRAFFIC_JOURNAL_V3_BUILD_GRANTED=1 \
OPENCODE_GRANTED_SESSION_ID=ses_e4386c6b5ffevCnI2y8xQOxPuJ \
python3 /tmp/opencode/traffic-journal-v3-build-checks.py
```

It checks the priority ticket, acquires one nonblocking lifetime flock,
rechecks priority and actual target free space inside the lock before each
invocation (>=50 GiB; conservative 10 GiB estimated growth leaving >=20
GiB), and uses ONLY `/tmp/opencode/T1a-target`, `-j2`. Commands:

```sh
cargo test -p spc-lab-txd -j2
cargo test -p spc-lab --test traffic_contract --test traffic_history_conformance -j2
cargo test -p spc-lab --lib traffic::projection::tests -j2
cargo fmt --check
cargo clippy -p spc-lab-txd -p spc-lab --all-targets -j2 -- -D warnings
```

It stops on a blocked slot, space failure or failed check, without polling.
No whole-workspace/Nix rebuild or service/live-chain proof is needed for
this core fixture gate. The original source-only receipts are preserved;
the new tested receipts below carry the actual results. The owner's
self-hosted, pinned-source Nix Ark deployment clarification changes no
code in this core packet and makes no Ark execution claim here.

## Preserved source-only and new tested freezes

Recipe/script: `/tmp/opencode/traffic-journal-v3-freeze.py`.
Receipts: `/tmp/opencode/traffic-journal-v3-{recipe,manifest-full,
manifest-code,manifest-delta}.json` and `traffic-journal-v3-freeze.log`.

The recipe is SHA256 of sorted-key compact JSON `{base,files}` with
path-sorted `{path,mode,sha256}` rows. Code excludes only this report.
Delta is measured against the exact approved 602-file baseline. It checks
all frozen input hashes/modes again, preserves the approved v2 golden and
all unowned files, and records script/log hashes. These are manifests only;
no second source-tree copy, target, cache or runtime datadir is created.

The original source-only full/code remain `1d84f88142d2f694f833ffbf6e9d4fe4ab02abe3c086adae5b99de4fc44b2928`
and `d106e200394d44b14012f41e2fd49573c1cba49771d15223ed4cf864a3e31baa`.
All old receipts/scripts plus the two subsequently edited documentation
rows are additionally preserved under
`/tmp/opencode/traffic-journal-v3-source-r0-receipts/`.

New tested recipe: `/tmp/opencode/traffic-journal-v3-tested-r1-freeze.py`.
Outputs: `/tmp/opencode/traffic-journal-v3-tested-r1-{manifest-full,
manifest-code,manifest-delta,source-before-after,recipe}.json` and
`traffic-journal-v3-tested-r1-freeze.log`. The recipe requires all five
actual step exit files to be zero, checks that all 12 new tests ran,
records per-log hashes/counts/disk checks and the before/after rows, and
preserves the exact original history-v2 baseline binding. Code snapshot
excludes only this report, so its hash also covers the decision's
post-test workflow clarification; executable Rust bytes did not change.
