# Regtest lab: five-model review synthesis

Date: 2026-11-05
Status: complete; five independent reviews and follow-up corrections collected.

Reviewed proposal: [REGTEST-LAB.md](../REGTEST-LAB.md).
Frozen proposal SHA256:
`444060b2c1c7d377960116032dc88af114c0dbc43741a8c719b33188e8b99098`.

The reviewers were given the same proposal, asked to inspect supporting source,
and instructed not to read one another's findings. Each had a different emphasis
while reviewing the whole plan. These are design/source reviews, not evidence
that a complete lab was executed. Recommendations here do not amend normative
pool behavior or node consensus.

## Executive summary

The architecture is viable: genuine SV2 CPU mining against rbitcoin, independent
pools and peered nodes, a headless runner, and an optional browser. The most useful
review feedback is to replace broad prerequisites with named, testable packets:

1. **rbitcoin lab profile:** automatic retargeting needs exposed node configuration,
   defined initial-difficulty/overlay and timestamp behavior, and harness-owned
   profile verification. Existing regtest PoW is not automatic retargeting.
2. **SV2 interoperability:** job activation order, target endianness/updates and
   Merkle-path order have concrete source-level defects. Independent wire/client
   tests must precede trusting a new in-repo miner's observations.
3. **Accounting meaning:** decide work units and explicitly confirm the window
   interpretation. The current `N x d` threshold represents N shares' worth of
   work at a common target, with whole-slice overshoot; it is not automatically
   N expected network blocks. This is an owner/spec decision.
4. **Trustworthy evidence:** separate ingress acknowledgments, credited work,
   candidate submission, node-local canonical inclusion, settlement and delivery.
   Absence from the active chain is not proof of invalidity or consensus rejection.
5. **Real isolation and lifecycle:** specify per-pool storage, sustained link
   partitions including reconnects/Ark backends, and runner-crash adoption/cleanup.
6. **Economic capability gates:** witness commitments, reward-profile agreement,
   funding/warmup and continued block production are prerequisites for the
   corresponding fee, wallet, Ark and maturity experiments.

Several alarming claims in the initial reviews were wrong or already covered by
the proposal. The corrections below are part of the result, not additional plan
defects. In particular, the proposal's approximately 16 hashes/block arithmetic
is correct, and its existing retarget/share-policy prerequisites were deliberate.

## Review records

| Requested model | Resolved model | Emphasis | Record/status |
|---|---|---|---|
| Fable | Claude Fable 5.1 | Architecture and consensus/control boundaries | [Review and clarifications](2026-11-05-regtest-lab-fable.md) |
| Opus | Claude Opus 5.5 | SV2, targets, accounting and chain acceptance | [Review and addendum](2026-11-05-regtest-lab-opus.md) |
| Kimi K3 | Kimi K3 | Experimental validity, scheduling and statistics | [Review and corrections](2026-11-05-regtest-lab-kimi-k3.md) |
| DeepSeek | DeepSeek V3.2 | Orchestration, topology, faults and event ordering | [Review and clarification](2026-11-05-regtest-lab-deepseek.md) |
| GLM | GLM 5.3 | Completeness, phases, privacy and independent checks | [Review and corrections](2026-11-05-regtest-lab-glm.md) |

## Evaluation method

The synthesis distinguishes:

1. A new omission or contradiction in the proposal.
2. An implementation prerequisite already explicitly documented by the proposal.
3. A useful request to make an existing requirement more concrete.
4. An unsupported, incorrect or out-of-scope review claim.

A reviewer's severity is not automatically the severity of a plan defect.
Independent agreement does not replace source verification, and disagreement
is retained where a policy decision is still required. The dispositions below
reflect source checks and arithmetic, not a majority vote.

## DeepSeek clarification

DeepSeek initially labeled several documented prerequisites as absent plan
requirements. On follow-up it corrected those characterizations in the review's
"Clarification and Disposition" section. Read that section with the original
findings; the original verdict alone overstates the case.

- Retarget-capability separation and unsupported-profile preflight already exist
  in proposal §3.2. The missing node configuration is an acknowledged dependency.
- CPU-scale share units and their policy amendment are already P0 decisions in
  §3.3; configurable share difficulty 1000 means harder, not fractional, work.
- Snapshot/tail recovery and command deduplication already appear in §5.2/§6.
- An all-ones numeric target was not demonstrated to violate SV2. The relevant
  concern is announced-versus-validated target consistency.
- One channel per connection does not prevent N miners using N connections.
- Suggested retarget CLI flags were illustrative, not discovered capabilities.
- No external compliance requirement was established for synthetic fixture data.

Useful proposals retained after checking the draft:

| Topic | Candidate amendment | Disposition |
|---|---|---|
| Network isolation | Select a concrete fault-control backend and test for bypass paths before P3 | Make existing requirement concrete |
| Observations | Specify actor-scoped predicates, freshness, timeout and multi-node predicate semantics | Make existing requirement concrete |
| Economic profiles | Require explicit reward schedule, starting float, Ark limits and confirmation policy | Make existing requirement concrete |
| Capabilities | Give each scenario requirement an evidence-backed capability check | Make existing requirement concrete |

## Opus findings: source verification

The following findings were checked against the pool source and, where noted,
the resolved upstream SV2 helpers. They are source evidence, not completed
interoperability tests.

| Finding | Evidence checked | Disposition |
|---|---|---|
| Job/prevhash order | `sv2_server.rs::send_job` sends prevhash before the referenced job; upstream `channels_sv2::client::extended::on_set_new_prev_hash` requires an already-cached future job | Concrete P1 interoperability blocker; strengthen the existing sequencing gate |
| Target byte order | Pool channel-open target uses `to_be_bytes`; upstream `stratum_translation::sv1_to_sv2` uses `to_le_bytes` | Concrete P1 interoperability blocker; require asymmetric independent wire vectors |
| Merkle path order | `block.rs::coinbase_merkle_branch` reverses the bottom-up path; upstream `channels_sv2::merkle_root` folds received nodes in order | Concrete P1 interoperability blocker; test a path with at least two levels and actual node acceptance |
| Window interpretation | SPEC §4.3 and `front.rs` use `N x d`, with d the share credit | With a common target this is an N-share-work threshold, enlarged by whole slices, not N network-block equivalents; request explicit owner confirmation before economic conclusions, not a silent policy replacement |
| Retarget/overlay interaction | rbitcoin `header.rs::apply_regtest_pow_overlay` can replace pow-limit bits on a non-boundary block following a boundary that reached the limit | Concrete node-profile design hazard; test or prohibit that combination |
| Witness transactions | `sv2-tp` branch `template_tp.rs::on_tx_success` rejects a witness-bearing template | Gate witness/modern-wallet/Ark transaction scenarios on commitment support; do not treat empty-mempool success as coverage |
| Profile verification | P0 says nodes reject mismatches, but profile compatibility is currently local configuration | Name the harness preflight authority and required node introspection; do not imply a P2P profile handshake already exists |
| Subsidy profile | TDP pool code derives subsidy from a network-specific halving interval | A custom node reward profile also needs matching pool economics or node-supplied breakdown |

Opus appended numeric qualifications after follow-up:

- Harder blocks mean a smaller target: a 64x work ratio uses a block target
  approximately one sixty-fourth of the share target.
- `N x d` is a work threshold with whole-slice overshoot, not an exact share count.
- The simple `n/(n-1)` short-period factor is a noiseless endpoint-count argument,
  not a proven stationary mean under random arrivals, clamps and timestamp rules.
  Use per-boundary algorithm checks and a model of the actual selected profile
  for statistical baselines rather than adopting that formula as ground truth.
- Expected work depends on `T + 1`; integer reference-target division needs an
  explicit rounding rule/error bound or an exactly representable target family.
  Lossless storage encoding does not prove exact proportional expected work.
- Reversing the example big-endian target into little-endian makes the target
  much smaller/harder. The source-level byte-order defect remains valid.

The TP work moved during review from the initially staged working-tree version
to branch `sv2-tp`. TP findings were cross-checked there; their original review
line numbers refer to the staged snapshot. That concurrent work was not changed
by these reviews.

## Kimi arithmetic checks

Two initial claims were independently checked with Python integer/Decimal
arithmetic and are not accepted as plan defects:

```
bits 0x200fffff:
  exponent = 32, mantissa = 0x0fffff
  T = mantissa << (8 * (exponent - 3))
  2^256 / (T + 1) = 16.0000152588036... hashes

32-bit nonce x 32-bit miner-controlled extranonce:
  search space = 2^64 distinct header candidates
  at 1,000,000,000 hashes/sec: about 584.94 years
```

Thus the proposal's "roughly 16" figure is correct. Cycling the nonce and
incrementing miner-controlled extranonce does not require a fresh server job
after each 2^32 attempts. The ordinary requirements to allocate unique work,
handle exhaustion and negotiate extranonce sizes still stand.

The share-unit recommendation also needs qualification: keeping network
difficulty below 1 while retaining the current share-target clamp still makes
every accepted share block-qualified. That does not meet P1's stated requirement
for non-block shares. A declared integer/fixed-point scale can preserve positive
work weights; whether the window should represent shares or network-block work
is a separate policy question.

Kimi appended corrections confirming the arithmetic and withdrawing the job-space
exhaustion blocker, TP-scaling omission, and claim that wall-clock skew alone
changes required bits for an identical candidate header/ancestry. It reframed
share/window units as an owner decision. Its initial numerical sample-size claim
was also withdrawn; no universal sample count is adopted here.

Retained experiment-design suggestions:

- A retarget scenario needs observed adjustment-boundary coverage, not just a
  retarget-capable binary. No covered boundary means the retarget assertion was
  not exercised.
- Statistical claims should declare estimator, null/alternative where relevant,
  planned observations and stopping rule; otherwise report descriptive metrics.
- A shortened target spacing also changes the minimum-difficulty time-gap rule
  if that rule remains enabled.
- Define node-scoped canonical-inclusion predicates and re-evaluate current
  inclusion after reorgs, while retaining historical observations as history.

## Fable findings: convergence and qualifications

Fable independently identified the same job/prevhash ordering, target byte-order,
target-update, authority-key and overlay/retarget problems as Opus. Those reviews
strengthen the case for a named SV2 interoperability work packet ahead of using
the miner for experiments; their source evidence matters more than the count of
reviewers agreeing.

Additional useful proposals:

- Give the rbitcoin configuration/profile work an explicit owner and acceptance
  contract separate from the pool share-policy work.
- Make a sustained partition survive the node's reconnect loop; a one-time
  disconnect is not sufficient. Choose relays, namespaces/firewall rules or an
  equivalent tested mechanism with no bypass path.
- Define runner-crash behavior and safe adoption or cleanup of surviving process
  groups, including persisted run identity and resource ownership.
- Calibrate block cadence against measured propagation/processing latency and
  record timestamp policy, rather than adopting arbitrary orphan-rate thresholds.

The pause/retarget finding needs a precise condition: rbitcoin checks the actual
submitted header timestamp. Rolling it across the minimum-difficulty boundary
while retaining pinned bits can cause rejected solutions; wall-clock passage
alone does not prove every previously issued job is invalid. A retained valid
job timestamp can behave differently. The recommended test must state whether
the miner holds or rolls timestamps. Disabling minimum difficulty is one possible
profile decision; correct renewal of affected jobs is another engineering option.

Fable appended clarifications confirming this condition. It also withdrew a
`--no-listen` recommendation that would prevent relays dialing recipient nodes:
recipient P2P listeners must remain reachable through the intended isolated
paths. The node's periodic fee-check interval is not a lower bound on tip-push
latency, and suggested orphan percentages were unverified hypotheses. A uniform
timestamp offset cancels within a steady-state adjustment window; transition,
uneven or ramped offsets require separate tests.

## GLM findings and corrections

GLM supports the architecture and privacy boundary. Its useful concrete additions
are per-pool storage/session-budget checks, explicit ownership and serialization
of effective profiles, height-progress prerequisites for maturity/settlement,
and retaining adequate evidence when chart series are decimated.

Several initial claims are not accepted:

- The approximately 16-hash figure is correct, as computed above. The target
  `0x200fffff` is approximately one-eighth of the usual regtest pow limit, not
  larger than it by many orders of magnitude.
- With the current easy-regtest policy, effective share target equals block
  target. A hash cannot simultaneously meet that share target and miss the same
  block target. CPU-scale non-block shares still require the policy work already
  identified in the proposal.
- An ordinary periodic retarget becomes applicable with a changed parent tip.
  The separate bits-only republish gap matters for valid same-tip target changes
  such as minimum-difficulty/time behavior on the RPC path, or an explicit test
  source. Restarting an existing datadir with another overlay is not the test.
- Target-wire tests should compare an independently decoded numeric target, or
  explicitly correct little-endian bytes; they should not freeze the current
  big-endian encoding.
- Overlay/retarget incompatibility needs the exact condition: a boundary result
  at pow limit can snap back to the overlay at the following non-boundary block.
  A first adjustment does not universally retarget from pow-limit bits.
- A candidate absent from the active chain may be a valid side branch, delayed,
  or unobserved. A competing canonical block or timeout does not establish
  consensus rejection. Report node-local, time-scoped inclusion and preserve an
  unknown/timeout state unless authoritative rejection evidence exists.

GLM appended corrections confirming these points and withdrawing its incorrect
arithmetic, already-supported-non-block-shares claim and periodic-retarget/same-tip
conflation. Its corrected review treats CPU-scale share policy as required P0
work, consistent with the proposal. The implementation guidance retained is:

| Topic | Recommended concrete gate |
|---|---|
| Independent pools | Start with separate Redis instances and Postgres databases unless an alternative isolation mechanism is implemented and tested; detect resource collisions before workload launch |
| TDP capacity | Budget actual TDP sessions per node/front; settlement RPC is a separate interface and does not itself consume a TDP session |
| Height-driven actions | A wait for H+6 or H+100 names which miners can advance that chain; an intentional pause reports blocked-on-height, and an unplanned missed deadline remains a failed or inconclusive scenario outcome |
| Evidence predicates | Record candidate hash, observing node, height, observed tip and time; canonicality is re-evaluated after reorgs without deleting historical observations |
| Oracle independence | Separate business-logic implementations and independent vectors; sharing a transport codec/type does not automatically destroy independence, but sharing the calculation being checked does |
| Artifacts | Decimation is a presentation transform; lost evidence required by an assertion prevents a pass |

## Recommended amendment order

The recommended order after all five reviews and clarifications is:

1. **Node profile packet:** expose and introspect the chosen retarget parameters;
   define initial target/overlay behavior and timestamp/min-difficulty policy;
   keep operational limits separate from consensus parameters.
2. **Share/window decision:** choose declared work units, target rounding and
   restoration compatibility; explicitly confirm whether the intended window is
   an N-share threshold or network-block-equivalent work. Update normative policy
   only through a reviewed decision.
3. **SV2 conformance packet:** fix job ordering, target encoding/lifecycle and
   Merkle order; test independent clients, negotiated lengths, acknowledgments,
   stable authority pins and same-tip job policy.
4. **Headless supervision and evidence:** specify durable command/run state,
   actor-scoped predicates and recovery/cleanup; retain historical observations
   while re-evaluating current canonicality after reorgs.
5. **Topology and economic capability gates:** sustained partitions with explicit
   peers and Ark chain backends; witness commitment and reward-profile compatibility
   before corresponding fee/wallet/Ark scenarios.
6. **Experiment contracts:** state warmup, target-boundary coverage, timestamp
   handling, resource saturation, estimator and stopping rule. Add the UI against
   the same established evidence contracts.

The frozen proposal is retained so all five records refer to the same text;
implementation and policy changes are separate follow-up work.

## Review provenance and checks

- The proposal hash remained unchanged throughout all five reviews.
- Original reviewer findings remain in their files, followed by their own
  corrections/addenda. Read the corrections with the original verdicts.
- Follow-up questions challenged factual claims; the reviewers did not exchange
  their initial reports with one another. This synthesis independently checked
  the central SV2 source claims and compact-target/search-space arithmetic.
- Only documentation and the README index were changed by this review task.
  Product tests were not run; this review is not a substitute for the proposed
  interoperability, node-profile and economic experiment gates.
- The staged TDP decision referenced by the frozen proposal moved to branch
  `sv2-tp` during review. If its relative Markdown link is absent in this worktree,
  the reviewed decision can be read with:

  ```sh
  git show 1ec1dd0:docs/decisions/2026-11-05-sv2-tp-chain-view.md
  ```

  The proposal's original links and hash were preserved to keep the review target
  stable. Source-baseline updates belong with the subsequent plan revision.
