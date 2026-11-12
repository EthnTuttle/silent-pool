# L1 native SV2 conformance - Fable approval gate review

Date: 2026-11-05
Reviewer: Fable (normative + key/security gate), subagent
Packet: L1 `lab-sv2-conformance`, base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`
Candidate report: `docs/reviews/2026-11-05-lab-l1-implementation.md` (in the worktree)
Snapshot: `/tmp/opencode/lab-l1-review-manifest.json`,
aggregate SHA256 `c3bad8d05a224398c20871980a5b3a9943b6661167c07af5bfc0ecbea0c22a1d`

## Verdict: REQUEST_CHANGES (bounded)

Three bounded fixes are required before approval (R1-R3 below). Everything
else in the packet checks out: the independent evidence is real, the merkle
order, LE256 targets, SetTarget lifecycle, authority provisioning, exact
extranonce handling, front header revalidation and correlated SV2 replies are
implemented as claimed and tested against upstream `channels_sv2` state. The
required changes are all in the "same-tip job republication" area: one new
normative rule is wrong for SV2 (nbits cannot ride a nonfuture job), one is
operationally harmful (curtime in the job identity at a 1 s refresh), and one
SV1 flag was flipped into incoherence with the latest-only stale policy.

Approval is tied to the exact snapshot hash above. Any source change produces a
new manifest and requires re-review of the changed files; do not treat this
review as covering a different tree.

## 1. Snapshot verification

- Recomputed `hashlib.sha256(json.dumps({'base': m['base'], 'files': m['files']}, sort_keys=True, separators=(',', ':')).encode()).hexdigest()`
  over the manifest: `c3bad8d0...22a1d`, equal to `snapshot_sha256`.
- All 26 per-file SHA256 values match the worktree contents (checked at the
  start and again at the end of the review; no drift).
- `git status` in the worktree shows exactly the 22 modified and 4 untracked
  files listed in the manifest; HEAD is `b0ab4ca`. No commits were made.
- Cargo.lock diff is one line: `channels_sv2` added to the `spc-pool` package
  dependency list. `cargo tree -e normal -i channels_sv2` shows `channels_sv2 8.0.0`
  was already in the normal graph via `stratum-apps 0.8.0`; the new entry is
  `[dev-dependencies]` only (`cargo tree -e dev`). No pin bump.

## 2. Evidence verified

Recorded logs under `/tmp/opencode/l1-*` were read, not trusted by summary:

| Claim | Log | Verified |
|---|---|---|
| Workspace 442 passed / 0 failed | `l1-final-workspace-tests.log` | Summed `test result:` lines: 442 passed, 0 failed. Includes 207 pool lib, 39 agent lib, `sv2_conformance` 3, `e2e_sv2` 2, `e2e_sv2_client` 2, `e2e_translator` 1, `e2e_sv1` 1. |
| `sv2_conformance` 3/3 | `l1-conformance-green.log` | 3 passed. |
| `e2e_sv2_client` 2/2 | `l1-agent-e2e.log` | 2 passed incl. `noise_authority_pin_survives_restart_and_rejects_a_different_key`. |
| fmt / clippy / check | `l1-final-fmt-check.log`, `l1-final-clippy.log`, `l1-final-check.log` | Clean; only the pre-existing cargo-cache permission warnings. |
| Regtest smoke | `l1-regtest-e2e.log` | `OK: regtest e2e passed`, blocks 101 -> 103, `spc-scan: 1 output(s) found`. Uses `testing/regtest-miner.py` (synthetic SV1 miner), so it does not exercise the native SV2 ack path in `main.rs`. |
| Red-first evidence | `l1-red-merkle.log`, `l1-red-target.log`, `l1-red-job.log` | Present; failures match the stated reasons. |

Independent re-runs by this reviewer (`nix develop`, `CARGO_BUILD_JOBS=2`),
logs in `/tmp/opencode/Fable-L1-*`:

- `cargo test -p spc-pool --test sv2_conformance --test e2e_sv2 --test e2e_sv1`: 3 + 2 + 1 passed (`Fable-L1-pool-integration.log`).
- `cargo test -p spc-agent --test e2e_sv2_client --test e2e_translator`: 2 + 1 passed (`Fable-L1-agent-e2e.log`).
- `cargo test -p spc-agent --lib -- sv2_client config::tests`: 17 passed (`Fable-L1-agent.log`).
- `cargo test -p spc-pool --lib -- block:: sv1::tests::deep sv2_server:: front::tests::header_only front::tests::forged front::tests::a_fully config::tests::front_authority`: 17 passed (`Fable-L1-pool-lib.log`).
- `cargo tree` dev/normal edge checks (`Fable-L1-tree-*.log`).

Conformance test quality (`bins/spc-pool/tests/sv2_conformance.rs`): it drives
upstream `channels_sv2::client::extended::ExtendedChannel` from raw wire
frames and asserts upstream state (future count, active job target, past and
stale counts), not the in-repo agent. It covers: pre-template open, first
SetTarget before the future job, future job -> SetNewPrevHash activation,
same-tip nonfuture job with no extra prevhash, SetTarget before a retargeted
nonfuture job with the old active job keeping its target, tip change
producing exactly future + prevhash, negotiation rejections, second-channel
rejection, distinct prefixes across connections, and `bad-extranonce-size`
rejected before any queue insertion. Target asymmetry (regtest network target
easier than difficulty 1, decoded independently with
`Target::from_le_bytes` / `Target::from_compact`) is asserted. The nonempty
two-level merkle check against upstream is in
`bins/spc-pool/src/block.rs` (`coinbase_branch_is_deepest_first_upstream_vector`,
via `channels_sv2::merkle_root::merkle_root_from_path_`) plus the SV1 PoW
vector and the agent fold test; it is not driven through the ExtendedChannel
wire path (see N10).

## 3. Required changes (ranked, bounded)

### R1 (high) - curtime must not be part of the job identity

- `bins/spc-pool/src/front.rs:561-568` adds `previous.curtime != template.curtime`
  to `job_changed`; `bins/spc-pool/src/main.rs:883` adds `ntime` to `Published`.
- `bins/spc-pool/src/template_rpc.rs:96` sets `curtime` from
  `getblocktemplate.curtime`, which bitcoind recomputes on every call
  (`UpdateTime`). `config.rs:322` defaults `refresh_interval_ms` to 1000.
- Consequence on any live GBT source: a new job id every refresh tick (~1 Hz),
  broadcast to every SV1/SV2 miner, plus `publish_jd` per tick
  (`main.rs:963`). Under the packet's own latest-only rule
  (`sv2_server.rs:633`, `sv1.rs:410`, front `StaleJob`) every share in flight
  at each tick is rejected as stale. For CPU-scale regtest this is invisible
  (shares are found in milliseconds); for any real miner it is a stale rate of
  roughly (one-way latency + job switch time) / 1 s, and the SV2 nonfuture job
  path gives upstream clients no stale marking at all, so they keep submitting
  on the old job and collect `stale-share`.
- Counterexample already in the tree: `front::tests::header_only_template_changes_bump_the_job`,
  case `"time"`, asserts that a `curtime + 1` template republishes the job.
- Nothing requires it: the pool accepts `ntime` within +/- 7200 s of the job
  (`sv1.rs:143,152`), and SV2 `min_ntime` only lower-bounds share time.
- Fix: drop `curtime` from `job_changed` and `ntime` from `Published`; keep
  `bits` and `version`. Flip the `"time"` case to assert no republish. Edit
  decision record items 1 and 5 and the changelog fragment accordingly
  ("bits and version", not "bits, version or curtime").

### R2 (high) - a same-tip nbits change cannot be delivered by a nonfuture job

- Decision item 1 and `sv2_server.rs:574-611`: `tip_changed` keys on
  `prev_hash` only, so a bits change at the same prevhash sends SetTarget and a
  nonfuture `NewExtendedMiningJob`, no `SetNewPrevHash`.
- SV2 carries `nbits` only in `SetNewPrevHash`. Upstream `channels_sv2 8.0.0`
  builds and validates headers from `chain_tip.nbits()`
  (`channels_sv2-8.0.0/src/client/extended.rs:873-879`), and the in-repo agent
  does the same (`bins/spc-agent/src/sv2_client.rs:167-181`, `nbits: tip.nbits`).
  The pool revalidates with `job.nbits` (`sv2_server.rs:388`, `front.rs`
  `revalidate_share`). After a same-tip bits change every SV2 miner therefore
  hashes a header the pool will never accept until the next tip.
- The conformance test's "same-tip nbits update" case passes because it only
  checks the *target* lifecycle; it never compares the upstream chain tip's
  nbits with the pool job's nbits or validates a share.
- Reachability: never on regtest/signet, only at retarget boundaries (which
  coincide with tip changes) on mainnet, but routinely on testnet3/testnet4
  (20-minute minimum-difficulty rule makes nbits depend on the template
  timestamp at a fixed prevhash). SPEC §6.3's example network is testnet4.
- Fix: treat `(prev_hash, nbits)` as the activation key in `send_job` (store
  `nbits` on `Channel`), so a bits change sends future job + `SetNewPrevHash`
  with the same `prev_hash` and the new `nbits`; upstream `on_set_new_prev_hash`
  has no same-prevhash check and will also mark the old job stale, which makes
  the latest-only rejection coherent for that case. Reword decision item 1.
  Extend the conformance case to assert `Wire::Future` then `Wire::PrevHash`
  and that the activated job's header nbits equals the pool job's nbits (or
  build a share through upstream and assert the pool accepts it).

### R3 (medium) - SV1 `clean_jobs` is now incoherent with latest-only

- `bins/spc-pool/src/front.rs:705` changes `clean_jobs: true` to
  `clean_jobs: chain_moved`. `bins/spc-pool/src/sv1.rs:410` still rejects any
  non-current job id as `stale job`, and `sv1.rs:292` only clears the
  per-session replay set on clean jobs.
- Result: on every same-tip republish SV1 miners are told they may finish
  current work, then every such share is rejected. The base behaviour
  (`clean_jobs: true`) matched the latest-only rule; this packet broke the
  match while the decision record describes latest-only as the explicit
  policy. Combined with R1 this would fire every second.
- Fix: either keep `clean_jobs: true` (minimal, consistent with the recorded
  decision) or accept the immediately previous same-tip job in `sv1.rs`,
  `sv2_server.rs` and the front (not claimed by this packet; do not do it
  here). Update `header_only_template_changes_bump_the_job`, which asserts
  `!republished.clean_jobs`.

R1-R3 are small edits in `front.rs`, `sv2_server.rs`, `main.rs`, the two
tests named above, the decision record and the changelog fragment. They do not
touch share work units, windows, payout, Ark or L3 scope.

## 4. Non-blocking findings (fix when convenient; none gate approval)

- N1 `bins/spc-pool/src/config.rs:90-101`: `Pool` derives `Debug` with the
  plaintext `authority_secret_key: String`. No current print path exists
  (grepped `main.rs`, `api*.rs`, agent; `Config` is `Deserialize` only and
  `pool_info` exposes explicit fields), so this is not a leak today, but a
  redacting newtype (`Debug` prints `[redacted]`) is cheap insurance.
  `Sv2Server` has no `Debug` derive (good).
- N2 Secret storage convention: other secrets use `*_path` files with mode
  checks (`[ark] order_key_path`, `seed_path`); the Noise secret is inline
  TOML. `docs/OPERATIONS.md:173` still says the pair lives only in front RAM.
  The candidate report flags both; the OPERATIONS row must be reconciled at
  merge and a `_path` variant is a reasonable follow-up.
- N3 `bins/spc-pool/src/main.rs:405-417`: every `Share` on `rx`, including
  SV1 shares, is resolved against the SV2 outcome hub. SV1 channel ids are
  random `u32` (`sv1_server.rs:180`) and SV2 ids are small sequential values,
  so a collision with a live `(channel_id, sequence)` is ~live_channels / 2^32
  per SV1 session and then needs a sequence match. Tag the ingress on `Share`
  or only resolve ids the SV2 server allocated.
- N4 `sv2_server.rs:304-318`: before any template, the effective target
  placeholder is `0xff..ff`, so a pre-template open is accepted only when the
  miner's `max_target` is also all-ones. The conformance test's
  `Target::MAX -> max-target-out-of-range` case passes for this reason, not
  because of the share target (no job is published in that test). The
  in-repo agent sends all-ones, so it is unaffected. Either document this or
  floor the placeholder at `target_for_difficulty(share_difficulty)`.
- N5 `sv2_server.rs:432-444`: the front outcome is awaited inline inside the
  connection `select!`, so each connection has one share in flight and does
  not read frames or jobs for up to 10 s. Fine for the lab; a real miner
  pipeline needs an out-of-line response path. On timeout the miner is told
  `invalid-share` while the front may still credit the share; the
  accounting side is unaffected and fail-closed for the miner.
- N6 `main.rs:405-407`: `credited_work` is recomputed with
  `credited_difficulty(share_difficulty, job.nbits)` rather than returned by
  `Front::on_share` as the `d` it booked (`front.rs:596,795`). Coherent today
  because the same config value feeds both; returning the booked `d` would
  remove the duplication.
- N7 The `main.rs` SV2 ack wiring (`front_error_code`, `credited_work`,
  resolution before block submission) has no automated test: the three
  integration tests call `resolve_share(1, 0, ...)` by hand and the regtest
  smoke uses the SV1 python miner. Acceptable for bounded L1; it is an
  LP1/LP2 condition (see section 6).
- N8 Pre-existing, not introduced here: the config does not require
  `extranonce1_bytes + extranonce2_bytes == 8`, but the SV2 listener requires
  prefix 4 + miner 4 (`sv2_server.rs:297-303`); a valid SV1 layout such as
  4 + 8 makes every SV2 open fail at runtime with
  `min-extranonce-size-too-large` instead of at startup.
- N9 `sv2_server.rs:233,304-339,463-469`: `jobs.subscribe()` happens before
  the open; a job published between subscribe and open is sent at open as
  future + prevhash and again from the broadcast as a nonfuture job with the
  same id. Benign duplicate; noted.
- N10 The conformance test templates have no transactions; multi-level merkle
  coverage against upstream is via `merkle_root_from_path_` in `block.rs`,
  the SV1 vector and the agent fold test. Adding a 2-tx template to the
  ExtendedChannel-driven test would close the wire-path gap.
- N11 `bins/spc-agent/src/sv2_client.rs:152`: `self.future.take().filter(..)`
  drops a queued future job whose id does not match the prevhash. Fine while
  the pool sends one future job per tip; upstream keeps a map.

## 5. Key and privacy review

- Authority provisioning: `config.rs:601-656` requires a non-empty, parseable
  `authority_secret_key` and a matching `authority_public_key` when the
  `front` role is selected; mismatch, missing and malformed cases are tested
  (`front_authority_is_provisioned_and_matches_its_pin`), and a payout-only
  config validates with both empty. `main.rs:281-291` always calls
  `with_authority_secret_key` for the front; the ephemeral key in
  `Sv2Server::new` is reachable only from test constructors. Coherent.
- Agent pinning: `sv2_client.rs:301` uses `connect_with_noise(stream, Some(pin))`;
  `translator.rs` parses the pin from config and fails startup if it is
  invalid (`config.rs` `upstream_authority`). There is no unauthenticated
  fallback. The wrong-key test uses a freshly generated ephemeral key as the
  wrong pin and asserts handshake failure; the restart test rebuilds the
  server from the same secret and reconnects with the old pin.
- Fixture keys: `9auqWEzQ...PH72` / `mkDLTBBR...Srn2n` is the widely published
  SRI example pair. It is a valid pair (the config validation test proves
  secret -> public match) and appears only in example configs, test fixtures
  and the four regtest scripts, each annotated as throwaway. It is not a
  generated production key and the examples say to replace both values.
  Recommendation (not blocking): refuse this specific fixture pair when
  `network != regtest`, since mainnet is already refused but testnet4 is not.
- No secret logging: grepped for `{:?}`, `?config`, `authority_secret`,
  `authority_sk` in pool and agent sources; the secret is only parsed and
  passed to `with_authority_secret_key`. No tracing of it. `tracing` in the
  translator logs sequence numbers and error codes only.
- Privacy boundary: this packet does not touch the `ark` role or its streams.
  `Share` still carries `sp_id`, `b_scan`, `b_spend` from ingress to the
  front only, unchanged from base. Nothing new crosses into `ark` (SPEC §11.3).
- No regtest bypass in production paths: the only test-scoped switch is
  `#[cfg(test)] revalidate_headers` in `front.rs:188-190`, which is compiled
  out of integration tests and binaries (`#[cfg(not(test))]` forces `true`).
- Conformance test clients connect with `None` as the pin; acceptable in a
  test, the server still runs the full handshake with its authority.

## 6. Remaining integration conditions (not completed by this packet)

These are stated correctly as open in the candidate report and remain open:

1. Native SV2 end-to-end against a node (rbitcoin or bitcoind) with a real
   SV2 miner through `main.rs`, including non-block shares and the correlated
   `SubmitSharesSuccess` / `new_shares_sum`: not run. The regtest smoke is SV1
   only. LP1/LP2 gate.
2. Non-empty multi-transaction template through the full node path with the
   corrected deepest-first order: not run here; the unit/upstream vectors
   cover the fold, not the node.
3. `sv2-tp` branch merge: verified on `sv2-tp` at `1ec1dd0`,
   `bins/spc-pool/src/template_tp.rs:495` still reverses TDP's deepest-first
   path (`cached.merkle_path.iter().rev()`). The merge must replace it with a
   plain clone, update its expected-path tests, and add the fixture pair to
   its `testing/regtest-rbitcoin-tp-e2e.sh` heredoc. Not merged in this
   review, as instructed.
4. SPEC splice: no SPEC frontmatter edit in the packet, by design
   (`docs/MERGE-QUEUE.md` changelog fragments). The fragment must be revised
   with R1/R2 before splicing. §6.3 config prose and `docs/OPERATIONS.md:173`
   need the `authority_secret_key` addition.
5. JDS and node-TDP authority provisioning, key generation/rotation tooling
   and a file-based secret store remain follow-ups.
6. SV1 submit boolean remains an ingress acknowledgement, not credit. Correct
   and explicit; do not relabel.
7. Latest-only stale policy is an explicit v1 restriction; a bounded same-tip
   history is future coordinated work, not claimed.

## 7. What this review did not do

No candidate files were edited, no commits or pushes were made, no subagents
were spawned, and no other worker branch was merged. The only file written is
this review in the main checkout.

## 8. Re-review

Resubmit with a new manifest and aggregate hash after R1-R3. Expected delta:
`front.rs`, `sv2_server.rs`, `main.rs`, `tests/sv2_conformance.rs`, the
decision record, the changelog fragment and the implementation report. If the
re-run of `sv2_conformance`, `e2e_sv2`, `e2e_sv2_client`, `e2e_translator`,
the pool/agent lib tests, fmt and clippy is green and the SV1 regtest smoke is
re-run, this reviewer expects to approve on the file-level diff alone.

---

# Second review (re-review after R1-R3) - 2026-11-05

Reviewer: Fable (normative + key/security gate), subagent
Same worktree and base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.
Snapshot: `/tmp/opencode/lab-l1-review-manifest.json`, aggregate SHA256
`fa532607fd20ce8f9dabcae1f552973e3c45c12b99ab7be1eb2f148e36056cd2`.

## Verdict: APPROVE (bounded L1)

R1, R2 and R3 are resolved in code, tests, the decision record and the
changelog fragment; N1 and N10 were also taken. No new regressions found.
Approval is tied to the exact aggregate hash above; any later source change
needs a fresh manifest and a diff-level re-check.

## Snapshot verification

- Recomputed the aggregate with the stated recipe: `fa532607...56cd2`,
  equal to `snapshot_sha256`. All 26 per-file SHA256 values match the worktree
  at the start and at the end of this re-review. Same 22 modified + 4 untracked
  files; HEAD unchanged at `b0ab4ca`; no commits.

## Disposition of first-review findings

| Id | Status | Where verified |
|---|---|---|
| R1 curtime out of job identity | Resolved | `front.rs:561-566` (`job_changed` = chain, fees, template id, bits, version); `main.rs:876-895` `Published` has `version`, `bits`, no `ntime`. Unbumped refreshes keep the issued `ntime` (`front.rs:700-708`). Test `ordinary_curtime_refresh_keeps_the_issued_job_immutable` asserts `refreshed == issued` for `curtime + 1`; red log `l1-r1-red-curtime.log` shows it failing on the old tree with `job_id` and `ntime` changed. |
| R2 same-prevhash nbits needs activation | Resolved | `sv2_server.rs:124,576-621`: `Channel.nbits`, `activation_required = prev_hash changed || nbits changed`; future job + `SetNewPrevHash` carrying the new `nbits`. Conformance test lines 341-362 now expect `Target`, `Future`, `PrevHash` and assert `upstream.get_chain_tip().nbits() == retargeted_bits` and 2 stale jobs; red log `l1-r2-red-nbits.log` shows `Active` vs `Future` on the old tree. Agent takes `nbits` from the prevhash (`sv2_client.rs` `activate`), so it is consistent. |
| R3 SV1 clean_jobs vs latest-only | Resolved | `front.rs:711` `clean_jobs: true`; test `bits_and_version_changes_bump_clean_latest_only_jobs` asserts clean jobs; red log `l1-r3-red-clean.log`. Decision item 1 now states every SV1 republication is clean. |
| N1 Debug redaction | Taken | `config.rs:90-131` `RedactedSecret` (`Deserialize` transparent, no `Serialize`, `Debug` prints `[redacted]`), unit test asserts the redaction. |
| N10 two-tx upstream wire test | Taken | `sv2_conformance.rs:381-473` `upstream_extended_channel_uses_wire_merkle_path_for_a_real_share`: 2-tx template, upstream `merkle_root_from_path` equals an independent three-leaf root, path order asserted as `tx1`, `H(tx2 || tx2)`, and upstream `validate_share` accepts a real nonce. The pool's own acceptance of that same share is not asserted in this test (pool-side fold is covered by `block.rs` upstream vector, SV1 vector and `e2e_sv2`); low, not required. |
| N2-N9, N11 | Open, non-blocking | Unchanged; the implementation report lists N3, N5, N6, N7 and N11 under its deferred LP1/LP2 gates with accurate wording. |

New low note (N12, non-blocking): with curtime no longer bumping jobs, an
issued job's `ntime` is only replaced when the job changes. If a template
stays otherwise identical for longer than `MAX_NTIME_ROLL_SECS` (7200 s),
shares at wall-clock time would fail `ntime_allowed`. Unreachable on a live
chain (blocks rotate jobs) and the regtest miner uses the job's `ntime`; a
future bump when `curtime - issued.ntime` exceeds half the roll window would
close it.

## Evidence verified

Recorded (`/tmp/opencode/l1-r123-*`): final workspace run sums to 444 passed /
0 failed (pool lib 208, agent lib 39, `sv2_conformance` 4, `e2e_sv2` 2,
`e2e_sv1` 1, `e2e_sv2_client` 2, `e2e_translator` 1); clippy `-D warnings`
clean; fmt check clean; SV1 regtest smoke `OK: regtest e2e passed`, blocks
101 -> 103, `spc-scan: 1 output(s) found`. Fail-first logs for R1, R2 and R3
show the intended assertion failures on the previous snapshot.

Independent re-runs by this reviewer (`nix develop`, `CARGO_BUILD_JOBS=2`,
logs `/tmp/opencode/Fable-L1-r2-*`):

- `cargo test -p spc-pool --lib -- front::tests sv1::tests sv2_server::tests block::tests config::tests`: 81 passed.
- `cargo test -p spc-pool --test sv2_conformance --test e2e_sv2 --test e2e_sv1`: 4 + 2 + 1 passed.
- `cargo test -p spc-agent` (all targets): 39 + 2 + 2 + 1 + 1 passed.
- `cargo clippy -p spc-pool -p spc-agent --all-targets -- -D warnings`: clean.
- `cargo fmt --all -- --check`: clean.

The full workspace was not re-run by this reviewer; the recorded 444/0 log was
read and its per-binary totals are consistent with the focused re-runs.

## Key and privacy (delta)

No change to the authority provisioning, pinning or fixture-key posture from
the first review. `RedactedSecret` removes the latent `Debug` exposure. No new
logging of secrets; no identity field crosses into `ark`. Fixture pair remains
the published SRI example pair, annotated throwaway; refusing it on
non-regtest networks is still a recommended follow-up.

## Remaining integration conditions (explicitly not completed)

Unchanged from the first review and correctly labelled as LP1/LP2 gates in the
implementation report:

1. Native SV2 end-to-end through `main.rs` against a node with a real SV2
   miner, non-block shares and correlated `SubmitSharesSuccess` /
   `new_shares_sum`.
2. 10 s in-line acknowledgement timeout semantics and out-of-line response
   handling (N5).
3. Ingress-source tagging so SV1 shares cannot resolve SV2 outcome slots (N3).
4. Multi-transaction template through the node path; `sv2-tp` merge must drop
   `template_tp.rs:495` `.iter().rev()` and add the fixture pair.
5. SPEC splice of the revised changelog fragment and §6.3 / `OPERATIONS.md:173`
   prose; JDS/TDP authority provisioning; key tooling and file-based secret
   storage.
6. SV1 submit boolean remains an ingress acknowledgement only.

## What this review did not do

No candidate edits, commits, pushes or subagents. The only write is this
appended section in the main checkout.
