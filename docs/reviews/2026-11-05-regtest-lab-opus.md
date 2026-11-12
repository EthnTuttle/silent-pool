# Review: docs/REGTEST-LAB.md (regtest network lab, real SV2 CPU mining)

- Reviewer model: Claude Opus 5.5 (provider amazon-bedrock, model id `us.anthropic.claude-opus-5-5`)
- Review date: 2026-11-05
- Baseline plan: `docs/REGTEST-LAB.md`, SHA256
  `444060b2c1c7d377960116032dc88af114c0dbc43741a8c719b33188e8b99098` (verified with
  `sha256sum` before review), proposal date 2026-11-05
- silent-pool tree: HEAD `b0ab4ca`, with the staged SV2 TP work (`template_tp.rs`,
  `regtest-rbitcoin-tp-e2e.sh`, decision 2026-11-05-sv2-tp-chain-view) left as found
- Note: while this review was being written, the staged TP files disappeared from the main
  worktree's index. The work now appears as commits on branch `sv2-tp` (`1ec1dd0`) in a
  separate opencode worktree. This reviewer did not cause that change; it ran only
  read-only commands. `template_tp.rs` and `main.rs` line numbers below refer to the staged
  copy as read, and may have shifted on that branch.
- rbitcoin tree: `/home/ethan/code/rbitcoin` HEAD `e8cf9c9d6`
- Upstream SV2 reference consulted: the crates resolved by `Cargo.lock`
  (`channels_sv2 8.0.0`, `mining_sv2 12.0.0`, `stratum_translation 0.3.0`) in the local
  cargo registry
- Independence: I did not read other reviewers' output. Nothing was executed apart from
  `sha256sum`, `git log/status`, `grep` and file reads. Every "test" below is proposed,
  not run. Line numbers refer to the trees named above.

## Overall verdict

**The direction is right, but the plan is not ready to start P1 as written.** It
correctly keeps PoW, block targets and block acceptance on the node, and it correctly
keeps the browser out of the run lifetime. Its source audit (§5.1 "Existing SV2 gaps")
points in the right direction but is too gentle. Three SV2 wire problems in the
current pool make it non-interoperable with a standards-following miner: job/prevhash
ordering, target byte order and merkle-path order (B1-B3). Because of them, a "native
SV2 CPU miner" built from the in-repo helpers would agree with the pool *because it
shares the same bugs*. The plan already warns about this in P1. It needs to become a
hard gate: the lab miner's protocol layer must use upstream `channels_sv2` client state
(or an equivalent independent oracle), not `spc-agent::sv2_client` or the `e2e_sv2.rs`
logic.

On target lifecycle and work accounting, the plan treats "fractional vs scaled work
units" as the open question. The bigger problem is the existing binding window rule
`N x d`, where `d` is the per-share credit (A1). It makes the window N *shares*, not N
blocks of work. That coincides with the intended semantics only while share target ==
block target, which is exactly the condition P1 sets out to remove. The plan tells
implementers to preserve this rule. Instead it should require an explicit decision
before any economic result is reported.

On retargeting, the plan correctly flags that a profile is needed. It misses three
concrete traps in rbitcoin: (1) the `--regtest-pow-bits` overlay interacts incoherently
with retargeting; (2) a shortened interval biases the equilibrium; (3) with
minimum-difficulty enabled, the TP's per-prevhash bit pinning can silently drop every
solution (A2-A4).

Finally, the staged TDP path refuses any template containing a witness transaction
(B4). Every fee, wallet and Ark scenario in §5.2/§7/P4 therefore stalls a
silent-pool-only network.

## Ranked findings

Severity scale: Critical (blocks the stated goal or yields wrong conclusions silently),
High (blocks a packet's acceptance or biases results), Medium (correctness/diagnosis gap
with workaround), Low.

Categories: **A = plan defect** (the document is wrong, missing or unfalsifiable),
**B = existing implementation blocker** (the code the plan reuses is wrong or missing a
capability), **C = optional future enhancement**.

### B1 - Critical - SV2 job/prevhash ordering is inverted and over-broadcast

- Plan sections: §5.1 bullets 2 and "Existing SV2 gaps" bullet 3; P1.
- Evidence: `bins/spc-pool/src/sv2_server.rs:423-461` (`send_job`). For *every* published
  job it sends `SetNewPrevHash{job_id}` **before** `NewExtendedMiningJob{same job_id,
  min_ntime: Some(..)}` (a non-future job). This includes same-tip refreshes (fee pushes,
  distribution-fingerprint changes; `front.rs:550-559,655-659`). Upstream client state
  `channels_sv2-8.0.0/src/client/extended.rs:740-755` returns `JobIdNotFound` when a
  `SetNewPrevHash` names a job that was not previously received as a future job. On
  success it also moves all past jobs to stale. The plan attributes the future-job
  correlation gap to "existing client helpers". The server itself is the source.
- Consequence: a compliant SV2 miner fails on the first job. A permissive one discards
  all work on every same-tip refresh, and stale rates rise for reasons unrelated to the
  network.
- Fix: on a tip change, send `NewExtendedMiningJob` with empty `min_ntime` (future), then
  `SetNewPrevHash` referencing it. On a same-tip change, send only a non-future
  `NewExtendedMiningJob`. Decide and document whether same-prevhash older jobs remain
  valid for submission (SV2 allows it; `submission_error` at `sv2_server.rs:480`
  currently accepts only the latest global job).
- Acceptance test: drive an upstream `channels_sv2::client::extended::ExtendedChannel` from
  the pool's wire messages across (a) channel open with an existing template, (b) a
  same-tip fee refresh, and (c) a tip change. Assert no `JobIdNotFound`, exactly one
  `SetNewPrevHash` per tip, and that a share on the pre-refresh same-tip job is handled
  per the documented policy (accepted, or rejected with `stale-share`).

### B2 - Critical - SV2 targets are encoded big-endian on the wire

- Plan sections: §5.1 "Existing SV2 gaps" bullet 1 ("independently verify its wire byte
  order"); P1 "target encoding".
- Evidence: `sv2_server.rs:240-241` builds `U256::from(&share_target.to_be_bytes())`. The
  unit test `announced_target_is_the_pool_share_target` (`sv2_server.rs:586-588,651-668`)
  freezes big-endian. Upstream encodes targets with `to_le_bytes()`
  (`stratum_translation-0.3.0/src/sv1_to_sv2.rs:32`,
  `channels_sv2-8.0.0/src/vardiff/classic.rs:186`) and compares share hashes as
  little-endian targets (`channels_sv2-8.0.0/src/server/extended.rs:1062`). SV2 integers
  are little-endian. The agent client sends an all-ones `max_target`
  (`bins/spc-agent/src/sv2_client.rs:109-114`), which is byte-order symmetric, so no
  current test can detect this.
- Consequence: a compliant miner reads a byte-reversed target. The regtest network target
  `7fffff00..00` becomes nearly impossible to meet, and difficulty-1 `00000000ffff00..`
  becomes nearly all-ones. Share submission rate is then wrong by many orders of
  magnitude.
- Fix: encode/decode every SV2 `U256` target little-endian (`OpenExtendedMiningChannelSuccess.target`,
  future `SetTarget.maximum_target`, inbound `max_target`). Replace the frozen BE test.
- Acceptance test: an asymmetric vector. For bits `0x1d00ffff` and share difficulty 4,
  `Target::from_le_bytes(success.target.inner_as_ref())` must equal
  `effective_share_target(4, 0x1d00ffff)`; the reversed interpretation must not.

### B3 - Critical - Merkle path order is topmost-first on both SV1 and SV2 wires

- Plan sections: §5.1 "Existing SV2 gaps" bullet 3 ("include nonempty merkle-path
  vectors"); P1.
- Evidence: `bins/spc-pool/src/block.rs:46-77` builds the branch bottom-up and then
  `reverse()`s it ("topmost sibling first"), calling that "Stratum order". The pool folds
  it with `.rev()` internally (`sv2_server.rs:287`, `sv1.rs:443`, `front.rs:1697`, and the
  e2e miner `bins/spc-pool/tests/e2e_sv2.rs:217`). It is sent unreversed
  (`sv2_server.rs:443-450`, `sv1.rs:550`). TDP's deepest-first path is deliberately
  reversed into this order (`template_tp.rs:24-26,495`). Upstream folds `merkle_path` in
  order, deepest first (`channels_sv2-8.0.0/src/merkle_root.rs`,
  `merkle_root_from_path_`). The repo's own SV1 CPU miner also folds in order
  (`testing/regtest-miner.py:85-87`), so it would disagree with the pool once there are
  two or more branch levels.
- Consequence: with an empty mempool or one transaction (branch length 0 or 1) everything
  works, which is why existing harnesses pass. With two or more non-coinbase
  transactions, every external miner computes a different merkle root. Shares fail
  `difficulty-too-low` (pool and miner hashed different headers) and no block is ever
  found. The plan's §2 table calls `regtest-miner.py` "a useful reference" without noting
  this disagreement.
- Fix: store and transmit deepest-first (SV1 and SV2). Remove the internal `.rev()` folds.
  Keep the TDP path in its native order.
- Acceptance test: a template with 3 non-coinbase legacy transactions (2-level branch).
  `merkle_root_from_path(coinbase_prefix, coinbase_suffix, extranonce, merkle_path)` from
  the wire message must equal the merkle root of the block the front assembles, and
  rbitcoin must accept a block found by `testing/regtest-miner.py` against that template.

### A1 - Critical - The preserved window rule `N x d` turns into "N shares" once shares are easier than blocks

- Plan sections: §3.3 ("Preserve the SPEC §4.3 window semantics (`N x d`) ... a unit
  conversion is not permission to replace that rule with a network-difficulty-based
  window"), §6 independent oracle, §7 economics, P1, P4.
- Evidence: `docs/SPEC.md:410,448` define the window target as `N x d` with
  `d = min(share_difficulty, max(1, round(D_net)))`. `front.rs:585-586` computes
  `window_n * credited_difficulty`, so the window target is N times the *per-share* credit:
  N shares, about 8 by default (`config/spc-pool.example.toml:61`). The same SPEC still
  says `N x D` in terminology (`docs/SPEC.md:100`) and in the §8.3 type-32 test
  (`docs/SPEC.md:858`). The decision rationale
  (`docs/decisions/2026-11-02-share-target-policy.md`, "Why") says a network-difficulty
  window would be "trivially short" on mainnet. In fact `sum(d) >= N x D_net` spans N
  blocks of expected work. Whole-slice granularity (§4.3) plus per-height sealing
  (`front.rs:559-566`) mean that in practice the window is usually just the most recent
  sealed slice, i.e. the shares since the previous *network* block.
- Why this matters for the lab: today's baseline has share target == block target, so
  "N shares" equals "N blocks". P1's stated acceptance ("observed non-block accepted
  shares") breaks that equality. From then on, window membership, payout variance and the
  fairness/pool-share studies in §7 depend on slice sealing and competing pools' block
  cadence, not on N. An oracle that faithfully encodes `N x d` will agree with the
  implementation and still describe an economically different system from the README's
  PPLNS intent.
- Classification: an existing normative inconsistency. Under AGENTS.md precedence it must
  be raised, not silently resolved. The plan defect is that it instructs implementers to
  entrench the rule instead of making it an explicit P0 gate.
- Fix: add a P0 item: "Confirm or amend the §4.3 window unit via a decision record before
  P1 introduces share target < block target. Fix SPEC §0/§8.3 so they agree." Until then,
  label any economic result as "under window rule N x d (N shares)".
- Acceptance test: with block target = 64 x share target, the oracle reconstructs the
  window for a found block. Under the current rule, `sum(d)` is in
  `[N x d, N x d + max slice]` and covers about 1 network block interval. Under the
  `N x D_net` reading it covers about N x 64 shares. The test fixes which one SPEC
  requires, and the front must match the oracle.

### A2 - High - `--regtest-pow-bits` overlay and retargeting are incoherent together

- Plan sections: §3.2 ("Specify initial difficulty/bootstrap behavior, pow limit ..."),
  P0.
- Evidence: `rbitcoin/crates/rbitcoin-consensus/src/header.rs:305-329`
  (`apply_regtest_pow_overlay`) rewrites any non-boundary `bits == pow_limit` to the
  overlay. With retargeting on and `allow_min_difficulty_blocks=false`, `min_diff_bits`
  returns `prev_bits` (`header.rs:340-341`). If a retarget clamps to `pow_limit`
  (`header.rs:276-289` via `from_next_work_required`, likely after the plan's "abrupt hash
  loss" scenario), the boundary block carries `pow_limit` but the next non-boundary block
  must carry the overlay. Difficulty then jumps back mid-period, independent of
  hashrate. Regtest `pow_limit` is `MAX_ATTAINABLE_REGTEST` (`params.rs:84`), about 2
  hashes per block, so the downward clamp has no lab-meaningful floor.
- Fix: the retarget profile MUST NOT use the overlay. Set initial difficulty with a profile
  `pow_limit` / genesis-bits parameter, or with a recorded warmup, and record the
  `pow_limit` as the difficulty floor. Preflight rejects overlay + retarget.
- Acceptance test (in-process, cheap): with params `no_pow_retargeting=false`,
  `allow_min_difficulty_blocks=false`, interval 4, overlay set, and a period whose
  timespan forces the result to `pow_limit`, `next_work_bits` at boundary+1 must equal the
  boundary bits. This fails today.

### A3 - High - Min-difficulty + per-prevhash pinned bits + ntime rolling can silently stop the chain

- Plan sections: §3.2 ("miners always use actual template bits"), §5.1 ("time"), §5.2
  pause, P4 "recovery after abrupt hash loss".
- Evidence: rbitcoin TP pins `n_bits` once per prevhash
  (`rbitcoin/crates/rbitcoin-sv2/src/session.rs:484-490`). It silently drops a
  `SubmitSolution` whose timestamp would need different bits (`session.rs:571-590,
  622-643`). Regtest keeps the min-difficulty rule, where `header_time > prev_time + 1200`
  requires `pow_limit` bits (`header.rs:291-297,325,344`). The pool accepts ntime within
  +/-7200 s of the job time (`bins/spc-pool/src/sv1.rs:139-156`) and credits the share
  before submission. TDP has no ack; the front waits up to 10 s per found block
  (`template_tp.rs:299-366`) inline in the share loop (`bins/spc-pool/src/main.rs:472-477`).
- Consequence: after any gap over 20 min (pause, hash loss, low-hash scenario), a miner
  that rolls ntime toward wall clock produces only dropped solutions until a different
  prevhash arrives, and none ever does. "Using template bits" is therefore not enough.
  Each drop also stalls the front for up to 10 s.
- Fix: (1) the retarget profile disables min-difficulty blocks (recommended), and the
  baseline profile declares that pauses over 2x spacing are unsupported. Or (2) the miner
  ntime policy is `min_ntime <= ntime <= prev_time + 2*spacing` while bits are pinned, and
  the pool rejects shares with ntime that would need other bits. Also move block
  submission off the share loop.
- Acceptance test: baseline profile, pause all hash workers for longer than 1200 s
  (or set node mocktime equivalently), then resume with wall-clock ntime. Assert that a
  block is accepted within K expected intervals, or that preflight refused the scenario.
  Today the expected outcome is "needs different bits; dropping" log lines and no tip
  movement.

### A4 - High - Shortened schedules bias equilibrium and are noisy; the plan gives no quantitative guidance

- Plan sections: §3.2, §7, §9 Q1, P4 acceptance ("follow the declared algorithm").
- Evidence: `period_first` is `height - interval`
  (`rbitcoin/crates/rbitcoin-query/src/header_read.rs:209-223`), and the timespan is
  `time(H-1) - time(H-interval)` (`header.rs:287`). That is `n-1` gaps measured against an
  `n`-gap target. Equilibrium spacing is `s * n/(n-1)`: +33% at n=4, +6.7% at n=16, about
  0 at 2016. Per-period relative noise is about `1/sqrt(n-1)` (58% at n=4), so the 4x
  clamps trigger often. Header time has 1 s resolution and must exceed the MTP of 11
  blocks, so spacings of a few seconds are dominated by quantization. Job `curtime` is the
  `SetNewPrevHash` timestamp and is not refreshed on same-tip templates
  (`template_tp.rs:600,650`). A miner that does not roll ntime therefore stamps each block
  with roughly its parent's discovery time, a one-block shift that matters at small n.
- Fix: P0 profile guidance: interval at least 32-64 and spacing at least ~10-30 s for
  response studies. Record n/(n-1) as a known bias. Define the miner ntime policy
  explicitly (for example `max(min_ntime, wallclock)` bounded per A3). P4 acceptance
  compares node bits with an *independent* reimplementation of the declared algorithm on
  the observed timestamps. "Interval converges to spacing" is not a valid assertion.
- Acceptance test: for each boundary, the oracle's `expected_bits(timestamps, prev_bits)`
  equals the node's header bits. A constant-hashrate run over at least 10 periods has a
  mean interval within the CI of `s*n/(n-1)`, not `s`.

### B4 - High - Witness transactions stall TDP-fed pools; this blocks fee/wallet/Ark scenarios

- Plan sections: §5.2 ("Introduce transactions ..."), §7 fee workloads, P4 transaction-fee
  and Ark outage/refill scenarios.
- Evidence: `bins/spc-pool/src/template_tp.rs:28-33,698-720` refuses the whole template if
  any transaction has a witness, and caches the refusal. `front.rs:540-543` fails closed
  likewise. The RPC path excludes witness transactions one by one
  (`template_rpc.rs:53-77`), which can also leave a legacy child whose parent was
  excluded. A silent-pool front with no template issues no jobs.
- Consequence: one P2WPKH/P2TR transaction (any modern wallet, any Ark round or board) in a
  shared node's mempool stops every silent-pool instance on that node until a non-pool
  miner confirms it. In the plan's topology there is no such miner except setup RPC
  generation.
- Fix: list BIP141 commitment support as a capability prerequisite for any transaction
  workload, and make preflight fail transaction/Ark scenarios until it lands. Or restrict
  the workload adapter to legacy-only transactions and label results accordingly.
- Acceptance test: during a TDP run, broadcast one P2WPKH transaction to the shared node.
  Within 2 refresh intervals, every pool still publishes a new job and the transaction is
  eventually mined by a pool block (capability present), or preflight rejected the
  scenario (capability absent).

### B5 - High - Ingress acknowledges, and the front trusts, unverified header fields; extranonce length is unchecked

- Plan sections: §3.3 ("retaining PoW verification at ingress and in the front"), §5.1
  gaps bullets 2 and 5, P1.
- Evidence: SV2 ingress appends `msg.extranonce` to the prefix without checking its length
  against `extranonce_size` (`sv2_server.rs:281-285`). It sends `SubmitSharesSuccess`
  after a fire-and-forget `shares.send` (`sv2_server.rs:342-349`), with
  `new_shares_sum: 1` regardless of credited work (upstream tracks work sum,
  `channels_sv2-8.0.0/src/server/share_accounting.rs:93,160`). The front never recomputes
  the header hash: it checks the ingress-supplied `share_hash`
  (`front.rs:733-737,801-802`) and does not check extranonce length. Front rejections are
  only logged (`main.rs:494`). The plan's claim of front PoW verification is therefore
  not true today.
- Consequence: a miner bug with a wrong extranonce length yields a malformed coinbase that
  is still credited (on easy regtest bits it even registers as a block candidate), and the
  miner sees success for work the front later drops.
- Fix: reject `len(extranonce) != extranonce_size` at ingress and front. Have the front
  rebuild the header from (job, coinbase, version, ntime, nonce) and rehash it before
  crediting. Either acknowledge only after the front accepts, or name the ack "ingress
  accepted" and correlate front outcomes in observations. Set `new_shares_sum` to the
  credited work in the agreed unit.
- Acceptance test: submit a 3-byte and a 5-byte extranonce on a 4-byte channel; each gets
  `SubmitSharesError` and no `ShareRec`. A forged `Share` with a correct-looking
  `share_hash` but a mismatched header is rejected by `Front::on_share`.

### B6 - Medium - Bits/version/time-only template changes do not bump or republish the job (RPC path)

- Plan sections: §5.1 gaps bullet 6 (already flagged; this pins the mechanism).
- Evidence: `front.rs:547-558` bumps `job_seq` only on chain/fee/txset changes.
  `published_key` (`main.rs:926-933`) omits bits/version/curtime. `front.job` and
  `template.bits` update while SV1/SV2 ingress keep the old job. The front assembles with
  the new bits (`front.rs:808-816`) a header the miner hashed with the old bits. Under
  TDP, bits are pinned per prevhash (A3), so this mainly affects the RPC
  min-difficulty path.
- Fix: include bits, version and curtime in the job-change predicate and in
  `published_key`. Or have the front validate against the job the share names, not the
  latest template.
- Acceptance test: a mock template source flips only `bits` at the same prevhash/txset. The
  job id changes, both ingresses republish, and a share on the old job is rejected as
  stale instead of being assembled with the new bits.

### B7 - Medium - Miner-facing Noise authority is ephemeral; the client does not pin

- Plan sections: §5.1 gaps bullet 4, P1 "authenticated session setup", §5.2 kill/restart.
- Evidence: `Sv2Server::new` generates a fresh keypair per process
  (`sv2_server.rs:73-77`), so no stable key exists to provision. The agent connects with
  `connect_with_noise(stream, None)` (`bins/spc-agent/src/sv2_client.rs:79`). Each restart,
  and each front of one pool, presents a different authority.
- Fix: configured per-pool authority key (MULTI-REGION: shared across fronts or a declared
  set), loaded fail-closed like `sv2_tp_authority_pubkey` (`main.rs:337-351`). The lab
  miner pins it.
- Acceptance test: restart a front; a miner pinned to the configured key reconnects. A
  miner pinned to a different key fails the handshake.

### B8 - Medium - Channel model ignores negotiation inputs and reuses channel id 1

- Plan sections: §5.1 gaps bullet 2, §9 Q3 (extranonce uniqueness).
- Evidence: `channel_id: 1` for every connection (`sv2_server.rs:222-223`), and it is part
  of the process-wide nullifier (`front.rs:743-750`). A second open replaces the channel
  (`sv2_server.rs:254`). `min_extranonce_size`, `max_target` and `nominal_hash_rate` from
  `OpenExtendedMiningChannel` are ignored (`sv2_server.rs:175-249`). The extranonce prefix
  is 4 random bytes (`sv2_server.rs:202-205`), so uniqueness across channels and fronts is
  probabilistic, not by construction. Workers are not represented in `ShareRec` (SPEC
  §4.2), so per-worker accounting can only be miner-side.
- Fix: allocate prefixes from a per-front counter namespaced by `front_id`. Reject
  `min_extranonce_size > 4` and any `max_target` the pool cannot honour (or clamp
  `target <= max_target` as SV2 requires). Assign unique channel ids. State in §4 that
  worker identity is a miner/observer concept, not a pool accounting key.
- Acceptance test: 1,000 opens across 2 fronts produce no duplicate prefix. An open with
  `min_extranonce_size=8` gets `OpenMiningChannelError`. An open with `max_target`
  below the effective target gets an error or a target `<= max_target`.

### A5 - Medium - "Nodes reject mismatched profiles" is not implementable as stated

- Plan sections: §3.2 last paragraph, P0 acceptance.
- Evidence: the profile (`regtest_pow_bits`, retarget params) is node-local config
  (`rbitcoin/crates/rbitcoin-node/src/config.rs:497,777-780,1631`). I found no P2P or RPC
  advertisement of it (`grep` over `rbitcoin-rpc/src`, `rbitcoin-net/src`). Nodes with
  the same genesis connect and only diverge at the first block whose bits differ. The pool
  also hard-codes the halving interval per network (`main.rs:343`,
  `template_tp.rs:136-143`). rbitcoin has `subsidy_halving_overlay`
  (`params.rs:45,228`), but it is not exposed in node config. A profile that changes it
  would make the pool misclassify subsidy as fees or fail closed.
- Fix: the harness owns the profile fingerprint: it renders identical config for every
  node and every pool, and verifies through an RPC introspection endpoint (a node-side
  addition) before starting. Include the pool's halving/subsidy expectations in the
  fingerprint.
- Acceptance test: start two nodes whose rendered profiles differ in one field; preflight
  refuses before any P2P connect. Start a pool with a halving interval different from
  the node's; preflight refuses.

### A6 - Medium - Topology omits chain-attached Ark infrastructure and P2P leakage paths

- Plan sections: §4 (links, partitions), §9 Q4.
- Evidence: the Ark prototypes require a `bitcoind` (`AGENTS.md`, `spc_ark_client/run_e2e.sh`).
  An Ark server and its chain backend are additional chain nodes. If they peer with the
  lab network, they relay blocks across a supposed partition. Nodes with listening P2P
  sockets accept inbound from any peer that learns their address via addr relay.
- Fix: model the Ark server and its chain backend as topology nodes with explicit links.
  Require connect-only, no-listen (or loopback-isolated) P2P with addr relay disabled on
  partitioned nodes, and assert peer sets through RPC during a partition.
- Acceptance test: during a partition, every node's peer list equals the declared set, and
  a block mined on side A never appears on side B (including via the Ark backend) until
  heal.

### A7 - Medium - Work-unit decision lacks a concrete integer option and a scale argument

- Plan sections: §3.3 open decision, §9 Q2.
- Analysis: with the current u64 `d >= 1` relative to difficulty-1, a share costs about 2^32
  hashes. At an assumed order of tens of MH/s per CPU core (an estimate, not measured), that
  is minutes per share per core. Block targets harder than that make block intervals of
  hours, so integer difficulty-1 shares do not fit a lab. The plan should not default to
  "fractional". A fingerprinted, declared reference target `T_ref` (for example `2^240`)
  with integer work units `d = T_ref / T_share` keeps every persisted field (`ShareRec.d`,
  `slice.difficulty` u128, `window_target` NUMERIC) integer and exactly proportional across
  mixed targets. It differs from the rejected "powLimit-relative difficulty"
  (decision 2026-11-02, alternatives) because `T_ref` is explicit and recorded, not
  inferred from the chain. Either way it needs a decision record (the plan says so), and
  the `min(.., round(D_net))` cap must be restated in the same unit.
- Fix: put the `T_ref` integer-unit option in the P0 decision matrix, with criteria:
  restrict share targets to `T_ref / d` (exact), no float on any accounting path,
  `T_ref` part of the run fingerprint and refused on mainnet.
- Acceptance test: two miners with share targets `T_ref/1` and `T_ref/4` hashing equal
  attempted hashes earn credited work within a chi-square bound of 1:1, and every
  persisted value round-trips exactly.

### B9 - Medium - Shared code makes several existing tests non-independent

- Plan sections: §6 "Independent checks", P1 interoperability evidence, §9 Q8.
- Evidence: `bins/spc-pool/tests/e2e_sv2.rs:223` uses `spc_pool::sv2_server::target_from_bits`.
  The e2e merkle fold mirrors the pool's `.rev()` (B3). `spc-agent::sv2_client` reads
  `SetNewPrevHash` before the job (`sv2_client.rs:148-180,290-352`), mirroring B1. All
  window/credit computation flows through `crate::sv1::credited_difficulty` and
  `spc_accounting::select_window`.
- Fix: the oracle and lab miner must not depend on `spc-pool`, `spc-accounting` or
  `spc-agent::sv2_client`. Use `channels_sv2` client state for protocol and a
  separately written window/credit oracle. Add a lint or `cargo tree` check in the lab
  crate.
- Acceptance test: `cargo tree -p <lab-miner/oracle>` contains no `spc-pool`,
  `spc-accounting` or `spc-agent`. Fault injection (for example, flip the merkle fold in
  the pool) is detected by the lab, not masked.

### A8 - Low - Smaller plan inaccuracies

- §2 table, "Existing CPU fixture": `regtest-miner.py` disagrees with the pool's merkle
  order (B3). It is a reference for SV1 *standard* behaviour, not for this pool.
- §7: the "node-side extension" for reward schedules partly exists
  (`params.rs` `subsidy_halving_overlay`) but is not exposed. The pool side also needs
  changing (A5).
- §5.1: "Submit miner-controlled extranonce bytes, not the pool prefix" already matches the
  server (`sv2_server.rs:281-282`). The real gap is the missing length check (B5).
- §3.1: worth stating that SV2 `SetNewPrevHash.nbits` changes only with prevhash under the
  rbitcoin TP. Retargets therefore always arrive with a tip change, and the "retarget vs
  share-target update" interaction (§3.3) is tied to the prevhash transition.

### C1 - Optional enhancements

- An upstream SRI pool or translator as a second "other pool implementation" adapter.
  Cheap interoperability evidence once B1-B3 are fixed.
- Per-channel VarDiff via `channels_sv2::vardiff` after the unit decision (A7).
- Network namespaces for per-link loss/latency instead of userspace proxies.
- Recorded-run replay that re-derives oracle results from artifacts only (no live DB).

## Answers to plan §9 questions (short)

1. Retarget realism: the existing in-process profile (`rbitcoin-rpc/tests/methods/mod.rs:6331-6334`)
   is the right base. Expose it without the overlay (A2) and with min-difficulty off (A3).
   Use n >= 32-64 and record the n/(n-1) bias (A4). The normal 2016 schedule is
   impractical for CPU labs; a shortened schedule is justified only if it is labelled.
2. Share units: prefer a declared integer `T_ref` scale (A7). Resolve the window unit first
   (A1).
3. SV2 correctness: not today. B1-B3 block interoperability; B5/B8 block correct
   accounting acknowledgments and uniqueness by construction.
4. Topology: add Ark/chain backends and P2P listen/addr controls (A6).
5. Rate control: use attempted-hash counters as ground truth. Discovery is per-hash
   memoryless, so batching biases only latency (staleness), not discovery probability.
   Keep batch duration well below job lifetime and report the stale fraction per batch
   size.
6. Event ordering: TDP has no ack (`template_tp.rs:299-306`). Block acceptance = node tip
   or chainwork containing the hash, canonical inclusion = depth on the declared node,
   settlement = Postgres row state. Each assertion records the node and the tip it held,
   and a reorg invalidates assertions scoped to the orphaned branch.
7. Economics: any result depends on A1. Also, B4 means no fee economics until witness
   support exists.
8. Independent checks: see B9.
9. Privacy: the plan is sound. Note that verifying output correctness must go through
   recipient-side scanning with fixture-held `b_scan`, never pool-side `a_send`.
10. First headless run: B1-B3, the B5 length check, a minimal unit decision (or keep
    share == block for the first run) and P2 isolation. Retargeting and multi-pool come
    after.

## Open questions

1. Is `N x d` (N shares) the intended window, given SPEC §0/§8.3 still say `N x D`? (A1)
2. Should shares on an older job with the same prevhash be accepted after a same-tip
   refresh? This determines stale rates under CPU-scale share rates. (B1)
3. Will the retarget experiment run with min-difficulty disabled, and is the baseline
   allowed to pause longer than 2x spacing? (A3)
4. Is BIP141 commitment support in scope before P4, or are transaction workloads limited to
   legacy transactions? (B4)
5. Is one authority key shared across fronts of one pool, or is a per-front key set
   advertised? (B7)

## Minimal recommended plan amendments

1. §5.1: replace the "existing SV2 gaps" bullets 1-3 with B1, B2 and B3 as concrete
   defects. Make "lab miner protocol layer uses upstream `channels_sv2` client state and
   shares no code with `spc-pool`/`spc-agent`" a P1 entry criterion (B9).
2. §3.3/P0: add a gate to confirm or amend the §4.3 window unit by decision record before
   any non-block shares. Add the `T_ref` integer option to the unit decision (A1, A7).
3. §3.2/P0: the retarget profile forbids the overlay, disables min-difficulty, declares
   `pow_limit` as a floor, uses interval >= 32 and documents the n/(n-1) bias. Define the
   miner ntime policy (A2-A4).
4. P0 preflight: the harness renders and verifies a single profile fingerprint across nodes
   and pools, including the halving interval. Remove "nodes reject mismatched profiles"
   (A5).
5. §5.2/P4: transaction and Ark scenarios are capability-gated on witness commitment
   support (B4).
6. P1 acceptance: add the extranonce-length, front rehash, ack-correlation, bits-only
   republish and pinned-authority tests (B5-B7).
7. §4: model Ark/chain backends as topology nodes and require isolated P2P configuration
   (A6).

## Addendum (2026-11-05): clarifications after coordinator follow-up

The original text above is kept unchanged. Where the two disagree, this addendum takes
precedence. B1-B3 are unchanged; the coordinator reports their source checks were
corroborated. As before, nothing was executed. The derivation in C3 is analytical, under
the idealized assumptions stated there.

### C1 - B2 illustrative direction corrected

The original example (B2, "difficulty-1 ... becomes nearly all-ones") is wrong in
direction. Reading a big-endian target as little-endian moves its high-order bytes to
the low-order end:

- Difficulty-1 `00000000ffff0000..00` (BE) read as LE has value `0xffff * 256^4`, about
  2^48. That is a far *smaller* (harder) target, not nearly all-ones.
- Regtest `7fffff00..00` read as LE has value about `0xffff7f`, about 2^24. That is also
  far harder.

For realistic targets, a compliant miner reading the pool's BE bytes would see an
essentially unreachable target and submit almost nothing. The source-level
incompatibility, the fix and the acceptance test in B2 are unchanged.

### C2 - A1 acceptance and wording corrected

- **Acceptance direction.** "Block target = 64 x share target" is backwards. To get
  harder blocks, the test needs **block target ~ share target / 64**, i.e. network
  difficulty about 64 x share difficulty, so about 64 accepted shares per block are
  expected.
- **What `N x d` means.** It is a work threshold of N shares' worth of credited work
  (`N x d` in the credited unit). It is not invariably "exactly 8 shares":
  - Selection takes whole slices, walking back from (and excluding) the finding slice,
    until the cumulative `sum(d)` reaches `N x d`. The crossing slice is included.
  - So the window holds at least N x d units, plus overshoot of up to the rest of the
    crossing slice.
  - Slices seal on a height change, on `max_slice_shares`, on the fee band, and on
    distribution-only job changes (`front.rs:559-566`, `655-659`;
    `crates/spc-accounting/src/lib.rs:92-101`).
  - The overshoot therefore depends on slice cadence. The window may hold many more than
    N shares, and with mixed credited `d` (the cap at `round(D_net)` across retargets) the
    share count is not fixed at all.
- **Corrected acceptance.** With network difficulty about 64 x share difficulty, the
  oracle checks:
  1. the reconstructed window satisfies `sum(d) >= N x d`;
  2. removing the oldest included slice would leave `sum(d) < N x d` (whole-slice
     minimality);
  3. the front's window equals the oracle's.

  The test then reports the window's expected-work coverage in network blocks
  (`sum(d) / D_net`). It does not assert a fixed share count.
- **Authority.** The current rule comes from the binding decision
  `2026-11-02-share-target-policy.md` and SPEC §4.1/§4.3. A1 does not claim it is wrong to
  implement. It flags that SPEC §0 (`docs/SPEC.md:100`) and §8.3 (`:858`) still say
  `N x D`, and that the lab's economic results depend on which reading is intended.
  Whether to keep or amend the rule is an owner/spec decision through a decision record
  and changelog. The lab plan should gate economic claims on that decision; it should not
  choose either reading itself.

### C3 - A4 qualified: `s * n/(n-1)` is not a derived stationary mean

The original A4 used two figures that were not derived for a stochastic process:
`s * n/(n-1)` as an "equilibrium" with "+33% at n=4", and a CI "centered" on it. That
formula is a **deterministic, noiseless finite-endpoint argument**: if every gap equals
the current mean exactly, the measured timespan covers `n-1` gaps against an `n`-gap
target. It does not give the stationary mean once intervals are random and fed back into
difficulty.

An idealized stochastic sketch shows the difference matters. The assumptions:

- constant hashrate H, exponential gaps;
- timestamps equal to the true discovery times (continuous);
- no 4x clamps, no `pow_limit` clamp, no compact-encoding or 1 s quantization, no MTP or
  future-time effects;
- expected interval at target T taken as `2^256 / ((T+1) H)`, approximately
  `1/(lambda T)`.

Under these assumptions:

- The measured timespan of period k is `G_k / (lambda T_k)`, with `G_k ~ Gamma(n-1, 1)`.
- The next target is `T_{k+1} = T_k * timespan / (n s) = G_k / (lambda n s)`. It does not
  depend on `T_k`.
- The expected block interval in period k+1 is `n s / G_k`. Averaged over periods (each
  has n blocks), it is `n s * E[1/G] = s * n/(n-2)` for n > 2. The variance is finite only
  for n > 3, and the distribution is heavy-tailed at small n.
- Under these assumptions that gives about 2s at n=4 (versus 1.33s deterministically) and
  about 1.14s at n=16 (versus 1.067s).

The 4x clamps break the independence above (they condition on `T_k`). So do quantization,
MTP and miner ntime policy (A3/A4), and the real algorithm's integer and compact
arithmetic. With them, neither closed form is the exact stationary mean, and I have not
derived one.

Corrected A4 guidance:

- Keep the deterministic per-boundary check: node bits equal an independent
  reimplementation of the declared algorithm on the observed timestamps and parent bits.
- For the statistical check, center the CI on a **Monte Carlo simulation of the exact
  declared algorithm**: same n, clamps, `pow_limit`, compact rounding, 1 s timestamps, MTP
  and ntime policy, under the run's measured hashrate profile. Do not center it on
  `s*n/(n-1)` or `s*n/(n-2)`. The `n/(n-2)` form is useful only as a sanity check for
  clamp-free regimes with large n.
- Prefer n large enough that clamps rarely bind. Report clamp-hit counts, and use enough
  periods for the heavy-tailed small-n case.

### C4 - A7 qualified: integer `T_ref` units are not generically exactly proportional

The original A7 claimed `d = T_ref / T_share` gives integer units "exactly proportional
across mixed targets". That is too strong. The expected work of a target is
`W(T) = 2^256 / (T+1)` (plan §3.1), so:

- With targets `T_d = floor(T_ref / d)`, the credit ratio d2/d1 differs from the
  expected-work ratio `(T_d1 + 1) / (T_d2 + 1)`. The cause is the floor and the `+1`.
- The relative error is bounded by about `d / T_ref` per target. With `T_ref ~ 2^240` and
  u64 `d`, that is below about 2^-176: negligible but not zero.
- The cap against a compact-encoded network target (mantissa x 256^(e-3), with `T+1` not
  of a convenient form) and the `round(D_net)` cap add their own rounding.

Two acceptable replacements for the blanket "exact" claim:

1. **Declare a tolerance.** Define credit as an integer approximation of expected work.
   State the rounding rule (for example `d = round(W(T_share) / W(T_ref))`, or require
   `T_share = floor(T_ref / d)` and accept a bounded error). Document the bound (for
   example relative error `<= 2^-k`) in the decision record, and apply the same rule in
   every consumer: config, `ShareRec`, slice aggregates, window target, oracle.
2. **Use a consistent representable scheme that is exact.**
   - Restrict share targets to `T = (T_ref + 1) / d - 1`, with `T_ref + 1` a power of two
     and `d` a power of two. Equivalently, `T = 2^k - 1`.
   - Then `W(T) = 2^(256-k)` exactly and credit ratios equal expected-work ratios exactly.
   - Network-target caps that do not land on this lattice are credited by the declared
     rounding rule from option 1, and logged.

The chi-square acceptance in A7 is statistical and stands. The "every persisted value
round-trips exactly" clause applies to storage encoding, not to work proportionality.
Add a deterministic test: for each allowed `(T_share, d)` pair, the stored `d` matches the
declared rule, and its error against `W(T_share) / W(T_ref)` is within the declared bound
(zero under option 2).
