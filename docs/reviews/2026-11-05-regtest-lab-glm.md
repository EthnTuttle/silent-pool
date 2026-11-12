# Independent review: docs/REGTEST-LAB.md (GLM)

- Reviewer model: GLM (global.zai.glm-5.3, via OpenCode subagent)
- Date: 2026-11-05
- Baseline plan: docs/REGTEST-LAB.md
- Baseline SHA256: `444060b2c1c7d377960116032dc88af114c0dbc43741a8c719b33188e8b99098`
- Scope: full plan (all 10 sections), against silent-pool HEAD
  `b0ab4ca34fda07c61a6335d05f598eaacc69678c` and rbitcoin checkout
  `e8cf9c9d61f7c63ae59cc11ab1179cace50ca142`. Source-only review; no test
  execution, no builds. Where this review cites a fact it names the file and
  line or function that was read. Reviewer did not read the other reviewers'
  findings and did not edit any shared document.

## Verdict

**Accept with amendments.** The plan is honest about the retargeting
prerequisite, correctly separates node-difficulty authority from pool share
targets, and its source-audit claims about the SV2 listener held up under
direct inspection on every point I checked. The packet structure P0-P5 is
feasible, and the privacy posture (no identity fields into the `ark` role or
its streams) survives the proposed omniscient observer because the plan keeps
the observer on the harness side and the Ark boundary in the system under test.

Three findings should be resolved before implementation starts because they
change what the first headless run can prove: the missing bits-only job
republish (B1, high - it silently invalidates the retarget experiment's
invariant), the missing SV2 share-target byte-order/multi-job liveness risk in
the existing listener (B2, high - it blocks P1's "target-update semantics"
gate), and the undefined `SubmitSolution` acceptance signal, which P0/P4
acceptance gates currently assume exists (C1, medium). The rest are
completeness gaps with straightforward fixes inside the plan's own framework.

## Ranked findings

### B1 - A bits-only template change republishes nothing, so a retargeting node can strand miners on stale work

- Severity: high (blocks P1/P4 acceptance; produces silent wrong data in the
  retarget experiment)
- Type: implementation blocker surfaced by the plan's own §3.2/§3.1 claims
- Affected plan section: §3.1 ("When the node changes expected difficulty, the
  new target propagates through the ordinary job path"), §5.1 ("Review job
  change detection for bits/version/time-only updates"), P1 acceptance
  ("Include network-target changes ... before claiming retarget
  compatibility"), P4.
- Evidence (silent-pool working tree at `b0ab4ca`):
  - `bins/spc-pool/src/front.rs::refresh` (lines 547-551): `job_changed =
    chain_moved || fees_moved || self.template_id != template_id`, where
    `template_id = merkle_root(&txids)` (line 545). `chain_moved` is
    `send.is_none() || height != || open.prev_hash != template.prev_hash`
    (547-549). None of these terms sees `template.bits` or `template.curtime`.
  - `bins/spc-pool/src/main.rs::refresh_and_publish` (lines 903-930) and
    `published_key` (851-858) publish SV1/SV2 jobs only when the
    `{height, prev_hash, template, fingerprint}` key moved.
  - The plan's own §5.1 observation ("Review job-change detection for
    bits/version/time-only updates") is correct but is written as a note; it
    is actually the load-bearing precondition for every difficulty-response
    claim in P4.
- Why it matters for the retarget experiment specifically: on a retarget
  boundary the node changes `nBits` at the *same* tip (same height, same
  prev-hash, usually same tx set). Every term in `job_changed` is false, so
  the front keeps publishing the old `nbits` job. The block-detection check
  `network_target(template.bits)` in `front.rs::on_share` (line 802) uses the
  front's stored template, so a miner that meets the *new* (typically harder)
  target on old-bits work is not detected, and a share meeting only the old
  easier target can be assembled into a block the node rejects. The
  experiment's headline invariant - "observed difficulty changes originate at
  nodes" - fails silently rather than loudly.
- Suggested fix: make the plan's P1 packet explicitly require: (a) `job_changed`
  includes `template.bits != self.template.bits || template.curtime !=
  self.template.curtime` (with the distribution fingerprint as before); (b) a
  node-served `nbits` change at constant height/prev-hash/tx-set must
  republish to SV1 and SV2 with a new job id and clean_jobs. This is pool
  code, not harness code, so it belongs in the plan's "controls that need
  node/pool changes" list for §9 question 10.
- Falsifiable acceptance test: run one rbitcoin regtest node with
  `--regtest-pow-bits 0x200fffff`, one front, one SV2 miner; after the first
  template, restart the node with a harder `--regtest-pow-bits` on the same
  datadir tip (or use two fronts pinned to nodes with different bits, or
  simulate by direct template-source injection of a same-tip/harder-bits
  template). Assert within one refresh interval: the miner receives a new
  job whose `nbits` equals the node's current expected bits, and a share
  meeting only the old target is answered `stale-share`/`difficulty-too-low`
  rather than credited. The plan's P1 gate "a static target-only smoke test
  is insufficient" should name this exact test.

### B2 - The SV2 listener's target semantics are unsound for a persistent CPU miner; the plan's list is correct but underweights two of them

- Severity: high (blocks P1)
- Type: implementation blocker (already listed in plan §5.1 "Existing SV2
  gaps"; this adds severity, byte-order evidence, and one gap the list misses)
- Affected plan section: §5.1, P1 acceptance.
- Evidence (silent-pool working tree):
  1. No `SetTarget` exists anywhere in the pool (`grep SetTarget` over
     `bins/spc-pool/src` returns only `target_from_bits` helpers and the
     all-ones constant). Confirmed: the channel target is announced once in
     `OpenExtendedMiningChannelSuccess` (`sv2_server.rs` lines 236-249) and
     never updated.
  2. `sv2_server.rs` line 238: with no current job, the server announces
     `Target::from_be_bytes([0xff; 32])` (accept everything), while the
     submit-time check (line 303) uses `effective_share_target` for the real
     job. The plan's "early opens can get an all-ones target while validation
     later uses a real job target" is verified exactly.
  3. Byte order: the target is serialized as `U256::from(&share_target
     .to_be_bytes())` (lines 240-241), i.e. big-endian numeric bytes in wire
     order. `binary_sv2` `U256` is `Inner<true,32,...>` whose `From<&[u8;32]>`
     copies the slice verbatim (`binary_sv2-7.0.0/src/datatypes/
     non_copy_data_types/inner.rs` lines 497-501), so the wire carries a
     big-endian target. The plan's "independently verify its wire byte order"
     is not pedantry: the natural alternative (SV2's little-endian
     `U256::from_le_bytes` style seen in the copy types) would invert the
     target. An independent vector or an external SV2 peer is the right gate
     (the plan already says so in P1; keep it mandatory, not "where
     feasible").
  4. **Gap the plan's list does not state:** the per-connection replay set is
     cleared when the *job id string* changes (`sv2_server.rs` lines
     308-313: `if seen_job != job.job_id { seen.clear() }`), and one
     `SubmitSharesSuccess` is sent per accepted submission with
     `new_submits_accepted_count: 1, new_shares_sum: 1` (lines 343-349). The
     front's process-wide `seen_shares` is cleared only on prev-hash change
     (`front.rs` 567-571). For a long-lived N-miner lab this is a correctness
     surface (duplicate shares across restarts within the same prev-hash, and
     a success stream that cannot be batched). The plan's §5.1 bullet
     "Support multiple outstanding submissions and correlate sequence
     acknowledgments" partially covers it; make the replay-set lifetime an
     explicit item.
- Suggested fix: none to the plan's content beyond (a) raising "Implement
  target lifecycle" to a numbered prerequisite of P1 with the byte-order
  vector as its acceptance test, and (b) adding the replay-set lifetime item
  to the §5.1 list. The existing text already says the right things; the
  review's point is that these two are not optional polish - without them the
  P1 gate "pool credit reflects the assigned target" cannot be evaluated.
- Falsifiable acceptance test: open a channel before the first template
  exists; then publish a job with bits harder than the announced all-ones
  target implies and submit a share that meets only the all-ones target.
  Assert the pool answers `SubmitSharesError{difficulty-too-low}` and that
  no `ShareRec` is created (front `seen_shares`/slice unchanged). Second
  test: decode the `OpenExtendedMiningChannelSuccess.target` bytes with an
  independent SV2 codec and assert they equal the big-endian bytes of
  `effective_share_target(share_difficulty, nbits)`.

### B3 - The retarget-capable consensus profile does not exist as a configurable node option; the plan's estimate of the work is optimistic and one algorithmic interaction is unlisted

- Severity: high for the self-retargeting goal (this is the user's explicit
  requirement); the plan already flags it as a prerequisite, so the finding is
  about scope precision, not discovery.
- Type: implementation blocker (node side, rbitcoin repo)
- Affected plan section: §3.2, P0, §9 question 1.
- Evidence (rbitcoin at `e8cf9c9`):
  - The cited in-process regression exists:
    `crates/rbitcoin-rpc/tests/methods/mod.rs::getmininginfo_names_the_tip_
    and_the_next_block_work` (lines 6328-6370) sets
    `params.btc.no_pow_retargeting = false; allow_min_difficulty_blocks =
    false; pow_target_timespan = 4 * pow_target_spacing` (lines 6331-6333).
  - There is no configuration path to those fields:
    `crates/rbitcoin-node/src/config.rs::chain_params` (lines 762-783)
    applies only `test_activation_heights`, `regtest_pow_bits`, and (for
    signet) `custom_signet`. Grep for `retarget`/`no_pow_retargeting` over
    `rbitcoin-node/src/config.rs` returns no conf keys. So "an exposed,
    validated multi-process regtest configuration" is not a thin wrapper
    around an existing option; it is a new consensus-profile feature:
    conf/CLI keys, `chain_params` plumbing, validation, and - per rbitcoin's
    own AGENTS.md hard stops - a test-first change with a release note, in a
    separate repo with its own process.
  - The plan's §3.2 list ("initial difficulty/bootstrap, pow limit, spacing,
    interval/algorithm and clamps, timestamp rules, minimum-difficulty
    exceptions, halving, maturity") is good. One interaction it does not
    list: `crates/rbitcoin-consensus/src/header.rs::
    apply_regtest_pow_overlay` (lines 305-329) returns the base bits
    unchanged at retarget-boundary heights (line 322: `if interval != 0 &&
    height.is_multiple_of(interval) { return bits; }`), and the overlay only
    substitutes when base bits equal the pow-limit bits. If the lab keeps
    `--regtest-pow-bits` as the *initial* difficulty on a retargeting profile,
    the first retarget boundary will jump to bits derived from
    `pow_limit`-anchored history, not from the overlay. The profile decision
    must either disable the overlay or specify which of the two supplies the
    period-start bits, or the "node-authoritative difficulty" narrative will
    have a hidden step. The plan's "Do not use restarts with different PoW
    overrides as a live speed dial" shows the right instinct; extend it to
    "a retargeting profile does not compose with `--regtest-pow-bits`".
  - Smaller note: `ChainParams::regtest()` in
    `crates/rbitcoin-consensus/src/params.rs` (lines 71-99) inherits
    `allow_min_difficulty_blocks = true` and `no_pow_retargeting = true`
    from `BtcParams::REGTEST` (verified in bitcoin-0.32.102
    `src/consensus/params.rs` lines 171-188), and the minimum-difficulty gap
    rule is `header_time > prev_time + 2 * spacing` (`header.rs::min_diff_gap`
    lines 294-297) - the plan's "strictly more than 1200 seconds" is exactly
    right.
- Suggested fix: in P0, state that the deliverable is (1) a named consensus
  profile (e.g. `--regtest-retarget=<interval>` or a profile flag) landed in
  the rbitcoin repo with its own test and changelog entry per that repo's
  conventions, covering the §3.2 checklist fields as *config*, not as a test
  fixture mutation; and (2) a decision on `apply_regtest_pow_overlay`
  composition. Also name the correct existing evidence: the capability is
  proven by an in-process test that mutates params directly, which is weaker
  than the plan's phrase "engine already has an in-process regression that
  enables retargeting" suggests to a reader who has not opened the file - the
  plan should say the mutation happens in the test, not in any served
  configuration.
- Falsifiable acceptance test: start `rbitcoin-node` on regtest with the new
  profile and N miners; drive hashrate up by a factor R for `interval + k`
  blocks; assert the node's `getmininginfo.next.bits` (or template `nBits`)
  changes at a boundary by an amount consistent with the declared clamps
  (e.g. within `[1/4, 4]` under Bitcoin's clamps) and that a miner hashing
  pre-boundary work after the boundary gets a new job within one refresh
  interval (ties to B1).

### C1 - "Candidate accepted" has no defined observation; `SubmitSolution` gives the pool no acceptance signal, and the plan's own gates need the concept pinned

- Severity: medium-high (P0/P2/P3/P4 acceptance gates all use phrases like
  "a candidate is accepted", "node validates submitted blocks", "observed
  acceptance"; without a defined signal each gate is unfalsifiable)
- Type: plan defect (evidence-definition gap) plus implementation constraint
- Affected plan section: §5 ("With TDP, `SubmitSolution` is not itself proof
  of node acceptance; reconcile the candidate hash with the node's actual
  chain view" - correct but under-specified), §5.2 ("Run until ... a
  candidate is accepted"), P1/P3/P4 acceptance.
- Evidence:
  - rbitcoin `crates/rbitcoin-sv2/src/session.rs::on_submit_solution`
    (lines 521-617): every failure path is a log line and `return Ok(())` -
    unknown template (527), undecodable coinbase (533), target miss (565),
    bits/timestamp mismatch (585); the acceptance path ends in
    `chain.accept_block_async(block).await` with only `rbitcoin_log::info!`
    for either outcome (612-615). There is no TDP message or channel that
    tells the pool accept vs. reject.
  - silent-pool's own decision `docs/decisions/2026-11-05-sv2-tp-chain-view.md`
    records: "TDP's `SubmitSolution` also has no acceptance acknowledgement,
    so the client treats the TP's ensuing tip push as the outcome and reports
    a competing tip as a rejection." That convention is the right raw
    material, but the lab plan never adopts it, and "tip push" alone is
    ambiguous under partitions (the plan's own §5.2 partition scenarios):
    a tip push can arrive for a competitor's block while your candidate is
    still being connected.
  - Compounding evidence for needing the *node RPC* rather than inference:
    `submit_timestamp_needs_other_bits` (session.rs 622-643) *drops*
    solutions whose timestamp needs different bits than the template pinned -
    a real drop path on regtest with min-difficulty enabled (the plan's §3.2
    "a long timestamp gap can invoke minimum difficulty" interacts here).
    A lab that infers acceptance from "no error" would misread this drop as
    a slow node.
- Suggested fix: add to §5 (or a small new §5.x "Evidence predicates") the
    definition the implementation already uses, made stronger: candidate
    acceptance == the candidate's block hash is (a) the hash of a block at
    `height H` on the node's *validated active chain* per the chain-view RPC
    (`getblockhash H` / `getblock` with confirmations >= 1), observed per
    node; canonical inclusion == same on every non-partitioned node's view;
    rejection == a competing block at `H` confirms while the candidate is
    absent from that node's chain. Log-derived "accepted" is forbidden as an
    assertion input. This also cleanly handles the rbitcoin drop path: a
    dropped candidate never appears, and the harness must see that as
    rejection-with-reason, not as "still pending" forever (bound it with the
    §5.2 declared timeout).
- Falsifiable acceptance test: in a two-node partition scenario, submit a
  candidate through pool A's node; while the partition is up, assert the
  harness reports `accepted-at-A / not-canonical` (two distinct states), and
  after heal assert it resolves to exactly one of `canonical` or `orphaned`
  per greatest-chainwork. Assert the artifact contains the per-node
  observation, not a node log line.

### C2 - The "consensus-profile fingerprint" has no definition, and the preflight gate depends on it

- Severity: medium (P0's headline acceptance gate - "nodes reject mismatched
  profiles; a requested unsupported capability fails preflight" - is not
  currently implementable because nothing defines the fingerprint or who
  checks it)
- Type: plan defect (underspecified mechanism)
- Affected plan section: §3.2 ("Every node on one experimental network uses
  the same consensus-profile fingerprint"), P0 acceptance.
- Evidence: `rbitcoin-node/src/config.rs::chain_params` is the only place
  consensus params are composed (lines 762-783); there is no serialization
  of the effective `ChainParams`, no peer exchange of them, and no
  "fingerprint" anywhere in the node. On the pool side nothing fetches such
  a thing. P2P does not negotiate consensus parameters: two nodes with
  different `regtest_pow_bits` will simply fork on the first block (this is
  in fact a usable detection, but the plan should say so rather than imply
  a protocol-level check exists). Note also `apply_regtest_pow_bits` refuses
  bits not harder than the pow limit (`params.rs` lines 384-411), so profiles
  are constrained; the fingerprint should capture the *effective* values
  (including overlays), not just the CLI flags.
- Suggested fix: define the fingerprint concretely in P0: a canonical
  serialization of the effective consensus-relevant fields (pow limit,
  spacing, timespan, retarget on/off, min-difficulty on/off, overlay bits if
  any, halving interval, maturity, genesis hash, activation heights) hashed
  (sha256), computed by the harness from each node's *served* configuration
  (e.g. a `getblockchaininfo`-equivalent RPC or the node's startup log),
  compared across nodes by `spc-lab` preflight, and recorded in the run
  artifact. Optionally, node-side enforcement (reject `addnode` from a peer
  with a different fingerprint) is a nice-to-have enhancement, not a
  prerequisite - the harness-side check covers the lab's needs.
- Falsifiable acceptance test: start two nodes with different effective
  profiles, ask the runner to start a run; assert preflight fails naming the
  mismatched field; then start a run with matching profiles and assert the
  artifact records one shared fingerprint value.

### C3 - SV2 share-target work-unit design (§3.3) misses that the current credited difficulty is capped at the network difficulty and regtest rounds to 1; the "non-block shares" premise needs an explicit profile decision, not just target selection

- Severity: medium (affects the design-space framing of §3.3 and P0's second
  bullet)
- Type: plan defect (incomplete analysis) + implementation constraint
- Affected plan section: §3.3, §9 question 2, P0.
- Evidence:
  - `bins/spc-pool/src/sv1.rs::effective_share_target` (lines 77-86): the
    share target is the *easier* of pool-target and network-target, so on a
    regtest chain whose network target is far easier than difficulty-1, the
    network target always wins. `credited_difficulty` (lines 108-110) =
    `min(share_difficulty, max(1, round(network_difficulty)))`, and
    `network_difficulty` on regtest-style bits is 1 (lines 94-105: `d < 1.0
    -> 1`). I verified numerically: `0x200fffff` decodes to a target ~9.9e27
    times the regtest pow limit and its difficulty-1-relative difficulty is
    ~3.7e-9, which rounds to 1.
  - So on any regtest-style chain, every accepted share credits `d = 1` and
    the window target is `N x 1`. This is exactly the plan's "this is what
    today's minimum integer difficulty of 1 produces on a much easier regtest
    chain" - verified.
  - The gap: the plan's proposed direction ("support CPU-scale share targets
    through the real SV2 target policy ... exact 256-bit targets and a
    defined integer/fixed-point work representation") addresses the *target*
    side, but the *credit* side has a second cap: even with a harder share
    target than the network target, `effective_share_target` takes the
    network (easier) target on regtest, so the share target cannot be made
    harder than a block, and credited difficulty cannot exceed network
    difficulty. Consequently "Different share targets must earn
    work-proportional credit" cannot be satisfied on a regtest chain with
    the current policy *at all*, for any configuration: the effective target
    is always the network target. Making non-block shares frequent on
    regtest therefore requires the share target to be *easier* than the
    network target (more shares than blocks), which the current code already
    supports via the network-target-wins rule... but then every share that
    meets the easier target and not the block target is a genuine
    non-block share, credited `d = 1` each - and window semantics become
    "N shares" rather than "N x d units". That is a legitimate lab profile,
    but it is a *different decision* from the fixed-point work
    representation, and the plan conflates the two.
  - The plan's "Preserve the SPEC §4.3 window semantics (`N x d`)" is
    compatible with both readings (d=1 makes it N shares), but P0's decision
    list should separate: (i) share target selection (easier-than-network on
    regtest - already the case), (ii) credited-d work representation (u64
    integer today; fixed-point if harder-than-network targets ever matter -
    they cannot on regtest while `effective_share_target` takes the easier
    of the two), (iii) whether `credited_difficulty`'s network cap should be
    relaxed for the lab. Also note `window_target` is persisted as
    `NUMERIC(40,0)` u128 (SPEC §5.1 `distribution_snapshot.window_target`;
    front.rs 585-587 computes it in u128) - any fixed-point representation
    must fit that column or carry a migration.
- Suggested fix: add to §3.3 an explicit statement of the two regimes and
  name the one the lab uses first: on regtest the pool share target is
  necessarily easier than or equal to the network target, so the lab's
  "frequent non-block shares" come from the easier side, credited at the
  existing integer `d` (no fractional units needed for P1); fractional work
  is future work for chains where the pool target is harder than network.
  This *reduces* P0's scope and should be recorded as such.
- Falsifiable acceptance test: with `share_difficulty = 1` on a
  `0x200fffff` regtest chain, run one miner; assert it submits shares
  meeting the effective target but not the network target (non-block
  shares exist), each credited `d = 1`, and the window fills after exactly
  `window_n` accepted shares' slice-sealing behavior (whole-slice
  granularity per SPEC §4.3).

### C4 - Multi-pool topology claims need three concrete collision audits the plan does not enumerate

- Severity: medium (P3's "two runs coexist" / "accounting is pool-local"
  gates will fail on collisions that are cheap to prevent and expensive to
  debug at 3 a.m.)
- Type: plan defect (completeness)
- Affected plan section: §4, P3 acceptance.
- Evidence (silent-pool working tree):
  - Redis stream names are hard-coded constants, not namespaced:
    `spc:slices`, `spc:front:events` in `crates/spc-store/src/bootstrap.rs`
    (lines 167-187), `spc:ark:orders` in `bins/spc-pool/src/payout.rs`
    (line 993). Two independent pool instances pointed at the same Redis
    (the default `redis://` local instance) will cross-consume each other's
    slices and ark orders. The plan says "independent ... Redis/DB
    namespaces or instances" but does not say which; there is no namespace
    support in the store crate today, so the initial answer must be
    "separate Redis instances and separate Postgres databases" - say it.
  - The existing harnesses already dodge this by using disjoint fixed ports
    (`testing/regtest-rbitcoin-reorg-e2e.sh` lines 56-62), and the plan
    rightly wants a supervisor to own allocation; keep the plan's
    "unique ports, data directories and process groups per run" and add
    "unique Redis DB index or instance, unique Postgres database, unique
    datadirs, unique authority keys" to the same sentence.
  - `front_id` is claimed by a Redis `SET NX` lease (SPEC §9); with shared
    Redis across pools, two pools' fronts can collide on `front_id = 0`
    *silently* - the lease is per Redis. With separate Redis instances the
    lease is moot for cross-pool safety; the plan should state that separate
    instances are what makes `front_id` pool-scoped.
  - Noise authority keys: `Sv2Server::new` generates a fresh ephemeral
    keypair per process (`sv2_server.rs` lines 73-90) and ignores the
    configured `authority_public_key` for the miner-facing listener (the
    config key exists - SPEC §6.3 `[pool] authority_public_key` - and the
    agent's `connect_with_noise(stream, None)` passes `None` for the remote
    authority, `sv2_client.rs` line 79). The plan's §5.1 gap list has
    "Miner-facing Noise authority pinning is not wired" - verified, and this
    is the mechanism. For the lab this is a non-issue *between pools* (each
    pool has its own key) but a correctness issue for miner identity: with
    no pinning, a misconfigured miner can be MITM'd by the harness itself.
  - The rbitcoin TDP listener caps concurrent sessions at 8
    (`crates/rbitcoin-sv2/src/lib.rs` line 32 `MAX_SESSIONS: usize = 8`;
    the plan's "caps concurrent sessions at 8" is verified). The plan
    correctly notes miners do not connect to TDP; with P pools per node
    (or one pool per node plus the settlement chain view), 8 is ample but
    the plan should state the intended pool-per-node session budget so P3
    does not discover it empirically.
- Suggested fix: add a P3 preflight checklist item: per-pool isolated
  Redis (separate instance or unique DB index), unique Postgres database,
  unique datadirs, unique ports, unique Noise authorities for every
  listener, and a documented node-TDP session budget per node. Mark Redis
  namespacing (key prefix) as an enhancement, not a prerequisite.
- Falsifiable acceptance test: start two pool instances against one shared
  Redis with default config; assert the supervisor/preflight refuses (or,
  if isolation is by config, assert pool A's slice stream never contains
  pool B's `front_id`). Then run the same scenario with isolated Redis and
  assert accounting is pool-local over a 100-share run.

### C5 - Observation independence: the plan's "independently implemented policy oracle" shares the run's inputs but the plan should pin the ladder constants and the frozen window source to make the oracle meaningfully independent

- Severity: medium (this is the review's "independent oracles" priority
  axis; the plan has the right shape and needs tightening, not redesign)
- Type: plan defect (precision)
- Affected plan section: §6 (three kinds of evidence), §9 question 8.
- Evidence:
  - SPEC §4.4 (denomination payouts) and §4.7 make the rungs
    `{500k, 1M, 3M, 5M}` *constant forever*; the oracle must hard-code the
    same four constants and the greedy largest-first rule, and §3.6's
    miner-side check likewise. The plan says "explicit floor and
    denomination rules" - good - but should add: the oracle does not import
    `spc-accounting` at all (the current `route_tiers`/`raw_payouts` live in
    `crates/spc-accounting/src/lib.rs`; a review of the oracle's imports is
    part of the gate), and the frozen window is reconstructed from the
    persisted `window_target` (SPEC §7.4, decision 2026-11-03) and
    `high_water`, not from the live window.
  - `window_target` is u128 persisted as NUMERIC(40,0) (SPEC §5.1); the
    oracle must parse it exactly (no f64). The plan's §6 "Serialize wide
    integers losslessly for JavaScript" is the browser-side echo of the
    same rule.
  - Two endpoints reading the same database are correctly excluded
    (plan §6 last paragraph) - the strongest single sentence in the plan's
    evidence section.
- Suggested fix: in §6 evidence kind 2, add: (a) the oracle is a separate
  crate/binary with no dependency on `spc-accounting`, `spc-store`, or
  `spc-pool`; (b) its inputs are recorded accepted-work events plus the
  artifact's frozen `window_target`/`high_water` and the four constant
  rungs; (c) its output is an expected-per-`sp_id` payout vector compared
  against node-accepted coinbases (evidence kind 3), with mismatch severity
  classified (floor placement, rail assignment, decomposition, rounding).
  Add "no code shared with the implementation under test" to the P4
  acceptance gate wording.
- Falsifiable acceptance test: inject a deliberately wrong routing (e.g.
  swap two `owed` amounts in a recorded run's input replay) and assert the
  oracle flags it while the production replay (evidence kind 1) stays green
  - proving the oracle is not merely re-running the implementation.

### C6 - Economics: rbitcoin's halving and the lab's warmup are stated, but coinbase maturity vs confirmation depth and the pool-output boarding dependency are not wired into scenario preconditions

- Severity: medium-low (correctness of long economic scenarios, not of the
  harness skeleton)
- Type: plan defect (completeness)
- Affected plan section: §7 ("50 BTC subsidy, halves every 150", warmup,
  float, confirmation policy), P4 economics scenarios.
- Evidence: `crates/rbitcoin-consensus/src/block/mod.rs::block_subsidy`
  (lines 1354-1361) - 50 BTC halving every `subsidy_halving_interval()`;
  regtest interval is 150 (`params.rs::subsidy_halving_interval` lines
  216-225); maturity is 100 blocks (`coinbase_maturity` line 232-235). The
  plan's numbers are verified. The unstated dependency: the pool's own
  economics need `H_f + 6` (Ark sends) and `H_f + 100` (pool output
  maturity -> float refill, SPEC §11.3) with *someone* mining blocks in
  between; in a lab where the only hashers are the scenario's miners, a
  "pause all miners" scenario stalls settlement indefinitely. The plan's
  §5.2 pause semantics ("process timers, network delivery and outstanding
  settlement continue") are right for the pool's internal timers, but
  height-driven timers cannot advance without blocks. Also, the regtest
  harness precedent mines 101 blocks via `generatetoaddress` before the
  pool starts (`testing/regtest-rbitcoin-tp-e2e.sh` line 110) - the plan's
  "fixture bootstrap uses node generation RPCs ... record that as setup
  outside the measured workload" needs the same 101-block rule for wallet
  spendability, which it implies but does not state (regtest coinbases are
  not spendable for 100 blocks; the lab's funding workload needs that
  head start or its own miner-driven warmup).
- Suggested fix: add to §7: (a) an explicit warmup contract - either the
  node-generation bootstrap (recorded as setup, as the existing harnesses
  do) or a declared miner-driven warmup phase inside the run; (b) a rule
  that height-advancement is a precondition for height-driven settlement
  assertions, so scenarios that pause all miners must either exclude
  height-driven assertions or run a declared background hashrate; (c) note
  that regtest maturity (100) means economic runs spanning > 150 blocks
  will see the subsidy halve mid-run and the harness must pin the era in
  the run manifest (the plan asks for a "declared reward schedule" - make
  it a manifest field).
- Falsifiable acceptance test: a scenario that pauses all miners at H and
  expects an Ark send at H+6 must be reported as `blocked-on-height` with
  no assertion failure (or the scenario declares a background miner and
  the send completes); the run artifact must label which.

### N1 - Browser decoupling: verified feasible; one addition

- Severity: low (positive finding + one gap)
- Type: confirmation, and a plan addition
- Affected plan section: §1 ("Keep visualization independent ..."),
  §6 (snapshot + SSE), P5 acceptance.
- Evidence: the current UI-less design of the pool (all roles are
  headless; the API is read-only JSON - SPEC §6.1) means nothing in the
  system today depends on a browser. The plan's "browser presence never
  controls run lifetime" is achievable with the proposed runner-owned
  command API; no conflict found in the code.
- Gap: P5's "inspect a completed CI run" requires the artifact format to
  carry the *full* event log, but §6 also allows "decimate high-volume
  chart data". The two are compatible only if decimation is a *derived
  view* over a retained raw event log (or the artifact explicitly records
  loss boundaries and the UI refuses to show chart-derived accounting
  claims). Say which: e.g. "decimation applies to rendering series only;
  the artifact's evidence log is complete for accounting checks or the run
  is marked inconclusive" - the plan already has the principle ("assertions
  requiring complete evidence fail or become inconclusive"); extend it to
  the artifact.
- Falsifiable acceptance test: take an artifact whose event log had
  losses; assert the UI labels affected charts as incomplete and the
  accounting check on that run reports `inconclusive`, not `pass`.

### N2 - Minor evidence corrections for the plan's §2 table

- Severity: low (factual precision in the evidence table)
- Type: plan defect (wording)
- Affected plan section: §2 table rows.
- Evidence and corrections:
  - "Native SV2 | `sv2_server.rs`, `e2e_sv2.rs`" - verified; add that the
    fixture's template has `transactions: vec![]` and `merkle_branches:
    vec![]` (`bins/spc-pool/tests/e2e_sv2.rs` lines 27-41) and searches
    against `target_from_bits(nbits)` (line 223) - the plan's later §5.1
    text says exactly this; cross-reference it from the table row.
  - "Node TDP | staged `template_tp.rs` and
    `testing/regtest-rbitcoin-tp-e2e.sh`" - at review time the staged TP
    work had moved to the `sv2-tp` worktree branch (`1ec1dd0`, containing
    `template_tp.rs`, the full-stack e2e, and the chain-view decision), no
    longer sitting uncommitted on master. The plan's caution ("Recheck at
    implementation time") is doing its job; the reviewer records the
    snapshot for provenance: staged-then-committed mid-review, staged
    integration preserved.
  - "Accounting | ... current routing/building still contains exact-amount
    outputs and capacity trimming" - verified both halves:
    `route_tiers` (`crates/spc-accounting/src/lib.rs` lines 405-460+) still
    does an amount-`T`-capped walk with whole-id capacity closure (the
    "trim" shape), and the denomination ladder is the dispatched-but-not-
    merged packet (STATUS: "Denomination ladder impl (WP0-WP2)" dispatched
    on branch `ladder`, worktree `df7c654`). The plan's "ladder
    implementation is tracked separately" is accurate.
  - The `--regtest-pow-bits 0x200fffff` claim ("roughly 16 hashes per
    block"): decoding gives target `0xfffff * 2^(8*(0x20-3))`; expected
    hashes per block = 2^256 / (T+1) ~ 2^256 / 2^245 = 2^11-ish; the
    "roughly 16" figure is off (it looks like a `0x210fffff`-style
    computation); the smoke-test character is right, the number is not -
    either recompute or drop the figure. (Falsifiable trivially: decode
    and count.)

## Separation of concerns (per the task's required taxonomy)

- Plan defects (fix the document): C1 (evidence predicate undefined), C2
  (fingerprint undefined), C3 (share-target/work-unit regimes conflated),
  C4 (isolation checklist missing), C5 (oracle independence not pinned),
  C6 (warmup/maturity preconditions), N1 (decimation vs evidence), N2
  (wording corrections, incl. the "16 hashes" arithmetic).
- Existing implementation blockers (fix the code, plan should schedule):
  B1 (bits-only job republish), B2 (SV2 target lifecycle/byte order/
  replay-set), B3 (rbitcoin retarget profile + overlay composition), plus
  the rbitcoin `SubmitSolution` drop path surfaced under C1.
- Optional future enhancements (do not gate P0-P2): node-side fingerprint
  enforcement (C2), Redis key-prefix namespacing (C4), fractional/fixed-
  point share difficulty for harder-than-network regimes (C3), per-channel
  VarDiff (the plan already defers it), containers/netns for link fault
  control (plan already defers), statistical multi-run machinery beyond
  the conservation/idempotency checks (plan already splits these).

## Open questions

1. Who owns B1 (bits-only republish) if it is a pool defect rather than a
   lab deliverable? The plan's packets are lab-side; B1 changes silent-pool
   core (`front.rs`), so it needs a merge-queue entry per MERGE-QUEUE.md
   (front.rs is not in the shared-file table, so a packet can own it, but
   it should be named). Recommend the plan list it under P1 as a required
   pool-side dependency with its own decision record if it changes SPEC
   §3.3 behavior.
2. For the retarget profile (B3), does the rbitcoin repo accept a
   regtest-only consensus-profile feature under its AGENTS.md test-first
   rule, or should the lab pin a fork? UPSTREAM_DEPS.md says pins are
   reference points; the lab plan should record the intended rbitcoin
   revision as a run input either way (it already says so).
3. Is `min_diff_bits`'s backward walk (`header.rs` lines 332-357) intended
   to stay enabled in the lab profile? On a short-interval retargeting
   profile, a >2*spacing timestamp gap resets to pow-limit bits, which can
   mask a hashrate-decrease experiment as a difficulty reset. The plan's
   "disable minimum-difficulty exceptions" phrasing suggests no; make it a
   named profile field with its own acceptance test.
4. Does the harness's omniscient observer need `sp_id`-level views for the
   restricted projections (miner, chain-observer, Ark-observer) to be
   *faithful* (plan §6 asks this; my read: yes, the omniscient side holds
   everything and each projection is a filter, which keeps SPEC §11.3's
   boundary intact because the ark role's inputs remain order-only - the
   risk is only that the *harness* becomes a privacy oracle, acceptable
   for a lab but worth a sentence).
5. Node-side `setmocktime` exists (`rbitcoin-rpc/src/methods/mine.rs` line
   685, regtest-gated); the plan's "Mock node time ... is an explicit
   scenario input" should say whether the lab uses it at all - mixing
   mocked time with real hashing makes difficulty eligibility
   node-authoritative but timestamps synthetic, which distorts exactly
   the retarget signals P4 measures. Recommend: not used in retarget
   scenarios; allowed in invariant scenarios only.

## Minimal recommended plan amendments

1. P1: add bits/curtime-only template change to the job-republish
   requirement with the B1 test (pool-side; name the file and the
   merge-queue implication).
2. P1: raise target lifecycle + wire byte-order vector + replay-set
   lifetime from the §5.1 note list to numbered prerequisites.
3. P0: state the retarget profile is a new rbitcoin feature (conf keys +
   `chain_params` plumbing + tests per that repo's rules), not a wrapper;
   add the `apply_regtest_pow_overlay` boundary interaction to the
   profile checklist; correct the in-process-regression wording.
4. New §5.x or §6 addition: define candidate-accepted / canonical /
   orphaned / rejected evidence predicates in terms of the chain-view RPC
   per node, and forbid log-derived acceptance as assertion input.
5. P0: define the consensus-profile fingerprint (canonical serialization
   of effective consensus fields, hashed; harness-computed and compared in
   preflight; recorded in the artifact).
6. §3.3: separate the two regimes (share target easier-than-network on
   regtest with integer `d`; fractional work only if a harder-than-network
   regime is ever needed); note `window_target`'s u128/NUMERIC(40,0)
   constraint on any future fixed-point unit.
7. P3: add the isolation checklist (separate Redis instance/DB index,
   unique Postgres DB, unique datadirs, ports, Noise authorities; TDP
   session budget per node).
8. §6: pin oracle independence (no shared crates with the implementation;
   inputs are recorded work + frozen `window_target`/`high_water` + the
   four constant rungs) and extend the completeness rule to artifacts.
9. §7: add the warmup/maturity contract (101-block-equivalent bootstrap or
   declared miner-driven warmup; height-advancement preconditions for
   height-driven assertions; era pinned in the manifest).
10. §2/N2: fix the "roughly 16 hashes" arithmetic (or drop the figure) and
    cross-reference the fixture's empty-transaction/merkle-path facts from
    the table row.

## Acceptance-test appendix (falsifiability index)

| Finding | Test in one line |
|---|---|
| B1 | same-tip harder-bits template -> new job id within one refresh; old-target share answered `stale-share` |
| B2 | pre-template open + below-pool-target share -> `difficulty-too-low`, no ShareRec; independent decode of announced target bytes == big-endian effective target |
| B3 | drive hashrate xR for interval+k blocks -> `next.bits` moves within declared clamps at a boundary |
| C1 | partitioned two-node candidate -> `accepted-at-A` != `canonical` observed as distinct states; heals to exactly one of canonical/orphaned |
| C2 | two nodes, different effective profiles -> preflight fails naming the field |
| C3 | `share_difficulty=1` on regtest -> non-block shares accepted, each credited d=1, window fills at N shares |
| C4 | two pools, shared Redis, default config -> supervisor refuses or zero cross-pool slices observed |
| C5 | injected wrong routing -> oracle flags it, production replay green |
| C6 | all-miners-paused scenario with H+6 assertion -> `blocked-on-height`, no false pass |
| N1 | lossy artifact -> affected charts labeled incomplete, accounting check reports `inconclusive` |
| N2 | decode `0x200fffff`, count expected hashes: 2^256/(T+1) |

---

# Correction and disposition (2026-11-05, second pass)

This section supersedes the parts of the original review it names. The
original findings above are preserved unchanged for provenance; where this
section says retracted, downgraded, or corrected, this section is what I
stand behind. Each correction was re-derived by direct arithmetic or direct
source reading, not accepted on authority. The baseline plan hash is
unchanged (`444060b2c1c7d377960116032dc88af114c0dbc43741a8c719b33188e8b99098`).

## 1. N2 arithmetic claim: RETRACTED - the plan's "roughly 16" is correct

My N2 item claimed the plan's "roughly 16 hashes per block" was wrong and
suggested "2^11-ish". The plan is right and my arithmetic was wrong.

Direct computation of the compact format (`mantissa * 256^(exp-3)`, the
same decode as `rbitcoin`'s `params.rs::apply_regtest_pow_bits` and
`Target::from_compact`):

```
0x200fffff: exp = 0x20 = 32, mantissa = 0x0fffff = 2^20 - 1, shift = 8*(32-3) = 232
T_override  = (2^20 - 1) * 2^232 = 2^252 - 2^232
           = 7236998675585915423409399128287131963803921590493563082079543837970346803200

expected hashes = 2^256 / (T_override + 1)
               = 16.00001525880361442910616789452352001526   (exact, Decimal(40))

0x207fffff (regtest pow limit): exp = 0x20 = 32, mantissa = 0x7fffff = 2^23 - 1
T_powlimit  = (2^23 - 1) * 2^232 = 2^255 - 2^232
expected hashes at pow limit = 2.000000238418607523275318536200348877948

T_override / T_powlimit = (2^20-1)/(2^23-1) = 0.1249998956918592085670481404123473658976
```

So the override target is about 1/8 of the pow limit (8x harder, ~16 hashes
vs ~2), exactly as the plan's smoke-test framing implies. My original
sentence in B3's evidence ("decodes to a target ~9.9e27 times the regtest pow
limit") came from a mis-transcribed pow limit in my own first verification
script (`0x7FFFFF << 136` instead of `(2^23-1) * 2^232`); it is RETRACTED.
The difficulty-1-relative figure I gave (~3.7e-9, so
`network_difficulty` rounds to 1) was correct: diff-1 target = `0xFFFF << 208`
= `Target::MAX` (~2^224), and 2^224 / 2^252 = 2^-28 = 3.73e-9. Disposition:
drop the N2 arithmetic bullet entirely; the plan's §2 sentence stands.

## 2. C3 main claim: RETRACTED in part - current config yields ZERO non-block shares; the explicit CPU-scale share-target change is required, not already available

My C3 asserted the current code "already supports" easier-than-network
shares via the network-target-wins rule. That is a contradiction, and the
set algebra shows it directly.

`sv1.rs::effective_share_target` (lines 77-86) takes the easier of
`target_for_difficulty(share_difficulty)` and the network target. With the
shipped default `share_difficulty = 1`,
`target_for_difficulty(1) = DIFFICULTY_1_TARGET` (~2^224). On an
easy regtest chain the network target (~2^252 at the overlay, or ~2^255 at
the pow limit) is far easier (larger), so:

```
effective_share_target = max(~2^224, network_target) = network_target
S = B
{hash : hash <= S and hash > B} = {hash : hash <= B and hash > B} = empty set
```

Therefore, under the current config, **no accepted share can miss the block
target**: every share the pool accepts is block-qualified, and non-block
accepted shares cannot exist. The `max` in the policy can never make the
effective target easier than the network target on regtest; it can only make
it equal. My "regtest the lab's non-block shares come from the easier side,
credited at the existing integer d ... no fractional units needed" regime
does not exist today, and my recommendation that this "reduces P0's scope"
was based on that false premise. RETRACTED. The plan's §3.3 statement is the
correct one: obtaining frequent non-block shares requires an explicit
CPU-scale share-target/work-unit change (a pool share target easier than the
network target, with a defined work representation), and it is required P0
work, not optional, not deferrable.

Retained from C3, unchanged: (a) harder-than-network share targets are
unreachable while `effective_share_target` takes the easier of the two, so
the harder-than-network regime (the only one that would need fractional
difficulty) is not what the lab's first goal needs - the required change is
on the easier side, as the plan says; (b) any future fixed-point work unit
must fit the persisted `window_target NUMERIC(40,0)` u128 column (SPEC
§5.1; `front.rs` 585-587 computes it in u128); (c) the §4.3 `N x d`
preservation requirement.

## 3. B1: DOWNGRADED high -> medium, narrative corrected - ordinary periodic retarget arrives with a changed prevhash

My B1 claimed "on a retarget boundary the node changes nBits at the same
tip". Wrong for ordinary Bitcoin retargeting. The bits for the block at a
boundary height H = k*interval are computed against a NEW parent (the block
at H-1, just mined): `next_work_bits_base` (`header.rs` lines 244-271) takes
the retarget branch only when `height.is_multiple_of(interval)`, and the
template for height H carries that new parent's hash. So the pool's refresh
sees `chain_moved` (new height, new prev-hash) and republishes the job with
the new bits riding along. The ordinary retarget path is covered by existing
code; my "retarget experiment fails silently" narrative is RETRACTED.

The genuine same-tip bits transitions are narrower, and the plan's own §3.2
already names the big one:

- **Minimum-difficulty timestamp rule** (regtest, `allow_min_difficulty_blocks`
  = true): `header.rs::min_diff_gap` (lines 293-297) makes expected bits
  timestamp-dependent - once a template's `header_timestamp` crosses
  `parent_time + 2*spacing` (1200 s at 600 s spacing), expected bits move to
  the pow limit at a constant tip. Since a template's timestamp advances with
  wall clock (`template.rs::stamp_for` takes `max(mintime, now)`, lines
  123-125) while `job_changed` in `front.rs` (547-551) does not include
  `bits` or `curtime`, the pool can keep publishing a stale-bits/stale-ntime
  job across this transition. `rbitcoin` guards exactly this on submit
  (`session.rs::submit_timestamp_needs_other_bits`, 620-643: solutions whose
  timestamp needs different bits than the template pinned are dropped,
  logged only) - so the failure mode is a silently dropped block, not a
  rejected share. This is the concrete residual defect.
- RPC-sourced templates with advancing `curtime` at a constant tip: same
  blindness, same fix.
- Explicit test injection.

Also RETRACTED: my proposed test mechanism ("restart the node with a harder
`--regtest-pow-bits` on the same datadir tip"). The plan forbids this
("Do not use restarts with different PoW overrides as a live speed dial";
a changed profile is a new run with fresh chain state), and it would confound
the datadir's persisted bits history. Corrected test: (a) drive the front
with a mock template source injecting a same-tip template whose `bits`
differ (the pool has `MockTemplateSource` and `Template` is directly
constructible in tests - no node needed), and assert a new job id with the
new `nbits` is published within one refresh interval; or (b) a genuine
same-tip min-difficulty transition on regtest (advance time past the gap,
e.g. via the regtest-gated `setmocktime` used only as an invariant-scenario
input) and assert the same. Ordinary retarget coverage stays separate and
now asserts the positive fact: the new job at the boundary height carries
the new bits together with the new prev-hash.

Disposition: the fix (include `bits` and `curtime` in the job-change
predicate and in `published_key`) remains correct, cheap, and still worth a
numbered P1 prerequisite - but as hardening for the min-difficulty/curtime
path and injected transitions, at medium severity, not as the load-bearing
precondition of every P4 claim.

## 4. B2 byte-order disposition: CORRECTED - compare decoded numeric targets or pin an exact LE vector; do not assert the BE spelling

My B2 item 3 verified that the server puts `to_be_bytes()` into the `U256`
field (`sv2_server.rs` 240-241) and that `binary_sv2`'s `U256` (`Inner<true,
32, 0, 0>`) passes bytes through verbatim (`From<&[u8; 32]>` copies the
slice; `to_slice` writes it back unchanged - `binary_sv2-7.0.0` inner.rs
497-501, 206-219). That much stands. What was wrong is my proposed
assertion: "decode with an independent SV2 codec and assert they equal the
big-endian bytes of effective_share_target" conflates the value with one
encoding and would enshrine the suspected bug. Upstream SV2's integer
fields are little-endian by convention (`binary_sv2`'s own fixed copy types
document "little-endian byte" order - `copy_data_types.rs` line 15;
`from_bytes_`/`to_slice` are LE, lines 95-111). A stock upstream consumer
reading the target field per that convention decodes the pool's BE emission
as a byte-reversed - astronomically harder - target. Nothing in-tree would
catch this today: the pool's own client ignores the announced target
entirely (`sv2_client.rs` 132-141 reads only `channel_id` and the extranonce
fields; `e2e_sv2.rs` 173-178 likewise), so the announced target has never
been exercised against any consumer. Corrected disposition and test: pin the
expected wire encoding against the pinned upstream producer (per
UPSTREAM_DEPS.md) and an independent vector, then either (a) decode the 32
wire bytes the way an upstream miner does and compare the resulting
NUMERIC target to `effective_share_target(share_difficulty, nbits)`, or
(b) assert an exact little-endian wire byte vector for a known target. The
finding (verify independently before reuse) stands; my assertion form is
retracted.

Also corrected per the same logic: my test 1's framing. The all-ones
announced target is a legal value, not an illegal one; the defect is the
divergence between what is advertised at channel open (all-ones when no
template exists yet) and what is enforced at submit (the real job target) -
a miner that trusts the announcement will hash and submit shares the pool
then rejects with `difficulty-too-low`. The test asserts advertise/validate
consistency (announced target must equal the enforced target for the job
the miner is given, at all times including before-first-template), not
illegality of a value. The remaining B2 prerequisites - target lifecycle
(`SetTarget` or equivalent update semantics), replay-set lifetime, response
correlation - are RETAINED unchanged; the gap list itself was accurate.

## 5. B3 overlay interaction: PRECISION-CORRECTED - base is prev bits on regtest; the hazard is conditional clamp-then-snap-back

My B3 said "the first retarget boundary will jump to bits derived from
pow_limit-anchored history". Overstated. On a retarget-enabled regtest
profile, `enforce_bip94 = false` (`params.rs::regtest()` line 97), so
`retarget_bits` (`header.rs` 276-289) uses `base = prev_bits`, with
`timespan = prev_time - first.time` clamped to `[timespan/4, timespan*4]`
and the result clamped at the pow limit (`from_next_work_required`,
bitcoin-0.32.102 pow.rs 391-415). The boundary result is derived from the
parent's bits in general, not from pow-limit history.

The verified hazard, stated precisely: `apply_regtest_pow_overlay`
(`header.rs` 305-329) declines to substitute at boundary heights (lines
321-324) and only substitutes when the computed bits equal
`pow_limit.to_compact_lossy()`. Therefore, IF the boundary retarget clamps
to the pow-limit bits (a sufficiently slow period), THEN the boundary block
carries pow-limit bits, and the next NON-boundary height's base bits equal
the pow-limit compact bits, so - absent a min-difficulty gap - the overlay
snaps the chain back to the configured overlay bits at boundary+1. With
`--regtest-pow-bits 0x200fffff` that is a one-block spike from ~16 to ~2
expected hashes and back. A retargeting lab profile composed with the
overlay must therefore either disable the overlay or document this
clamp-then-snap-back condition as part of the profile fingerprint; my
original unconditional phrasing is corrected to this condition.

RETAINED unchanged from B3: the retarget profile is a new rbitcoin
feature (conf/CLI keys + `chain_params` plumbing + tests per that repo's
AGENTS.md test-first rule), not a thin wrapper - the cited regression mutates
`params.btc` fields inside the test body (`rbitcoin-rpc/tests/methods/
mod.rs` 6331-6333) and `chain_params()` (`rbitcoin-node/src/config.rs`
762-783) exposes no such keys; the wording precision about "in-process
regression"; the §3.2 checklist of profile fields to fix as config.

## 6. C1 evidence semantics: CORRECTED - absence is not rejection; canonicality is per-node and time-scoped

My C1 defined "rejection == a competing block at H confirms while the
candidate is absent from that node's chain". That equates losing a race
with being rejected, which is wrong: an absent candidate may be a valid
side branch, still propagating, or simply unobserved by that node. Corrected
semantics for the evidence predicates:

- **Accepted-in-node-view at time t**: the candidate's hash is the block at
  height H on that node's validated active chain per its chain-view RPC.
  Per-node and time-scoped, revisable by reorg - never a global permanent
  verdict.
- **Seen-but-not-canonical at t**: the node knows the block (side branch);
  the system's own settlement machinery already models the revisable case
  via `confirmation_depth` and `status='orphaned'` (SPEC §7.4), and the
  harness should mirror that rather than invent a stronger claim.
- **Absent/unobserved at t**: no claim. Not rejection.
- **Rejected**: requires authoritative rejection evidence. TDP
  `SubmitSolution` provides none - every failure path in rbitcoin's handler
  is a log line and `return Ok(())` (`session.rs` 521-617), including the
  bits/timestamp drop at 584-590 - so under TDP a timed-out candidate must be
  reported `unknown`/`inconclusive`, with the timeout recorded, never
  `rejected`.
- Canonicality assertions are always "canonical in node X's view at t",
  and scenarios must say which nodes' views a gate reads (the plan's §5.2
  already notes a partitioned network has no single global tip).

RETAINED from C1: the requirement to define the predicates in the plan
(P0/P2 gates currently say "candidate is accepted" without one); the
forbid-log-derived-acceptance rule; the observation that the drop path
makes "no news" ambiguous, which is exactly why the declared timeout must
produce an explicit inconclusive state rather than pending-forever. My
C1 test is likewise corrected: after heal, the candidate resolves to
accepted-canonical, accepted-then-orphaned, or remains a valid unobserved
side branch - the assertion must accept all three as honest outcomes, with
only the artifact recording which occurred.

## 7. C5 oracle scope and test: CORRECTED - business logic only; mutant output or known ground truth

Two corrections. First, my "no dependency on spc-accounting, spc-store, or
spc-pool" is too strong as written: sharing wire codecs, stream/event
types, and the frozen test vectors is acceptable; what must not be shared is
the business logic under test - work aggregation, floor placement, frozen
window reconstruction, denomination routing, receipt reconciliation. The
gate is "no shared business-logic code", not "no shared crates of any
kind". Second, my falsifiable test was logically flawed: injecting a wrong
routing into a replay produces a different actual output (the replay is
deterministic; it has no opinion about its inputs), so "oracle flags it
while the production replay stays green" proves nothing about oracle
independence. Corrected tests: (a) mutant form - seed the implementation
under test with a known bug (or use a recorded run from a known-buggy
build), record its actual outputs, and assert the oracle flags the
divergence from known ground truth; or (b) ground-truth form - construct a
small window with a hand-computed expected payout vector and assert the
oracle matches it exactly (floors, rails, decomposition included) while any
deliberately mis-ranked implementation copy does not. RETAINED unchanged:
the evidence-level qualifications - the oracle's inputs are recorded
accepted work plus the artifact's frozen `window_target`/`high_water` and
the four constant rungs; its output is compared against node-accepted
coinbases (evidence kind 3); the u128/NUMERIC(40,0) exactness rule; and
the "two endpoints reading the same database are not independent payout
evidence" principle.

## Retained recommendations (unchanged by these corrections)

1. **Resource/DB isolation checklist (C4, as written):** per-pool isolated
   Redis (separate instance or unique DB index - stream names are hard-coded
   constants: `spc:slices`/`spc:front:events` in `spc-store/src/bootstrap.rs`
   167-187, `spc:ark:orders` in `payout.rs` 993), unique Postgres database,
   unique datadirs/ports/Noise authorities per listener, and a documented
   node-TDP session budget (MAX_SESSIONS = 8). Namespacing-by-prefix stays an
   enhancement.
2. **Profile fingerprint ownership (C2, as written):** harness-computed
   (canonical serialization of effective consensus fields, hashed),
   compared across nodes in preflight, recorded in the artifact; node-side
   enforcement is an enhancement. The fingerprint must capture effective
   values including overlays, and - per correction 5 - the overlay/boundary
   composition decision is one of the fields it records.
3. **Height-progress preconditions (C6, as written):** warmup contract
   (node-generation bootstrap recorded as setup, exactly as
   `regtest-rbitcoin-tp-e2e.sh` line 110 does with 101 blocks, or a
   declared miner-driven warmup inside the run); height-driven assertions
   declare a background hashrate or expect `blocked-on-height`; era pinned
   in the manifest (150-block regtest halving verified in
   `block_subsidy`/`subsidy_halving_interval`).
4. **Evidence decimation requirements (N1, as written):** decimation is a
   rendering-series transformation only; the artifact's evidence log is
   complete for accounting checks or the run is marked inconclusive; the UI
   labels loss-affected charts and never derives accounting claims from
   decimated data.
5. **B2 prerequisites (with corrected disposition per item 4):** target
   lifecycle/update semantics, advertise/validate consistency including the
   pre-first-template case, replay-set lifetime, response correlation, and
   byte-order verification against the pinned upstream producer via decoded
   numeric comparison or an exact LE vector.
6. **B1 hardening (narrowed per item 3):** include `bits`/`curtime` in the
   job-change predicate; test via injection or a genuine same-tip
   min-difficulty transition; ordinary retarget coverage asserted as the
   positive bits-with-prevhash fact.
7. **B3 framing (with corrected overlay condition per item 5):** the
   retarget profile is a new rbitcoin feature under that repo's rules;
   profile fields fixed as config; the conditional clamp-then-snap-back
   hazard recorded in the profile checklist and fingerprint.
8. **C1 predicates (with corrected semantics per item 6):** define
   accepted/seen/absent/unknown states, per-node and time-scoped, with
   inconclusive as a first-class outcome; forbid log-derived verdicts.
9. **C3 constraint notes (with the main claim retracted per item 2):** the
   u128/NUMERIC(40,0) window-target constraint and the `N x d` preservation
   requirement. The CPU-scale share-target/work-unit design itself is
   required P0 work exactly as the plan states.

## Corrected bottom line

The original verdict (accept with amendments) stands, with a different top
three. The pre-implementation items are now: **B2** (target lifecycle,
advertise/validate consistency, byte order vs upstream - P1's gates are
unverifiable without it), **C3-corrected** (the explicit CPU-scale
share-target/work-unit decision is required, not already available - the
plan's own framing was right), and **B3** (the rbitcoin retarget profile
feature, with the precisely-stated overlay interaction). B1 drops to
medium (min-difficulty/curtime path and injected transitions). C1's
predicate work remains necessary but with observational semantics -
absence is unknown, not rejection, and canonicality is per-node and
time-scoped. C2, C4, C5, C6, N1 and the remaining N2 wording corrections
are unaffected by this pass.
