# Regtest network lab: real SV2 CPU mining

Date: 2026-11-05
Status: proposal for cross-model review; not an implementation specification.

## 1. Goal and agreed direction

Build a local, controllable Bitcoin mining network using real rbitcoin nodes,
real CPU proof of work, real SV2 mining connections, and the real silent-pool
processes. An optional web page exposes controls and visualizations. The same
experiment runner operates headlessly for integration tests, parameter studies,
and CI, and exports runs that can be inspected afterward.

User requirements established in the design discussion:

- rbitcoin's PoW-enabled regtest is the node backend.
- Miners perform actual SHA256d work; discovered shares and blocks follow the
  normal validation and submission paths.
- The node is authoritative for network difficulty. The intended network can
  adjust difficulty in response to changes in aggregate hashing power.
- Support N independently controllable miners, multiple competing pools, and
  network-wide scenarios, not just one pool with one synthetic competitor.
- Focus the CPU miner on native SV2. SV1 translation is not an initial dependency.
- Keep visualization independent of orchestration, workload generation,
  observations, and assertions. Browser presence never controls run lifetime.

The automatic-retarget requirement needs a node capability decision: ordinary
regtest disables periodic retargeting, and the inspected rbitcoin PoW override
does not by itself enable it. See §3. This is an implementation prerequisite,
not a reason to move mining into a mathematical simulator.

This document proposes a harness and identifies decisions still needed. It
does not change payout policy, difficulty-credit units, or node consensus.
Accepted changes to normative pool behavior need a decision record and SPEC
changelog integration under [MERGE-QUEUE.md](MERGE-QUEUE.md). Existing authority
remains [DESIGN.md](../DESIGN.md), then [SPEC.md](SPEC.md), then the architecture
documents. In particular, the constant denomination ladder and Ark separation
remain constraints on the expected-result checker.

## 2. Current implementation evidence

Inspected on 2026-11-05 at silent-pool HEAD
`b0ab4ca34fda07c61a6335d05f598eaacc69678c`, with pre-existing staged SV2 Template
Provider work. These observations describe that working tree, not a claim that
all features are merged, tested together, or complete. Recheck at implementation
time; overview documents contain stale capability descriptions.

| Area | Current evidence | Consequence for the lab |
|---|---|---|
| Full-stack node tests | `testing/regtest-rbitcoin-reorg-e2e.sh` launches rbitcoin, Redis, Postgres and `front,payout,api`; checks settlement and orphan handling | Reuse setup and assertions; replace fixed ports and shell lifetime with a persistent supervisor |
| Network target | `bins/spc-pool/src/sv1.rs::network_target` decodes template `nBits`; `front.rs` checks it before block submission | Block difficulty already comes from the node |
| Share policy | `[accounting] share_difficulty` is an integer, default 1; `effective_share_target` and `credited_difficulty` apply separate pool policy | CPU-scale ordinary shares need target/work-unit design; §3.3 |
| Native SV2 | `bins/spc-pool/src/sv2_server.rs`, `bins/spc-pool/tests/e2e_sv2.rs` | Reuse protocol setup and fixtures for a persistent SV2 CPU miner |
| Node TDP | Staged `bins/spc-pool/src/template_tp.rs` and `testing/regtest-rbitcoin-tp-e2e.sh` | Prefer the native TDP integration as it lands; distinguish its evidence from the merged RPC harness |
| Chain observation | Decision [2026-11-05-sv2-tp-chain-view.md](decisions/2026-11-05-sv2-tp-chain-view.md) keeps RPC for confirmations, coinbase reads and economics with TDP templates | SV2 mining/TDP does not remove the chain-observation RPC requirement |
| Existing CPU fixture | `testing/regtest-miner.py` hashes genuine pool jobs over SV1, looking for block solutions | Useful reference, not the requested long-lived SV2 miner |
| Accounting | `crates/spc-accounting` is pure; current routing/building still contains exact-amount outputs and capacity trimming | The independently expected denomination policy must not be inferred from this executable path; ladder implementation is tracked separately |
| API/metrics | `bins/spc-pool/src/api_backend.rs` and `metrics.rs` expose a subset of state; capacity is config-derived and some query failures become zero/empty | Add trustworthy observations with freshness/error information before relying on graphs |

The existing rbitcoin harnesses use `--regtest-pow-bits 0x200fffff`, roughly
16 hashes per block. That is a smoke-test setting, not a baked-in lab target.
The rbitcoin revision and effective consensus parameters are separate run
inputs; [UPSTREAM_DEPS.md](../UPSTREAM_DEPS.md) records the upstream reference.

## 3. Difficulty authority, retargeting, and work units

### 3.1 Network target belongs to rbitcoin

The pool obtains the current block target from its node's template/prevhash
messages. The CPU miner obtains the corresponding job from its SV2 pool.
Neither the browser nor the miner substitutes a block target in that header.
When the node changes expected difficulty, the new target propagates through
the ordinary job path.

Difficulty adjustment responds to observed block timestamps and chain history;
the node does not consume the harness's requested hash budgets. Those budgets
control actual work, and resulting discoveries provide the feedback.

For a target T and actual aggregate rate H:

```
probability per hash       = (T + 1) / 2^256
expected hashes/solution   = 2^256 / (T + 1)
expected block interval    = expected hashes/block / H
```

Intervals remain random. More miners initially make blocks faster at the
existing difficulty; a retargeting node subsequently responds according to
its configured consensus algorithm and adjustment limits. The harness should
not secretly counteract a hashrate change to hold block intervals constant.

### 3.2 Automatic retargeting is a node-side prerequisite

The inspected rbitcoin regtest profile disables periodic retargeting.
`--regtest-pow-bits` is a startup consensus overlay, not a runtime controller
that estimates hashrate. It also retains timestamp/height edge cases; miners
always use actual template bits rather than assuming the override applies to
every block. In particular, a long timestamp gap can invoke minimum difficulty.

The engine already has an in-process regression that enables retargeting,
disables minimum-difficulty exceptions and shortens the adjustment interval
(`rbitcoin-rpc/tests/methods/mod.rs`). The missing capability is an exposed,
validated multi-process regtest configuration, not necessarily a new difficulty
algorithm. Ordinary regtest inherits 600-second spacing, a 2016-block interval
and disabled retargeting. Its minimum-difficulty exception uses a timestamp
strictly more than 1200 seconds after the parent; include genesis bootstrap,
same-tip time changes and adjustment boundaries in node/TDP tests.

Two capability labels are useful:

- **PoW baseline:** today's regtest overlay, useful for integration smoke tests.
- **Retargeting experiment:** the user's intended self-adjusting network;
  requires an explicitly supported and tested rbitcoin consensus profile.

A requested retargeting experiment should fail preflight if unsupported,
rather than silently run the baseline and label it self-regulating.

Review and implement the node profile before claiming difficulty-response
results. Specify initial difficulty/bootstrap behavior, pow limit, target
spacing, adjustment interval/algorithm and clamps, timestamp rules, and whether
minimum-difficulty exceptions remain enabled. Decide whether the experiment
uses Bitcoin's normal adjustment schedule or a deliberately shortened lab
schedule. A shortened schedule changes consensus assumptions and needs to be
recorded as such. Include subsidy-halving interval and coinbase maturity in
the profile even when unchanged.

Every node on one experimental network uses the same consensus-profile
fingerprint. Changing it creates a new run with fresh chain state. Do not use
restarts with different PoW overrides as a live speed dial.

### 3.3 Pool share target is separate from the network target

The node validates blocks; it does not select how frequently miners report
partial work to a pool. SV2 carries a pool-assigned share target separately
from the block target. Both tests use the same genuinely hashed header:

```
hash <= share target  -> SV2 share submission, validation and credit
hash <= block target -> additionally a candidate block for node validation
```

Usually the share target is larger/easier. If both targets are equal, every
accepted share is block-qualified. This is what today's minimum integer
difficulty of 1 produces on a much easier regtest chain. Changing only the
node's easy block target does not provide frequent non-block shares.

Proposed direction: support CPU-scale share targets through the real SV2
target policy, retaining PoW verification at ingress and in the front. Use
exact 256-bit targets and a defined integer/fixed-point work representation;
do not round all tiny difficulties to zero or silently reinterpret existing
persisted difficulty fields. An initial common target for all miners avoids
making VarDiff a prerequisite; per-channel target adjustment can follow.

Open decision: generalized fractional work support versus a documented,
regtest-only target/work-scale profile. Assess compatibility across config,
share records, slice aggregates, wire payloads, persistence, metrics, auditing,
and settlement. Every consumer needs the same unit and rounding rules.
Different share targets must earn work-proportional credit, not equal credit
per submission. Preserve the SPEC §4.3 window semantics (`N x d`) unless a
separate accounting decision changes them; a unit conversion is not permission
to replace that rule with a network-difficulty-based window.

Network retargets and pool share-target updates are distinct events. Test their
interaction, outstanding old jobs, and target-update ordering. The current
`>= 1` policy is a recorded decision, so changing it requires an explicit
decision rather than a hidden miner bypass.

## 4. Topology: N miners, P pools, R nodes

These are independent dimensions, not fixed counts:

```
SV2 CPU miners A1..An ---> pool A fronts ---> rbitcoin A
                              |                 |
                       payout/API/storage      P2P
                                                |
SV2 CPU miners B1..Bm ---> pool B fronts ---> rbitcoin B
                              |                 |
                       payout/API/storage      P2P
                                                |
SV2 CPU miners C1..Ck ---> pool C fronts ---> rbitcoin C

         headless supervisor + link controls + observations
                         |
                 CLI / browser / CI
```

Model explicit IDs and relationships for network, node, pool, front, miner
identity, worker, and SV2 connection/channel. One miner identity can have
multiple workers; several identities are not simulated by changing a username
inside a connection that is bound to one identity. A miner can switch pools
through a real disconnect/reconnect and required registration sequence.

Initially, competing pools can be independent silent-pool instances. They
have independent accounting state, descriptors, credentials, Redis/DB
namespaces or instances, and optional Ark wallets. Supporting another pool
implementation later requires a declared adapter/capability contract; cloned
silent-pool instances do not establish compatibility with other software.

Multiple fronts of one pool share accounting under [MULTI-REGION.md](MULTI-REGION.md).
Multiple competing pools do not share an accounting window. Keep these two
topologies distinct in scenarios and the UI.

A shared node is sufficient for an initial connected multi-pool experiment.
Separate peered nodes are needed for independent mempools, propagation delays,
partitions, competing branches and reorgs. Control all relevant links: miner
to pool, pool to template provider/chain RPC, node P2P, and pool role/storage
links. Explicit peer configuration prevents unintended paths around a
partition. Chain convergence is by validated chainwork, not highest height.

Resource and protocol limits are capability inputs. The inspected rbitcoin
TDP listener caps concurrent sessions at 8; do not promise unlimited pool fronts
on one node without testing or changing that limit. N miners connect to pools,
not individually to node TDP, so miner count and TP session count scale differently.

## 5. Components and interfaces

Proposed names describe responsibilities; package and binary names remain open.

| Component | Responsibility |
|---|---|
| `spc-lab` runner | Validate scenario and capabilities, allocate resources, supervise processes, schedule actions, evaluate assertions, persist run artifacts |
| SV2 CPU miner driver | Host controllable miners/workers, maintain real SV2 sessions, execute bounded hash batches, submit results, report measured work |
| Node/pool adapters | Launch pinned binaries, render supported config, manage links, discover readiness, obtain templates/chain evidence through supported interfaces |
| Observation/reconciliation layer | Collect independent evidence, maintain view snapshots, calculate expected results and track eventual state transitions |
| HTTP service and web UI | Submit typed commands; consume snapshots/events; render live or recorded runs |

Native subprocesses in the Nix environment are the initial launch mechanism.
Use unique ports, data directories and process groups per run. Containers or
network namespaces can provide stronger per-link fault control later. Separate
logical topology from process placement so N miners need not imply N operating
system processes.

Prefer node TDP -> pool and SV2 Mining Protocol -> CPU miners. These are two
different protocols under the SV2 umbrella. RPC still supplies chain reads and
explicit test controls. With TDP, `SubmitSolution` is not itself proof of node
acceptance; reconcile the candidate hash with the node's actual chain view.

### 5.1 SV2 CPU miner

- Establish Noise and SetupConnection using the actual pool authority and
  supported extensions. Open the channel type implemented by the pool;
  extended mining channels are the initial integration target.
- Assemble coinbase/header work from real jobs, extranonce allocation, merkle
  paths, version, time and network bits. Handle future jobs and prevhash
  activation in protocol order.
- Track share targets and job validity in session state; apply target updates
  according to negotiated protocol semantics. Target processing must not race
  header construction or submission bookkeeping.
- Partition nonce/extranonce search space among workers, renew work on
  exhaustion, and cancel stale batches promptly. Avoid repeated hashing of
  identical headers after pause/reconnect.
- Use bounded CPU worker threads separate from async network/control tasks.
  Pace actual hash attempts in small batches with per-worker budgets. Submit
  qualifying shares promptly instead of holding found blocks to simulate time.
- Support multiple outstanding submissions and correlate sequence acknowledgments
  and errors correctly. Report submitted, accepted, rejected and block-qualified
  work separately; a local solution is not an accepted canonical block.
- Measure requested/achieved hashes per second, attempted hashes, target/work
  units, active job, rejected/stale work and reconnects. Resource saturation is
  visible; adding identities must not silently redistribute other miners' budgets.

A workload seed can choose fixture identities, nonce traversal and scheduled
actions. Pool sending keys retain their required OS randomness and erasure.
No requirement for identical hashes or network schedules across repeated runs.

#### Existing SV2 gaps to resolve or explicitly bound

The native SV2 fixtures demonstrate a useful basic path, but currently search
against network bits, ignore the announced share target and use empty transaction
lists. They do not establish complete upstream protocol interoperability.
Source inspection identified these prerequisites for the proposed miner:

- The server announces a target on channel open but does not emit `SetTarget`;
  early opens can get an all-ones target while validation later uses a real job
  target. Implement target lifecycle and independently verify its wire byte order.
- The current connection holds one channel with a fixed channel ID. Start with
  one independent connection/channel per identity; do not rely on unsupported
  multi-channel multiplexing. Check requested/negotiated extranonce sizes and
  submitted lengths. Submit miner-controlled extranonce bytes, not the pool prefix.
- Existing client helpers do not fully correlate future jobs and prevhash
  activation or enforce all job time/version constraints. Build an explicit
  session/job state machine and include nonempty merkle-path vectors.
- Miner-facing Noise authority pinning is not wired like the in-flight node-TDP
  pinning. Add pool authority provisioning and client pinning; node-TDP encryption
  does not prove authenticated miner-to-pool connections.
- Current ingress success precedes front-side acceptance/accounting, and client
  helpers ignore some share responses. Distinguish ingress acknowledgment from
  accounted work until end-to-end outcomes are correlated. Verify the meaning of
  `SubmitSharesSuccess` fields instead of counting every reply as credited work.
- Review job-change detection for bits/version/time-only updates. A retarget
  experiment needs proof that announced jobs and validated work cannot diverge.

These are source-audit findings to reproduce and resolve, not claims of completed
tests. Relevant files are `sv2_server.rs`, `front.rs`, the role/job wiring in
`bins/spc-pool/src/main.rs`, and `bins/spc-agent/src/sv2_client.rs`. Reuse helpers
only after checking these assumptions against the pinned upstream SV2 format.

### 5.2 Commands, scenarios, and completion

The runner owns one typed command interface used by scripts, CLI and browser:

- Create/start/stop a run; add/remove a miner or worker; change actual hash budget.
- Connect a miner to a pool; switch pool; pause/resume selected hash workers.
- Partition/heal selected links; add latency; kill/restart a role or node.
- Introduce transactions through a funding/mempool workload adapter.
- Run until a node reaches a height, a candidate is accepted, a pool settles a
  block, an Ark payment is observed, or a declared timeout expires.

Each command has an ID, preconditions, lifecycle and observed outcome. Retries
of the same command ID do not accidentally add a second miner or repeat a
destructive action. Record command order and conflicting-action resolution.

Scenarios combine topology, initial state, workload schedules, event predicates,
and assertions. A trigger such as "after node B observes height H" names its
node; a partitioned network has no single globally observed tip. Prefer causal
barriers with timeouts over arbitrary sleeps. Keep statistical hashrate-change
experiments distinct from invariant checks under a particular event sequence.

"Run until next block" schedules real work until observed acceptance. It is
not an instant synthetic discovery or a guarantee of exactly one network-wide
block under concurrency. Step mode stops scheduling further work at its barrier
and reports any already-in-flight work. Pause initially pauses hash scheduling;
process timers, network delivery and outstanding settlement continue.

Mining belongs to the live experiment. If fixture bootstrap uses node generation
RPCs to prepare funds, record that as setup outside the measured workload. Block
and network-behavior assertions use actual CPU discoveries through the declared
pool paths.

## 6. Observations, UI, and evidence

Commands express intent; events report evidence. Capture miner submissions and
responses, pool slice/distribution/block/ledger events, each node's tip/chainwork
and actual coinbases, and recipient scan/wallet outcomes. Preserve source cursor,
run ID, actor IDs, command correlation where known, wall and monotonic timestamps,
and relevant height/hash. A collector sequence is an ingestion order, not proof
of a global causal order across processes.

The collector uses independent read cursors and never acknowledges the pool's
consumer-group work. Bounded buffering and explicit loss/gap events prevent a
slow browser or observer from blocking the mining path. Assertions requiring
complete evidence fail or become inconclusive if required data was lost.

Expose snapshot plus resumable event stream, for example HTTP commands and SSE
observations. Reconnection obtains a consistent snapshot/cursor or replays the
retained tail. Decimate high-volume chart data without discarding the evidence
required for accounting checks. Serialize wide integers losslessly for JavaScript.
Distinguish unavailable/stale/error observations from genuine zero values.

Initial views:

- Network topology, links, per-node tips and competing branches.
- Per-miner requested/actual work, share rate, target and pool assignment.
- Network target history, adjustment boundaries, block intervals and pool wins.
- Accounting-window membership, raw entitlement, expected versus actual payout.
- Money lifecycle: immature coinbase, Ark obligations, delivery, error pending,
  forfeiture, float and maturity-driven refill. Do not double-count obligations
  and the assets backing them.
- Timeline of commands, jobs, shares, blocks, settlement, faults and assertions.

Fixture-only attribution can support an omniscient view plus restricted miner,
chain-observer and Ark-observer projections. Keep the observer on the SPc/harness
side. No `sp_id`, `B_scan`, `B_spend`, `P_i`, block hash or snapshot is added to
Ark-role inputs/streams (SPEC §11.3). Do not record `a_send`, shared secrets or
transient tweaks to make replay easier. Harness artifact retention is separate
from the system's scrub behavior and cannot be used as evidence of production
privacy guarantees.

For correctness, compare three kinds of evidence:

1. Production-function replay to detect storage/event/order inconsistencies.
2. An independently implemented policy oracle over recorded accepted work and
   the exact frozen window/target, with explicit floor and denomination rules.
3. Actual node-accepted coinbases and recipient-side outcomes.

Two endpoints reading the same database are not independent payout evidence.
Expected policy remains authoritative even while an implementation is in flight.

## 7. Time, economics, reproducibility, and scale

Record and distinguish chain height, header time, process wall time and monotonic
timer time. Faster hashing does not accelerate RPC polling, retry timers, Ark
rounds or mailbox authorization expiry. Mock node time can also change difficulty
eligibility; it is an explicit scenario input, not a generic speed multiplier.

The initial real-time profile should leave enough processing headroom to observe
normal behavior. A saturation profile deliberately explores queue growth and
stales. An accelerated-retarget profile additionally needs agreed timestamp and
clock semantics across nodes; scaling just block timestamps is not equivalent
to scaling the entire system.

rbitcoin currently starts with a 50 BTC regtest subsidy and halves every 150
blocks. PoW difficulty does not change this or coinbase maturity. Long economic
runs need a declared reward schedule and warmup state, with a node-side extension
if a different schedule is required. Include fee workloads, accounting-window
warmup, pool market share, initial balances/float, Ark limits and confirmation
policy. Real mining does not automatically make those assumptions representative.

Artifacts should contain effective config and consensus fingerprint, topology,
source revisions plus dirty-tree/build identities, binary/dependency versions,
hardware and measured capacity, workload seeds, commands/events, diagnostics,
assertions and aggregate results. Keep a shareable manifest free of credentials;
private fixture material has separate access and retention.

Distinguish replaying a recorded visualization, rerunning a scenario, and restoring
a whole-system checkpoint. Start with fresh isolated runs and semantic assertions.
A checkpoint needs coordinated node, Redis, Postgres and wallet state and a
defined restart policy for ephemeral front state. Saving one node directory does
not restore a distributed experiment.

Population size, work concentration, process count and aggregate hashrate are
separate parameters. Report hardware limits and scheduling skew. With real PoW,
seeded actions do not guarantee identical discoveries. Fairness, pool win share,
retarget response and float-shortfall probabilities need enough independent runs
and statistical intervals; conservation/idempotency checks apply to each run.

## 8. Proposed implementation packets and acceptance

### P0 - Resolve consensus profile and share-work design

- Verify the exact rbitcoin build and obtain a tested retarget-capable profile
  for the intended self-adjusting network; keep baseline capability explicit.
- Decide CPU-scale share target/work units and required pool spec/data changes.
- Freeze initial scenario vocabulary, source/observer contracts and capability
  negotiation. Integrate with the in-flight TDP and ladder packets.
- Acceptance: reviewers can identify which component sets each target; nodes
  reject mismatched profiles; a requested unsupported capability fails preflight.

### P1 - Genuine persistent SV2 CPU mining

- Implement Noise/session/job/target state plus a bounded hashing executor.
- Exercise several independent identities against one real pool and rbitcoin.
- Acceptance: observed non-block accepted shares as well as block solutions;
  pool credit reflects the assigned target; node validates submitted blocks;
  stale/duplicate/below-target submissions are rejected; work continues through
  job changes and nonce exhaustion. Verify recipient outputs independently.
- Test authenticated session setup, target encoding, extranonce negotiation,
  nonempty merkle paths, future-job activation and response correlation. Use
  independent protocol vectors or an external SV2 peer where feasible so matching
  bugs in this pool and its miner do not count as interoperability evidence.
- Include network-target changes and share-target updates before claiming
  retarget compatibility. A static target-only smoke test is insufficient.

### P2 - Persistent headless supervisor and run artifacts

- Replace shell lifetime with isolated resource allocation, dependency readiness,
  command execution, condition waits, bounded observation and reliable cleanup.
- Acceptance: two runs coexist without collisions; browser-free scenarios finish
  with machine-readable results; failures retain sufficient diagnostics; stopping
  a browser does not stop work; duplicate command IDs do not duplicate effects.

### P3 - N miners and competing pools/nodes

- Introduce independent pools, configurable miner assignments and P2P topology.
- Acceptance: non-winning pools observe new tips and update jobs; accounting is
  pool-local; miners reconnect/switch correctly; hash budgets are independently
  measured; partition/heal selects a valid greatest-work branch and handles its
  orphaned payouts without double settlement.

### P4 - Retarget, fault and economic scenario suite

- Run sustained hash increases/decreases across adjustment boundaries and test
  target propagation, timestamps, recovery after abrupt hash loss and min-target
  behavior under the selected profile.
- Add restart, stale-job, transaction-fee, confirmation-boundary, shallow/deep
  reorg, stream-redelivery, and Ark outage/refill scenarios as capabilities land.
- Acceptance: observed difficulty changes originate at nodes and follow the
  declared algorithm; independent payout/conservation and idempotency checks
  hold; stochastic metrics carry uncertainty and identify incomplete evidence.

### P5 - Web visualization and recorded-run review

- Implement the views in §6 against the same API/artifact contracts as the CLI.
- Acceptance: attach/detach/reconnect during a run; inspect a completed CI run;
  trace a block from work through settlement; display pending/failed commands,
  stale observations and competing node views accurately.

These packets establish architecture and gates, not calendar estimates. The UI
can proceed in parallel once command and event schemas have stable fixtures.

## 9. Questions for independent reviewers

Return findings with severity, evidence, affected section, suggested change and
a test that could falsify the proposed behavior. Mark unsupported assumptions
separately from implementation bugs. In particular:

1. **Retarget realism:** which rbitcoin profile implements self-adjustment? Is
   the normal schedule practical, or is a shortened lab schedule justified?
   What timestamp, minimum-difficulty and clamp interactions could bias results?
2. **Share units:** generalized fractional difficulty or a regtest-specific work
   scale? How are precision, overflow, persisted units, mixed targets, target
   changes and exact window reconstruction handled consistently?
3. **SV2 correctness:** does the chosen channel/job model preserve extranonce
   uniqueness, target-update semantics, future-job activation and submission
   acknowledgments under concurrency and retargets? Which current gaps block it?
4. **Topology:** are independent pools, regional fronts, miner identities,
   workers and nodes sufficiently distinct? What links or discovery mechanisms
   could let a supposed partition communicate inadvertently?
5. **Rate control:** can bounded batching deliver the requested work proportions
   without starving network tasks or creating burst-biased discoveries? How will
   achieved work and saturation be measured across machines?
6. **Event ordering:** which observations establish block acceptance, canonical
   inclusion, settlement and delivery? How do reorgs invalidate prior assertions?
   Can snapshot/tail gaps or observer overhead change the experiment?
7. **Economics:** which reward/fee/population/float assumptions and warmup periods
   are needed? Can a fast regtest run support the proposed inference, and what
   sample sizes and uncertainty reporting are appropriate?
8. **Independent checks:** which computations share code with the implementation
   under test? How can the oracle detect errors in work aggregation, floor
   placement, frozen windows, denomination routing or receipt reconciliation?
9. **Privacy:** do fixture observation and artifact retention preserve the
   actual Ark input boundary and secret-erasure requirements? Are restricted
   observer views faithful to information available to those observers?
10. **Scope and sequencing:** what is missing for a useful first headless run?
    Which proposed controls need node/pool changes, and which are purely harness
    work? Are the acceptance gates concrete enough for implementation packets?

## 10. Reading and evidence map

- [README](../README.md), [DESIGN](../DESIGN.md), [SPEC](SPEC.md) §3.3-§3.4,
  §4, §6.1, §8.4 and §11.3: product behavior and invariants.
- [ARCHITECTURE](ARCHITECTURE.md), [MULTI-REGION](MULTI-REGION.md),
  [WIRE-FORMATS](WIRE-FORMATS.md): process boundaries, pool-wide versus
  pool-local state, and event payloads; SPEC wins over stale descriptions.
- [ROADMAP](ROADMAP.md) Phase 8 and [STATUS](STATUS.md): existing harness scope
  and in-flight work, including denomination implementation.
- [Share target decision](decisions/2026-11-02-share-target-policy.md): the
  existing integer-difficulty policy that a CPU-share design must address.
- [SV2 TP chain-view decision](decisions/2026-11-05-sv2-tp-chain-view.md): TDP
  template/solution transport versus RPC settlement observations.
- [UPSTREAM_DEPS](../UPSTREAM_DEPS.md): reference pins; record the actual tested
  rbitcoin binary revision as well.
- rbitcoin source (in its separate checkout):
  `crates/rbitcoin-consensus/src/params.rs` (regtest profile and PoW overlay),
  `crates/rbitcoin-consensus/src/header.rs` (expected bits and retarget rules),
  `crates/rbitcoin-node/src/config.rs` (available configuration), and
  `crates/rbitcoin-sv2/src/session.rs` (template/solution validation).
