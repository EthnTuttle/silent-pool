# spc-cpu-miner process and lab protocol (version 1)

Date: 2026-11-05

This is the stable LP-2 process contract for LP-3. It is a local JSONL control
and observation protocol, not a supervisor, durable run log, accounting API or
SV1 translator. The runner must journal commands before starting external
effects. This process keeps idempotency outcomes only for its own lifetime.

## 1. Invocation and streams

```sh
spc-cpu-miner --config /path/to/miner.toml [--check-config]
```

`--check-config` performs strict TOML and semantic validation, then exits before
starting network connections or hash workers. Normal operation reads commands
from stdin and writes one UTF-8 JSON object plus `\n` per stdout line. Empty
input lines are ignored. Cargo, Nix and diagnostics go to stderr; protocol
stdout contains no log text. EOF, Ctrl-C, and `stop` stop all connections,
cancel jobs, stop hash threads and close the process.

The configuration starts any number of connections. Each connection has one
Noise session, one identity and one extended channel. Each worker belongs to
one connection; worker IDs are unique across the process. `connections = []` is
valid and permits a control-only process.

```toml
config_version = 1
miner_id = "miner-1"
event_queue_capacity = 4096

[[connections]]
id = "alice-a"
endpoint = "127.0.0.1:34254"
identity = "tsp1q..."
authority_public_key = "9auqWEzQDVyd2oe1JVGFLMLHZtCo2FFqZwtKA5gd9xbuEu7PH72"
min_extranonce_size = 4
reconnect = { enabled = true, initial_backoff_ms = 100, max_backoff_ms = 5000, handshake_timeout_ms = 10000 }
workers = [
  { id = "alice-0", budget = { hashes_per_second = 100000, total_attempts = 1000000 }, ntime_policy = "hold", max_ntime_roll_seconds = 600, batch_size = 256 },
]
```

Unknown TOML fields are rejected. The authority accepts 32-byte x-only hex,
33-byte compressed SEC hex, or SV2 key-utils base58check. A matching public key
is mandatory on every Noise handshake; there is no unauthenticated fallback.
The public identity is opaque to the miner and validated by the pool.
`min_extranonce_size` is 4..=32 and at most 256 workers are allowed per
connection. Batch size is 1..=4096. A budget requires a positive
`hashes_per_second`, a positive `total_attempts`, or both.

See `fixtures/miner.toml`, `fixtures/two-pools.toml` and `fixtures/control.jsonl`.
The example identities are placeholders for the runner's actual identities.

## 2. Command envelope

Every input line has exactly these top-level fields. Unknown envelope fields are
rejected. `operation` selects the payload shape.

```json
{
  "protocol": "spc-cpu-miner-control",
  "version": 1,
  "command_id": "cmd-unique-in-this-process",
  "operation": "snapshot",
  "payload": {}
}
```

`command_id` is a non-empty string. `protocol` and `version` must match exactly.
Rust command, miner, connection, identity, worker and job identifiers use
transparent nominal newtypes; JSON values are unchanged. The operations are:

| `operation` | Required payload |
|---|---|
| `add_connection` | `{ "connection": ConnectionConfig }`, where `ConnectionConfig` has the connection fields shown above, except `min_extranonce_size` is optional with default 4. |
| `remove_connection` | `{ "connection_id": "..." }` |
| `set_worker_budget` | `{ "worker_id": "...", "budget": { "hashes_per_second": 50000, "total_attempts": null } }` |
| `pause_workers` | `{ "worker_ids": ["w0", "w1"] }`; the list must not be empty. Pause stops new batches within the configured bounded batch. Network ingress and replies continue. |
| `resume_workers` | Same shape as pause. Resume advances the saved cursor; it does not restart the previous nonce. |
| `switch_pool` | `{ "connection_id": "...", "endpoint": "host:port", "identity": "...", "authority_public_key": "...", "min_extranonce_size": 4 }`; `min_extranonce_size` may be null to retain it. Worker definitions and IDs are retained. |
| `snapshot` | `{}` |
| `stop` | `{}` |

Payload objects are not `deny_unknown_fields` in this pre-release version 1:
additional keys inside a recognized operation's payload are ignored. Do not
interpret that extension tolerance as a strict payload-schema guarantee. Unknown
top-level envelope fields are still rejected.

`switch_pool` returns only after the old connection has been asked to close and
its task has ended. It then performs a new Noise handshake, setup, version
negotiation and channel registration. It never rewrites an identity in a live
channel. New addition returns `running` because startup continues asynchronously.
Unknown IDs, duplicate entities and an invalid list fail before changing state.
`stop` wins all later operations because processing is in input order.

### 2.1 Results and idempotency

Every command produces:

```json
{
  "protocol": "spc-cpu-miner-control",
  "version": 1,
  "kind": "command_result",
  "command_id": "cmd-unique-in-this-process",
  "outcome": "succeeded",
  "result": {}
}
```

The outcomes currently emitted are `running`, `succeeded`, `failed`, and
`rejected_precondition`. `error` is omitted on success and otherwise has:

```json
{ "code": "unknown_worker", "message": "unknown worker w0" }
```

Notable error codes are `invalid_json`, `invalid_command`,
`command_id_conflict`, `connection_exists`, `worker_exists`,
`unknown_connection`, `unknown_worker`, `connection_start_failed`,
`switch_start_failed`, and `miner_stopping`.

A retry of the same command ID with the same protocol, version, operation and
payload replays the exact stored result without another effect. The operation
must not be inferred from post-command state; for example, replaying an old add
after its connection was removed still replays the historical `running` result
and does not re-add it. The same ID with a different payload returns
`rejected_precondition` with `command_id_conflict` and does not replace the
original record. Invalid JSON has no idempotency guarantee; when possible its
`command_id` is still extracted for the error response.

The in-memory ledger is deliberately not durable. The runner's durable
`LabCommand` log is authoritative across process/restart ownership boundaries.

## 3. Event envelope

Asynchronous observations are separate stdout objects:

```json
{
  "protocol": "spc-cpu-miner-events",
  "version": 1,
  "kind": "event",
  "source": "process",
  "event_id": 42,
  "source_cursor": 42,
  "miner_id": "miner-1",
  "occurred_at_wall_ms": 1762390000000,
  "occurred_at_mono_ms": 1234,
  "dropped_before": 0,
  "event": { "name": "connection_state", "payload": {} }
}
```

`event_id` and `source_cursor` are the same integer. A short shared publication
mutex serializes ID assignment, the cumulative drop-count read and `try_send`; it
is not a claim of wait-free producer execution. Stdout and consumer I/O never run
under that mutex. Event IDs strictly increase in stdout order. A gap of `k`
between adjacent delivered events means exactly `k` publication attempts were
lost between them. `dropped_before` is the cumulative drop count at publication
and is non-decreasing in stream order. Queue-full publication still drops rather
than waiting for the consumer. Snapshot `events_dropped_total` also exposes
trailing loss after the last delivered event. Saturation is not used as evidence
of acceptance.

The `event.name` values and payload contents are:

- `connection_state`: `connection_id`, nullable `session_slot`, `state`, nullable
  `detail`. States include `connecting`, `noise_authenticated`,
  `channel_open`, `disconnected`,
  `reconnect_wait`, `stopped`, and `failed`.
- `target_updated`: `connection_id`, `target_generation`, `share_target_le`.
- `job_activated`: `connection_id`, `job`, using the active job shape below.
- `jobs_invalidated`: `connection_id`, `job_ids` array, `reason`.
- `submitted`: `share`, using the submission shape below.
- `ingress_ack`: `connection_id`, `sequence_number`, `accepted_count`,
  `new_shares_sum`. Multiple submissions can be outstanding. A batched success
  resolves the named last sequence itself. For `new_submits_accepted_count >= 2`,
  the number of no-error pending predecessors below it must equal `count - 1`;
  only then is the fully identified oldest-prefix-plus-last set resolved. A
  mismatch is a protocol error and changes no entries. Later, unprocessed
  submissions remain pending. `new_submits_accepted_count == 1` resolves exactly
  its named last sequence. Empty or exactly repeated responses are no-ops and do
  not resolve another share. An inconsistent count or never-issued sequence is a
  protocol error, not a guessed assignment. `new_shares_sum` stays
  a batch-level field and is never divided or relabeled across jobs. The
  `ingress_accepted_reported` counter counts only newly resolved shares, so
  repeated ACKs cannot inflate it.
- `ingress_rejected`: `connection_id`, `sequence_number`, `error_code`.
- `submissions_unknown`: `connection_id`, `sequence_numbers`, `reason`, emitted
  for pending shares left by the `outstanding-cap-evicted` or
  `known-history-cap-evicted` bounds and for pending shares left when transport
  ends before their ingress verdicts. Already ACKed/rejected shares never become
  `transport_unknown`.
- `submissions_evicted`: `connection_id`, `sequence_numbers`, `reason`, records
  resolved shares evicted by the bounded known-history window or pending shares
  trimmed from that presentation window while still tracked as outstanding. A
  presentation-history trim is not loss and does not make them transport unknown.
  Only outstanding-cap eviction or a transport ending without a verdict can
  produce `submissions_unknown`.
- `late_verdict`: `connection_id`, `sequence_numbers`, `outcome`
  (`ingress_ack` or `ingress_rejected`). Outstanding-cap eviction retains a
  bounded retired-ID set, so a late verdict for a previously issued sequence is
  reported explicitly without an invented per-share success or session teardown.
  It does not recreate per-share history or increment unique-resolution credit.
  A never-issued sequence remains a protocol error.
- `block_qualified`: `connection_id`, `sequence_number`, `block_hash`, and
  `candidate_submission: "unknown"`. This is local PoW classification only.
- `worker_search_exhausted`: `connection_id`, `worker_id`, `job_id`, `ntime`,
  `reason`.

`fixtures/events.jsonl` supplies schema-only examples. It is not mining or
acceptance evidence.

## 4. Job and submission JSON

An active `job` has `job_id`, `target_generation`, `version`,
`version_rolling_allowed`, `prev_hash_le`, `min_ntime`, `nbits`,
`share_target_le`, `network_target_le`, `merkle_path_le`,
`coinbase_tx_prefix`, `coinbase_tx_suffix`, and `extranonce_prefix`. All byte
strings are lower-case hex. `_le` hashes and targets are the exact 32 bytes in
protocol/header order, not display-reversed hashes. Merkle siblings stay
**deepest first**. Prefix/suffix/extranonce hex is not byte-reversed.

A `share` has:

- correlation: `connection_id`, `worker_id`, `session_slot`, `sequence_number`,
  `job_id`, `target_generation`;
- attempted header inputs: `nonce`, `ntime`, `version`, local `extranonce` hex;
- independent verification: the actual 80-byte `header` hex, conventional
  display `block_hash`, header-order `hash_le`, `share_target_le`, and
  `network_target_le`;
- `block_qualified` (hash is inclusively below or equal to the exact job network
  target), `status`, `accounted`, `candidate_submission`, and optional
  `error_code`.

`status` is `submitted`, `ingress_ack`, `ingress_rejected`, or
`transport_unknown`. Submission results are keyed by
`(session_slot, sequence_number)` to prevent reconnect aliasing. `accounted` is
always `unknown` in this process. Even `SubmitShares.Success` means an ingress
response, not independently observed accounting. `candidate_submission` is
`not_applicable` for a non-block share and `unknown` for a block-qualified share.
Only an independent pool/block-path observer can change the latter meaning.

## 5. Snapshot

`Operation::Snapshot` returns a result with `snapshot_version: 1`, `miner_id`,
`occurred_at_mono_ms`, `events_dropped_total`, `stopping`, `connections`, and
`workers`. Connection records include endpoint, identity, lifecycle state,
session/channel/group IDs, negotiated size, effective channel target, target
generation, nullable `active_job`, `future_jobs`, actual counters and the newest
256 submissions. At most 1024 shares may await ingress responses; overflow is
explicit eviction to `transport_unknown`, never silent discarding. Future job
records contain job ID, version and the exact job byte/prefix fields; future jobs
do not have a frozen target. `block_qualified_submissions` counts submissions,
not distinct blocks or node candidates.

Each worker record has:

```json
{
  "id": "alice-0",
  "connection_id": "alice-a",
  "paused": false,
  "requested": { "hashes_per_second": 100000, "total_attempts": 1000000 },
  "attempted": "123456",
  "elapsed_ms": 1500,
  "achieved_hps": 99700,
  "budget_hps": 100000,
  "saturated": false,
  "queue_depth": 0,
  "active_job_id": 77,
  "stale_cancelled": 0
}
```

`attempted` is a decimal string so no JavaScript `Number` rounding occurs.
Elapsed and achieved rate cover active work time, including real pacing sleeps,
not paused/no-job time. Every attempted nonce counts once. A total-attempt
budget never resets on pause, job replacement or reconnect within one worker
lifetime. `switch_pool` creates a new worker lifetime, so attempted/elapsed start
from zero for the new identity. A share is sent before its worker's pacing wait.
Rate-budget updates start a new rate epoch at the current attempt/elapsed ratio;
they cannot turn old history into a multi-second sleep. Pacing waits are condvar
waits that stop, budget changes, pause and job replacement can interrupt.
Lifetime `saturated` means two consecutive unpaced 250 ms samples below 95% of
requested rate with work available and no pacing sleep; pause, no job, and
total-budget exhaustion are not saturation. Lifetime achieved HPS is an honest
measurement, not a promise that a newly lowered HPS will already equal the new
budget on the very next snapshot.

## 6. Work and protocol guarantees

- SHA256d runs on dedicated bounded OS hash threads, separate from async I/O.
  Extended-channel coinbase construction is exactly
  `prefix || upstream_extranonce_prefix || local_extranonce || suffix`.
  Header bytes are actual 80 bytes; no success path fabricates work.
- Local extranonce encodes reconnect-unique session (2 bytes), worker index
  (1 byte), and a roll counter (remaining 1..29 bytes). Nonces also use disjoint
  worker partitions. Exhaustion rolls a fresh counter or stops; it never wraps
  into an old header. Reduced-domain tests do not claim fast full-domain
  exhaustion. Reconnect discards protocol jobs and increments session state.
- `hold` uses the job's `min_ntime`. `roll` uses elapsed time, capped by
  `max_ntime_roll_seconds` (maximum 7200); no found block is held for pacing.
  The pool's job minimum is authoritative for same-tip jobs, including advances
  beyond the previous job minimum; no local elapsed-time ceiling is imposed.
  Sequence numbers are monotonic per session, and wrap fails closed as
  `submit sequence exhausted` rather than recycling IDs.
- Targets are actual channel/job values interpreted as inclusive LE256 values.
  `nbits` supplies the network target through consensus compact decoding,
  including overflow/sign/zero rejection. There is no difficulty estimate or
  local share-credit calculation.
- A future job takes the target effective at matching `SetNewPrevHash`
  activation, with that prevhash and nbits. An active job keeps its captured
  target after `SetTarget`. New immediate same-tip jobs use the latest
  prevhash/nbits and current target. Unknown activation, active-before-prevhash,
  duplicate job IDs and wrong channel IDs fail closed. `job_activated` and worker
  assignments are emitted only for `changes.activated`, not for a future-job
  receipt or `SetTarget` that leaves the active job unchanged. Worker stale
  cancellation counts track actual replacement/invalidation.
- No optional extension-specific messages are used or requested. Setup, version
  negotiation and open request IDs are checked. Submit acknowledgements and
  errors are independently correlated, not awaited inline before further mining.

This packet does not claim L1/LP1 acceptance evidence, a node-valid candidate,
front accounting, payout or canonical inclusion from its own events.
