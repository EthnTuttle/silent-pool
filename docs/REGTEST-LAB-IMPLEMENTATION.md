# Regtest lab implementation contracts (L3)

Date: 2026-11-05
Status: proposed implementation contract for Fable review. This document does
not claim that a lab, a miner, a supervisor or the described tests exist today.
Base revision: `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.

Inputs:

- [REGTEST-LAB.md](REGTEST-LAB.md): the frozen proposal and its packet outline.
- [Five-model review synthesis](reviews/2026-11-05-regtest-lab-review-summary.md):
  source checks and corrections. Raw reviewer records were read only together
  with their correction/addendum sections; this document does not treat an
  uncorrected reviewer claim as a requirement.
- [REGTEST-LAB-STATUS.md](REGTEST-LAB-STATUS.md): the TFF/Fable workflow and
  packet ownership.
- [MERGE-QUEUE.md](MERGE-QUEUE.md): shared-file ownership and changelog
  fragments.
- [SPEC.md](SPEC.md), [ARCHITECTURE.md](ARCHITECTURE.md),
  [WIRE-FORMATS.md](WIRE-FORMATS.md), [ROADMAP.md](ROADMAP.md), and
  [DESIGN.md](../DESIGN.md).

The decisions that change product behavior are separate, dated records:

- [2026-11-05-lab-share-work-units.md](decisions/2026-11-05-lab-share-work-units.md)
- [2026-11-05-window-network-block-work.md](decisions/2026-11-05-window-network-block-work.md)

Where this document and `SPEC.md` disagree before those decisions are applied,
`SPEC.md` wins. After Fable approves the decisions and the merge session applies
their changelog fragments, the amended spec wins. Do not consume these contracts
to change payout policy until that approval exists.

## 1. Scope, non-goals and in-flight inputs

### 1.1 In scope

This packet supplies:

1. the work-unit and window decisions the review called owner decisions;
2. a versioned scenario vocabulary with distinct network, node, pool, front,
   identity, worker and miner concepts;
3. contracts for a genuine SV2 CPU miner, command outcomes, evidence states,
   snapshots, observers, process adoption, resource isolation, link faults and
   capability gates;
4. bounded, phased implementation packets that a TFF implementer can dispatch
   after Fable approval;
5. an explicit list of gaps that are not covered by this design.

### 1.2 Out of scope

- Any change to `DESIGN.md` derivation, tier policy or Ark delivery.
- The denomination ladder, the SLICE score formula, `pending_balance` rails, the
  pool fee, or Ark order semantics. The window decision changes which slices
  are selected, not how a selected window is scored.
- Mainnet, a production mining pool, VarDiff, multi-channel multiplexing,
  SV1-based lab mining, or a full browser product.
- A new node consensus algorithm. L2 owns the rbitcoin profile.
- Claimed interoperability from this repository's own miner talking only to
  this repository's own pool. L1's independent conformance work is a
  prerequisite for that claim.

### 1.3 In-flight integration inputs

These are inputs, not files to overwrite:

| Input | Where | Contract with this packet |
|---|---|---|
| L1 SV2 conformance | `lab-sv2-conformance` worktree | Owns job/prevhash ordering, target byte order/lifecycle, Merkle path order, authority pinning, response correlation. This packet consumes a conformant session; it does not decide the fixes. |
| L2 rbitcoin retarget profile | separate rbitcoin worktree | Owns node flags, consensus profile composition, effective-profile introspection and node tests. This packet names the semantic fields the harness needs; it does not invent current flag names. |
| Node TDP work | branch `sv2-tp` (`1ec1dd0`) | Supplies TDP templates/solutions and keeps RPC as the settlement chain view. The lab treats `SubmitSolution` as submission, not acceptance. |
| Ladder implementation | branch `ladder` (`df7c654`) | Supplies the coinbase-denomination shape. The lab's expected-result oracle must use the ladder from the merged policy, not infer amounts from the current executable path. |

A lab scenario may run against an input branch only when the scenario declares
that dependency and the preflight checks the effective capability. A branch
name is not evidence of a capability.

## 2. Invariants the lab must preserve

1. **Real work only.** A share or block is produced by hashing a real header
   assembled from a real job. No synthetic discovery, no sleep that pretends to
   be hashes, no post-hoc timestamp insertion.
2. **Node owns block target.** The pool may copy the template's `nbits`; it does
   not choose or override the network target. The miner uses the job's
   `nbits`, not a locally guessed target.
3. **Pool owns share target.** The pool selects the share target from its work
   profile. It is not derived from the browser, the harness or the miner's
   requested budget.
4. **One connection, one identity.** A miner connection is authenticated as one
   `sp_id`. Several identities are several connections, never one connection
   with a changing username. Workers are nonce partitions under one identity.
5. **No absence-equals-invalid.** Missing evidence, a competing branch, a
   timeout or a node that has not seen a block yields `unknown` or a time-scoped
   non-inclusion observation. It does not yield "invalid".
6. **Observers never gate mining.** A slow browser, artifact writer or
   evidence collector cannot block the pool, the miner or the chain. Lost
   evidence makes an assertion inconclusive; it does not create a pass.
7. **Snapshots are consistent with a cursor.** A reconnecting observer gets a
   snapshot plus a cursor, or a resync instruction. It never silently skips a
   gap.
8. **Resources are owned and unique.** Each pool has its own Redis instance and
   Postgres database; each run has unique ports and a resource manifest. A
   second run cannot collide by accident.
9. **Processes survive observation but not ownership.** A browser closing does
   not stop a run. A supervisor restart can adopt only processes whose persisted
   run token and identity match; it never kills an unrelated PID.
10. **Fail closed on profile ambiguity.** Unknown scenario version, unknown work
    version, mismatched consensus profile, unsupported witness capability, or
    mixed v1/v2 work fails preflight/restore rather than running a baseline and
    labelling it the requested experiment.

## 3. Versioned scenario and entity model

### 3.1 Version and strictness

The scenario file has a top-level `scenario_version` integer. Version 1 is the
schema below. An unknown version is rejected before any process starts. Within
version 1, unknown required fields are rejected; the implementation may reject
unknown top-level fields to catch typos rather than silently ignoring them.
Breaking changes increment `scenario_version`; they do not mutate version 1.

The scenario is validated twice: structurally (types, references, uniqueness)
and semantically (capabilities, ports, work profile, time policy, statistics).

### 3.2 Distinct types

The implementation MUST use distinct Rust newtypes (or an equivalent nominal
type system), not bare strings or integers, for these IDs:

| Type | Scope | Meaning |
|---|---|---|
| `RunId` | one supervisor run | Durable identity of an experiment. |
| `NetworkId` | one consensus/P2P network | The nodes that validate the same chain. |
| `NodeId` | one node process | A chain backend, its RPC/TDP/P2P listeners. |
| `PoolId` | one accounting domain | An independent silent-pool instance with its own storage. |
| `FrontId` | one front process/listener | A role of a pool; many fronts may share one pool. |
| `IdentityId` | one miner identity | A BIP 352 address and its `sp_id`; reusable across connections. |
| `WorkerId` | one nonce/search partition | A logical hashing worker under one identity. |
| `MinerId` | one miner driver process | Hosts zero or more workers and connections. |
| `ConnectionId` | one SV2 session | One Noise session and channel binding an identity to a front. |
| `JobId` | one mining job generation | A template/target generation; carries the work profile. |
| `ShareId` | one accepted share | A submitted hash that reached accounting, with its job. |
| `CandidateId` | one block submission | A block-qualified hash submitted to a node. |
| `CommandId` | one client command | Idempotency key. |
| `LinkId` | one controlled link | A fault-controlled path between two endpoints. |
| `ArkBackendId` | one test Ark service | A stub or external Ark server used by a scenario. |

Required relationships:

- A node belongs to exactly one network.
- A pool belongs to exactly one network and names one or more node IDs it may
  use for templates/chain view. A shared node is allowed for a connected
  multi-pool experiment.
- A front belongs to exactly one pool. A pool may have several fronts; those
  fronts share the pool's accounting, per `MULTI-REGION.md`.
- An identity is not owned by a pool. A connection binds an identity to a
  front; an identity may switch pools only through a real disconnect and a new
  registration sequence.
- A worker belongs to exactly one identity, one miner process and one
  connection. Multiple workers may share one identity and connection; they have
  disjoint nonce or extranonce search ranges.
- A miner process may host several connections and workers. Logical placement
  is separate from process placement: N logical miners may share one process or
  use N processes.
- A link names exactly two typed endpoints and one direction policy. No link is
  implicit.
- An Ark backend is an external service node; it is not a pool and is not a
  miner identity.

### 3.3 Scenario shape

The exact Rust serialization is an implementation detail; these are the
required semantic fields. A representative file:

```json
{
  "scenario_version": 1,
  "run": {
    "id": "run-20261105-a",
    "seed": 12345,
    "time_policy": {
      "chain_time": "node_header",
      "miner_ntime": "hold",
      "mock_node_time": null
    },
    "warmup": {
      "blocks": 0,
      "window_fills": 1,
      "exclude_from_statistics": true
    }
  },
  "network": {
    "id": "net-a",
    "consensus_profile": {
      "required_capability": "rbitcoin.retarget",
      "profile_fingerprint": "sha256:..."
    }
  },
  "nodes": [
    {
      "id": "node-a",
      "network_id": "net-a",
      "implementation": "rbitcoin",
      "p2p": { "listen": "127.0.0.1:0" },
      "rpc": { "listen": "127.0.0.1:0" },
      "tdp": { "listen": "127.0.0.1:0", "session_cap": 8 }
    }
  ],
  "pools": [
    {
      "id": "pool-a",
      "network_id": "net-a",
      "node_ids": ["node-a"],
      "implementation": "silent-pool",
      "work_profile": {
        "work_unit_version": 2,
        "reference_target": "0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        "share_work_multiplier": 2,
        "max_work_multiplier": 4294967296,
        "max_window_target_units": "18446744073709551616",
        "window_rule": "network_block_work",
        "window_n": 8
      },
      "storage": {
        "redis_instance_id": "redis-pool-a",
        "postgres_database_id": "pg-pool-a"
      },
      "ark_backend_id": "ark-stub"
    }
  ],
  "fronts": [
    {
      "id": "front-a1",
      "pool_id": "pool-a",
      "roles": ["front", "payout", "api"],
      "listen": { "sv2": "127.0.0.1:0", "sv1": null, "jd": null }
    }
  ],
  "identities": [
    { "id": "id-alice", "address_source": { "fixture": "alice" } },
    { "id": "id-bob", "address_source": { "fixture": "bob" } }
  ],
  "miners": [
    {
      "id": "miner-1",
      "implementation": "sv2-cpu",
      "connections": [
        { "id": "conn-alice", "front_id": "front-a1", "identity_id": "id-alice" },
        { "id": "conn-bob", "front_id": "front-a1", "identity_id": "id-bob" }
      ],
      "workers": [
        { "id": "w-alice-1", "identity_id": "id-alice", "connection_id": "conn-alice", "budget": { "hashes_per_second": 100000 } },
        { "id": "w-bob-1", "identity_id": "id-bob", "connection_id": "conn-bob", "budget": { "hashes_per_second": 100000 } }
      ]
    }
  ],
  "ark_backends": [
    { "id": "ark-stub", "implementation": "lab-stub", "listen": "127.0.0.1:0" }
  ],
  "resources": {
    "redis_instances": [
      { "id": "redis-pool-a", "owner_pool_id": "pool-a", "port": 0, "dir": "run/redis/pool-a" }
    ],
    "postgres_databases": [
      { "id": "pg-pool-a", "owner_pool_id": "pool-a", "database": "spc_pool_a", "role": "spc_pool_a" }
    ]
  },
  "links": [
    { "id": "link-alice", "from": { "kind": "miner", "id": "miner-1" }, "to": { "kind": "front", "id": "front-a1" }, "policy": "normal" },
    { "id": "link-pool-node", "from": { "kind": "pool", "id": "pool-a" }, "to": { "kind": "node", "id": "node-a" }, "policy": "normal" }
  ],
  "phases": [
    { "id": "phase-1", "commands": [] }
  ],
  "assertions": [
    { "id": "assert-nonblock", "kind": "evidence", "predicate": "exists(share_accounted && !candidate)", "deadline_secs": 120 }
  ],
  "statistics": [
    {
      "metric": "block_interval_seconds",
      "estimator": "mean",
      "planned_n": 20,
      "stopping_rule": { "kind": "fixed_n" },
      "confidence": 0.95,
      "exclusions": ["warmup"]
    }
  ]
}
```

The example uses placeholder values; it is not a runnable fixture and it does
not claim that every field is currently accepted by `spc-pool` or `spc-lab`.

### 3.4 Ports and resources

- A scenario may request port `0` for a listener, but the supervisor records
  the actual bound port in the run manifest before any dependent process starts.
- Every logical listener (node P2P/RPC/TDP, pool front SV2/SV1/JDP, API, Redis,
  Postgres, Ark stub, miner control) gets a unique recorded endpoint.
- No scenario may contain two resources with the same port on the same address
  namespace. The allocator checks the manifest, the OS, and the run lock before
  launch.
- Pool storage is isolated: separate Redis instances and separate Postgres
  databases per pool. See section 8.

## 4. Work-unit and window implementation contract

### 4.1 Version matrix and profile validity

| `work_unit_version` | Per-share credit | Allowed `window_rule` | Status |
|---|---|---|---|
| 1 | legacy integer difficulty `d` | `share_work_legacy` (code 0) | Frozen; existing rows |
| 2 | exact reference multiplier `m_issue` | `network_block_work` (code 1) | Lab profile |

Any other combination is a configuration error. The detailed arithmetic,
profile validity, stepdown/step-up, rejection rules, fingerprint and tests are
in the two decision records. This section is the implementation checklist; the
decisions win if they disagree.

v2 restricts the reference to `R + 1 = 2^k` with `1 <= k <= 255`, and the
configured multiplier to `m_cfg = 2^j_cfg` with `0 <= j_cfg <= k - 1`.
Preflight rejects `R + 1` that is not a power of two with
`reference_not_power_of_two`, and a non-power/non-range `m_cfg` with
`multiplier_not_in_family`. A v2 profile is valid only if
`R >= T_powlimit` for the network's effective `pow_limit_bits`; otherwise it
fails with `reference_below_pow_limit`. The recommended regtest profile is
`R = 2^255 - 1` (`k = 255`, `E(R) = 2`), because it is easier than the regtest
pow limit `0x207fffff`. `R = 2^254 - 1` (`k = 254`) is a negative `pow_limit`
vector; `R = 2^254` is a negative non-power vector.

Bounds are hard caps: `m_cfg <= max_work_multiplier <= 2^32`;
`Q <= max_window_target_units <= 2^64`; `max_slice_shares <= 65 000` unless a
new width analysis is recorded. These are policy limits used by the checked
arithmetic, not claims that all larger values are mathematically impossible.
The per-slice width comments also assume the lab's fee bound `f < 2^51`; a
custom supply profile must re-evaluate them.

### 4.2 Required functions and issuance selection

The share-policy packet (LP-1) owns one pure module with these operations. Names
are illustrative; the behavior is not.

```text
decode_compact_target(bits) -> Result<Target, TargetError>
target_family(k, j) -> Result<Target, FamilyError>
issuance_exponent(k, j_cfg, T) -> Result<u32, FamilyError>
share_target_is_acceptable(S, network_target) -> Result<(), TargetError>
window_target_network_work(k, network_target, N) -> Result<u128, WindowError>
accumulate_slice_work(slices, Q) -> Result<WindowSelection, WindowError>
```

Rules:

- `E(T) = 2^256/(T+1)` is exact rational arithmetic. The only display
  conversion is an explicitly labeled decimal approximation.
- `R = 2^k - 1`, `1 <= k <= 255`; reject a non-power `R+1` with
  `reference_not_power_of_two`.
- `m = 2^j`, `0 <= j <= k - 1`; `target_family` returns `2^(k-j)-1`. Reject a
  non-power `m_cfg` with `multiplier_not_in_family`.
- `issuance_exponent` is O(1):
  `j_issue = min(j_cfg, k - ceil_log2(T+1))`, where
  `ceil_log2(n) = (n-1).bit_length()`. It MUST NOT scan `[1, m_cfg]` or search
  for divisors. `m_issue = 2^j_issue`; `d = m_issue`.
- `issuance_exponent` emits `multiplier_stepdown` when `j_issue < j_cfg` and
  the next generation with `j_issue == j_cfg` emits `multiplier_stepup`.
  `target_profile_unplannable` is a defensive guard; with a valid profile and
  `m = 1` it is unreachable.
- `d` is not recomputed from `m_cfg` at settlement.
- `Q(T) = ceil(N*(R+1)/(T+1))`; use a wide intermediate and checked `u128`
  result; reject `Q > max_window_target_units` (hard cap `<= 2^64`) with
  `window_target_too_large`.
- Compact decode uses the consensus `SetCompact` rule: reject sign bit,
  overflow for `size`/`word`, and a zero result. Exponent 33 with
  `word <= 0xffff` and exponent 34 with `word <= 0xff` are valid; there is no
  exponent-32 cutoff. The node enforces `target <= pow_limit`.
- The finding slice is excluded before accumulation. Whole slices are taken
  newest-first; the crossing slice is included. Derive `actual_sum`,
  `overshoot = max(0, actual_sum - window_target)`,
  `underfill = max(0, window_target - actual_sum)` and
  `underfilled = underfill > 0` from the slices at/below `high_water` and the
  persisted `window_target`; do not add columns for them. If the available tail
  ends before `Q`, the distribution is underfilled, pays from the available
  slices, and is excluded from window-length/statistical claims.
- v1 functions remain available for frozen rows, but a v2 build path must not
  call a v1 conversion function to "adapt" a target.

### 4.3 Job, channel and distribution ordering

1. A new template creates a job generation with `nbits`, the selected
   `share_target`, `m_issue`, work-unit version/fingerprint, monotonically
   increasing generation id and activation state.
2. A share submission names its job. Validation and accounting look up that
   job's target. A share for a job that is no longer valid is stale; it is not
   re-priced.
3. A target update or a network retarget creates a new generation. The pool
   must not mutate the old generation's `m_issue`.
4. A distribution is built against a specific job generation and records the
   selected `window_target`, `window_rule`, work version and fingerprint.
5. Settlement uses the distribution's frozen values. It never reads the current
   `nbits` to reconstruct `Q` or `N * d`.
6. **SV2 `SetTarget` follows the standard protocol.** Pinned sv2-spec
   `d8bf5b8` `05-Mining-Protocol.md` section 5.3.21 says `SetTarget` applies to
   future jobs and already-received jobs with empty `min_ntime`; it does not
   apply to already-received jobs with `min_ntime = nTime`. A future job's
   target is the channel target in effect when its `SetNewPrevHash` activates
   it. L1 implements this; LP-1 does not invent a pool-local extension.
7. A `job_issued` evidence record is emitted for every generation with
   `front_id`, `channel_id`, `job_id`, `generation`, `nbits`, `share_target`,
   `m_issue`, work-unit version/fingerprint, `issued_at` and
   `activation (future|active)`. `accounted` carries `job_id` and `generation`.
   The independent oracle needs this record; see section 7.2.

### 4.4 Persistence, wire fields and restore

New columns on `slice` and `distribution_snapshot`:

```text
work_unit_version       SMALLINT NOT NULL
work_unit_fingerprint   BYTEA          -- NULL only for v1
window_rule             SMALLINT NOT NULL -- on distribution_snapshot
window_target           NUMERIC(40,0)  -- existing; v2 stores Q, v1 stores N*d
```

Constraints:

```text
CHECK (work_unit_version IN (1, 2))
CHECK ((work_unit_version = 1) = (work_unit_fingerprint IS NULL))
CHECK (window_rule IN (0, 1))          -- 0 share_work_legacy, 1 network_block_work
```

Migration defaults existing rows to `work_unit_version = 1`,
`work_unit_fingerprint = NULL`, `window_rule = 0`. A v2 front restore rejects
any live-window slice with a different version/fingerprint. A settlement of an
old snapshot uses the row's values.

The wire fields are definite. Append after the existing fields:

- `spc:slices`: `U8 work_unit_version`, `B0_32 work_unit_fingerprint`.
- `spc:blocks`: `U8 work_unit_version`, `B0_32 work_unit_fingerprint`,
  `U8 window_rule`.

v1 sends a 32-byte all-zero fingerprint sentinel on the wire; the database
stores NULL for v1. A v2 event with a zero fingerprint fails decode. Front and
payout deploy atomically; old-format events fail decode. JS clients receive
wide integers as decimal strings, not rounded `Number`s.

`underfilled`, `actual_sum` and `overshoot` are derived from `high_water` and
`window_target`; they are recorded in lab evidence/metrics, not as database
columns. Missing slices for the derivation make the result `unknown`.

### 4.5 Required regression boundary

The existing `N * d` tests are not deleted or flipped. They become explicit v1
fixtures. New v2 fixtures cover the exact family, profile validity against the
pow limit, power-of-two family rejection (`R+1` non-power, `m_cfg = 3`),
stepdown/step-up including the exact power boundary, `Q(T)` (`65` for
`R = 2^255 - 1` on `0x200fffff`), floor-vs-ceil, restore rejection, retarget,
mixed-target credit, compact decode, and every checked SPEC section 4.2/4.4
intermediate. v1 keeps its documented saturating behavior. This is how the lab
avoids a hidden conversion.

## 5. Native SV2 CPU miner contract

### 5.1 Process and connection model

- The lab miner is a native SV2 miner. SV1 translation is not a dependency.
- One `ConnectionId` is one Noise session and one negotiated mining channel
  (extended channels first). One connection carries one identity. Multiple
  identities use multiple connections.
- A `MinerId` process may host several connections and several workers. A
  worker is a nonce/extranonce search partition under one identity.
- A worker never hashes a header that is not derived from the current job's
  coinbase, merkle path, version, time, `nbits` and extranonce prefix.
- A miner may switch pools only through a real disconnect, new session setup
  and new registration; it may not rewrite a username in a live channel.

### 5.2 Hashing and budget

- Hash with real SHA256d over the assembled 80-byte header. Count every
  attempted nonce. Hash batches are bounded (for example a few thousand hashes)
  and yield to network/control tasks between batches.
- A worker's budget is a requested rate or a total attempt budget, in hashes or
  hashes/second. The driver reports `requested`, `attempted`, `elapsed`,
  `achieved_hps`, `budget_hps`, `saturated` and `queue_depth` per worker.
- A budget is enforced by pacing actual attempts, not by sleeping after
  synthetic work. If the CPU cannot reach the requested rate, the reported
  `achieved_hps` is lower and `saturated = true`; the harness does not inflate
  it.
- The driver searches a disjoint space per worker. It increments the
  miner-controlled extranonce on nonce exhaustion and obtains a fresh job when
  the protocol requires it; it does not rehash an identical header after
  pause/reconnect.
- On a clean job or target generation, the worker abandons stale batches. On a
  pause, it stops scheduling hashes; network delivery, timers and settlement
  continue.

### 5.3 Target and job state

- The driver stores the channel's announced target and the job generation's
  target. It validates a found hash against the job's share target before
  submission.
- `SetTarget` follows pinned sv2-spec `d8bf5b8` section 5.3.21: it applies to
  future jobs and already-received jobs with empty `min_ntime`, not to
  already-received jobs with `min_ntime = nTime`. A future job's target is the
  channel target in effect when its `SetNewPrevHash` activates it. This is a
  standard per-job target, not an extension.
- A target update is applied in protocol order. A submission uses the target
  recorded for the header it hashed; the driver does not relabel an old hash
  with the new target.
- The pool may step an issuance multiplier down at a new generation when the
  network target becomes easier (for example at the pow limit) and step back up
  at a later generation. The driver consumes the announced target; credit
  remains the pool's job generation, not a miner guess.
- A block-qualified hash (`hash <= network target`) is submitted promptly and
  reported as a candidate. It is not held to simulate time.
- Reconnect resets nonce/extranonce state and obtains a new target/job; old
  session state is discarded.

### 5.4 Submission outcomes

The driver records at least:

- requested hash budget and actual attempts;
- submitted shares (with job, nonce, ntime, version, extranonce, hash);
- ingress replies correlated by request/sequence number;
- accepted/rejected categories from the pool reply;
- block-qualified local solutions and their candidate submission;
- reconnect and stale-cancel counters.

`SubmitShares.Success` is not proof of front accounting. The driver's report
must distinguish "ingress acknowledged" from "accounted". L1 owns the exact
reply-field meaning; the lab consumes the corrected meaning.

### 5.5 Control surface

The driver exposes a local control endpoint (or stdin/stdout protocol) with
semantic operations, not a promise of existing CLI flags:

```text
add_connection(front, identity)
remove_connection(connection)
set_worker_budget(worker, budget)
pause_workers(workers)
resume_workers(workers)
switch_pool(connection, front, identity)   # disconnect, setup, register
snapshot() -> effective target, jobs, counters, measured rates
```

Every operation has a `CommandId` and a durable outcome (section 6). The control
surface is not the SV2 wire protocol and must not be able to inject a share or
change a target outside the pool's normal messages.

## 6. Commands, runs and durable outcomes

### 6.1 Envelope

```text
LabCommand {
  command_id: CommandId,
  run_id: RunId,
  issued_at_wall: timestamp,
  actor: ActorId,
  kind: CommandKind,
  payload: ...
}
```

`CommandId` is unique per run (UUID/ULID or equivalent). The command log is
durable before the command has an external effect.

### 6.2 Idempotency and conflicts

1. Retrying the same `command_id` with the same payload returns the stored
   outcome; it does not add a second miner, repeat a partition, or re-run a
   destructive action.
2. The same `command_id` with a different payload is a conflict: reject it and
   record the mismatch. Never "last writer wins" silently.
3. Commands have preconditions (run state, entity state, ownership). A failed
   precondition is a durable `rejected_precondition`; it is not a process crash.
4. Conflicting commands are ordered by the command log. The implementation
   declares precedence for opposite actions (for example `stop` beats a
   concurrent `start`; `remove` beats a concurrent `add` for the same ID). The
   resolution is recorded in the outcome, not left to task scheduling.
5. A command interrupted by a supervisor crash is `unknown` until reconciled
   from the process/resource manifest. It is not assumed successful.

### 6.3 Outcomes

```text
accepted | rejected_precondition | running | succeeded | failed |
cancelled | blocked_on_height | unknown
```

`blocked_on_height` is for a wait whose declared chain producers cannot advance
the named node because they are paused or absent. It is a reportable outcome,
not an infinite hang. A timeout is `failed` or `inconclusive` according to the
assertion, never `succeeded`.

### 6.4 Run lifecycle

```text
created -> preparing -> running -> stopping -> stopped
                         |            |
                         +-> failed   +-> completed_with_errors
```

A run has a durable manifest with base revision, dirty/build identity, binary
hashes, effective profiles, resource map, process records, command log, event
log and artifact paths. `created` and `preparing` may be retried; `running` is
not recreated by a retry of a command with the same ID.

### 6.5 Command vocabulary

The initial vocabulary (semantic names) is:

- create/start/stop a run; add/remove a miner, connection or worker;
- set a worker's hash budget; connect/switch/pause/resume;
- partition/heal a link; add latency/loss; kill/restart a role or node;
- inject a transaction through a declared funding/mempool adapter;
- wait for a node height, candidate, accounting event, settlement, delivery or
  timeout;
- assert an evidence predicate.

The exact JSON field names are the runner packet's deliverable. They are not
current `spc-pool` flags.

## 7. Observations, evidence and snapshots

### 7.1 Event envelope

Every observation carries:

```text
run_id, event_id (collector-monotonic), source, source_cursor,
actor {kind, id}, command_id?, occurred_at_wall, occurred_at_mono,
ingested_at, payload
```

`event_id` is an ingestion order, not a global causal order. A source cursor is
per source (Redis stream ID, Postgres LSN/timestamp, process counter, node
poll). Two sources reading the same database are not independent evidence.

### 7.2 Evidence states

These states are distinct and must not be collapsed:

| State | Emitted by | Meaning | Not evidence of |
|---|---|---|---|
| `job_issued` | front job path or harness | A job generation was issued with `front_id`, `channel_id`, `job_id`, `generation`, `nbits`, `share_target`, `m_issue`, work-unit version/fingerprint, `issued_at` and `activation`. | That any miner received or hashed it. |
| `submitted` | miner | The miner hashed a real job and sent a share meeting its job target. | Pool acceptance or accounting. |
| `ingress_ack` | front ingress | The ingress decoded the submission and replied with the protocol's success/ack. | Front accounting or payout. |
| `ingress_rejected` | front ingress | The ingress rejected the submission with a protocol error code. | That the share was invalid under a different target or that the pool is dishonest. |
| `accounted` | front accounting | The share was appended to the open slice with an exact `d`, `job_id`, `generation` and work profile. | That a slice was sealed, persisted, or paid. |
| `accounting_rejected` | front accounting | The front rejected the share after ingress. | Consensus invalidity. |
| `candidate` | pool block path | A block-qualified hash was assembled and submitted to a node. | Node acceptance or canonical inclusion. |
| `canonical_at(node, hash, anchored_tip, time)` | node observer | A time-scoped result: `yes`, `no` or `unknown` for whether the node's active chain anchored at `anchored_tip` contains `hash` at its height. | Consensus validity, permanence, or another node's view. |
| `settled` | payout | The payout role committed the block's settlement/ledger transaction under its confirmation gate. | That the block remains canonical forever; a later reorg adds an invalidation event. |
| `delivered` | recipient-side observer | A recipient's scanner/wallet/Ark receipt observed a spendable payment. | That the pool's accounting matches an independent policy oracle. |
| `unknown` | any observer | Evidence is unavailable, timed out, gapped, conflicting or not yet observed. | A negative result. |

A `canonical_at` record has at least:

```text
node_id, block_hash, height, anchored_tip, observed_at,
source, result: yes | no | unknown, detail
```

- `yes` requires the node's active chain at the anchored tip to contain the
  block at that height.
- `no` means the node's active chain at the anchored tip has a different block
  at that height, or the block is known but not active. It is **not** "invalid".
- `unknown` covers an RPC error, an unavailable node, a tip that moved during
  the observation, an unknown block, pruned data or a timeout.

A node's explicit rejection (for example a block submission returning an
authoritative rejection reason) is a separate `node_rejected` event. Only that
event can support "invalid". A competing canonical block, an absent candidate,
a missing event, or a timeout cannot.

### 7.2.1 Oracle inputs and generation correlation

The v2 policy oracle needs one place to resolve `job_id -> (nbits, share_target,
m_issue)`. Its complete input set is:

- `job_issued` records (including generation and activation state);
- `accounted` records carrying `job_id` and `generation`;
- sealed slice blobs and `slice_credit` aggregates;
- the frozen `distribution_snapshot` `window_rule`, `window_target`,
  `high_water`, work-unit version and fingerprint;
- the actual on-chain coinbase for the found block;
- node `canonical_at` observations for current-status checks.

For each sampled share, the oracle recomputes the header/share hash, checks it
against the job's `share_target`, computes `d = m_issue` from `job_issued`, and
compares the sum per identity with `slice_credit.diff_sum`. A missing or
ambiguous job record yields `unknown`, not a guessed credit. A share whose
credit matches a different generation is a failure.

The type-32 v2 audit is not claimed by this packet: the current share
blob/leaf does not carry `job_id -> (share_target, m_issue)`. A future
`share-accounting-ext-deltas.md` change must define a mapping source before a
v2 type-32 claim; v1 type-32 behavior is unchanged.

### 7.3 Reorgs and status derivation

- Historical observations are never deleted by a reorg.
- Current status is derived from the latest `canonical_at` observations and
  settlement records, with the observation time and anchored tip preserved.
- A `settled` record can be followed by `settlement_invalidated`/`orphaned`;
  the historical settlement remains auditable.
- An assertion must say whether it means "at a named observation time" or
  "eventually after re-evaluation". A bare "is canonical" is ambiguous and
  rejected by the scenario validator.

### 7.4 Observer isolation

- The collector reads with its own cursor. It MUST NOT acknowledge or consume
  another role's Redis consumer-group work. If a consumer group is used, it is
  a separate group with no effect on the pool's groups.
- Publication uses a bounded queue. A full queue produces an explicit
  `event_lost`/`gap` record and an assertion state of `inconclusive`; it never
  blocks the mining path.
- A slow browser cannot slow the collector. A slow collector cannot slow the
  pool.
- The observer is on the SPc/harness side. It never injects an identity field
  into the Ark role's inputs/streams (SPEC section 11.3).

### 7.5 Snapshot and resumable events

The runner exposes one contract to CLI, headless tests and the browser:

```text
GET  /api/lab/v1/runs/{run_id}/snapshot?cursor=<event_id>
GET  /api/lab/v1/runs/{run_id}/events?after=<event_id>   # SSE
POST /api/lab/v1/runs/{run_id}/commands
GET  /api/lab/v1/runs/{run_id}/artifacts/...
```

- The snapshot contains the run manifest summary, current entity states,
  evidence watermarks, gaps and the cursor it is consistent with.
- If `cursor` is older than the retained tail, the response is
  `resync_required` with a new snapshot cursor. There is no silent skip.
- Wide integers (targets, work sums, sats) are decimal strings in JSON. A
  JavaScript client must not round them through `Number`.
- The browser attaches to the same snapshot/stream. Closing it has no effect on
  the run. Reopening either resumes from a cursor or asks for a resync.
- Recorded-run review reads the same artifacts; it is not a separate UI-only
  format.

### 7.6 Evidence versus chart decimation

- Raw evidence is append-only (event log plus retained source payloads). It is
  what assertions read.
- Chart series are a separate derived projection. A decimation/aggregation
  policy records its algorithm, bucket size, source range and whether it
  dropped points.
- Decimating a chart never deletes the raw evidence required by an assertion.
  If retention policy removes raw evidence before artifact export, the run
  records a gap and the affected assertion is `inconclusive`, not passed.
- A rendered chart is never evidence of a payout, a canonical block or a
  delivered payment.

### 7.7 Artifact privacy and retention

- The shareable run manifest contains revisions, binary hashes, topology,
  effective profiles, command/event summaries and aggregate results. It
  contains no credentials, private fixture keys, mailbox material or Ark
  secrets.
- Identity-bearing evidence (`sp_id`, `slice_credit`, `accounted` details,
  recipient observations) is private fixture material with separate access and
  retention from the shareable manifest. The harness may need it for the
  oracle, but it is not published by default.
- Do not record `a_send`, ECDH shared secrets, transient tweaks, Ark order
  signing keys, `b_scan`/`b_spend`/`b_ark` secrets, or mailbox secrets to make
  replay easier. The Ark role's inputs/streams still carry no identity field
  (SPEC section 11.3).
- Redacting or aggregate-only export is a presentation choice. It is not
  evidence of production privacy guarantees, and it does not make the lab a
  mainnet privacy test.
- Retention deletion of identity-bearing evidence is recorded; an assertion
  that needed it becomes `inconclusive`, not passed.

## 8. Process, resource and isolation contract

### 8.1 Supervisor and adoption

The supervisor is a native process manager, not a shell pipeline. A run
manifest records at least:

```text
run_id, owner_instance, created_at, base_revision, dirty_tree_hash,
binary_paths_and_hashes, resource_manifest, effective_profiles,
process_records[], locks, state
```

Each process record records `pid`, process group, start-time token
(`/proc/<pid>/stat` field 22 or equivalent), executable hash, argv, environment
run token, log paths and role/entity IDs.

Adoption/reap algorithm:

1. Take the run lock. A live owner heartbeat prevents a second supervisor.
2. For each record, verify the PID's start-time token and run token. A matching
   process is adopted; its logs and control socket are reattached.
3. A process with a matching run token but a dead/mismatched PID is an orphan
   of this run, not a random process. Reap it only if the run manifest authorizes
   it and the identification matches; otherwise mark it `orphan_unverified`.
4. Never kill by PID alone. Never kill a process whose run token or
   start-time token differs.
5. On graceful stop, signal the process group, wait, then force-kill only after
   the deadline. Release ports, namespaces, Redis, Postgres and temp directories.
6. On a supervisor crash, leave processes and resources in place for adoption.
   The next start reports `adopted` or `stale_resources`; it does not silently
   start a duplicate run.

### 8.2 Unique per-pool storage and ports

- Every pool gets its own Redis server instance: unique port, unique data
  directory, no shared database index, and its own credentials/ACL. A pool
  cannot read another pool's stream namespace.
- Every pool gets its own Postgres database and role, on a run-owned cluster.
  The default is one Postgres cluster per run with a unique port and one
  database/role per pool; a per-pool cluster with its own unique port is also
  valid and is required when a scenario claims storage-process isolation.
- The supervisor records both the requested and actual endpoints. It checks
  collisions before launch and reports the exact colliding resource.
- A shared Redis or shared database is not allowed for competing pools in the
  first lab. A later isolation mechanism needs its own tested justification.
- Pool A's front cannot be configured with Pool B's Redis URL, Postgres URL or
  credentials; preflight compares the run manifest to the effective config.

### 8.3 Link-fault backend

The authoritative backend for partition/latency experiments is Linux network
namespaces with veth pairs and nftables rules. It is chosen because a one-time
disconnect is not a sustained fault: a node's reconnect loop must keep failing
until the link is healed, and no alternate route may bypass the controlled path.

Contract:

1. Each endpoint that needs isolation runs in a network namespace. A link is a
   veth pair with static addresses and explicit routes. There is no default
   route to the host or to another namespace.
2. Links start in the scenario's declared state. A fault installs a drop rule
   (or an equivalent tested block) in both directions while the listener stays
   bound and the process stays alive. A heal removes the rule.
3. Latency/loss uses `tc netem`/`tc qdisc` on the link, not `sleep` in the
   application. The scenario records the exact qdisc and units.
4. Reconnect attempts are visible: the node/pool/miner logs and link counters
   show repeated failures during the fault and successful reconnection after
   the heal. A single disconnect does not count as a sustained partition.
5. No-bypass preflight: from each namespace, attempt to reach every other
   entity's non-link address. All such attempts MUST fail. The preflight also
   checks the nftables ruleset and rejects a link with an unremoved host route.
6. Listener reachability after heal: the listener must still be bound, accept a
   new session, and resume jobs. A test that only checks "process alive" is
   insufficient.
7. Ark links are modelled explicitly: `pool -> ark-stub` and
   `identity/miner -> ark-stub` (and mailbox paths where used) have their own
   links and faults. The Ark stub has a reachable listener and no production
   keys. A pool or miner whose Ark link is down must report an Ark-specific
   pending/failed state, not silently use a direct route.
8. If the environment cannot grant the required namespace capabilities, a
   userspace relay may be used only for a smoke scenario tagged
   `isolation=relay`. It cannot satisfy a partition/no-bypass assertion; those
   assertions are `inconclusive`, never passed. CI must report the backend it
   actually used.

An unprivileged runner may use user namespaces (`unshare -Urn`) to create its
own network namespace with veth/nftables/`tc` inside it on kernels that permit
unprivileged user namespaces. This is a capability probe, not a claim that
every host allows it: `sch_netem` must be available for latency scenarios, and
the probe records the kernel, namespace privileges, loaded qdisc and the exact
backend. If the probe fails, the relay fallback and its weaker evidence apply.
LP-4's merge note must add `iproute2`, `nftables` and `util-linux` to the flake
inputs; this packet does not edit `flake.nix`.

The implementation may choose another backend only if it demonstrates the same
sustained-fault and no-bypass properties with a test. A proxy with a fail-open
direct fallback is not equivalent.

### 8.4 Startup, capability and height gates

Preflight compares the scenario's required capabilities with effective
introspection. It never guesses a CLI flag. The L2 node interface is pending;
the semantic fields the harness needs are:

```text
consensus_profile_id, profile_fingerprint, pow_limit_bits,
initial_target_bits_or_difficulty, target_spacing_secs,
retarget_interval_blocks, retarget_algorithm_and_clamps,
minimum_difficulty_policy (enabled, time_gap_secs),
timestamp_rules, subsidy_initial_sats, subsidy_halving_interval_blocks,
coinbase_maturity_blocks, witness_commitment_supported,
tdp_session_cap
```

Required gates:

| Gate | Evidence | Failure |
|---|---|---|
| PoW baseline | node effective profile | Preflight fail; do not label a baseline run as retargeting. |
| Self-adjusting profile | node profile fields and a covered adjustment-boundary test | Preflight fail or `inconclusive`; never silently use the baseline. |
| Profile fingerprint match | node introspection vs scenario | Preflight fail; name both values. |
| Cross-node profile match | all nodes on the network | Preflight fail; a mismatched node is not part of the experiment. |
| Halving/subsidy profile match | node and pool effective economics | Preflight fail before any economic scenario. |
| Coinbase maturity | node profile and pool config | Preflight fail before maturity/settlement waits. |
| Witness commitment | node and pool/template source capability | Preflight fail before any witness-bearing transaction/fee/wallet scenario. |
| TDP session capacity | node profile and planned fronts | Preflight fail or require the tested limit; do not promise unlimited fronts. |
| Pool work profile | pool effective config/API | Preflight fail on unknown/mixed version or fingerprint, `R < T_powlimit` (`reference_below_pow_limit`), `m_cfg > max_work_multiplier`, or `Q > max_window_target_units`. |
| SV2 conformance | L1 test evidence and negotiated session facts | Experimental scenario only; no interoperability claim without independent vectors. |
| Ark stub/backend | stub readiness and link manifest | Preflight fail for scenarios requiring Ark; do not fall back to a real server. |
| Observer capacity | retained cursor/queue capacity | Preflight warning or fail; a known gap makes assertions inconclusive. |

Startup is readiness-based, not fixed-sleep-based: resources, node RPC/TDP,
Redis, Postgres, pool roles, front bootstrap, miner sessions and Ark stub each
have a bounded readiness probe and a deadline. A failed probe cleans up and
reports the failing component.

A height-driven action names its node and its chain producers. Waiting for
`H + 6` with no miner able to advance that node is `blocked_on_height`; a
timeout is `failed` or `inconclusive`. Fixture generation via node RPC is setup,
recorded outside the measured workload; it is not evidence of mining.

## 9. Time, economics and statistics

### 9.1 Time domains

Record and distinguish:

- chain height and node header time;
- process wall time;
- process monotonic time;
- node mock time, if a scenario explicitly sets it;
- link/queue timestamps and collector ingestion time.

A uniform mock-time offset is not a generic speed multiplier. It can change
difficulty eligibility and timestamp rules. Scaling block timestamps alone is
not equivalent to scaling the whole system. An accelerated retarget profile
must declare all nodes' time policy and test the interaction with minimum
difficulty and adjustment boundaries.

The miner's `ntime` policy is explicit: `hold` the job time within the allowed
range, or `roll` it with declared bounds. A test that rolls across a
minimum-difficulty boundary states that it does so and expects the profile's
actual rejection/renewal behavior; wall-clock passage alone is not asserted to
invalidate a job.

### 9.2 Warmup and economic declarations

A scenario that makes a statistical or economic claim declares:

- accounting-window warmup and the number of fills excluded;
- coinbase maturity and confirmation depth;
- node subsidy/halving profile and the pool's matching economics;
- initial balances/float and Ark limits, if used;
- fee workload and mempool assumptions;
- pool market share / hash-rate population assumptions.

Warmup observations are retained but excluded only by an explicit rule.

### 9.3 Statistics

Every inferential metric declares:

```text
estimator (mean, ratio, quantile, probability, ...),
planned_n (independent runs or observations),
stopping_rule (fixed_n, boundary_count, confidence_width, ...),
confidence/interval method,
exclusions (warmup, failed runs, incomplete evidence),
unit and time domain
```

Without those declarations, the report is descriptive only. There is no
universal sample count. A retarget experiment additionally requires observed
adjustment-boundary coverage; if no boundary was crossed, the retarget claim is
`inconclusive`, not "no effect". Real PoW means repeated runs do not guarantee
identical discoveries; report intervals and incomplete evidence.

## 10. Packets and runnable vertical slices

Each packet is bounded to a reviewable vertical slice. The commands below are
acceptance contracts for the packet's deliverables; they are not claims that
the current tree has these packages, flags or scripts. A packet may not add a
guessed flag to an existing binary instead of creating its declared interface.

### LP-0 - Decisions and contracts (this packet)

Goal: make the work-unit/window decisions and runner/evidence contracts
reviewable and implementable.

Deliverables:

- this document;
- the two decision records and changelog fragments;
- the L3 review report.

Depends: proposal and review synthesis only.

Acceptance:

```sh
git diff --check
python3 scripts/check-lab-docs.py   # ASCII, links, arithmetic/vector checks
```

Exit criteria: Fable approves the decisions and contracts, or returns
actionable changes. No code may consume the decisions before approval.

Fable gate checklist:

- Are v1 and v2 meanings unambiguous, with no hidden conversion?
- Does the v2 reference cover the network pow limit, with a deterministic
  multiplier stepdown/step-up?
- Is `E(T)` and `Q(T)` exact, with a stated rounding/error bound?
- Are all SPEC section 4.2/4.4 intermediates checked and named, rather than
  assumed to fit `u128`?
- Is compact decode the consensus rule, including exponents 33/34?
- Are per-job credit, `job_issued` oracle inputs and restore rejection
  testable?
- Are evidence states, no-absence-invalid, observer isolation and process
  adoption concrete enough to implement?
- Are packet boundaries and dependencies on L1/L2 explicit?

### LP-1 - Share policy v2 and window rule

Goal: implement the pure v2 work-unit/target family and the network-block
window rule, with v1 frozen.

Depends: LP-0 approval, then **L1 merged** for `bins/spc-pool/src/sv2_server.rs`
and the target-lifecycle parts of `bins/spc-pool/src/front.rs`. Announced-target
plumbing is an L1 merge note; LP-1's own diff starts after L1 so the two
packets do not write the same files in parallel. Pure arithmetic may be
prototyped earlier, but it is not dispatched as an integration packet until L1
is in.

Owned paths (topic worktree; merge notes for shared files):

```text
crates/spc-accounting/src/work_unit.rs        # new pure module
crates/spc-accounting/src/lib.rs              # re-export / window integration
crates/spc-accounting/src/tests.rs            # v1 regression + v2 vectors
bins/spc-pool/src/sv1.rs                      # v1 target helpers; v1 untouched
bins/spc-pool/src/front.rs                    # post-L1 job generation/window
crates/spc-store/src/models.rs                # version/fingerprint/rule rows
crates/spc-store/src/wire.rs                  # event fields
crates/spc-store/migrations/0005_lab_work_units.sql
```

Shared-file merge notes (not owned by the packet): `bins/spc-pool/src/config.rs`,
`bins/spc-pool/src/main.rs`, `config/spc-pool.example.toml`, `Cargo.toml`,
`Cargo.lock`, `docs/SPEC.md`, `docs/STATUS.md`, and the L1 handoff for
`sv2_server.rs` announced-target/job-target plumbing.

Required tests:

```sh
cargo test -p spc-accounting work_unit
cargo test -p spc-accounting window_network
cargo test -p spc-accounting accounting_overflow
cargo test -p spc-pool share_target_v2
cargo test -p spc-pool window_target_v2
cargo test -p spc-pool job_generation
cargo test -p spc-store work_unit_restore
cargo test -p spc-pool --test e2e_sv2
cargo test --workspace
```

Acceptance evidence:

- power-of-two family vectors and rejections from
  `2026-11-05-lab-share-work-units.md`, including `R = 2^255 - 1`,
  `profile_rejects_reference_below_pow_limit`,
  `reference_rejects_non_power_of_two`, `multiplier_rejects_non_power`,
  `issuance_steps_down_to_power_of_two` and the exact-power boundary;
- `Q(T) = 65` for the reference vector and the pow-limit `Q = 9`, floor
  values 64/8 rejected;
- the checked SPEC section 4.2/4.4 overflow set (`sum(d)`, `sum(d*f)`,
  `diff_fee_sum`, `D_win`, payout products, `u64` conversion), with named
  errors and v1 saturating regressions;
- consensus compact-decode vectors from the decision record;
- v1 `N * d` tests still pass unchanged;
- a retarget test, mixed-target credit, multiplier stepdown/step-up, and a
  `job_issued` oracle test that catches wrong-generation credit;
- v1/v2 restore mismatch and definite wire decode tests;
- a test showing no `f64` in the v2 target/credit/window path.

Fable gate: approve or return changes before LP-2/LP-3 consume the policy. The
packet must not claim mainnet readiness or change the ladder/Ark rails.

### LP-2 - Native SV2 CPU miner

Goal: real CPU SHA256d work over a conformant SV2 session, with measured
budgets and correct target/job handling.

Depends: LP-0; L1 conformance for end-to-end interoperability claims; LP-1's
target contract for credit tests.

Owned paths:

```text
bins/spc-cpu-miner/                 # new binary
bins/spc-cpu-miner/src/session.rs   # Noise/setup/channel/job state
bins/spc-cpu-miner/src/hash.rs      # header assembly and SHA256d loop
bins/spc-cpu-miner/src/budget.rs    # pacing and measured rates
bins/spc-cpu-miner/tests/           # vectors, target, budget, reconnect
```

Shared-file merge notes: `Cargo.toml`, `Cargo.lock`, `flake.nix`, any workspace
lints, and `scripts/lab/*` launcher (owned by LP-3).

Required tests:

```sh
cargo test -p spc-cpu-miner
cargo test -p spc-cpu-miner --test sv2_session
cargo test -p spc-cpu-miner --test target_budget
scripts/lab/run-once.sh --scenario scenarios/lab/one-pool-two-miners.toml --headless
```

Acceptance evidence:

- observed non-block accepted shares and at least one block solution;
- a node validates the submitted block;
- stale/duplicate/below-target submissions are rejected with the protocol
  codes;
- a target update changes the per-share credit without re-pricing old shares;
- nonce/extranonce exhaustion renews work without duplicate headers;
- measured `achieved_hps` is under/at the budget and `saturated` is truthful;
- a reconnect starts a new session and does not reuse old jobs;
- the miner reports ingress acknowledgment separately from accounting.

Fable gate: review the actual diff and the run artifacts. Do not accept a
self-consistent pool/miner pair as interoperability evidence; require L1's
independent vectors.

### LP-3 - Headless supervisor, evidence and thin browser

Goal: one supervised run path, durable commands, snapshots/SSE, and a thin
read-only browser over the same contracts.

Depends: LP-1 schema/decision; LP-2 for a real vertical slice; can start with
fixtures before the miner is complete.

Owned paths:

```text
crates/spc-lab/                     # scenario types, validation, allocator
crates/spc-lab/src/command.rs       # command envelope/idempotency
crates/spc-lab/src/evidence.rs      # event/state/snapshot types
crates/spc-lab/src/supervisor.rs    # process adoption/reap
crates/spc-lab/src/links.rs         # backend interface
bins/spc-lab/                       # CLI/HTTP/SSE and static web assets
scripts/lab/                        # launchers, preflight, assertion helpers
```

Shared-file merge notes: `Cargo.toml`, `Cargo.lock`, `flake.nix`.

Required tests:

```sh
cargo test -p spc-lab --test scenario_v1
cargo test -p spc-lab --test command_idempotency
cargo test -p spc-lab --test evidence_states
cargo test -p spc-lab --test oracle_generation_credit
cargo test -p spc-lab --test snapshot_resume
cargo test -p spc-lab --test supervisor_adoption
scripts/lab/run-once.sh --scenario scenarios/lab/one-pool-two-miners.toml --headless
```

Acceptance evidence:

- two runs coexist without port/storage/database collisions;
- a browser-free scenario finishes and writes machine-readable artifacts;
- duplicate command IDs do not duplicate effects; a conflicting payload is
  rejected;
- stopping a browser does not stop work; reattach resumes from a cursor;
- a supervisor restart adopts the surviving process group or reports
  `stale_resources`, never kills an unrelated PID;
- an observer overflow produces a gap and an inconclusive assertion, not a
  pass;
- the oracle recomputes `d` from `job_issued` and the share hash, matches
  `slice_credit.diff_sum`, and catches a wrong-generation credit; a missing
  record is `unknown`;
- the thin browser reads the same snapshot/SSE and never controls run lifetime;
- chart decimation leaves raw evidence intact;
- identity-bearing evidence is in the private artifact store, not the
  shareable manifest, and no `a_send`/secret/tweak is recorded.

Fable gate: review command/evidence schemas before the browser is expanded. A UI
that invents a second contract is sent back.

### LP-4 - Topology, link faults and experiments

Goal: multi-pool/multi-node topologies, sustained link faults, Ark stub links,
retarget and evidence-backed acceptance.

Depends: LP-3; L2 profile for retarget scenarios; L1 for SV2 claims.

Owned paths:

```text
scripts/lab/netns-preflight.sh
scripts/lab/link-fault.sh
scripts/lab/ark-stub.*
scenarios/lab/*.toml or *.json
crates/spc-lab/src/links/netns.rs
```

Shared-file merge notes: `flake.nix` must add `iproute2`, `nftables` and
`util-linux` for the capability probe and backend; `Cargo.toml`/`Cargo.lock`
if a netlink helper is introduced. LP-4 does not edit them directly.

Required tests:

```sh
scripts/lab/netns-preflight.sh
scripts/lab/ci.sh --scenario scenarios/lab/two-pools-sustained-partition --isolation=netns
scripts/lab/ci.sh --scenario scenarios/lab/retarget-boundary --isolation=netns
scripts/lab/ci.sh --scenario scenarios/lab/ark-link-fault --isolation=netns
cargo test -p spc-lab --test evidence_reorg
cargo test --workspace
```

Acceptance evidence:

- two pools have separate Redis instances and Postgres databases and cannot
  read each other's state;
- a pool loses a link and its reconnect loop keeps failing for the declared
  fault duration; the listener is still bound; healing resumes work;
- a direct/non-link connection attempt from each namespace fails; the
  no-bypass preflight is part of the run artifact;
- `scripts/lab/netns-preflight.sh` records whether `unshare -Urn`, veth,
  nftables and `sch_netem` are available; a missing capability downgrades to
  the declared relay backend, never a silent bypass;
- non-winning pools observe new tips and update jobs;
- accounting is pool-local; a reorg selects the greatest-work branch and does
  not double-settle;
- the Ark link fault produces an Ark-specific pending/failed state and a
  successful retry after heal, without a direct bypass;
- a retarget run covers at least one adjustment boundary and reports the
  observed bits/timestamps; without coverage the result is inconclusive;
- statistics include estimator, planned N, stopping rule and incomplete
  evidence markers.

Fable gate: review the topology manifest, fault logs and evidence queries. A
one-time disconnect, a proxy fail-open, or a chart-only observation is sent
back.

## 11. Known gaps and explicit non-claims

These are not hidden in the packet acceptance:

1. L1's SV2 defects (job/prevhash ordering, target byte order/lifecycle,
   Merkle-path order, authority pinning, response correlation) are not fixed by
   this packet. The miner is not interoperability evidence until L1 lands.
2. L2's rbitcoin profile and exact flag/introspection interface are pending.
   This document names required semantic fields, not current flags. Automatic
   retargeting is not established by a PoW overlay.
3. `sv2-tp` `SubmitSolution` has no acceptance acknowledgement. Candidate
   submission is not canonical inclusion; the RPC chain view remains needed.
4. The ladder branch is an in-flight input. This packet does not implement or
   re-derive the denomination shape, and the lab oracle must use the merged
   ladder policy.
5. Witness-bearing transactions are gated on witness commitment support. Empty
   mempool success is not coverage.
6. Ark stubs and links are lab infrastructure, not a production Ark server. No
   production keys or real funds are used.
7. This packet does not implement VarDiff, multi-channel multiplexing, SV1 lab
   mining, multiple miners per channel, or a full browser product.
8. There is no code for `spc-lab` or the CPU miner in this worktree. The
   commands above are deliverables and acceptance contracts.
9. Network-namespace isolation may require privileges unavailable in some CI.
   The user-namespace path is a capability probe; the relay fallback is
   explicitly weaker and cannot pass partition assertions.
10. v2 type-32 auditing is not claimed. The current share blob/leaf has no
    `job_id -> (share_target, m_issue)` source; a future
    `share-accounting-ext-deltas.md` change must define one. The lab oracle uses
    `job_issued` evidence instead.
11. The harness artifacts may contain identity-bearing evidence. Artifact
    privacy rules are stated, but this is not a production privacy or
    mainnet-representativeness claim.
12. No universal statistical sample size is adopted. A numeric result without a
    declared estimator, planned N and stopping rule is descriptive only.
13. No mainnet, production payout, or economic-representativeness claim is made.

## 12. Traceability to the review recommendations

| Review recommendation | Where handled |
|---|---|
| Work units and exact target arithmetic | section 4 and `2026-11-05-lab-share-work-units.md` |
| Pow-limit profile validity and stepdown/stepup | section 4.1-4.3 and the work-unit decision |
| Checked SPEC 4.2/4.4 accounting intermediates | sections 4.2, 4.5 and the work-unit decision |
| Consensus compact decode | section 4.2 and the work-unit decision |
| Oracle `job_id -> target` source | section 7.2.1 and LP-3 |
| Window N-share versus network-block meaning | section 4 and `2026-11-05-window-network-block-work.md` |
| SV2 conformance before trusting the miner | section 1.3, section 5, LP-2 gate |
| Distinct evidence states, no absence-invalid | sections 7.2-7.3 |
| Snapshot/tail and observer isolation | sections 7.4-7.5 |
| Durable commands and conflict resolution | section 6 |
| Distinct topology entities | sections 3.2-3.3 |
| Sustained partitions with no bypass | section 8.3 |
| Process adoption/reap | section 8.1 |
| Per-pool Redis/Postgres and unique ports | section 8.2 |
| Capability, witness and halving gates | section 8.4 |
| Height-progress semantics | section 8.4 |
| Evidence versus chart decimation | section 7.6 |
| Time policy, warmup, estimators and stopping | section 9 |
| Independent oracle and actual delivery | sections 7.2.1, 7.3, LP-1/LP-3/LP-4 |
| UI shares the headless contract | section 7.5, LP-3 |
| Packets are bounded and reviewable | section 10 |

## 13. Merge notes for the merge session

This contract originally edited only its own new files. The later documentation
integration packet applies the SPEC frontmatter/body, WIRE, OPERATIONS and
type-32 delta splices; it still does not edit `docs/STATUS.md`,
`bins/spc-pool/src/config.rs`, `bins/spc-pool/src/main.rs`,
`config/*.example.toml`, `Cargo.toml`, `Cargo.lock` or `flake.nix`.

Shared-file status:

- `docs/SPEC.md`: the approved changelog fragments and the section edits are now
  applied by the documentation integration packet.
- `docs/REGTEST-LAB-STATUS.md`: record the L3 review outcome and approval.
- `docs/MERGE-QUEUE.md`: update the `lab-contracts` row after Fable review.
- `docs/OPERATIONS.md`: v2 profile, type-32 refusal, retention and the host-lab
  pointer are applied; the detailed lab guide is `docs/LAB-RUNTIME.md`.
- `docs/STATUS.md`: merge-session-owned; no edit here.
- The 2026-11-06 lab-runner/pool-latency records stay in
  `docs/decisions/` and `docs/changelog/`; they are not normative SPEC
  frontmatter entries.

The original proposal and independent review records are evidence inputs. This
packet does not edit them.

## Appendix A. LP-runtime host slice (2026-11-06)

This appendix records the integration that combined the approved LP-1, LP-2,
LP-3 and LP-4a inputs. It does not change the contracts above.

`spc-lab runtime` reads a versioned scenario and renders, rather than merges:

- rbitcoin node arguments and a private RPC credential/cookie;
- one `spc-pool` TOML per front (v2 exact profile, SV1 off, type-32 off, real
  `[api]`/`[jd]` listeners and the generated per-pool Noise authority);
- one strict `spc-cpu-miner` TOML per miner (connections still arrive through
  the approved v1 control protocol);
- one Redis config per pool and one PostgreSQL cluster/database per pool.

Every run keeps private identity, credential, authority and generated config
files at mode 0600 under a mode-0700 `private/` directory. Node, pool, miner,
Redis and PostgreSQL binaries are explicit paths; the runtime has no developer
home path fallback.

The public browser surface is the existing durable HTTP/SSE API plus
`GET /api/lab/v1/runs/<id>/catalog`. The catalog lists fronts/pools, declared
identities and their used state, and the live miner connection/worker specs.
The browser builds the existing typed `add_miner`, `switch_pool`,
`set_worker_budget`, `pause_workers` and `resume_workers` commands from it.
Closing the tab never stops a process.

Runtime evidence is collector-fed: the pool log is tailed into the LP-1
`spc_evidence` adapter from the file start; the node tip is observed through
authenticated RPC; PostgreSQL and Redis are read with authenticated clients;
the scanner runs `spc_scan::scan_block` over canonical blocks fetched by RPC.
`runtime_metrics` reports requested and measured rates separately and counts
submitted/ACK/rejected/unknown/block-qualified/stale/canonical observations.
A block-qualified share is not a canonical block.

### LP-4 handoff boundary

The host slice runs real processes in the host namespace. The stable interface
for the following namespace/address item is:

- owned paths: `<run_dir>/private`, `<run_dir>/logs`, and the per-run storage
  directories already recorded in `run-manifest.json`;
- node/pool/front/miner identities are already declared in the scenario and
  resource manifest, so placement and address publication do not need to parse
  generated configs;
- process control remains the approved `spc-cpu-miner-control` v1 JSONL over
  stdin/stdout; a namespace controller must preserve that transport rather than
  invent a second command schema;
- the current `Supervisor` refuses re-exec wrappers and relies on the run-token
  environment except for `redis`, which clears its environment. Namespace
  placement must provide a controller/exec boundary that preserves verifiable
  ownership (or extend the record with an equally explicit identity check);
- `spc-pool` does not spawn with a normal stdin control pipe, so a placement
  controller cannot assume one;
- PostgreSQL refuses to run as root inside a user namespace; the host slice was
  validated outside that constraint.

The host-only slice did not emulate namespace placement. The follow-on
namespace-runtime candidate implements that boundary as follows: the controller
stays in its host user/network namespaces; actors enter user+network namespaces
through a verified exec helper; published addresses precede rendering;
per-pool Redis runs in the storage namespace with authenticated published URLs;
PostgreSQL stays host-local over private Unix sockets; node/front control uses
bounded namespace helpers; miner JSONL stays on the original owned pipes.
Crash cleanup is exercised by an explicit controller-SIGKILL proof, not assumed
from a process abstraction. Convergence is an opt-in native
`phase.convergence` declaration with timeout/observation/resume bounds and a
required final pause/re-read; absent declarations receive no implicit hash.
Topology phase/convergence payloads are versioned types rather than opaque
strings; legacy records remain readable. Resource-bearing private manifests
remain mode 0600 and are not a public projection; helper credentials are passed
as paths, not argv secrets.

See [`LAB-RUNTIME.md`](LAB-RUNTIME.md) sections 10-12 and
[`reviews/2026-11-06-lab-network-runtime-implementation.md`](reviews/2026-11-06-lab-network-runtime-implementation.md)
for the actual scenario, Nix provenance, packet accounting, canonical views,
no-bypass counters, cleanup and remaining non-claims. Approval of the older
host snapshot does not approve this combined candidate.
