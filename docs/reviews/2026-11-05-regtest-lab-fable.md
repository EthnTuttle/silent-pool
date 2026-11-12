# Review: docs/REGTEST-LAB.md (regtest network lab proposal)

- Reviewer model: Claude Fable 5.1 (independent review, no other reviewer output read)
- Date: 2026-11-05
- Baseline plan: `docs/REGTEST-LAB.md`, SHA256
  `444060b2c1c7d377960116032dc88af114c0dbc43741a8c719b33188e8b99098`
- silent-pool tree: HEAD `b0ab4ca` plus the staged SV2 TP work (`template_tp.rs`,
  `testing/regtest-rbitcoin-tp-e2e.sh`, decision 2026-11-05-sv2-tp-chain-view)
- rbitcoin tree: `/home/ethan/code/rbitcoin` at `e8cf9c9d6`
- Method: read AGENTS.md, DESIGN.md, SPEC.md (§3.3, §3.4, §4.3, §7.4, §11.3),
  the two cited decision records, ROADMAP Phase 8, MERGE-QUEUE.md; inspected
  every source path in the plan's evidence map plus the upstream `channels_sv2`
  8.0.0 client in the cargo registry. No code was executed; every claim below is
  from reading. Where I say "would", it is an inference from source, not an
  observed run.

## 1. Overall verdict

Direction approved, plan not yet implementable as written. The plan is right on
the load-bearing points: the node owns the block target, pool share target is a
separate policy, automatic retargeting is a node prerequisite, visualization is
decoupled from run lifetime, and the pool-side SV2 path has unverified gaps. It
is honest about what it has not tested.

What it understates or misses:

1. The regtest minimum-difficulty rule is not an "edge case" to test; with the
   inspected rbitcoin TDP it is a hard stall of the pool after any inter-block
   gap longer than `2 x spacing`, and with the RPC template path it produces an
   invalid block from the pool. Both are reachable with the plan's own `pause`
   command. This makes `allow_min_difficulty_blocks = false` (or an equivalent)
   a requirement of every lab profile, and it is not exposed by the node today.
2. Three pool-side SV2 defects are blockers for the proposed native SV2 CPU
   miner, not "gaps to resolve or bound": message order on job delivery is
   incompatible with the upstream client, the announced target is almost
   certainly byte-reversed on the wire, and there is no `SetTarget` at all.
   The in-house client tolerates all three, which is exactly the matching-bug
   risk the plan warns about in P1, already realised.
3. The share-unit question (plan §3.3, Q2) can be closed with a much smaller
   change than "fractional work": make the difficulty-1 base target a declared,
   fingerprinted deployment parameter and keep `d` an integer everywhere. The
   plan should propose that rather than leave it open.
4. The retarget profile needs a concrete list of rbitcoin changes (none of
   `no_pow_retargeting`, `allow_min_difficulty_blocks`, `pow_target_timespan`,
   `pow_target_spacing`, or the subsidy-halving overlay is reachable from
   `rbitcoin-node` config), and the `--regtest-pow-bits` overlay interacts
   badly with retargeting rather than seeding it.

Recommended path: rewrite P0 as two named upstream packets (one rbitcoin, one
pool) with the fixes below as their acceptance, then proceed to P1.

## 2. Ranked findings

Severity scale: High = blocks a stated user goal or produces wrong results
silently; Medium = biases results or blocks a packet gate; Low = quality or
scope. "Plan defect" = the plan's text is wrong or missing an assumption.
"Implementation blocker" = existing code must change before the plan's packet
can pass. "Enhancement" = optional.

### A. Plan defects

#### A1 (High) Minimum-difficulty rule is a stall/invalid-block hazard, not an edge case

- Affected: §3.2 (paragraph on the 1200 s exception), §5.2 `pause`, §8 P0/P4.
- Evidence:
  - `rbitcoin/crates/rbitcoin-consensus/src/header.rs:291-297` (`min_diff_gap`:
    `header_time > prev_time + 2*spacing`), `:332-358` (`min_diff_bits` returns
    `pow_limit` bits when the gap holds), `:305-329` (overlay keeps `pow_limit`
    in that case). Regtest `allow_min_difficulty_blocks` is true by default
    (rust-bitcoin `Params::REGTEST`; `params.rs:71-99` does not override it).
  - TDP path: `rbitcoin/crates/rbitcoin-sv2/src/session.rs:484-490` pins
    `n_bits` once per prev hash ("nBits comes once per prev hash"), and
    `:571-590` drops a `SubmitSolution` whose timestamp "needs different bits"
    (`submit_timestamp_needs_other_bits`). Consequence: once wall time passes
    `tip_time + 2*spacing` with no new block, every solution the pool can
    produce for that tip is dropped at info level; the TP never re-stamps the
    same tip, so the pool cannot advance the chain until something else does.
  - RPC path: `rbitcoin/crates/rbitcoin-sv2/src/template.rs:123-132` (and Core
    GBT) compute bits from the template timestamp, so the template's bits flip
    to `pow_limit` at the same height. The pool's job identity ignores bits
    (`bins/spc-pool/src/front.rs:547-551`: `chain_moved || fees_moved ||
    template_id`), so the SV2 job keeps the old `nbits` while the front tests
    `network_target(template.bits)` and assembles with `template.bits`
    (`front.rs:802, 812`). The share hash was computed over `job.nbits`
    (`sv2_server.rs:298`); the assembled header differs from the hashed one and
    the node rejects it.
- Why it matters for the lab: the PoW baseline at 16 hashes/block never hits
  this. Any CPU-scale target, any `pause` longer than `2 x spacing`, any
  minority partition whose pool stops finding blocks, or a short-spacing
  retarget profile with the exception left on, hits it immediately.
- Suggested fix: (a) every lab profile that can see gaps `> 2 x spacing` MUST
  set `allow_min_difficulty_blocks = false` (the rbitcoin in-process tests
  already do: `rbitcoin-rpc/tests/methods/mod.rs:6331-6332`); (b) the pool
  includes `bits` (and `version`) in job identity so a bits-only template
  change bumps the job; (c) the plan's capability model lists "min-difficulty
  exception off" as a hard preflight requirement, not an open question.
- Falsifiable test: PoW baseline, `--regtest-pow-bits 0x200fffff`, TDP source.
  Mine one block through the pool, then `setmocktime` the node to
  `tip_time + 1300`, resume mining. Prediction under current code: node log
  shows "timestamp needs different bits; dropping" and height does not advance
  within 60 s. After the fix: either the height advances or the pool emits an
  explicit job change with the new bits.

#### A2 (High) `--regtest-pow-bits` is not an initial-difficulty mechanism for a retargeting profile

- Affected: §3.2 ("startup consensus overlay"), §8 P0 "Specify initial
  difficulty/bootstrap behavior".
- Evidence: `header.rs:299-329`. The overlay only replaces bits that equal the
  `pow_limit` bits, and its doc comment assumes "regtest does not retarget".
  With `no_pow_retargeting = false`: height 1 takes the overlay (genesis bits
  equal `pow_limit`), later heights carry the parent's bits, and the first
  boundary retargets from the overlay. But if any retarget clamps to
  `pow_limit`, the boundary block carries `pow_limit` bits (`:322-324`) and the
  very next block snaps back to the overlay (`:318`, `:328`): a one-block
  discontinuity that the declared algorithm does not predict. The overlay is
  therefore a seed with a hidden floor, and the plan's "observed difficulty
  changes originate at nodes and follow the declared algorithm" (P4) would be
  falsified by the node's own code.
- Suggested fix: the retarget profile either forbids the overlay and seeds
  initial difficulty differently (a documented warmup from `pow_limit`, or a new
  explicit "initial bits" parameter applied only at height 1), or rbitcoin
  redefines `apply_regtest_pow_overlay` for the retarget case with a test. The
  plan should say which and record it as part of the consensus fingerprint.
- Falsifiable test (rbitcoin unit test, no network): params with
  `no_pow_retargeting = false`, `allow_min_difficulty_blocks = false`,
  interval 4, overlay set; construct timestamps so the retarget at height 4
  clamps to `pow_limit`; assert `next_work_bits` at height 5. Current code
  returns the overlay; the declared algorithm says `pow_limit`.

#### A3 (High) Share-unit decision left open; the minimal answer is a declared difficulty base, not fractional work

- Affected: §3.3 "Open decision", §9 Q2, §8 P0 second bullet.
- Evidence: `bins/spc-pool/src/sv1.rs:60` fixes `DIFFICULTY_1_TARGET =
  Target::MAX` (mainnet difficulty-1, about 2^224). CPU-scale lab targets are
  2^228 to 2^240 (plan §3.1 arithmetic), so `network_difficulty` (`:94-105`)
  is always `< 1` and rounds to 1, `credited_difficulty` (`:108-110`) is always
  1, and `effective_share_target` (`:77-86`) is always the network target. The
  plan correctly observes that every share is then a block. Decision
  2026-11-02 rejected "chain powLimit-relative difficulty" only in its implicit
  form (mispricing other chains); an explicit, configured base with mainnet
  default avoids that objection.
- Also missing: SPEC §4.3 window is `N x d` with `window_n = 8` default
  (`config.rs:335-337`). When `d` becomes the pool share difficulty rather than
  a block-equivalent, the window is eight shares, which degenerates every
  fairness and pool-share scenario in §7/P4. The plan says "preserve N x d" but
  does not note that `N` must then be sized per scenario.
- Suggested fix: propose `[accounting] difficulty_unit_bits` (compact target
  defining difficulty 1 for this deployment; default `0x1d00ffff`). Keep `d`
  as `u64`, keep `N x d`, keep all persistence. Record the unit in the
  bootstrap snapshot and `distribution_snapshot` so a restore under another
  unit fails closed. Requires a decision record amending 2026-11-02 and a SPEC
  changelog fragment per MERGE-QUEUE.md. State in §7 that `window_n` is a
  scenario parameter derived from expected shares per block.
- Falsifiable test: unit `0x207fffff`, block bits around `0x1e0fffff`,
  `share_difficulty = 2^k` with `2^k` well below the block work. Prediction:
  the miner receives `SubmitSharesSuccess` for shares that are not blocks,
  slice rows carry `d = 2^k`, the persisted `window_target` equals
  `window_n * 2^k`, and restarting the front with a different unit refuses to
  restore the snapshot.

#### A4 (Medium) The node capability gap is understated and has no owner

- Affected: §3.2 ("the missing capability is an exposed, validated
  multi-process regtest configuration"), §8 P0, §9 Q10.
- Evidence: `rbitcoin/crates/rbitcoin-node/src/config.rs` exposes
  `regtest_pow_bits` (`:497-498`, `:616`) and `test_activation_height`
  (`:495`), and nothing for `no_pow_retargeting`, `allow_min_difficulty_blocks`,
  `pow_target_timespan`, `pow_target_spacing`. The subsidy-halving overlay
  exists in params (`params.rs:227-230`) but has no config key (grep of
  `config.rs` for `halving` returns nothing). There is no `setban` RPC (see A5).
  rbitcoin's own contribution rules (`rbitcoin/AGENTS.md`: test-first, no
  second path, no editing CHANGELOG on a branch) apply to every one of these.
- Also: "nodes reject mismatched profiles" (P0 acceptance) is not a node
  behaviour. Nodes have no profile handshake; they reject individual headers
  with `bad-diffbits`. The acceptance must be phrased as the harness's own
  fingerprint check plus an observable header rejection.
- Suggested fix: P0 names an "rbitcoin lab profile" packet with the exact
  surface: `--regtest-retarget` (or a `[regtest]` conf group) carrying
  spacing, timespan, min-difficulty flag, optional halving interval; all
  rejected outside regtest; fingerprint of the effective `ChainParams`
  printed at startup and in `getblockchaininfo` so the harness can compare
  nodes. P0 acceptance: two nodes with different profiles connected over P2P;
  a block mined on A is rejected by B with `bad-diffbits` visible in B's log
  and `getchaintips`; the harness preflight refuses the topology before launch.
- Falsifiable test: as stated in the acceptance sentence above.

#### A5 (Medium) Partition on a single host is not real without harness-owned relays

- Affected: §4 ("Control all relevant links ... Explicit peer configuration
  prevents unintended paths"), §5 ("Containers or network namespaces ...
  later"), §9 Q4.
- Evidence: `rbitcoin-rpc` has `addnode` and `disconnectnode` but no `setban`
  (grep of method names). `--connect` targets are redialed every
  `REDIAL_INTERVAL = 2 s` (`rbitcoin-net/src/service.rs:652`). So
  `disconnectnode` heals in 2 s and a "partition" is a 2 s blip. Default
  `use_seeds = true` (`config.rs:141`) is probably inert on regtest, but
  `listen` defaults to loopback accept, so a third node can relay between two
  "partitioned" ones unless every node is `--connect`-only with `--no-listen`,
  which then prevents inbound from the pool's TP client? No: the TP is a
  separate listener. Still, the plan must state the exact node flags per role.
- Suggested fix: from P2 onward, every P2P link runs through a harness-owned
  TCP relay (one listener per directed link) that can drop, delay, or close;
  nodes `--connect` only to relay addresses and run `--no-listen` for P2P.
  This gives the plan its "add latency" command too, which `disconnectnode`
  cannot provide. Containers remain optional.
- Falsifiable test: partition A|B for 30 s with `disconnectnode` only; predict
  `getpeerinfo` on A shows B again within about 2 s. With relays closed,
  predict no peer for the full 30 s and divergent `getchaintips`.

#### A6 (Medium) Timestamp integrity is the retarget experiment's hidden input

- Affected: §3.2 ("timestamp rules"), §7 (time), §9 Q1.
- Evidence: pool accepts `ntime` within `+/- 7200 s` of the job time
  (`bins/spc-pool/src/sv1.rs:143-156`, used by `sv2_server.rs:483` and
  `front.rs:727`); the node accepts header time up to `now + 7200`
  (`header.rs:196-199`) and above MTP. Retargeting measures
  `prev_time - first.time` (`header.rs:287`). A miner that rolls `ntime`
  forward lies to the retarget; a pool that accepts a 2 h roll makes that lie
  legal at the pool. On mainnet this is noise against 2016 x 600 s; at a lab
  spacing of 10 s the roll window is 720 spacings.
- Suggested fix: the plan states that the lab miner never sets `ntime` above
  wall clock or below `min_ntime`, that a lab pool config tightens the roll
  window to a small multiple of spacing, and that the harness records per-block
  `header.time - wall_time_at_acceptance` as an observation with an assertion
  bound. Also record that all nodes share one host clock, and that
  `setmocktime` on one node is a different experiment.
- Falsifiable test: two otherwise identical runs; in one, miners add 600 s to
  `ntime`. Predict the second run reaches a measurably lower difficulty after
  the first boundary while hashrate is unchanged.

#### A7 (Medium) Shortened schedule needs numbers, not a label

- Affected: §3.2 ("Decide whether the experiment uses Bitcoin's normal
  adjustment schedule or a deliberately shortened lab schedule"), §7, §9 Q1.
- Evidence and arithmetic from source: warmup from `pow_limit` (about 2^255)
  to a CPU-scale target (about 2^228) is 27 bits; the retarget clamps at 4x
  per period (rust-bitcoin `from_next_work_required`), so about 14 periods of
  near-instant blocks before the chain is at the intended difficulty. Pipeline
  latency: TP `TEMPLATE_INTERVAL = 5 s` (`rbitcoin-sv2/src/lib.rs:51`), pool
  RPC refresh 1 s (`refresh_interval_ms` default), P2P redial 2 s, stale grace
  10 s (`config.rs:602`). At a 10 s spacing these are 10-50 percent of a block
  interval and the stale/orphan rate dominates "pool wins".
- Suggested fix: §3.2 states a minimum spacing relative to measured pipeline
  latency (suggest `>= 10 x` the observed job-propagation time; 30-60 s is
  realistic on one host), a warmup criterion ("difficulty within 2x of the
  equilibrium for `H`, measured over one full period") recorded as setup, and
  a stale-share and orphan-rate ceiling as a P4 acceptance input.
- Falsifiable test: P3 scenario at 10 s and at 60 s spacing with identical
  hashrate split; predict orphan rate above 10 percent at 10 s and below 2
  percent at 60 s; if both are low the latency argument is wrong and the
  minimum can be relaxed.

#### A8 (Low) Economic schedule bounds missing

- Affected: §7.
- Evidence: regtest halving every 150 blocks (`params.rs:220-222`); after 33
  halvings (about 4950 blocks, roughly 14 h at 10 s spacing) the subsidy is
  zero and only fees remain. Coinbase maturity is fixed at 100
  (`params.rs:233-235`). The halving overlay is in-process only (A4).
- Suggested fix: §7 declares a maximum economic run length in blocks per
  profile, or the rbitcoin profile packet exposes the halving overlay.
- Falsifiable test: a run past height 4950 with no fee workload shows zero
  coinbase value; the harness's expected-payout oracle must predict that, not
  flag it as a conservation failure.

#### A9 (Low) Runner is the single point of failure for run lifetime; P0 freezes contracts too early

- Affected: §5 (components), §5.2, §8 P0 third bullet, P2.
- Evidence: the plan decouples browser presence from run lifetime but every
  process is "supervised" by one runner with no stated persistence of the run
  manifest or process-group ownership. "Freeze initial scenario vocabulary,
  source/observer contracts" is listed in P0 before any miner exists.
- Suggested fix: P2 requires an on-disk run manifest (pids, ports, data dirs,
  fingerprint) written before launch, process groups per run, and a
  `spc-lab adopt|reap <run>` that works after the runner died. Move contract
  freezing from P0 to P2 where there is something to observe.
- Falsifiable test: kill the runner mid-run; predict mining continues; run
  `adopt`; predict the same run id, no duplicate processes, clean `stop`.

### B. Existing implementation blockers (pool side)

#### B1 (High) Job delivery order breaks upstream-compliant SV2 clients

- Affected: §5.1 bullets 2 and 3, §8 P1 ("external SV2 peer").
- Evidence: `bins/spc-pool/src/sv2_server.rs:423-461` (`send_job`) writes
  `SetNewPrevHash{job_id}` first, then `NewExtendedMiningJob` with
  `min_ntime = Some(..)` (a non-future job). The upstream client
  (`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/channels_sv2-8.0.0/src/client/extended.rs:740-755`)
  resolves `SetNewPrevHash.job_id` against `future_jobs` and returns
  `ExtendedChannelError::JobIdNotFound` when absent. The in-house client
  (`bins/spc-agent/src/sv2_client.rs:148-185`) accepts either order, so the
  existing e2e cannot see this.
- Suggested fix: send a future job (`min_ntime = None`) then `SetNewPrevHash`
  referencing it on every chain move; send non-future jobs only for same-tip
  updates (fees, distribution). Keep per-channel job ids.
- Falsifiable test: connect the upstream `channels_sv2` extended client (or
  SRI `mining-device`) to the pool; predict the current server produces
  `JobIdNotFound` on the first job and no shares; after the fix it activates
  the job and submits.

#### B2 (High) Announced channel target is big-endian in a little-endian field

- Affected: §5.1 bullet 1 ("independently verify its wire byte order").
- Evidence: `sv2_server.rs:240-241` encodes `share_target.to_be_bytes()` into
  `U256`. The SV2 data-type table defines `U256` as a little-endian 256-bit
  integer; rbitcoin's TDP encodes the same quantity with `to_le_bytes()`
  (`rbitcoin-sv2/src/template.rs:210`). Every current fixture uses the
  all-ones target (`template_tp.rs:1329` and siblings, `e2e_sv2.rs:143-148`,
  `sv2_client.rs:109-114`), which is palindromic, so the bug is invisible.
- Suggested fix: encode with `to_le_bytes()`; add a vector asserting the
  pool's announced bytes for compact bits `b` equal rbitcoin's TDP `target`
  bytes for the same `b` when `share_difficulty` makes the effective target the
  network target.
- Falsifiable test: the vector above. If it passes today, I am wrong and this
  finding is withdrawn.

#### B3 (High) No `SetTarget`; the effective target changes with every nbits change

- Affected: §3.3 last paragraph, §5.1 bullet 1, §8 P1 last bullet.
- Evidence: grep of `sv2_server.rs` for `SetTarget` returns nothing. The
  announced target is computed once at channel open (`:235-239`) and the
  per-share check recomputes `effective_share_target(share_difficulty,
  job.nbits)` (`:303`). Under A3's unit change the effective target is the
  network target during warmup and flips to the share target after the first
  hard retarget; a compliant miner keeps the open-time target and is then
  either mass-rejected with `difficulty-too-low` or under-submits valid work.
  Under a retarget experiment this corrupts the share stream in both
  directions.
- Suggested fix: emit `SetTarget` whenever the effective target changes
  (compare against the last sent per channel), before the job that uses it;
  acknowledge in the miner state machine that a target applies to all open
  jobs.
- Falsifiable test: retarget profile, assert after every `SetNewPrevHash`
  that the miner's channel target equals the pool's `effective_share_target`
  for the new bits; count `difficulty-too-low` rejections, predict zero after
  the fix and non-zero before.

#### B4 (Medium) `[pool] authority_public_key` is dead config; the SV2 authority key is ephemeral

- Affected: §5.1 bullet 4 ("not wired like the in-flight node-TDP pinning").
- Evidence: `config.rs:96` declares `authority_public_key`; the only
  non-config references are test fixtures (`api_backend.rs:220,245`,
  `front.rs:1205`). `sv2_server.rs:73-77` generates a fresh random keypair at
  construction and uses it for Noise (`:138-139`). The agent connects with
  `connect_with_noise(stream, None)` (`sv2_client.rs:79`). So the key an
  operator advertises is never the key on the wire; a pinning miner would
  fail every handshake.
- Suggested fix: load the pool authority secret from config (mirroring the
  node's `sv2_tp_authority_sec`), verify the advertised public key matches,
  and pin it in the miner. This is also a production correctness item, not
  only a lab one.
- Falsifiable test: start the pool twice; predict the Noise static key
  differs between runs today and is stable after the fix; a miner pinning
  `authority_public_key` fails today and succeeds after.

#### B5 (Medium) Single current job; any non-current job is `stale-share`

- Affected: §5.1 bullets 3 and 6, §6 stale-work observations.
- Evidence: `sv2_server.rs:266-282` validates every submission against
  `self.current` and rejects any other `job_id` as `stale-share`
  (`submission_error`, `:480-482`); the front does the same (`front.rs:718`).
  Jobs change on every fee move (`front.rs:550`) and every distribution
  fingerprint change (`front.rs:655-659`). With a transaction workload the
  pool bumps jobs frequently and all in-flight shares on the previous job are
  rejected, even when the prev hash is unchanged. The SV2 model keeps past
  same-tip jobs valid until `SetNewPrevHash`; the upstream client tracks
  `past_jobs` for that reason (`client/extended.rs:757-770`).
- Suggested fix: keep a bounded set of valid same-tip jobs in the SV2 server
  and the front (keyed by job id, cleared on chain move), so stale means
  "wrong tip", not "not the newest". Until then the lab must report
  pool-induced stale rate as a known bias.
- Falsifiable test: constant hashrate, inject a fee-changing tx every 2 s;
  predict a rejected-share rate proportional to the job-bump rate today and
  near zero after the fix.

#### B6 (Low) Minor SV2 server details that distort lab measurements

- `sv2_server.rs:343-348` hard-codes `new_shares_sum: 1`; after A3 the miner
  cannot use this field to measure accepted work.
- `sv2_server.rs:354-363` treats `Lagged` on the 16-slot job broadcast
  (`:78`) as a no-op; a slow connection silently misses jobs.
- `sv2_server.rs:202-205` draws a random 4-byte extranonce prefix per channel
  with no uniqueness check; `channel_id` is always 1 (`:223`), so the
  process-wide nullifier (`front.rs:743-747`) relies on prefix uniqueness.
  Collision probability is low but non-zero; a lab with hundreds of
  reconnects should allocate prefixes, not draw them.
- Test: run N=64 channels; assert distinct prefixes and that
  `SubmitSharesSuccess.new_shares_sum` equals the credited `d`.

#### B7 (Medium) Job identity excludes bits and version

- Affected: §5.1 last bullet.
- Evidence: `front.rs:547-551`; see A1 for the consequence. Independently of
  the min-difficulty case, a template whose `version` changes (versionbits
  signalling) would also not bump the job, and `front.rs:724` checks
  `template.version` against the share while the SV2 server checks
  `job.version` (`sv2_server.rs:486`).
- Suggested fix: include `bits` and `version` in `job_changed`.
- Falsifiable test: feed the front two templates equal in everything but
  `bits`; predict today `job_seq` does not move; after the fix it does.

### C. Optional enhancements

- C1: SHA256 midstate for the CPU miner (the first 64 header bytes are
  constant per job); halves hashing cost and makes rate budgets more precise.
- C2: use rbitcoin's `waitfornewblock` / `waitforblockheight` RPCs for
  barriers instead of polling; they exist (grep of `rbitcoin-rpc` methods).
- C3: cross-check the harness's hashrate estimate against the node's
  `getnetworkhashps` and `getdifficulty` and report the discrepancy as an
  observation with uncertainty.
- C4: rbitcoin already has a params-compatibility predicate in
  `crates/rbitcoin-test/src/chain_fixture.rs:72-73`; reuse its field list as
  the basis of the consensus fingerprint.
- C5: rbitcoin's config already mentions Warnet tanks (`config.rs:97`); a
  later container profile can target Warnet instead of a bespoke one.
- C6: VarDiff per channel after `SetTarget` exists (B3), using the upstream
  `channels_sv2` vardiff module as the reference behaviour.

## 3. Answers to the plan's §9 questions (short form)

1. Retarget realism: no exposed profile exists; the in-process one
   (`rbitcoin-rpc/tests/methods/mod.rs:6331-6334`) is the template. A shortened
   schedule is justified only with the min-difficulty exception off (A1),
   overlay semantics settled (A2), spacing bounded by pipeline latency (A7)
   and miner `ntime` bounded by wall clock (A6).
2. Share units: a declared difficulty base, integer `d`, fingerprinted unit;
   not fractional work (A3). `window_n` becomes a scenario parameter.
3. SV2 correctness: B1, B2, B3 block it; B4, B5, B7 bias it.
4. Topology: node roles and flags per node are missing; redial and shared
   loopback break partitions without relays (A5).
5. Rate control: acceptable as proposed; add the saturation acceptance test:
   K miners at rate r with `K*r < 0.7` capacity each within 5 percent of r,
   and adding one more above capacity raises a saturation flag rather than
   shrinking the others.
6. Event ordering: anchor every assertion to `(hash, height, depth)` from a
   named node; A1 shows "a solution left the pool" is not even "the node saw
   it" under TDP.
7. Economics: A8; plus the window-size issue in A3.
8. Independent checks: the oracle must read `window_target` from the
   `spc:blocks` event and slice rows, and must carry its own unit conversion;
   sharing `spc-accounting` with the pool makes it a replay, not an oracle.
9. Privacy: the plan's constraints are consistent with SPEC §11.3; the
   omniscient view must live outside the `ark` role's process and never write
   to its streams. No change requested.
10. Scope: the first useful headless run needs exactly A1 (profile flag), A3
    (unit), B1-B3 (pool SV2), a miner with a target/job state machine, and a
    minimal supervisor. Everything in §6 beyond a JSONL event log can wait.

## 4. Open questions

1. Does the user want the retarget profile to keep Bitcoin's exact algorithm
   (4x clamp, interval = timespan/spacing, BIP94 off) with only spacing and
   timespan changed, or is a different responsiveness acceptable? The plan
   should say "exact algorithm, shortened constants" unless told otherwise.
2. Is the pool expected to interoperate with external SV2 miners in the lab
   (B1/B2 then block P1), or only with the in-house miner (then they are P1
   exit criteria but not entry criteria)?
3. Should the share-unit change land as a lab-only config or as the
   production policy (my recommendation: production config with mainnet
   default; it removes a hidden constant either way)?
4. Who owns the rbitcoin packet, and does the rbitcoin maintainer accept a
   regtest-only consensus flag group? Its AGENTS.md "Ask first" list may
   apply.
5. Will the lab ever need more than 8 TP sessions per node
   (`rbitcoin-sv2/src/lib.rs:32`)? With one front per pool per node, 8 pools
   per node is the cap; multi-front pools consume one session per front.

## 5. Minimal recommended plan amendments

1. §3.2: add a paragraph "Minimum-difficulty exception" stating the TDP
   pinning behaviour and the RPC bits-flip behaviour with the file references
   in A1, and making `allow_min_difficulty_blocks = false` a preflight
   requirement for any profile where gaps `> 2 x spacing` can occur.
2. §3.2: replace "startup consensus overlay" with the precise semantics in A2
   and state that the retarget profile does not use `--regtest-pow-bits` as
   its initial difficulty unless rbitcoin redefines it.
3. §3.3: replace the open decision with the declared difficulty base proposal
   (A3), list `window_n` as a scenario parameter, and name the decision
   record and changelog fragment that must accompany it.
4. §4/§5: add per-role node flags and harness-owned P2P relays (A5); move the
   container remark to C-level.
5. §5.1: promote B1, B2, B3 from "gaps to resolve or explicitly bound" to P1
   entry criteria with the tests in this review; add B4, B5, B7 as P1 exit
   criteria.
6. §7: add miner `ntime` bounds and the timestamp observation (A6), the
   minimum-spacing rule and warmup criterion (A7), and the halving bound (A8).
7. §8 P0: split into "rbitcoin lab profile packet" (A1 flag, A2 semantics,
   fingerprint in `getblockchaininfo`, optional halving overlay) and "pool
   share-unit and SV2 conformance packet" (A3, B1-B3), each with the
   falsifiable tests above; rewrite "nodes reject mismatched profiles" per A4;
   move contract freezing to P2 (A9).
8. §10: add `rbitcoin/crates/rbitcoin-sv2/src/session.rs` lines around
   `on_submit_solution` and `crates/rbitcoin-net/src/service.rs`
   (`REDIAL_INTERVAL`) to the evidence map; add the upstream `channels_sv2`
   client as the interoperability reference.

## 6. What I did not verify

- No binaries were built or run; no test was executed. All line references
  are to the trees named at the top and may drift.
- I did not inspect `bins/spc-pool/src/template_tp.rs` beyond its target
  fixtures and authority parsing, nor `spc-agent/src/jdc.rs`; JD-mode
  interactions with the lab are out of this review's scope.
- I did not read `docs/MULTI-REGION.md` in full; the multi-front remarks in
  the plan were taken at face value.

## 7. Clarifications (appended 2026-11-05, same reviewer; sections 1-6 unchanged)

These correct over-statements in my own findings after a second source pass.
The source-verified SV2 findings (B1 message order, B2 endianness, B3 target
lifecycle) and the overlay finding (A2) stand as written.

### 7.1 A1: the stall is conditional on the submitted header timestamp, not universal

- The TP drop keys on the timestamp in the submitted header, not on wall
  clock: `submit_timestamp_needs_other_bits(chain, prev, header_time, pinned)`
  (`rbitcoin-sv2/src/session.rs:622-643`, called from `:574-590` with
  `header.time`). A miner that keeps hashing with the job's original
  `min_ntime` stays valid at an unchanged tip indefinitely, subject to the
  node's `> MTP` and `<= now + 2h` bounds, which a pre-gap timestamp
  satisfies. A miner that rolls `ntime` to wall clock crosses
  `prev_time + 2 x spacing` and from then on every solution it submits is
  dropped.
- Same-tip job renewal is a second path to the gap: the pool sets the job
  time from the template (`front.rs:684`, `ntime: template.curtime`), and a
  TP rebuild on an unchanged tip stamps `header_timestamp = max(mintime, now)`
  (`template.rs:123-125`) with the pinned bits (`session.rs:487-490`). But
  same-tip rebuilds are fee-change driven (`check_fees`, `session.rs:383-394`:
  no rebuild when `template_updates` is unchanged), not periodic; with no
  transaction workload the job and its timestamp do not move.
- My reproduction in A1 was therefore incomplete. `setmocktime` plus resuming
  the existing fixtures does not roll header time: `e2e_sv2.rs:184,230` and
  `testing/regtest-miner.py:81` both submit the job's `ntime` unchanged, so
  that reproduction predicts no drop under current code. Corrected
  reproduction, with the timestamp policy explicit:
  1. PoW baseline, TDP source, no transaction workload. Mine one block, pause
     mining, advance node time past `tip_time + 1200` (wall or `setmocktime`).
  2. Resume with a miner whose policy is `ntime = job.min_ntime` (the existing
     fixtures). Prediction: the next solution is accepted; no stall.
  3. Repeat with a miner whose policy is `ntime = wall clock` (or a fee
     workload that forces a same-tip rebuild after the gap). Prediction: the
     node logs "timestamp needs different bits; dropping" and height does not
     advance within 60 s.
  If step 2 stalls or step 3 does not, the analysis is wrong.
- The RPC-path consequence in A1 (job identity ignores bits, front assembles
  with `template.bits`) has the same conditionality: it requires a template
  refresh whose `current_time` is past the gap, which is the normal GBT
  behaviour on a refresh, but still a condition.
- Disabling `allow_min_difficulty_blocks` is one correct profile choice, not
  the only one. An equally valid implementation keeps the exception and makes
  the pool handle it: treat `bits` as part of job identity (B7), renew the
  same-tip job and `SetTarget` when the required bits change, and have the TP
  re-stamp (or the pool re-request) the tip's bits past the gap. Which path to
  take is a P0 decision for the user; my recommendation stays "exception off
  for the retargeting profile" because a shortened spacing makes the gap
  trivially reachable, while the PoW baseline can live with either.
- The lab miner's timestamp policy is therefore a required scenario input in
  its own right (it also drives A6), and `pause` is a conditional stall risk,
  not a universal failure.

### 7.2 A5: withdraw the exact node flags; keep the relay requirement

- `--no-listen` on every node would remove the P2P listeners the relays need
  to dial into. The arrangement I meant is: each node keeps one P2P listener
  on a loopback port that only the harness knows; the node's outbound
  `--connect` targets are relay listener addresses, and each relay forwards to
  exactly one node's listener. Isolation then rests on no node being
  configured to dial another node directly (no `--connect` to a node address,
  no seeds, no `addnode`) and on the node not learning and dialing addresses
  gossiped by peers. I have not verified rbitcoin's `addr` relay and outbound
  address selection behaviour on regtest, so that last condition is a thing
  to test, not a thing I assert. The redial finding (`REDIAL_INTERVAL = 2 s`,
  no `setban`) stands: `disconnectnode` alone is not a partition.
- Corrected acceptance test: with relays closed for 30 s, `getpeerinfo` on
  both sides shows no peer for the full 30 s and `getchaintips` diverges;
  additionally, start a third node reachable by both and confirm it does not
  bridge them unless a relay to it is open.

### 7.3 A7: the numbers are hypotheses, not bounds

- `TEMPLATE_INTERVAL = 5 s` is the fee-check cadence, not the tip-push
  latency: a new tip publishes a template and `SetNewPrevHash` through the tip
  event path (`session.rs:482-508`) without waiting for the interval. The
  pipeline latency that matters is tip event -> TP build -> pool job -> miner,
  plus P2P propagation and the 2 s redial where relevant, and I have not
  measured it.
- The "above 10 percent at 10 s, below 2 percent at 60 s" orphan figures in
  A7 are unverified hypotheses offered to make the test falsifiable; they are
  not established bounds and should not be copied into the plan as such. The
  recommendation reduces to: measure job-propagation latency first, then set
  the minimum spacing as a multiple of the measured value and record it in
  the profile.

### 7.4 A6: a constant, uniform timestamp offset mostly cancels

- Retargeting uses `prev_time - first.time` over one period
  (`header.rs:287`). If every miner adds the same constant to `ntime` at
  every height, both ends of a period shift equally and the timespan is
  unchanged; the only visible effect is in the single period that contains the
  transition into the offset, where the first block is unshifted and the last
  is shifted, producing a one-time difficulty drop that later periods do not
  repeat. Steady-state uniform offsets therefore do not bias the equilibrium.
- What does bias results: offsets that differ across miners (so the period's
  first and last blocks are stamped by different policies), offsets that grow
  over time (a ramp), or rolling that is uneven relative to period boundaries.
  The 2 h future cap bounds how far any single header can be pushed.
- Corrected A6 test: start the `+600 s` policy on a subset of miners
  mid-period (not at a boundary) and run at least three full periods. Predict
  a difficulty drop at the first boundary after the switch relative to the
  control run, then convergence back toward the control's trajectory in the
  following periods if the offset is uniform and constant, or a persistent
  divergence if the offset differs between the miners that find period-first
  and period-last blocks. The policy requirement in A6 (miner `ntime` bounded
  by wall clock, pool roll window tightened for the lab) is unchanged; the
  justification is the uneven and ramped cases, not the uniform one.
