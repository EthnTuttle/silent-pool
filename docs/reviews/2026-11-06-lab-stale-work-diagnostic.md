# Lab stale-work diagnostic: Redis blocking-reader serialization

Date: 2026-11-06
Scope: read-only diagnosis of `/tmp/opencode/LP-live-retarget-run6`
References: pool `c16032a7`, miner `5e27e918`, node `d2f37b91`

## Result

The dominant stale-work delay is in the pool, before the first SV2 publication:
a blocking Redis stream read and event publication share one physical Redis
client. Redis blocks that whole client during `XREADGROUP ... BLOCK 1000`, so
publishing an unrelated slice/block event on the same client can wait for the
remainder of the read timeout. The front executes such publication inline both
on the found-block path and before `Sv2Server::publish`.

A fresh-Redis counterexample reproduced the mechanism without mining or a
build. With a 600 ms blocking read, an unrelated same-client publish sent at
50 ms returned after another **641 ms**. A separate control client answered in
0.49 ms. With an isolated publisher and the same blocking reader, publication
returned in **0.58 ms** while the reader remained blocked for 603 ms. A second
iteration gave 641/0.58 ms for those publish operations.

This explains the approximately one-second repeated stages and the failure of
the configured 250 ms refresh cadence. It is not a claim that every long tail
comes from one Redis call. A separate SV2 outcome wait adds reconnect and
multi-second fan-out tails, addressed below.

**Recommended bounded implementation:** give the front event reader its own
Redis connection, and allow per-channel SV2 job publication to proceed while a
front share outcome is pending. Do not change mining, validation, per-job
credit, job activation, or event durability ordering. This report is a proposal
for the pool/runtime owner, not an approval or a validated hashrate-response
result.

## Boundaries and evidence integrity

No frozen source, recorded run artifact, or active runtime was changed. The
only repository file written is this report; analysis scripts and outputs are
under `/tmp/opencode/LP-stale-*`. No Git mutation, miner run, or Rust build was
performed. The only lab service process launched was a fresh, private Redis
instance for the minimal counterexample; it exited cleanly.

The primary analyzer and outputs are:

- `/tmp/opencode/LP-stale-analyze.py`
- `/tmp/opencode/LP-stale-analysis.json`
- `/tmp/opencode/LP-stale-analyze-final.stdout`
- `/tmp/opencode/LP-stale-redis-probe.py`
- `/tmp/opencode/LP-stale-redis-probe/result.json`

The Redis probe uses only Python's standard library and the exact node-run
Redis 8.10.3 binary. It starts a new datadir and random loopback port, sends
real `XREADGROUP` and `XADD` commands, checks the completion order, and cleans
up its child with start-process ownership.

The complete miner cursor stream has 471,256 contiguous event IDs and final
`dropped_before = 0`. Thus event ordering and event loss are not explanations
for this run. `submissions_evicted` events are bounded local correlation-history
transitions, not dropped miners, silently accepted work, or event-sink loss.

### Timestamp conventions

All compared wall-clock timestamps are from the same host:

- node `tip:` and `node-api.jsonl.ts` are ISO wall timestamps;
- pool log timestamps and `job_issued.issued_at` are wall timestamps;
- miner `occurred_at_wall_ms` joins those wall timestamps;
- `occurred_at_mono_ms` is only a miner-process-relative duration and was not
  compared directly with node or pool monotonic values.

Canonical transitions use node `tip:` timestamps, not integer-second header
`ntime`. Hash-qualified candidates are joined by full header hash, not by their
approximately ten-second spacing.

For this node version, `crates/rbitcoin-rpc/src/server.rs:1203-1213` computes
`wall_ms = t0.elapsed()` around `dispatch_tip_connect`, and
`rbitcoin-log/src/api_log.rs:81-99` timestamps the log afterward. Thus
`wall_ms` is **server dispatch duration rounded down to milliseconds**. It is
not total client RPC latency. `ts - wall_ms` is only an approximate upper bound
on dispatch start. A zero GBT value proves dispatch below 1 ms, not absence of
client/transport delay. The 0/-1 ms differences between node API and `tip:` are
rounding effects, not reverse causality.

## Measured stage attribution

The analysis joins the first job issued for each canonical parent to miner
activation of that exact job ID. Later same-tip distribution jobs are excluded
from this fan-out measurement.

| Stage | p50 | p90 | p95 | p99/max | Interpretation |
|---|---:|---:|---:|---:|---|
| canonical `tip:` to first following GBT (72) | 2,585 ms | 7,946 ms | 9,942 ms | 33,652 ms max | Dominant pre-publication delay, despite inline found-block refresh |
| first post-tip GBT to first new-tip job issue (72) | 1,019 ms | 1,804 ms | 2,023 ms | 2,091 ms max | Second repeated Redis-publication interval candidate, inside `refresh_and_publish` |
| canonical tip to first activation of that job (72) | 3,591 ms | 8,978 ms | 11,011 ms | 35,676 ms max | Work remains on the previous job for seconds |
| exact first-job issue to first activation (72) | 4 ms | 13 ms | 15 ms | 41 ms max | Miner SV2 parsing/switch is normally fast |
| first-job issue to last observed issue (72) | 0 ms | 1,025 ms | 12,226 ms | 35,348 ms max | Secondary per-channel/outcome/reconnect tail |
| first-job issue to last observed activation (72) | 16 ms | 1,031 ms | 12,267 ms | 35,389 ms max | Delay is upstream of the miner, not proof of miner slowness |
| winning candidate send to `submitblock` dispatch end (71 mined-to-anchored heights) | 151 ms | 1,470 ms | 2,390 ms | 13,544 ms max | Usually much shorter than tip-to-next-work, with important outliers |
| server `submitblock` dispatch (73) | 40 ms | 499 ms | 2,120 ms | 11,599 ms max | Real consensus work contributes to tails; it must not be bypassed |
| server pool-shaped GBT dispatch (369) | 0 ms | 0 ms | 0 ms | 0 ms max | No evidence that node template construction dominates |

`refresh_interval_ms = 250`. Across the 761.933 seconds from the initial node
`tip:` to height 73 there were 369 pool-shaped GBT calls (plus one differently
parameterized preflight call): mean inter-completion 2.086 s, p50 1.003 s,
p95 8.667 s, maximum 33.816 s. A 250 ms timer would have about
3,048 opportunities over that wall interval. `MissedTickBehavior::Skip`
prevents queued ticks, but does not explain why each individual refresh/publication
iteration takes seconds; the Redis client structure does.

All `getblockcount` activity is harness traffic and is not mistaken for pool
GBT polling. Node and miner clock fields were not mixed.

### Two concrete joins

Height 2 is an ordinary example:

```text
06:14:29.793  winning miner submits the real candidate
06:14:29.854  node submitblock dispatch log; canonical tip at .853
06:14:30.757  first pool GBT after that tip       +904 ms
06:14:31.765  first exact next-tip job issue      +1,008 ms after GBT
06:14:31.771  miner activates                    +6 ms after issue
```

Nearly all 1.918 seconds from canonical tip to next activation is before the
job is sent. It is not batch hashing time or `SetNewPrevHash` interpretation.

The height-42 next-job fan-out illustrates the secondary wait:

```text
06:21:08.466  canonical parent tip
06:21:19.436  first exact job issue
06:21:19.477  first activation                  +41 ms
06:21:31.141  last issue of that same job       +11.705 s after first issue
06:21:31.183  last activation                   +42 ms after that issue
```

The last activation again tracks the late issue, not a slow miner. The same
window contains `share_outcome_unknown` events, consistent with the inline
outcome-wait/timeout path described below.

Height 31 is a long counterexample to any fixed-ten-second explanation: after
the previous block's submit completed at 06:19:09.847, the next GBT was at
06:19:43.498 and first next-job activation at 06:19:45.522. Multiple paired
outcome timeouts and reconnects occur in that interval. A single fixed batch
or poll-duration model cannot explain these actual event sequences.

## Stale and canonical outcomes

There are 163,809 submitted events and 120,052 `stale-share` replies (73%).
The transport state is not equivalent to accounting or canonical confirmation:

- Pool `accounted` and miner `ingress_ack` transport replies both total 4,577.
  Those accepted real shares retain their exact job credit; an ACK is not a
  block.
- All 28 pool `share_outcome_unknown` events remain an explicit unknown-state
  category, not a silent rejection or synthesized acceptance. They alone do not
  prove whether a queued share was subsequently accounted. There are 27
  peer-close disconnects and reconnect cycles in the connection stream.
- Miner transport/pending history also contains unresolved/evicted shares;
  these were not counted as credited work or as canonical failure evidence.
  The analyzer retains `late-ingress_ack` and `late-stale-share` separately.

For the 2,529 locally block-qualified real hashes:

| Outcome | Count | Canonical node block |
|---|---:|---:|
| `ingress_ack` resolved in tracked history | 72 | 72 |
| `ingress_ack` already evicted from local tracked history | 1 | 1 |
| `stale-share` resolved in tracked history | 1,805 | 0 |
| `stale-share` already evicted from local tracked history | 41 | 0 |
| transport remains unknown | 610 | 0 |
| **Total** | **2,529** | **73** |

"Already evicted" is a local correlation-history state, not a different pool
verdict or a synthetic event name. Each of the 73 ACKed block-qualified
candidates joins an exact node `submitblock` and canonical `tip:`. The other
2,456 were never submitted to the node. The evidence file is anchored at height
72; the chain reaches height 73 when the resumed workers produce one more block
before cleanup. There is no inference from the approximately ten-second header
cadence.

### The submissions were not already obsolete at the miner

At each `submitted` event, the replayed local job activation identifies the
assignment the worker/connection actually used:

- only 200 of all submissions (0.12%) were already obsolete relative to that
  local activation stream when sent;
- **all 2,529 block-qualified candidates were sent while their job was still
  the locally active assignment**.

Worker batch scheduling and retained obsolete found-queue entries are
therefore not the main explanation. At approximately 218 kH/s, a
1,024-real-hash batch is about 4.7 ms, not multi-second stale work.

The pool-publication join gives a complementary result. Of the 120,052 stale
replies, 118,650 (98.8%) named the job still shown as published at the exact
miner send timestamp; it was superseded before the reply. At reply time,
51,331 stale replies follow a same-prev-hash distribution/job roll and 68,703
follow a new-prev-hash roll; 18 are front/publication races where the same ID
is still the latest published job. Pool receive-entry time is not logged, so
this does not pretend to locate the transition between socket receive and
front outcome more precisely.

There were 195 unique job IDs but only 74 unique previous hashes, hence 121
same-prev publication changes. They matter, but changing job validity or
crediting them would violate the required per-job semantics. The fix is to
reduce unnecessary delay, not to accept old work under a newer job.

## Root mechanism in the frozen code

### 1. One physical Redis client combines blocking reads and publication

`crates/spc-store/src/redis.rs`:

- `Streams` holds one `MultiplexedConnection` (`18-28`). Clones use that same
  underlying client; clones are not independent TCP connections.
- `read_group` clones that connection and issues `XREADGROUP ... BLOCK 1000`
  (`56-76`).
- `add` clones the same connection for `XADD` (`30-34`).

`bins/spc-pool/src/main.rs` connects once for the front (`249`), passes a clone
to `spawn_front_event_reader` (`367-372`), and also uses the same `streams` for
`publish_events` (`850-873`) and acknowledgements. The reader continuously
re-enters its one-second blocking read.

Redis blocks the client associated with `XREADGROUP BLOCK`; multiplexing does
not turn one client into independent Redis server clients. An `XADD` to
`spc:slices` or `spc:blocks` does not wake a read on `spc:front:events`.

The standalone probe demonstrates this exact sequence on a real Redis server:

```text
same client:   XREADGROUP BLOCK 600
same client:         XADD spc:slices     (sent 50 ms later)
other client:  PING                      (0.49 ms, server is responsive)
same client:   XREADGROUP timeout + queued XADD result at ~691 ms

isolated client A: XREADGROUP BLOCK 600
isolated client B: XADD spc:slices       (sent at 50 ms, result in 0.58 ms)
isolated client A: still returns only at its ~603 ms read timeout
```

### 2. That wait is placed directly on both required job paths

In `main.rs`:

- periodic refresh calls `refresh_and_publish` inline (`391-403`);
- a found block performs `submit_block`, then `publish_events`, then
  `refresh_and_publish` inline (`421-439`);
- `refresh_and_publish` calls `front.refresh`, but then publishes pending
  slice/block events **before** `sv2.publish` (`946-977`, especially `967` and
  `975`).

A new chain job can consequently pay a same-client wait after block submission
and another before SV2 publication. Ordinary slice sealing also invokes event
publication and repeatedly stalls front outcomes and template cadence. This
matches the approximately one-second `GBT -> issue` component without blaming
GBT itself.

The recommendation does **not** move events after new job publication. Slice
and block durability/recovery ordering is outside this packet and must not be
weakened to make the graph faster.

### 3. Secondary per-connection wait

In `sv2_server.rs`, a valid current share:

1. registers an outcome (`459`),
2. awaits bounded entry into the front share queue (`460`),
3. awaits the front outcome with a ten-second timeout (`465-477`),
4. writes success/error (`479-492`).

Only after those steps can that connection's outer `select!` revisit
`jobs.recv()` (`497-505`). Therefore an unresolved front outcome delays
`NewExtendedMiningJob`/`SetNewPrevHash` for that channel. This explains the
long issue outliers and some reconnect tails, though not the dominant median
pre-first-issue delay. The 28 ten-second unknown events are consistent with
this common-path head-of-line behavior.

## Bounded implementation proposal

Ownership: **LP-1/pool runtime owner**, coordinated with the active lab
runtime. No LP-2 miner change is requested. No new Git commit or runtime
installation is implied by this report.

### Change A (primary): dedicated Redis reader connection

Exact files/functions:

- `bins/spc-pool/src/main.rs`: front setup and
  `spawn_front_event_reader` (`367-372`, `738-800`).
- `crates/spc-store/src/redis.rs`: retain existing command semantics; no
  change is required unless the owner chooses to encapsulate connection
  construction there.

Preferred minimal design: have `spawn_front_event_reader` accept the Redis URL
and call `Streams::connect` inside its own task, instead of accepting a clone
of the front's publishing `Streams`. A separate `Streams::connect` creates an
independent Redis client. Return/log connection errors through the same path
currently used for reader initialization failures.

Equivalently, main may explicitly create a second `Streams` and pass only that
instance to the reader. The first form makes accidental future connection
sharing less likely.

Keep unchanged:

- the group and consumer names;
- pending replay and ACK behavior;
- exact `XADD` payloads and stream names;
- bounded queues and 1,024-entry front event channel;
- slice-before-job publication ordering and failure behavior;
- all accounting and validation.

Do not substitute `BLOCK 0`, a faster polling spin, event dropping, an
unbounded retry queue, or a shorter cosmetic `BLOCK` value. The issue is
client ownership, not the duration constant. Payout and Ark also call the same
blocking helper; review them separately rather than expanding this packet.

### Change B (tail): poll jobs while awaiting one share outcome

Exact file/function: `bins/spc-pool/src/sv2_server.rs`, accepted-share outcome
wait around `459-492` and the existing `jobs.recv()` branch at `497-505`.

Use a small nested `tokio::select!` around the existing pinned ten-second
outcome future and `jobs.recv()`. If a job arrives first, call the existing
`send_job` using the same channel state and writer, then resume waiting for
that share outcome. Once the outcome arrives, emit exactly the same
success/error response as today. Existing `abandon`, unknown-outcome evidence,
timeout, lag, and connection-error behavior remains unchanged.

This deliberately does not spawn one writer task per share, reorder responses,
add an unbounded queue, or read additional submissions while the one accepted
outcome is pending. It also does not promise preemption of physical socket
backpressure: an actual frame write may legitimately await. It removes only
the artificial outcome-await barrier before job delivery.

Keep `send_job` (`604-684`) exactly intact: optional target update, future-job
marker, prev-hash/bits activation check, and `SetNewPrevHash` ordering are
required SV2 behavior.

### Changes explicitly rejected for this packet

- Do not drain/discard old queued shares or synthesize their verdicts.
- Do not credit a share under a later job or accept stale prev-hash work.
- Do not lower share difficulty, skip hash/header revalidation, or skip
  `submitblock` consensus processing.
- Do not make ephemeral per-share write tasks or an unbounded outcome queue.
- Do not merely make the main `select!` biased toward a continuously ready
  ticker; that can starve outcomes when refresh is already late and leaves the
  Redis serialization intact.
- Do not change the approved miner publication ordering or worker pacing.
- Do not claim hashrate-budget validation from an improvement in charts alone.

## Minimal regression and live validation plan

### A. Real Redis regression

Add a gated integration regression alongside the existing real-Redis tests
(for example `crates/spc-store/tests/redis.rs`) and a front wiring test owned
by `main.rs`:

1. Create the front reader using the production constructor and keep a
   separate publisher.
2. Establish the actual front group and hold the reader in `BLOCK 1000` with
   no new front event.
3. Publish a slice-shaped event after the read is known to be pending.
4. Require bounded, prompt `XADD` completion independent of the read timeout;
   require the reader to remain blocked until a front event or its normal
   timeout arrives.
5. Publish one front event, then assert normal decode/apply/ACK and pending
   replay behavior still works with the same group/consumer and exact payload.

Also retain the negative topology check: using one raw client for both
operations must reproduce the delayed completion, proving the regression is
not passing because Redis behavior or the test setup changed. Use monotonic
durations with generous scheduling tolerance, not a 10 s sleep.

### B. SV2 activation-before-outcome regression

Add to the existing native SV2 integration tests:

1. Open a real Noise/SV2 channel and submit a valid real share.
2. Wait until the test front receives it, but deliberately do not resolve its
   outcome.
3. Publish a different-prev-hash job.
4. Require receipt of that exact `NewExtendedMiningJob` followed by
   `SetNewPrevHash` while the old outcome is still unresolved. Assert job ID,
   prev hash, bits, and future activation fields.
5. Resolve the old share with accepted work `d = 1024`; require one success for
   its original sequence and exactly that work sum, with no fabricated second
   success.
6. Publish/queue additional old and new work under load. Require every hash to
   be processed at most once and every old-job submission to retain its exact
   current-job validation outcome. Assert no `difficulty-too-low` relaxation,
   no silent queue drop, and unchanged 10-second unknown handling.

This regression must fail on the current code because step 4 cannot run until
step 5, and pass with Change B. It proves preemption of outcome wait without
requiring wall-clock mining.

### C. Combined load and budget replay

After owner integration, rerun a fresh live cluster with the run6 share policy,
1,024-real-hash batches, two identities, and actual retarget node. Record all
stage timestamps and independently recompute the node retarget vector, per-job
`m_issue`, slice credit sums, finding-slice exclusion, and payouts.

Gate runtime acceptance on behavior, not a target stale percentage invented
after the run:

- no queued same-client `XADD` waits and no unexplained `share_outcome_unknown`;
- the exact new tip reaches both connected miners before obsolete queued
  outcomes can delay it, with future-job/prev-hash activation intact;
- every block-qualified hash still resolves through exact transport, node
  submission, and canonical confirmation categories;
- every accepted share has one real header and exact job-specific `m_issue` and
  credit appears in at most one matching slice;
- no synthetic attempts, candidate pacing, dropped shares, or fake credit;
- miner cursor streams remain strictly ordered with no unaccounted gaps.

For meaningful budget experiments, first establish an unsaturated CPU range in
the new runtime. Then compare requested budgets within that range and report
`attempted / active_elapsed` separately from wall block time. The run6 high
phase already reports saturation, so 800k requested H/s must not be presented
as an attained budget. A successful retarget arithmetic check and a shorter
block period do not independently prove causal hashrate control.

## Bottom line

The measured duty-cycle failure is principally a pool publication-path
problem, not a miner hashing/pacing or SV2 activation problem. The concrete
one-second mechanism is a shared Redis client used simultaneously for a
one-second blocking read and required slice/block publication. Isolating that
reader is the smallest evidence-matched fix with no accounting or protocol
semantic change. A narrowly scoped nested SV2 select removes the remaining
outcome-before-job head-of-line barrier and supplies the required deterministic
preemption regression.
