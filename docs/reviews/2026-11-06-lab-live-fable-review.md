# LP-live real-node v2 and retarget evidence: Fable review

Date: 2026-11-06
Reviewer: Claude Fable 5.1 (`amazon-bedrock/us.anthropic.claude-fable-5-1`),
subagent session.
Packet: `lab-live-e2e` (TFF Kimi), uncommitted in
`/home/ethan/.local/share/opencode/worktree/ad8886/lab-live-e2e`, base
`b0ab4ca34fda07c61a6335d05f598eaacc69678c`.
Manifest: `/tmp/opencode/lab-live-review-manifest.json` (file SHA-256
`f6ce44c4b79f65b6cd2b78b079d41f18f45a9fabac3f878e29ba57a03f537105`), five
files, snapshot
`afb48ddc4127757866a25093a48d77fd61dcae94016f42295e610d8631309599`.

## Result

**REQUEST_CHANGES**, bounded, tied to snapshot `afb48ddc`.

The live evidence is real and reconciles end to end. I independently
re-derived, from the private artifacts and a copy of each run's Postgres data
directory, every quantity the report asserts: header hashes, compact targets,
`m_issue`, `Q`, high-water selection, finding-slice exclusion, per-identity raw
payouts for all 15 (baseline) and 68 (retarget) settled blocks, coinbase
conservation, the light proofs the scanner consumed, settlement depth for every
settled block, and all 73 retarget headers against my own hand-written
next-work formula. I also re-ran the baseline acceptance command into a fresh
run directory; it passed with the same binaries. None of that needs to be
regenerated.

Two defects block approval of the packet as a reusable gate and as a record of
what was proven. Both are text- and assertion-level, bounded, and do not call
the recorded pool, miner, node or settlement behaviour into question:

- **B1.** The approval-record check (`regtest-lab-v2-e2e.py:800-805`,
  `819-822`) accepts any snapshot hash that appears anywhere in a review file
  that also contains `**APPROVE` anywhere. Every Fable review record is a
  multi-pass document that names rejected snapshots; I demonstrate that each
  rejected LP-1, LP-2 and L2 snapshot passes the check.
- **B2.** The retarget run is described, in the report and in the harness's
  assertion names and error strings, as a hashrate/budget response. The
  recorded data does not support causation: block pace was set by the stale-job
  cycle, not by the budget, and the two "response" assertions are satisfied by
  almost any pair of periods. The boundary arithmetic itself is correct and
  independently confirmed; only the causal framing must change.

Items 3-9 are non-blocking. A third pass can be confined to the diff for B1/B2
plus `--self-test`, `--check-scenario` and one `--validate-only` preflight; no
new live run is required unless the change touches the live path beyond
strings, names and the approval parser.

## Snapshot and provenance verification

```sh
cd /home/ethan/.local/share/opencode/worktree/ad8886/lab-live-e2e
git rev-parse HEAD                 # b0ab4ca34fda07c61a6335d05f598eaacc69678c
git status --porcelain -uall       # exactly the five ?? files in the manifest
PYTHONDONTWRITEBYTECODE=1 python3 - <<'EOF'
import json,hashlib
m=json.load(open('/tmp/opencode/lab-live-review-manifest.json'))
for f in m['files']:
    assert hashlib.sha256(open(f['path'],'rb').read()).hexdigest()==f['sha256'], f['path']
files=sorted(({'path':f['path'],'sha256':f['sha256']} for f in m['files']), key=lambda x:x['path'])
body=json.dumps({'base':m['base'],'files':files},separators=(',',':'))
assert hashlib.sha256(body.encode()).hexdigest()==m['snapshot_sha256']
EOF
```

All five per-file hashes match; the snapshot reproduces as SHA-256 of compact
JSON `{"base", "files"}` (identical with or without `sort_keys`). No
`__pycache__` exists under `testing/` after my runs. `git diff --check` is
clean; the five files contain no non-ASCII bytes and no trailing whitespace.

Executed binaries, re-hashed by me, equal the report's table:
pool `f3b35915...`, scanner `462877fd...`, miner `7e02b0fd...`, baseline node
`0b3a0cba...`, retarget node `eb257af7...`. Source trees: `lab-share-integration`
and `lab-cpu-miner` still verify against the LP-1 (`c16032a7`) and LP-2
(`5e27e918`) manifests; `/tmp/opencode/rbitcoin-lab-retarget-profile` is HEAD
`e8cf9c9d` plus the 18 L2 manifest files (`d2f37b91`); `/home/ethan/code/rbitcoin`
is HEAD `e8cf9c9d` with a clean tree.

Build provenance (see item 3): pool and miner were linked at the time of their
logs (2026-11-05 23:44/23:45). The retarget node binary's mtime is 2026-11-05
18:07 while its "build log" is 2026-11-06 00:14; cargo found the binary
up to date against the frozen L2 sources and did not relink, which is
acceptable association evidence. The baseline node binary (mtime 2026-11-03
20:47) was not built by this packet and has no build log here; its association
with `e8cf9c9d` (committed 2026-11-03 20:33 -0500) rests on the clean tree at
run time and the mtime ordering.

## Approval records as used by the harness

- LP-1 `c16032a7`: named by the r3 `**APPROVE - bounded LP-1.**` verdict
  (`2026-11-05-lab-lp1-fable-review.md:600-619`). Correct input.
- LP-2 `5e27e918`: `**APPROVE** (bounded LP-2)` at line 801-802. Correct input.
- L2 `d2f37b91`: `**APPROVE**` at line 299; the stability follow-up has since
  been adjudicated `CLEARED_WITH_RECORDED_BASELINE_LIMITATIONS`. That
  adjudication concerns node stability only; it is not treated here as a
  harness proof, and the suite outcomes are not relabelled.

## Static checks and self-tests (my runs)

```sh
./testing/regtest-lab-v2-e2e.py --check-scenario testing/lab-live/baseline.json   # validated
./testing/regtest-lab-v2-e2e.py --check-scenario testing/lab-live/retarget.json   # validated
CARGO_BUILD_JOBS=2 PYTHONDONTWRITEBYTECODE=1 nix develop -c python3 testing/regtest-lab-v2-e2e.py --self-test
# Ran 12 tests in 0.326s  OK
```

The 12 self-tests are meaningful: consensus `SetCompact` accept/reject
vectors, the L3 `R = 2^255 - 1` vectors (`Q = 65/9`, stepdown 2 -> 1, 8 -> 4
at `2^252`), a payout vector with a negative (`difficulty != credit sum`),
duplicate-active-job disambiguation with a negative (`00..00` header), retarget
drift detection at height 48, strict-scenario negatives (unknown field,
mainnet, retarget flag on the baseline fixture), manifest tamper detection,
port collision, redaction, and a real child that ignores SIGTERM: refusal to
signal on a wrong start token, then SIGKILL after grace. No sleeps are used to
farm time; the only waits are bounded probes.

## Independent live-evidence verification

Method: I did not start the recorded clusters. For each run I copied
`private/postgres-data` to a scratch directory, started a throwaway Postgres on
a private socket, extracted `slice`, `slice_credit`, `block_found`,
`distribution_snapshot` and `ledger_history` as TSV, stopped it and deleted
the copy. Everything else came from `private/*.json`, `pool.log`,
`node-api.jsonl`, `miner.jsonl` and `canonical-light-proof.bin`.

### Baseline (`/tmp/opencode/LP-live-baseline-verified`, summary `f8f62fba...`)

- Light proof: parsed as `u32 len || header || u8 n || siblings || u32 len ||
  coinbase`; `n = 0`; SHA256d(header) is the candidate
  `09bc94a1...6203a8`; the coinbase txid equals the header Merkle root;
  outputs are `[50000001 (pool P2TR, declared script), 2038235294, 2911764705]`,
  sum `5000000000`; header nBits `0x200fffff`; hash <= target.
- `Q = ceil(8 * 2^255 / (T + 1)) = 65` for `0x200fffff` and `9` for the
  pow-limit template at height 1 (the height-1 snapshot stores `Q = 9`,
  which is correct: the pool's first template carried `207fffff`, and the pool
  log shows exactly one `multiplier_stepdown` to `m = 1` and one
  `multiplier_stepup` back to `m = 2` at generation `1793946486127`).
- Window: for all 15 settled blocks, walking sealed slices newest-first by
  `(height, slice_id)` with `slice_id <= high_water` until the sum reaches `Q`
  reproduces `high_water` as the newest consumed slice; no selected slice has
  a height above its block; heights 1-10 are underfilled (warmup) and the
  harness correctly refused them (`require_full_window`); height 11 has work
  68 (overshoot 3). `floor(4950000000 * d_i / 68)` per identity equals the
  `ledger_history.paid_sats` rows (`2038235294`, `2911764705`), rail
  `coinbase`, Ark/forfeit/pending all zero, for every one of the 15 blocks,
  not only the selected one. `pay_P = 5000000000 - 4949999999 = 50000001`.
- Finding-slice exclusion: the finding share (nonce 155) is sealed in slice
  `504404952212020269`, which is above the high-water `504404952212019964`.
- Share evidence: 71 `accounted` events; all 69 sealed leaves join to an
  accounted event on `(nonce, ntime, version, extranonce, share_index)`; every
  slice's `slice_credit.diff_sum` equals the sum of accounted `d` for its
  leaves; `job_id >> 32` equals the slice height for every leaf. 70 shares at
  `m = d = 2`, one at `m = d = 1` (the pow-limit template). The reported
  non-block share re-hashes to `13f692e4...` from its header, is above the
  network target, below `2^254 - 1`, and `m = 2` is what the L3 formula gives
  for `0x200fffff`. 50 non-block proofs cover both identities (19/31).
- Settlement depth: for every settled `block_found` row, the tip at
  `settled_at` (counted from the node's `submitblock` API log) gives
  confirmations in `[6, 11]`, minimum exactly 6. This is a positive check that
  no block in the run settled early, not only an observation of one block at
  10. The two clocks are the same host (Postgres `now()` vs node log), so the
  resolution is well below the block spacing.
- Scanner: `scan-{alice,bob}.log` recover vout 1/2 with the exact amounts at
  height 11, matching the frozen `p_x` entries.
- Node: `node-api.jsonl` contains no `generate*` or `setmocktime` calls; all 20
  `submitblock` calls are `ok`; `node.log` reports no outbound peers.
- Cleanup: `processes.json` shows postgres/redis/node/miner exit 0, pool -15,
  no cleanup errors; none of the PIDs exist.

### Retarget (`/tmp/opencode/LP-live-retarget-run6`, summary `93ddfabc...`)

- All 73 headers: SHA256d(header_hex) equals the RPC hash, the raw nBits equal
  the RPC `bits`, and the prev-hash chain is unbroken. My own formula
  (not the oracle module: Core next-work with `first = H - 24`, clamp
  `[60, 960]`, `prev * elapsed / 240`, ceiling `min(prev << 2, floor)`, then
  pow limit, Core `GetCompact`) reproduces every header's bits, including
  `1e76eeee` at 48 (`0x7fffff * 223 // 240 = 0x76eeee`) and the floor clamp
  at 72 (`0x76eeee * 263 // 240 = 0x8254c2`, above the floor). The oracle's
  `validate_retarget_vector` returns no mismatches on the same vector.
- Effective profile from `getblockchaininfo.consensus` matches the L2
  approval: `207fffff` limit, `1e7fffff` floor and overlay, 10 s / 240 s / 24,
  retargeting on, min-difficulty off.
- `Q = 524289` at `1e7fffff` and `564257` at `1e76eeee`; the 68 snapshots
  carry exactly these values by height (48-68 at `564257`). The window walk,
  high-water, finding-slice exclusion (nonce 2025215 sealed in
  `...258079 > high_water ...254143`), and all 136 ledger rows reproduce from
  the slices as in the baseline. 4577 accounted events; all 4520 sealed leaves
  join; all credit sums match; `m = d = 1024` throughout; 4303 non-block
  proofs across both identities. Light proof and scanner outputs verify
  (vout 2/1, height 8). Settlement confirmations at `settled_at` are in
  `[6, 9]` for all 68 blocks. 73 `submitblock` all `ok`; no generation or
  mock time.
- Workers: both have positive attempts; `pause_workers` / `set_worker_budget`
  / `snapshot` / `resume_workers` happen within ~0.5 s at each boundary after
  `height_reached`, so no solution is held.

### Independent baseline rerun

```sh
# exact report command, new run dir, Nix, jobs 2, PYTHONDONTWRITEBYTECODE=1
--run-dir /tmp/opencode/Fable-LP-live-baseline-rerun
# exit 0; status passed; source_gate acceptance; all nine assertions true
# public summary SHA-256 23640b2a4caf35e41420b08c2129780d3a28f2500721682f22c8cc8f5b7aec4d
```

Different chain, same binaries: block at height 9, `Q = 65`, work 70
(overshoot 5), `pay_P = 50000000`, total paid `4950000000`, both identities
recovered by the scanner, 40 non-block proofs (20/20), all children exited
clean. This is the same fixture at the same inputs; it confirms the harness
is deterministic in its checks and repeatable, not that it covers more.

## Blocking findings

### B1 (High) Approval-record check does not bind APPROVE to the snapshot

- File: `testing/regtest-lab-v2-e2e.py:800-805` (pool, miner), `819-822`
  (node). Logic: `snapshot in text` and, separately,
  `re.search(r"(?:\*\*APPROVE\b|Fable\s+APPROVE)", text)`.
- Counterexample (run by me against the records the harness actually read):

  ```text
  lab-lp1-fable-review.md  rejected r1 snapshot 81fa17d2...  -> passes: True
  lab-l2-fable-review.md   rejected    snapshot 646656a8...  -> passes: True
  lab-lp2-fable-review.md  rejected r1 snapshot af531370...  -> passes: True
  ```

  Each record is a multi-pass document that names its rejected snapshots and
  ends with `**APPROVE`. A pool manifest for the r1 LP-1 tree would pass
  acceptance. The miner (`APPROVED_LP2_SNAPSHOT`, line 43) and the
  retarget node (`d2f37b91...`, lines 814 and 1190) are protected by
  hard-coded constants; the pool is not, and the node in baseline mode has no
  approval check at all. The report's "Approval records used by the harness"
  (lines 38-39) therefore describes a weaker gate than it implies for a
  reusable proof.
- Concrete change. Bind the marker to the hash: accept only when a verdict
  line of the form `APPROVE ... <full 64-hex snapshot>` (or the record's
  explicit "tied to snapshot `<hash>`" phrasing, allowing the 8-char prefix
  only when the full hash appears in the same section) is found, and reject
  when the same hash appears in a `REQUEST_CHANGES` verdict with no later
  `APPROVE` for it. Simplest robust rule: split the record on `## Verdict` /
  `## Result` headings, and require an `APPROVE` verdict section whose text
  contains the full snapshot hash or whose section (up to the next heading)
  contains it. Apply the same rule to pool, miner and node, and drop or keep
  the hard-coded constants as a second factor. Add a self-test with a
  two-pass fixture (rejected hash A, approved hash B) that rejects A and
  accepts B.
- Acceptance expectation. `test_approval_record_binds_snapshot` in the self
  tests; the three real records still accept `c16032a7`, `5e27e918`,
  `d2f37b91` and reject `81fa17d2`, `af531370`, `646656a8`.

### B2 (Medium) Retarget evidence is framed causally; the data shows an overhead-bound pace

- Files: report lines 182-190 ("excluded from hashrate-response claim",
  "first hashrate-responsive boundary", "after the actual budget change");
  `regtest-lab-v2-e2e.py:1004` ("measured retarget response asserted
  separately"), `1820-1825` (error codes `retarget_not_observed` /
  `budget_change_not_observed`, strings "did not respond to the measured live
  period", "did not respond after actual LP2 budget changes"), `1836`
  (`hashrate_response_block_covered`); `testing/lab-live/retarget.json:21-22`
  (`first_responsive_height`, `second_responsive_height`).
- What the recorded data shows (all from `run6/private`):

  ```text
  expected hashes per block: 131072 at 1e7fffff, 141064 at 1e76eeee
  phase 25-47 (800k requested): per-worker attempts/elapsed = 273 kH/s
      23 intervals took 223 s; pure hashing at that rate would take ~5.5 s
  phase 49-71 (200k requested): 191 kH/s; 263 s elapsed; hashing ~8.5 s
  elapsed ratio 263/223 = 1.18; achieved-rate ratio 1.43; requested ratio 4.0
  miner events by phase (submitted / ingress_ack / stale-share / block_qualified):
      p1  49288 / 1379 / 34360 / 787
      p2  59055 / 1374 / 38272 / 926
      p3  49893 / 1640 / 44512 / 731
  whole run: 163809 submitted, 120052 stale-share rejects (73%),
             2529 block-qualified solutions, 73 canonical blocks (~3%)
  ```

  Accepted work per phase is flat (1374 vs 1640 acks) while the requested
  budget changed 4x; the block pace (~9-11 s) is set by how long the miner
  keeps hashing a superseded job before the next `SetNewPrevHash`, not by the
  budget. The retarget at 48 responded to 223 s of recorded timestamps and the
  one at 72 to 263 s; both are correct arithmetic, neither is evidence of a
  hashrate response. The two assertions are also near-vacuous: 48 differs from
  the floor whenever the period is faster than 240 s, and 72 differs from 48
  unless the period is exactly 240 s.
- Concrete change. (1) Rename and reword: `first_responsive_height` ->
  `first_full_period_height`, `hashrate_response_block_covered` ->
  `full_period_boundaries`, error strings to "boundary 48 bits equal the
  floor; no retarget movement observed in the first full period" and
  "boundary 72 bits equal boundary 48", and the report's phase table header
  to "requested budget (not attained; see measurements)". Drop the words
  "hashrate-responsive" and "after the actual budget change" or qualify them
  as "after the budget command; elapsed time is not attributed to it". (2) Add
  to `retarget-evidence.json` and the summary the per-phase counts above
  (submitted, ingress_ack, stale rejects, block_qualified, canonical blocks),
  derived from `miner.jsonl` and `pool.log`, so the duty cycle is part of the
  record. (3) State in the report that the retarget profile was exercised in
  both directions (tighten at 48, clamp to floor at 72) with correct
  arithmetic, and that the block pace was dominated by stale work, which is
  itself a finding for LP-1/LP-2 (item 9).
- Acceptance expectation. No causal language remains in the five files;
  `--self-test` and `--check-scenario` pass with the renamed fields; the
  existing run6 artifacts are re-described, not re-run.

## Non-blocking findings

3. (Should fix) Report line 35: state that the baseline node binary predates
   the packet and has no build log here; association is HEAD `e8cf9c9d` plus a
   clean tree at run time and mtime ordering. Line 50/55: state that the
   retarget node log is a no-op rebuild that confirmed the binary was up to
   date against the frozen L2 sources. No cryptographic attestation is
   claimed, and none should be.
4. (Should fix) Hard-coded metadata is wrong in retarget runs:
   `regtest-lab-v2-e2e.py:771` writes `"retarget_claim": False` into
   `run_created`, and `1163` writes `"profile": "baseline_real_pow",
   "retarget": False` into `preflight.json`; both are false for run6 (verified
   in its private files). Use the scenario value.
5. (Should fix) Oracle constants: `oracle.py:421` clamps at `60/960` and
   `regtest-lab-v2-e2e.py:1804,1809` use `height - 24` and `60/960` literally
   instead of `timespan/4`, `timespan*4` and the profile interval. Correct for
   this profile, wrong for any other. `oracle.py:413` reassigns `interval`.
6. (Should fix) `oracle.encode_compact_target` (`oracle.py:397`) shifts right
   for `size <= 3`; Core shifts left (`0x1234 -> 0x02123400`, the oracle gives
   `0x02000012`). Unreachable for any real target but wrong as a reference.
7. (Should fix) `normalized_job_id` (`regtest-lab-v2-e2e.py:1889`) parses an
   all-digit hex job id as decimal (about 2.3% of random 8-hex ids), which
   would surface as a spurious `job_issued_missing` unknown. Carry the pool's
   encoding explicitly or try both and require a unique hit.
8. (Optional) The fee term of SPEC 4.4 was never exercised live: `fees = 0`
   in all 83 settled blocks across both runs (empty regtest mempool). The
   unit vector covers it; the report should say so. The 28
   `share_outcome_unknown` pool events in run6 are parsed but not surfaced in
   evidence; record their count. `source_git_provenance` writes to a fixed
   `/tmp/opencode/LP-live-git-provenance.log` (line 661); concurrent runs
   collide.
9. (Observation for LP-1/LP-2, not this packet) 73% of submissions are
   stale-share rejections and ~97% of block-qualified solutions never become
   blocks; the chain advances at ~10 s per block regardless of hashrate. The
   new-tip to new-job path (template poll, `SetNewPrevHash` fan-out, or miner
   job switch) is slow relative to regtest block times. Worth a packet of its
   own; it does not affect the correctness proven here.

## Checks against the brief

- Hashes, recipe, binaries, source trees, approval inputs: verified (above).
- Job generation join: pool millisecond `generation` and miner
  `target_generation` are kept distinct; the join is on normalized job id plus
  nBits and share-target equality; I reproduced the join for every sealed leaf.
- Window, high-water, whole-slice, finding exclusion, `Q`, underfill: verified
  for every settled block in both runs, not only the selected one.
- Payout floors and conservation with actual subsidy/fees: verified for all
  83 settled blocks; `pay_P` is the exact remainder.
- Scanner light proof and header Merkle: verified from the proof bytes.
- Payout policy: explicitly `pool_template_exact_owed_pre_ladder`; no ladder,
  witness, Ark, type-32 or general interop claim is made, and the summary
  carries each as `false`.
- Confirmation boundary: no premature settlement in either run (minimum 6 at
  `settled_at` across all 83 blocks). The report's "confirmed at 10" is now
  backed by a negative check; please cite this result rather than the single
  observation.
- Strict scenario/CLI, private 0600/0700, public summary free of addresses,
  keys, cookies and tweaks (grepped), fresh datadir per run, owned-child
  cleanup with start-token check, no peers, no generation, no mock time: all
  observed.
- Oracle independence: the oracle imports no pool code and recomputes `m`,
  `Q`, targets and payouts from L3/SPEC formulas and node data; the slice
  reconciliation to accounted work is real (every leaf joins) rather than a
  comparison of pool outputs to themselves.
- Hidden injection: none found; every chain block in both runs has a
  `submitblock` entry carrying the exact header, and no RPC generation exists.

## Re-review procedure

Return a new manifest at `/tmp/opencode/lab-live-review-manifest.json` with
the same recipe. I will verify hashes, run `--self-test` and both
`--check-scenario` calls, run one `--validate-only` preflight with the report's
baseline arguments, check B1 against the three real records plus a two-pass
fixture, and read the B2 wording. The existing run artifacts remain valid
evidence for the fixed packet provided the live path is unchanged apart from
names and strings.

## Verdict

REQUEST_CHANGES on snapshot
`afb48ddc4127757866a25093a48d77fd61dcae94016f42295e610d8631309599`. Blockers:
B1 (bind APPROVE to the snapshot in the approval-record check) and B2 (remove
causal hashrate-response framing; record the per-phase duty cycle). The live
evidence itself is verified and reusable. No candidate file, other worktree or
commit was touched; the only file written is this review. My scratch Postgres
copies were deleted; my rerun artifacts remain at
`/tmp/opencode/Fable-LP-live-baseline-rerun{,-driver.log}`.

---

# Second pass (same day, same reviewer)

Snapshot under review:
`a3e2117be3fb26487242a0f0e6b14f79c271c5d39b0565d84ce528f996dcb79d`
(`/tmp/opencode/lab-live-review-manifest.json`, file SHA-256
`42fc8c187c715054b8de584b1b3cb78e8c025500c8e4ca0a1c74e50e7f462bb8`, base
`b0ab4ca`, the same five uncommitted files in `lab-live-e2e`; `baseline.json`
unchanged, the other four revised). Scope: B1, B2 and items 3-8, plus a
regression check that the first-pass evidence verification still applies.

## Result

**REQUEST_CHANGES**, narrowed to one small blocker (C1) plus one should-fix
(N1). B1 is resolved as a fail-closed gate: on the three real records exactly
the approved hash is accepted and every previously rejected hash is refused,
and every synthetic failure case I tried fails closed. B2's causal framing is
gone from the code, fixture and report, and the per-phase miner counts, work
rates and the 28 unknown outcomes are now derived from the actual artifacts
with input hashes and reproduce byte-identically. Items 3-8 are fixed as
described. C1 is new only because the first pass could not reach it: the
"canonical" per-phase block counts added for B2 are hard-coded constants that
do not match the node evidence, and the live path would write them into every
future retarget summary. The fix is a few lines plus a re-derive of the two
cheap derived artifacts; no live run is needed. If C1 is fixed and nothing else
material changes, the third pass is an APPROVE confined to that diff.

## Snapshot verification

```sh
cd /home/ethan/.local/share/opencode/worktree/ad8886/lab-live-e2e
git rev-parse HEAD              # b0ab4ca34fda07c61a6335d05f598eaacc69678c
git status --porcelain -uall    # exactly the five ?? files
# per-file SHA-256 and compact-JSON {base, files} snapshot recomputed: all match
find testing -name __pycache__  # none (PYTHONDONTWRITEBYTECODE=1 throughout)
```

## What I ran (Nix, jobs 2, no bytecode)

- `--check-scenario` on both fixtures: `validated`.
- `--self-test`: 15 tests, OK (the three new ones are
  `test_approval_record_binding_and_revocation`,
  `test_actual_approval_and_all_previous_rejections`,
  `test_compact_left_shift_and_effective_clamps`).
- `--validate-only --acceptance` with the retarget fixture and the real
  LP-1/LP-2/L2 records and manifests: `validated`; `preflight.json` now reads
  `profile=self_adjusting_retarget, retarget=true` (item 4 fixed) and all three
  approval records are bound.
- `--derive-retarget-from /tmp/opencode/LP-live-retarget-run6
  --derived-output-dir /tmp/opencode/Fable-LP-live-r2-derived`: exit 0; both
  outputs byte-identical to the implementer's
  (`600e5feb...41b80`, `8effe83d...a8c5`); the historical `summary.json`
  (`93ddfabc...`), `retarget-evidence.json` (`4fdd8756...`), `miner.jsonl`,
  `pool.log` and `observations.jsonl` are unchanged before and after.
- Direct probes of `approval_verdicts` / `approval_allows_snapshot` on all
  eight `2026-11-05-lab-*-fable-review.md` records and on synthetic fixtures
  (below).

The first-pass evidence verification (83 settled blocks, finding-slice
exclusion, payouts, light proofs, settlement depth, 73-header oracle, baseline
rerun) is unaffected by this revision: the live path changed only in names,
strings, metadata sources, the approval parser, the oracle clamps/interval
derivation and the compact encoder's `size <= 3` branch, none of which touch a
value I verified. The derive step's own replay reproduces the three boundary
results and the light-proof/payout amounts from the frozen artifacts.

## B1 verification

Real records (what each pass binds to; `accepted` is the set of all 64-hex
strings in the file that the parser accepts):

```text
lab-lp1  REQUEST_CHANGES 81fa17d2 | REQUEST_CHANGES 69ac72b6 | APPROVE c16032a7   accepted: [c16032a7]
lab-lp2  REQUEST_CHANGES af531370 | bcb9b342 | 6e2eb2aa      | APPROVE 5e27e918   accepted: [5e27e918]
lab-l2   REQUEST_CHANGES 646656a8 | APPROVE d2f37b91                              accepted: [d2f37b91]
lab-lp1a APPROVE 45df0eb3                                                         accepted: [45df0eb3]
lab-l1, lab-l3, lab-lp3, lab-lp4a                                                  accepted: []  (see N3)
```

Synthetic cases:

```text
rejected A then approved B ................................ A refused, B accepted
approved B then later REQUEST_CHANGES on B ................ B refused (revocation)
REQUEST_CHANGES on A, later APPROVE on the same A ......... A refused (permanent veto; N3)
APPROVE pass whose body also contains **REQUEST_CHANGES** .. refused (ambiguous kind)
8-char prefix only, no full hash anywhere ................. refused
two full hashes on one "tied to snapshot" line ............ refused
two verdict headings in one pass, same hash ............... refused
APPROVE B with an unrelated bare-line hash A in the pass ... both refused (ambiguous)
lowercase "# first pass" headings (no pass split) ......... refused
APPROVE with prefix in verdict + ONE unrelated bare-line
  hash C in the same pass ................................. C ACCEPTED  (N1)
```

The key property holds: old rejected A plus approved B never accepts A, on
the real records and in fixtures. The pool is now also pinned to `c16032a7`
as a second factor, so for the executed runs the gate is sound twice over.

### N3: permanent veto is a documented conservative limitation, not a bug

`approval_allows_snapshot` requires exactly one verdict naming the hash and
that it be `APPROVE`; a later APPROVE of an identical hash after an earlier
REQUEST_CHANGES is therefore refused. The report states this
("any explicit rejection of the same hash is a veto even when a different pass
approves it"). The scenario is realistic (the L2 first pass was a
REQUEST_CHANGES pending an external gate on the same bytes), so a reviewer who
later approves unchanged bytes will have to issue the approval in a new record
or the harness will need an explicit supersession marker. That is the right
side to fail on; no change required now. Likewise L3's passes carry both
`## Result` and `## Verdict` and are parsed as ambiguous, and LP-3/LP-4a
approvals cite only prefixes; both are refused. None of those is an input to
this harness; the limitation is already stated ("missing/multiple verdicts ...
fail closed").

## B2 verification

- Causal language: no occurrence of "responsive", "hashrate-response" or
  "responded" remains in the five files; `retarget.json` uses
  `first_full_period_height`/`second_full_period_height`; the predicates at
  `regtest-lab-v2-e2e.py:1887-1891` are named `first_full_period_unchanged` /
  `second_full_period_unchanged` with descriptive strings; the evidence block
  carries `causal_attribution: "not established by this evidence block"`.
- Per-phase counts are assigned from each miner event's `occurred_at_wall_ms`
  against the recorded `retarget_phase_boundary` observation timestamps
  (`mine_phase_duty_counts`, lines 1988-2017). My first-pass counts used the
  `workers_resumed` timestamps (~0.1-0.5 s later), which explains the small
  differences (for example 1387 vs 1379 ACKs in the coverage phase, 730 vs 731
  block-qualified in the changed period). Totals agree exactly: 163809 /
  4577 / 120052 / 2529. ACKs are reported as miner-side ingress ACKs, not
  equated with accounted or canonical; the derived file labels them as such.
- Work rates from consecutive height snapshots (same worker ids checked):
  273073/273128 H/s over 220.8 s, then 191007/190100 H/s over 267.6-268.6 s;
  matches my first-pass figures.
- The 28 `share_outcome_unknown` pool events are parsed from `pool.log` and
  retained with reason text ("front outcome timed out or was lost"), classified
  unknown.
- Originals unchanged; outputs labelled `derived_from_historical_artifacts`,
  with the generating script hash and an input manifest, and the derive step
  re-hashes every input after writing.

## Blocking finding

### C1 (Medium, small) "Canonical" per-phase block counts are constants and do not match the node evidence

- Files: `regtest-lab-v2-e2e.py:2057-2063` (`canonical_period_counts()`
  returns `{24, 23, 23, 3}`), `2130-2132` (written into every phase as
  `canonical_blocks_in_following_retarget_interval`), `2126-2145`
  (`retarget_duty_cycle`, which the **live** path calls at line 1896 and
  stores in `retarget_evidence` / the public summary), `2175-2182` and `2209`
  (`reference_counts`, my first-pass numbers hard-coded as
  `fable_review_reference_phase_counts`); report lines 147-151 ("canonical
  retarget intervals" column, including `3` for "other").
- What fails. Classifying the 73 `submitblock ok=true` records in
  `node-api.jsonl` by the same `retarget_phase_boundary` timestamps gives
  `coverage 24, live 24, changed 24, other 1`. The retarget arithmetic
  periods (`0..23`, `24..47`, `48..71`) each contain 23 inter-block intervals.
  The constants `24/23/23/3` are neither; the `3` has no source. Because the
  live path emits them, a future retarget run with a different chain would
  publish the same constants as "canonical" evidence. The derived artifact
  cited by hash in the report (`600e5feb...`) therefore carries an incorrect
  canonical column next to the very counts that B2 is about.
- Concrete change. Derive both quantities and name them distinctly:
  `canonical_blocks_by_phase` from `node-api.jsonl` (`method == "submitblock"`,
  `ok is True`, `ts` classified with `retarget_phase`), asserting the total
  equals the header count minus one; and `retarget_period_intervals` =
  `difficulty_adjustment_interval - 1` per full period, taken from the
  effective profile, not a literal. Delete `canonical_period_counts()` and
  `reference_counts` / `fable_review_reference_phase_counts` from the code
  (the comparison with my first-pass counts belongs in the report, where it
  already is); the harness must not embed one run's numbers. Re-run
  `--derive-retarget-from` on run6 into a new directory and update the two
  artifact hashes in the report; replace the report table's figures with the
  harness-derived ones (49277/1387/34360/787, 59049/1374/38272/926,
  49900/1640/44512/730, other 5583/176/2908/86) and the derived canonical
  column, keeping my first-pass numbers only as the cross-check sentence.
- Acceptance expectation. A self-test with a synthetic `node-api.jsonl` and
  observation boundaries that yields a non-uniform split; `--derive-retarget-from`
  on run6 reports `24/24/24/1`; no literal phase counts remain in the driver.

## Non-blocking findings

- **N1 (Should fix)** `approval_verdicts` line 730-731: a bare 64-hex line
  anywhere in an APPROVE pass is a snapshot candidate. If the verdict cites only
  a prefix and the pass contains exactly one unrelated bare hash (for example a
  manifest file hash in a code block), the approval binds to that hash. Not
  exploitable without a SHA-256 preimage and further guarded by the pinned
  constants, but semantically unsound. Fix: when the verdict line carries a
  short prefix (`tied to snapshot \`[0-9a-f]{7,16}\``), require the bound full
  hash to start with it; otherwise require the full hash on the verdict line or
  a `snapshot_sha256` association. Add the case to the fixture test.
- **N2 (Nit)** `retarget_phase` line 1962 is unreachable. `mine_phase_duty_counts`
  reads the whole `miner.jsonl` (300 MB here) with `read_text().splitlines()`
  inside the live path; stream it line by line.
- **N3** Documented limitation, see above; no change required.
- **N4 (Nit)** `require_timespan` rejects a timespan not divisible by four;
  Core has no such rule (integer division). Harmless for this profile; say so
  in the docstring or drop the check.
- Item 3 wording (report lines 55-70) is now precise and matches what I
  established in the first pass (baseline node: clean tree at `e8cf9c9d` plus
  mtime ordering, no build log; retarget node: no-relink check against frozen
  sources). Items 5, 6, 7, 8: verified in code (`expected_retarget_bits`
  derives interval and clamps from the profile and the new 8-block/400-s vector
  exercises them; `encode_compact_target(0x1234) == 0x02123400`;
  `miner_job_number` decimal vs `pool_job_number` strict 8-lowercase-hex with
  the `"12345678"` disagreement test; fees=0 disclosure; run-scoped
  provenance log path).

## Re-review procedure

Return a new manifest with the same recipe. I will verify hashes, run
`--self-test` and both `--check-scenario` calls, re-run
`--derive-retarget-from` on run6 and compare the two artifact hashes with the
report, check that `24/24/24/1` and the interval figure are derived, and grep
for literal phase counts. No live run is required.

## Verdict (second pass)

REQUEST_CHANGES on snapshot
`a3e2117be3fb26487242a0f0e6b14f79c271c5d39b0565d84ce528f996dcb79d`. One
blocker, C1: derive the per-phase canonical block counts and retarget-interval
counts from node evidence and the profile, remove the hard-coded run numbers
from the driver, re-derive the two artifacts and update the report. N1
recommended, N2/N4 optional. B1, B2 and items 3-8 are otherwise resolved. No
candidate file, other worktree or commit was touched; my scratch outputs are
`/tmp/opencode/Fable-LP-live-r2-validate` and
`/tmp/opencode/Fable-LP-live-r2-derived{,.log}`.

---

# Third pass (same day, same reviewer)

Snapshot under review:
`aa3b37b67960071ec6bd4f09c7cd0ac83e1b526404231994c42dc331dc44ded2`
(`/tmp/opencode/lab-live-review-manifest.json`, file SHA-256
`4aa10e605e8815358cddfe61ece7e05adb939802fca76ae8f197547ee99326d0`, base
`b0ab4ca`, the same five uncommitted files; `baseline.json` and
`retarget.json` unchanged from the second pass, driver/oracle/report revised).
Scope: C1, the folded N1/N2/N4, and a regression check that B1, B2 and
items 3-8 stayed resolved.

## Result

**APPROVE - bounded LP-live**, tied to snapshot `aa3b37b6...ded2`.

C1 is resolved by derivation from node evidence, N1 is fixed, N4 is fixed, N2
is fixed in substance (streaming read; one dead `return` remains, cosmetic).
Nothing resolved in passes one and two was reopened. The approval covers: the
harness and oracle as a reusable regtest acceptance gate for the v2
`pool_template_exact_owed_pre_ladder` profile on frozen, Fable-approved
LP-1/LP-2/L2 inputs; the two frozen live runs (`LP-live-baseline-verified`,
`LP-live-retarget-run6`) and my independent baseline rerun as evidence of real
native-SV2 work, node-canonical blocks, exact frozen-window payouts, settlement
at the configured depth, and scanner recovery; and the retarget run as evidence
of correct effective-profile boundary arithmetic in both directions. It does not
cover, and the packet does not claim, any ladder/witness/Ark/type-32/interop
capability, any causal budget-to-block-pace relation, a 10 s block-time model,
live fee-term coverage (all 83 settled blocks had `fees = 0`), or the LP-1/LP-2
stale-work pipeline behaviour (owned elsewhere; separate future gate). Any
material change to these five files after this snapshot requires another
review.

## Snapshot verification

```sh
cd /home/ethan/.local/share/opencode/worktree/ad8886/lab-live-e2e
git rev-parse HEAD              # b0ab4ca34fda07c61a6335d05f598eaacc69678c
git status --porcelain -uall    # exactly the five ?? files
# five per-file SHA-256 and the compact-JSON {base, sorted files} snapshot recomputed: all match
find testing -name __pycache__  # none
```

## What I ran (Nix, jobs 2, `PYTHONDONTWRITEBYTECODE=1`)

- `--check-scenario` on both fixtures: `validated`.
- `--self-test`: 17 tests, OK. New since pass two:
  `test_approval_prefix_does_not_match_unrelated_full_hash` (N1) and
  `test_canonical_evidence_join_and_nonuniform_intervals` (C1), the latter
  building a fixture from run6's real `node-api.jsonl` plus an injected
  duplicate, a rejected (`ok=false`) and an unparseable submission, and
  asserting `73` accepted, `24/24/24/1`, the three negative classifications,
  `76` total records, and the tail at height 73. It skips when the frozen run6
  artifact is absent, which is acceptable for a historical-fixture test.
- `--derive-retarget-from /tmp/opencode/LP-live-retarget-run6` into a fresh
  directory: exit 0; both outputs byte-identical to the implementer's
  (`c4ed5f87...5845`, `4e4e5896...8593`); run6's `summary.json`,
  `retarget-evidence.json`, `node.log` and `node-api.jsonl` unchanged.
- Direct probes of the approval parser on all eight real records and on the
  pass-two synthetic cases (below).

## C1 verification

- `canonical_period_counts()`, `reference_counts` and
  `fable_review_reference_phase_counts` are gone; `grep` for the literals
  `24, 23`, `49277`, `1387` in the driver finds nothing.
- `retarget_canonical_evidence` (driver 2152-2224) classifies every
  `submitblock` API record: parse the 80-byte header from `params` (handling
  the api log's ellipsis truncation), SHA256d it, then require `ok is True`
  (else `rejected_submission_not_counted`), first occurrence
  (`duplicate_submission_not_recounted`), a join to an observed node best-tip
  record (`node.log` "tip: best=<hash> height=<n>", 74 lines for heights
  0..73) or to the independently re-hashed `retarget-evidence.headers`
  (`unknown_missing_canonical_join` otherwise), no competing hash at that
  height (`unknown_reorg_or_conflicting_tip`), header bytes equal to the
  recorded header where available, and a parent join with
  `parent.height + 1 == tip.height` (`unknown_missing_parent_join`). Only
  `observed_canonical_best_tip` increments a phase. This anchors the count to
  the node's own tip observations and the header chain, not to RPC success.
  The scope is "best tip as observed at the time"; conflicts are retained,
  not resolved, which is the honest treatment for a historical record.
- Output on run6: `canonical_blocks_by_phase = 24/24/24/1`, heights
  `1-24 / 25-48 / 49-72 / 73` by wall-time phase (my own classification of
  the 73 `submitblock ok` timestamps in pass two gave the same split);
  `total_accepted_canonical = 73 = total_submitblock_records`;
  `observed_non_genesis_tip_heights = 73`; zero conflicts; all 73 records
  classified `observed_canonical_best_tip`.
- `retarget_period_intervals` derives `(0..23, 24..47, 48..71)` with 23
  height differences each from the effective
  `difficulty_adjustment_interval` and the recorded boundary heights, and
  labels the count kind as distinct from wall-time phase counts. Boundary
  inclusivity is therefore explicit and data-driven.
- The report table (lines 150-160) now shows the driver's own numbers
  (49277/1387/34360/787/24, 59049/1374/38272/926/24,
  49900/1640/44512/730/24, 5583/176/2908/86/1) and explains the tail block;
  my first-pass figures appear only as the cross-check they were.

## N1 / N2 / N4 verification

- N1: a bare hash line now counts only with the `(matches)` suffix and, when
  the verdict cites a prefix, only if it starts with that prefix; a bare
  unrelated hash alongside a prefixed verdict is refused (probe: `False` for
  both the mismatching and the matching-prefix-but-no-`(matches)` cases).
  Real records: LP-1, LP-2, L2 and LP-1a each accept exactly their approved
  hash; L1, L3, LP-3, LP-4a accept nothing (documented fail-closed
  limitation). "A rejected, B approved" still refuses A and accepts B; the
  permanent veto on an identical hash is unchanged and documented.
- N2: `mine_phase_duty_counts` streams the JSONL (`map(json.loads,
  miner_path.open(...))`). The unreachable `return` at driver line 1966 is
  still present; cosmetic, no behaviour.
- N4: `require_timespan` now requires only a positive timespan, with a
  comment that Core has no divisibility rule.

## Regression check on passes one and two

Re-read for drift: approval parser and its two real-record tests; the
retarget predicates and strings (descriptive only, no "responsive"/"caused"
language in any of the five files); `expected_retarget_bits` (profile-derived
interval and clamps, Core `GetCompact` including the `size <= 3` left shift);
`miner_job_number` / `pool_job_number`; scenario-derived `run_created` and
preflight metadata; run-scoped provenance log; build-association wording;
fees-zero disclosure; the 28 unknown-outcome events retained. Unchanged and
still correct. The live acceptance path differs from the first-pass driver
only in names, strings, metadata sources, the approval parser, the oracle
clamp/interval derivation, the compact encoder's small-target branch and the
added evidence derivation; none of these touches a value in my first-pass
verification of the 83 settled blocks, the 73-header oracle, the light proofs,
settlement depth, or my baseline rerun, so that evidence stands for this
snapshot.

## Notes for the merge session (not conditions of approval)

- Remove the dead `return` at driver line 1966 when convenient.
- `test_canonical_evidence_join_and_nonuniform_intervals` reads the frozen
  run6 artifact from `/tmp/opencode`; if the suite is to run on another host,
  commit a small redacted fixture (headers, `node.log` tip lines, a handful of
  `submitblock` rows with `params` truncated to the header) instead.
- `docs/REGTEST-LAB-STATUS.md` and `docs/MERGE-QUEUE.md` rows for LP-live
  should record this approval and snapshot `aa3b37b6`.
- The stale-work observation (73% stale shares, ~3% of block solutions
  canonical, ~10 s pace independent of budget) is now the runtime owner's
  packet; it is not a defect of this harness.

## Verdict (third pass)

APPROVE on snapshot
`aa3b37b67960071ec6bd4f09c7cd0ac83e1b526404231994c42dc331dc44ded2`, bounded
as stated in the Result. No remaining blockers. No candidate file, other
worktree or commit was touched across all three passes; the only file written
is this review. Scratch outputs from this pass:
`/tmp/opencode/Fable-LP-live-r3-derived{,.log}`.
