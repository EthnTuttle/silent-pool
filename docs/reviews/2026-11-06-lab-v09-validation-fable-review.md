# rbitcoin v0.9.0 + L2 lab port, independent validation gate: Fable review

Date: 2026-11-06 (live observations dated 2026-11-07 UTC)
Reviewer: Claude Fable 5.1 (`amazon-bedrock/us.anthropic.claude-fable-5-1`),
subagent session, read-only on the candidate.
Packet: `lab-v09-validation` (independent TFF release validator), uncommitted
in `/home/ethan/.local/share/opencode/worktree/ad8886/lab-v09-validation`,
base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.
Manifest: `/tmp/opencode/lab-v09-validation-manifest.json` (file SHA-256
`2193e52ae2b1538012c1c99b6cc18fa5732afb089b0e1de939a5060d01c23eb9`), eight
files, snapshot
`bb8e800932a457fefb17b780f55ff4f7337c423ce6a38b8c2be740890773ac7b`.

## Result

**REQUEST_CHANGES**, bounded, tied to snapshot `bb8e8009...ac7b`.

The live evidence is real, produced by the exact new node
(`bcb6d5ee...a171`, version `0.9.0`, source `0e037d35...dcda` on official
`9614b27e`), and reconciles to the same standard I applied to the approved
LP-live runs: I re-derived the baseline window, payouts, settlement depth,
light proof and share joins from a copy of the run's Postgres data; re-hashed
all 73 retarget headers against my own next-work formula and reproduced the
derived replay byte-for-byte; and proved from the raw node logs that the
namespace partition was a genuine same-ancestor fork healed by a 7-block
reorg. None of that needs to be re-mined.

Two defects in the **gate code** block approval of the packet as a reusable
release check. Both are fail-open predicates with concrete counterexamples I
ran against the real artifacts; both are small fixes with tests:

- **G1.** The namespace fork predicate accepts a non-forked, one-block-lagging
  node as a "fork" (`independent_checks.py:193`), and its heal-transition
  predicate is satisfied by ordinary pre-partition competing blocks
  (`independent_checks.py:216-219`).
- **G2.** The approval parser can bind more than one full hash to a single
  APPROVE pass (`release_lib.py:218-242`): a prose `snapshot_sha256 \`X\`` or a
  `Snapshot under review: X` line whose hash disagrees with the verdict's
  own prefix is approved alongside the real one.

Neither defect changes the recorded outcome: on the real records exactly the
intended hashes are accepted, and the real namespace run is a real fork. The
profile stage was `incomplete` in this snapshot's own runs (no stock binary
supplied); a later supplement run with the unchanged gate against the official
stock asset `16f47f80...79f7` passed and is verified below, so the stock
comparison gap is factually closed. After G1/G2 the next pass can be an
APPROVE bounded to all four functional stages (new node plus lab port on the
approved namespace runtime), still excluding production deployment, broad CI
green and quiet A/B stability.

## Snapshot and static verification

```sh
cd /home/ethan/.local/share/opencode/worktree/ad8886/lab-v09-validation
git rev-parse HEAD              # b0ab4ca34fda07c61a6335d05f598eaacc69678c
git status --porcelain -uall    # exactly the eight ?? files
# eight per-file SHA-256 and the canonical {base, sorted files} snapshot: all match
PYTHONDONTWRITEBYTECODE=1; umask 077
python3 testing/lab-release/v09_release.py --check-contract                   # passed, adcba5c9...
python3 testing/lab-release/v09_release.py --static-audit /tmp/opencode/v09-audit   # passed, candidate_port_proven=false
python3 testing/lab-release/test_release.py                                   # 9 tests OK
python3 testing/lab-release/v09_release.py --inputs /tmp/opencode/LP-v09-candidate-input.json --release-check --check-input
# exit 0; mode release-check; node/pool/miner/l2_reference/lp_live/runtime approved; scanner not_run (by design); stock_node not_run
find testing -name __pycache__  # none
```

Inputs re-verified by me: node binary SHA-256 `bcb6d5ee...` at
`/tmp/opencode/rbitcoin-v0.9-lab/target/dev/debug/rbitcoin-node`, tree HEAD
`9614b27e` with the 19 manifest files plus the mutable `docs/reviews/`; the
binary mtime (2026-11-06 20:12:03.09) is within 0.1 s of the build log's
`Finished ... in 1m 06s` line (20:12:03.17), so the log records the actual
link of this binary; the handoff record binds source, manifest, snapshot,
binary, build log and port report hashes. Pool `f3b35915`, miner `7e02b0fd`,
scanner `462877fd`, LP-live driver `aa3b37b6`, old L2 `d2f37b91`, runtime
`553cd4e8`/`2d6c752d`, and the four Nix binaries with `store_verified` all
match the report. Node Fable record: `**APPROVE**` tied to `0e037d35` and
binary `bcb6d5ee`, with "Workspace suite green: Not established" recorded;
this review does not relabel that.

## Live evidence verification (new node, not the historical runs)

### Baseline `LP-v09-live-baseline-r2-lp` (child summary `f53333de...`)

From a scratch copy of `private/postgres-data` (deleted afterwards):

- 25 sealed slices, all `work_unit_version 2`, fingerprint
  `7fe9ed1a...`, `difficulty == sum(slice_credit)` for every slice; 61 sealed
  leaves all join to one of 62 pool `accounted` events on
  `(nonce, ntime, version, extranonce, share_index)`; every slice's credits
  equal the sum of accounted `d`; `job_id >> 32` equals the slice height.
- Job nBits: 2 `job_issued` at `207fffff` (height-1 template), 50 at
  `200fffff`; `Q = 9` then `65` in the snapshots accordingly.
- For all 8 settled blocks: the newest-first whole-slice walk reproduces
  `high_water`; no selected slice is above its block height; heights 1-6 are
  underfilled warmups; `floor(4950000000 * d_i / 68)` equals every
  `ledger_history.paid_sats` (`coinbase` rail, Ark/forfeit/pending zero);
  `pay_P = 50000001`.
- Finding share (height 7) is sealed in slice `...669562`, above high-water
  `...667554`; window heights 2-7.
- Settlement: confirmations at `settled_at` (Postgres clock vs node
  `submitblock` log) are in `[6, 9]` for all 8 blocks; none below 6.
- Light proof re-parsed: header hash is the candidate `0f45ce61...0418`,
  coinbase txid equals the Merkle root, outputs
  `[50000001 pool, 3639705882, 1310294117]`, sum `5000000000`; scanner
  recovered vout 1/2 at height 7 with those amounts; non-block share (Bob)
  re-hashes, is above network and below `2^254 - 1`, `m = 2`; 45 non-block
  proofs (15/30).
- Node: `starting version=0.9.0`, 13 `submitblock` all `ok`, no `generate*`
  or `setmocktime`; children exited 0 / pool `-15`; no PIDs survive.

### Retarget `LP-v09-live-retarget-r2-lp` (child summary `c6fc811b...`)

- 73 headers: SHA256d(header_hex) equals the RPC hash, raw nBits equal RPC
  `bits`, prev-hash chain unbroken. My own formula (Core next-work,
  `first = H - 24`, clamp `[60, 960]`, `prev * elapsed / 240`, ceiling
  `min(prev << 2, floor)`, pow limit, Core `GetCompact`) reproduces every
  header: `0x7fffff * 181 // 240 = 0x608887` at 48 and
  `0x608887 * 226 // 240 = 0x5ae6f6` at 72, genesis-anchored clamp at 24.
- Effective profile from `getblockchaininfo.consensus` matches the L2
  approval (10 s / 240 s / 24, floor and overlay `1e7fffff`, retarget on,
  min-difficulty off).
- Derived replay: I re-ran the frozen `aa3b37b6` driver's
  `--derive-retarget-from` into a fresh directory; both outputs are
  byte-identical to the packet's (`21959f91...da21`, `20ceade4...f7f9`).
  Canonical blocks by wall-time phase `24/24/24/0` = 72 = `submitblock`
  records, zero conflicts; period intervals 23 each; payout replay
  `alice 2503779069 / bob 2446220930`; boundary oracle `1e7fffff / 1e608887 /
  1e5ae6f6`.
- Causal attribution is `not established` in both the live and derived
  records; achieved rates 271 kH/s then 200 kH/s per worker are recorded as
  measurements. The pool `share_outcome_unknown` count is 23 in the live
  summary (read mid-run) and 27 in the derived artifact (final `pool.log`,
  which I confirmed has 27); the report quotes 27. State both in the report.
- 72 `submitblock` all `ok`; no forbidden RPC; clean exits.

### Namespace `LP-v09-live-namespace-r2-rt` (result `225b7130...`)

Independently from `logs/node-node-{a,b}.log` "tip: best" lines (48 and 60
records) and `private/evidence.jsonl`:

- Common ancestor: both nodes carried `7548ae23...` at height 7 before the
  partition. During the partition, heights 8-14 have **different blocks on
  the two nodes at the same height** (node-a `2d513e8f...` vs node-b
  `45a40d88...` at 8, through `4e4cf648...` vs `22ff6092...` at 14).
- Heal: node-b's tip log re-announces heights 8-14 with node-a's hashes and
  then adopts node-a's `388c545a...` at 15, i.e. a 7-block reorg; the
  `canonical_at result=no` record for node-b's height-13 block
  `61551bcd...` anchored to `388c545a...` is that displacement. Final paused
  view: both nodes height 47, hash `47bd16f9...`.
- Both nodes ran `bcb6d5ee`; fronts `aac1278f`, miners `443de438`; all ten
  actors `exited`; `/run/netns` empty; no `LP-v09-live` process alive.
  Node API logs: 1433 and 1021 well-formed records, 47 and 24 `submitblock`
  (47 canonical + 24 including node-b's 7 orphaned blocks), no `generate*`
  or `setmocktime`. `no_bypass`: 276 checks, zero violations, in the
  `partition-p2p` phase; link commands `-> partition` and `-> normal`
  recorded.

So the run proves a real same-history fork and heal. The gate's predicate
does not (G1).

### Profile `LP-v09-live-profile-r3` (summary `8039ec88...`)

Version `rbitcoin-node 0.9.0`; seven negative CLI cases refused with the
contracted diagnostics; default-regtest and retarget-regtest `consensus`
objects match the contract; initial templates `207fffff` (documented
real-clock min-gap case) and `1e7fffff`. Stage status `incomplete`, stock
probe `not_run`. The static audit shows stock v0.9.0 lacks the lab flag and
the `consensus` RPC object. The stock comparison was completed afterwards by a
separate run of the unchanged gate; see the supplement section below.

## Blocking findings

### G1 (High) Namespace fork predicate is fail-open for a lagging, non-forked node

- Files: `testing/lab-release/independent_checks.py:187-200` (fork
  predicate `fork_views[0]["hash"] != fork_views[1]["hash"]`, heights
  compared only against `wait_height` minimums) and `216-219` (heal
  transition: any `canonical_at result == "no"` anywhere in the run).
- Counterexample (run by me against a copy of the real `namespace-r2-rt`
  artifacts): set the pre-heal node-b view to height 14 with **node-a's own
  height-14 hash** `4e4cf648...bf1c` (a synced node one block behind, no
  fork). `check_namespace_artifacts` returns `status: passed`. The displacement
  requirement is independently satisfied by the baseline-phase competing
  blocks at heights 2-3 (node-b's log shows reorgs there before any
  partition), so neither predicate requires the partition to have isolated
  anything. The packet's own negative test (`test_release.py:133-136`) only
  copies node-a's hash onto node-b at node-b's height, which is a different
  and artificial shape.
- Concrete change. Derive per-node chains from evidence, not from two tip
  views: read each node's "tip: best" sequence (or `getblockhash` records)
  and require (a) a common block at the last pre-partition paused height,
  (b) at the common height `min(h_a, h_b)` of the pre-heal paused views the
  two nodes report **different** hashes, (c) each pre-heal tip descends from
  the common block through its node's own announced chain, and (d) after
  heal, one node's chain re-announces heights above the fork point with the
  other node's hashes (the reorg) and the `canonical_at result == "no"`
  record's `height` lies in the partition window with `anchored_tip` on the
  surviving chain. Add the lag counterexample above to `test_release.py` and
  keep the existing one.
- Acceptance expectation. The real run still passes; the lag fixture and a
  "displacement only at heights 2-3" fixture are refused with named codes
  (`namespace_no_real_fork`, `namespace_reorg_evidence`).

### G2 (Medium) Approval parser binds extra hashes to an APPROVE pass

- File: `testing/lab-release/release_lib.py:218-242`
  (`_associated_with_snapshot`), used by `approval_decision` at `254-268`.
- Counterexamples (fixtures I ran): (1) a single pass
  `The first-pass snapshot_sha256 \`A\` was rejected elsewhere. ## Result
  **APPROVE**, tied to snapshot \`B\`` approves **both** A and B; (2)
  `Snapshot under review: \`A\`` followed by `**APPROVE**, tied to snapshot
  \`B\`` (full B) approves both. Rules at lines 238 and 240 associate any
  `snapshot_sha256 X` / `Snapshot under review: X` in the pass without
  requiring agreement with the verdict's cited hash or prefix, and the
  function returns per-hash, so a pass has no uniqueness constraint. On the
  15 real review records the accepted sets are exactly the intended single
  hashes (`0e037d35`, `553cd4e8`, `aa3b37b6`, `c16032a7`, `5e27e918`,
  `d2f37b91`, ...) and the vetoes cover every earlier rejection, so no live
  input is affected today; the packet is weaker than the already-approved
  `aa3b37b6` parser it sits beside, which refuses a pass with more than one
  candidate hash.
- Concrete change. Compute the set of full hashes associated with an
  APPROVE pass once; if the verdict line cites a prefix, drop candidates that
  do not start with it; if more than one distinct hash remains, treat the
  pass as ambiguous (approve nothing). Keep vetoes broad (a REQUEST_CHANGES
  pass may veto every hash it names). Add both fixtures to
  `test_approval_negative_fences`.
- Acceptance expectation. Fixtures (1) and (2) approve neither hash; the
  real-record test still passes with the same `approval_sections`.

## Non-blocking findings

- N1 (Should fix) Report line 214 says 27 unknown outcomes; the live
  `retarget_evidence.duty_cycle` says 23 because `retarget_duty_cycle` reads
  `pool.log` before pool shutdown. Say so, or have the report cite the derived
  value only.
- N2 (Should fix) Report section 1 opens with "The assigned v0.9 functional
  compatibility gates passed" before stating the profile stage is
  `incomplete`; lead with "three of four stages `release_checks_passed`;
  profile `incomplete` pending the stock probe" so no reader takes an
  all-four claim.
- N3 (Nit) `test_existing_namespace_raw_reference_and_no_fork_negative`
  exercises the **old** runtime fixture
  (`/tmp/opencode/LP-network-kimi-final-live-v2`, node `eb257af7`) and skips
  when absent; after G1 it should also run against `namespace-r2-rt` with the
  new node hash so the test covers the evidence actually under review.
- N4 (Nit) `_pass_sections` splits on `^# [^#]` outside code fences but
  `_verdict_entries` still matches a `**APPROVE**` line inside a fenced block
  (fixture: verdict inside ``` approves). Skip fenced lines in
  `_verdict_entries` too.
- N5 (Observation) The LP-live child's own node provenance in the retarget run
  is `source_git_provenance` (HEAD `9614b27e`, tracked diff, 3 untracked
  files) because the old pin forbids passing the new manifest; the outer gate
  binds the manifest before and after. This is the designed split and is
  correctly described in `review_contract.md`; no change.
- N6 (Observation) `positive_profile_probe` for `default_regtest` accepts
  either `200fffff` or the pow-limit `207fffff` at height 1 (documented
  real-clock min-gap semantics, confirmed byte-identical against the old
  node's `observed-profile.json`). Correct, but the report's "every declared
  `consensus` field matched" should mention that the initial template bits
  were `207fffff`, not the overlay, in this run.

## Supplement: stock profile comparison (received after the snapshot, verified)

The parent handed off a stock-binary profile run executed with the **unchanged**
`bb8e8009` gate code (driver script hash `b2dd6cbc...` recorded in the summary,
contract `adcba5c9...`), a new input file and a new run directory. I verified:

- Artifact hashes: summary
  `/tmp/opencode/LP-v09-live-profile-stock-r1/public/summary.json`
  `a96fa067...ef24` (`status: release_checks_passed`, profile stage `passed`);
  input `3065e724...68e9` (differs from the original input only in
  `handoff_id` and `stock_node`); provenance `ec84d707...e40f`; comparison
  `d2eee8bc...ad90`; stock binary
  `/tmp/opencode/v09-stock-bin/rbitcoin-node-x86_64-linux` SHA-256
  `16f47f80...79f7`, size 22025104.
- Stock association: the hash equals the `SHA256SUMS.linux-musl` entry for
  `rbitcoin-node` and the release-API asset digest (asset id 718197301); the
  `v0.9.0` tag object points at `9614b27e`. This is an official release asset
  association, **not** a source build attestation; the summary records
  `source_build_attestation: false`.
- Stock behaviour (my own checks, not the summary): `--version` prints
  `rbitcoin-node 0.9.0`; `--help` has `--regtest-pow-bits` and **no**
  `--retarget-regtest` (the patched node's help has both);
  `observed-profile.json` for the stock probe has no `consensus` object,
  `chain regtest`, `blocks 0`, initial template height 1 bits `207fffff`
  (the same real-clock min-gap semantics as the patched default profile,
  whose observed profile is byte-identical to the old node's,
  `53927377...`). The stock refusal of `--retarget-regtest` exited 2 with
  `error: --retarget-regtest requires a value`; refusal plus help absence
  together establish the flag is not implemented in stock.
- Patched node in the same run: version `0.9.0`, seven negatives refused,
  default and retarget profiles matched the contract, retarget initial
  template `1e7fffff`.
- Hygiene: run tree `0700`/`0600`; no process matching the stock binary or
  the run directory survives.

Chronology: the frozen implementation report (`02dff0a4...`) correctly says
the stock probe was `not_run` for `profile-r3`. The supplement closes that gap
with a later run under the same gate; the report need not be re-hashed for
this alone, provided the merge session records the supplement's four artifact
hashes next to the report. One nit (N7): `_stock_refusal` requires only a
nonzero exit and the substring `retarget` in the output, which the echoed
argument satisfies trivially; the stock probe should also assert that
`--retarget-regtest` is absent from stock `--help`, as I did by hand.

## Checks against the brief

- Hash/source/binary/report bindings: tight for every pinned component; the
  only loosenesses are G2 (parser multi-binding) and the fork predicate (G1).
  A release pass cannot be claimed from unknown/failed/provisional/missing
  data: `_write_summary` maps any `incomplete` stage to `incomplete`,
  unknown/timeout to `unknown`, and LP-live child must be `passed` and
  explicitly `provisional`; missing records raise `not_run`/`failed`, never
  pass. Post-run re-verification re-hashes and re-parses approvals.
- Permanent veto scope: any REQUEST_CHANGES pass naming the hash (full or
  `tied to snapshot <prefix>`) vetoes; a different hash's rejection does not.
  Appropriate.
- Node approval vs suite green: the port record approves source/port/node
  gate and records suite green as not established; the gate checks the
  snapshot and binary prefix, not a suite claim. Correct.
- Namespace with the new node pair: both nodes `bcb6d5ee`; the old L2-based
  runtime approval is for the runtime code `553cd4e8`/`2d6c752d`, and the
  gate requires it separately from the node approval. Correct.
- Counts derived, not literal: duty-cycle, canonical and period counts come
  from the approved driver's derivation; namespace counts (276, 71/71, 80,
  1433/1021, 10) are read from the artifacts.
- Error/cleanup/permissions: owned-token cleanup with start-time check,
  `0700/0600` trees, fresh datadirs, no symlinks; no survivors observed.
- Stock profile gap: the snapshot's `profile-r3` is factually `incomplete`;
  the later stock supplement under the same gate is verified below and
  counted as closing the comparison, not as changing the frozen report.

## Re-review procedure

Return a new eight-file manifest with the same recipe. I will verify hashes,
run `--check-contract`, `--static-audit`, `test_release.py` and
`--check-input --release-check`, and re-run my two counterexamples against the
revised predicates. The stock supplement (`a96fa067...`) is already verified
and carries forward if the profile code path is unchanged. No re-mining is
required unless the live path changes.

## Verdict

REQUEST_CHANGES on snapshot
`bb8e800932a457fefb17b780f55ff4f7337c423ce6a38b8c2be740890773ac7b`. Blockers:
G1 (fork predicate must prove a same-ancestor fork and a heal reorg, not
unequal hashes at unequal heights) and G2 (one unique full-hash binding per
APPROVE pass). The raw evidence of all four stages (baseline, retarget,
namespace, and profile including the later stock supplement) is
independently verified against the new node and remains valid for the next
pass. This is a lab-functionality finding set only: no production deployment,
broad workspace-green or quiet A/B stability conclusion is implied either
way. No candidate
file, other worktree or commit was touched; the only file written is this
review. My scratch outputs: `/tmp/opencode/Fable-v09-check-input.json`,
`/tmp/opencode/Fable-v09-retarget-derived{,.log}`; the Postgres copy was
deleted.

---

# Second pass (same day, same reviewer)

Snapshot under review:
`5473dbc851b01b731f05810af792d810e002e5a004a760a342ae070dcf3f9da0`
(`/tmp/opencode/lab-v09-validation-manifest.json`, file SHA-256
`00e18e4c5380ce333a73941f089e86dcfa2bc418bb69f305fafbc5dcb47d03f4`, base
`b0ab4ca`, now ten uncommitted files: the eight from the first pass with
`independent_checks.py`, `release_lib.py`, `v09_release.py`, `test_release.py`,
`review_contract.md` and the report revised, plus new
`testing/lab-release/namespace_replay.py` and
`testing/lab-release/fixtures/namespace-v09-structure.json`;
`contract.json` and `candidate.example.json` unchanged). Scope: G1, G2, the
first-pass nits, and a regression check that the raw four-stage evidence is
unaffected.

## Result

**APPROVE - bounded v0.9 lab release validation**, tied to snapshot
`5473dbc851b01b731f05810af792d810e002e5a004a760a342ae070dcf3f9da0`.

G1 and G2 are resolved by code I exercised with my own counterexamples; the
nits are addressed; the real four-stage evidence (baseline, retarget,
namespace, profile with the stock comparison) is unchanged and verified. The
approval covers: the release gate in `testing/lab-release/` as a reusable,
fail-closed check of the v0.9.0 + L2 lab port (`0e037d35` / `bcb6d5ee`) on
the approved namespace runtime (`553cd4e8` / `2d6c752d`) and approved
LP-live/pool/miner inputs; and the recorded evidence that the new node passes
the baseline, retarget, namespace and profile stages, including the live stock
comparison against the official `16f47f80` asset. It does **not** cover
production deployment, broad workspace-green or quiet A/B node stability,
full-fee or witness coverage, Ark, general SV2 interoperability, general
convergence claims, the runtime N1-N7 integration cleanup under separate
review, or any pin/adoption change. Any material change to these ten files
after this snapshot requires another review.

## Snapshot and static verification

```sh
cd /home/ethan/.local/share/opencode/worktree/ad8886/lab-v09-validation
git rev-parse HEAD              # b0ab4ca34fda07c61a6335d05f598eaacc69678c
git status --porcelain -uall    # exactly the ten ?? files
# ten per-file SHA-256 and the canonical {base, sorted files} snapshot: all match
PYTHONDONTWRITEBYTECODE=1; umask 077
python3 testing/lab-release/v09_release.py --check-contract           # passed, adcba5c9 (unchanged)
python3 testing/lab-release/v09_release.py --static-audit /tmp/opencode/v09-audit   # passed
nix develop -c python3 testing/lab-release/test_release.py            # 15 tests OK (6 new namespace negatives)
python3 testing/lab-release/v09_release.py --inputs /tmp/opencode/LP-v09-r2-profile-stock-input.json --release-check --check-input   # exit 0
python3 testing/lab-release/namespace_replay.py --run-dir /tmp/opencode/LP-v09-live-namespace-r2-rt --output <fresh>/evidence.json
find testing -name __pycache__  # none
```

The replay I produced is byte-identical to the packet's
(`a5322a67...2480`); the packet's counterexample artifact (`08bd240f...95fe`),
stock r2 summary (`d7a9931f...b4b8`), stock r2 input (`1e0178d7...1898`) and
evidence-validation record (`7b840f21...6eec`) hash as stated. The committed
fixture `namespace-v09-structure.json` contains tips, headers, links,
displacements and phases only; no cookies, passwords, keys or addresses.

## G1 verification (namespace fork/heal proof)

`namespace_replay.py` now: re-hashes every successful `submitblock` header
(SHA256d of the raw 80 bytes, parent from bytes 4..36; a hash with two
different raw headers is a conflict), reads each node's own "tip: best"
observations with their log timestamps, takes the partition window from the
`link_policy` events' wall times, requires both paused pre-partition views to
be the same block observed by both nodes before the partition, walks each
node's pre-heal tip down by raw parent links requiring every level to have
been announced **by that node inside the window** (`namespace_cross_node_chain`
if another block was announced there, `namespace_chain_hole` unknown if
nothing was), requires the two chains to differ at the common pre-heal height,
and requires a post-heal re-announcement of the whole winning chain by the
loser plus a post-heal `canonical_at result=no` whose block is the loser's own
partition block in `(common, common_max]` and whose anchor is the winner's
pre-heal tip or its raw descendant. Pre-heal views come from the paused
`quiesce-before-heal` phase before `heal-p2p`, so later log entries cannot
contaminate them. `check_namespace_artifacts` calls this proof
(`independent_checks.py:215-216`); the old two-view inequality is gone.

Real run result (my replay): common block height 7 `7548ae23...`; node-a
chain 8..15, node-b chain 8..14 with distinct blocks at every height;
`fork_hashes_at_common_height[14] = 4e4cf648... / 22ff6092...`; heal
`node-b -> node-a`, node-b re-announced 8..15 after the heal, displacement
event 724 (node-b block `61551bcd...` at 13) anchored to the winner tip
`388c545a...` at 15.

Counterexamples I ran through `check_namespace_artifacts` on copies of the
real artifacts (all refused; codes as shown):

```text
unchanged real artifacts ................................ PASSED (cmp_h 14, node-b -> node-a)
lag: node-b pre-heal view = node-a h14 (result only) .... namespace_no_real_fork / failed
lag + node-b log announcing node-a's chain in window .... namespace_no_real_fork / failed
displacements above the fork point removed .............. namespace_reorg_evidence / failed
node-b own h10 tip observation removed .................. namespace_chain_hole / unknown
cross: node-b pre-heal view = node-a tip h15 ............ namespace_no_real_fork / failed
future: node-b pre-heal view = its post-heal h20 block .. namespace_no_real_fork / failed
node-b never re-announces the winner chain .............. namespace_reorg_evidence / failed
```

The packet's own six new tests (lag, pre-partition-only displacement, missing
ancestor and hole as unknown, cross-node confusion, fork outside the phase
window, committed-fixture structure) pass under Nix and encode the same
cases. G1 resolved.

## G2 verification (approval binding)

`_approval_pass_facts` now builds one candidate set per pass from explicit
declarations only (`snapshot_sha256 X`, full `tied to snapshot X`,
`X (matches)`, `Snapshot under review:` with the hash on the same or the
next line, `APPROVE on snapshot` followed by the hash, and the one explicit
runtime code/full pair); fenced lines are excluded from both verdict and
candidate parsing; a cited prefix/suffix must agree with the single candidate;
more than one candidate, or a candidate disagreeing with the cited prefix,
approves nothing; REQUEST_CHANGES still vetoes every hash or prefix it names.
My fixtures:

```text
A rejected then B approved ........................ A refused, B accepted
reject A then approve A (same hash) ............... refused (permanent veto)
single APPROVE B with prose snapshot_sha256 A ..... neither accepted
Snapshot under review A, verdict tied full B ...... neither accepted
Snapshot under review C, verdict prefix bbbbbbbb .. neither accepted
prefix only, no full hash anywhere ................ refused
prefix + matching full under-review hash .......... accepted (legitimate)
unrelated bare hash near verdict .................. not a candidate; B accepted
fenced **APPROVE** ................................ refused
APPROVE and REQUEST_CHANGES same pass same hash ... refused
two (matches) hashes, one verdict prefix .......... neither accepted
prefix-only REQUEST_CHANGES then full APPROVE ..... vetoed
```

Real records: exactly the current approvals bind (`0e037d35`, `553cd4e8` with
its declared code pair `2d6c752d`, `aa3b37b6`, `c16032a7`, `5e27e918`,
`d2f37b91`, `3edcd4e7`, `45df0eb3`, `1d40b2b7`), every earlier rejection is
vetoed, and this review's own first pass vetoes `bb8e8009`. No approval record
was invented and no new format is accepted by a looser fallback. G2 resolved.

## Nits and the stock comparison

- N1/N2/N6: the report now states 23 mid-run vs 27 final unknown outcomes with
  the read-timing reason, opens with the chronology (three original passes,
  original profile `incomplete`, then the stock supplement completing all
  four), and records that the default-regtest probe actually returned
  `207fffff` under the documented real-clock min-gap rule rather than the
  requested overlay.
- N7: `_stock_refusal` now requires stock `--help` to list
  `--regtest-pow-bits` and **not** `--retarget-regtest`, and the refusal log to
  carry a recognized parser reason (`requires a value` in the official
  binary), recorded as `help_retarget_absent` / `not_mere_argument_echo`. The
  non-mining profile rerun `LP-v09-r2-profile-stock-r1` (`d7a9931f...`,
  driver `14469af5...` = this snapshot's `v09_release.py`) reports
  `release_checks_passed` with the stock binary `16f47f80...` bound and the
  candidate `bcb6d5ee...`; it supersedes nothing and re-mines nothing.
- N3: the real new-node namespace run is now covered by
  `test_committed_v09_namespace_structure_and_raw_run`.
- N4: fenced verdicts are ignored (verified above).

## Regression check

No live mining path changed: baseline/retarget invocation, LP-live child
`--provisional` use, outer before/after re-verification, Nix store checks and
summary status mapping are as in the first pass. The four stages' raw
evidence I verified in the first pass (Postgres-reconciled baseline, 73
hand-checked retarget headers with a byte-identical derived replay, raw-log
fork/heal, profile negatives/positives and the stock asset association) is
unchanged, and the packet's read-only revalidation (`7b840f21...`) agrees.

## Notes for the merge session (not conditions of approval)

- Record the supplement and r2 artifact hashes (`a96fa067`, `d7a9931f`,
  `a5322a67`, `08bd240f`, `7b840f21`) next to the report in
  `docs/REGTEST-LAB-STATUS.md` / `docs/MERGE-QUEUE.md`.
- The node approval record's "workspace suite green: not established" and the
  separate quiet A/B question remain open items for adoption, outside this
  packet.

## Verdict (second pass)

APPROVE on snapshot
`5473dbc851b01b731f05810af792d810e002e5a004a760a342ae070dcf3f9da0`, bounded as
stated in the Result. No remaining blockers. No candidate file, other worktree
or commit was touched across both passes; the only file written is this
review. Scratch outputs from this pass: `/tmp/opencode/Fable-v09-r2-check-input.json`,
`/tmp/opencode/Fable-v09-r2-replay/evidence.json`.
