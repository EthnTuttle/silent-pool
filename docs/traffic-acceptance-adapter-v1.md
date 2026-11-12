# T1 acceptance adapter v1 - QA handoff

## Activation and API

Follow-up: early stage hooks and the separate before-sign version constructor
are specified in `docs/acceptance-version-rejection-v1.md`. The constructor is
not an additional arbitrary hook mode. Hook delay/loss semantics stay unchanged.

Set `traffic.lab_acceptance: true` and `expanded_access: true` before creating
the run. Both private `run-manifest.json.traffic_lab_acceptance` and each
`actor.json.lab_acceptance` must match. Default false disables all hooks.
Use the existing authenticated operator command endpoint. User claims cannot
submit the following command; it is not a new HTTP/RPC endpoint.

The normal LabCommand envelope contains a unique command id and this kind:

```json
{
  "type": "traffic_acceptance",
  "key": {
    "driver_id": "txd-0",
    "incarnation": "actual-current-incarnation",
    "command_id": "chosen-payment-command"
  },
  "control": {
    "action": "arm",
    "mode": "hold_before_intent",
    "timeout_ms": 120000
  }
}
```

Arm before issuing the chosen payment/injection. The target id is the durable
wallet command, not the hook-control command. Observe the private hook `signed`
record, then change the actual node chain. Release using the same key and
`{"action":"release","raw_sha256":"<exact signed checkpoint hash>"}`.
Cancel uses `{"action":"cancel"}`. Each action has its own durable command id;
retries reuse that id and payload. Hooks cannot be rearmed for the same target.
At most 64 targets per run are admitted. Timeout is 100..120000 ms.

For one real response loss, arm `mode: "lose_one_response"` instead. The
actual request still reaches the normal transport, and its actual result is
logged. One correlated response is withheld from the actor and represented as
transport Unknown. This does not manufacture a node error or an acceptance.

The original payment stays pending while held. Only this closed operator
control vocabulary bypasses its controller command mutex, to release/cancel.
No arbitrary command gains a concurrent mutation bypass. Actor acknowledgments
remain bounded (150 seconds for explicitly enabled acceptance runs). Expired
or cancelled targets stay inhibited; restart with a different incarnation
never silently releases a previously selected target.

Hook control transitions also hold the existing forwarding mutex. Cancel
therefore drains any already-dispatched call before acknowledging, and the
dispatch path checks cancellation/expiry again immediately before its attempt
record. A released hook is never a cached grant to bypass a later cancellation.

## Private evidence schema

All rows live in `artifact_paths.private_commands`, the existing 0600 strict
CommandLog. They have `record: traffic_audit`, global contiguous `seq`,
`version: 1`, `boot_id`, `monotonic_ms`, and a closed typed `event` object.
`command/traffic_audit.rs` is the exact serde schema; unknown fields, duplicate
JSON keys, invalid transitions and malformed complete records fail replay.

- `session`: run/driver/incarnation, actual controller and actor SHA256, opt-in.
- `forward_attempt`: incarnation, actor RPC request id, node id, method,
  send sequence, params SHA256, gate permit revision, private ForwardContext.
  Context contains durable command id (nullable for background reads), core
  journal sequence, last chain-observed revision, intent record sequence and
  signed raw-byte SHA256. Send contexts are required and raw hashes are checked.
  The session supplies the common run/driver identity without repeating it.
- `forward_result`: attempt's global seq, actual outcome class and serialized
  RpcOutcome SHA256, and `response_suppressed` (true for the explicit loss seam).
  An unsuppressed result is not proof of actor receipt/acknowledgment; a later
  pipe failure still leaves the actor's durable intent Unknown.
- `watermark`: incarnation, last attempt seq, completed count, next permit
  revision, closed flag and optional barrier command id. Each completed RPC
  gets a watermark; pause records closure before and after the actor barrier;
  resume records reopening. An unresolved attempt prevents completeness.
- `measured_boundary`: bootstrap command id, common tip hash/height. The row
  precedes actual AddConnection forwarding; it is not inferred from a config.
- `miner_attempt`: run/miner/incarnation, exact miner wire command id, SHA256
  of actual serialized JSONL bytes (including newline), controller/actor hashes.
  Its callback runs under the miner stdin mutex before any byte is written.
- `miner_result`: attempt seq and whether the correlated response succeeded.
- `hook`: exact key plus armed/signed/released/cancelled/response_discarded
  transition. Signed includes raw SHA256 and actor journal revision;
  response_discarded references the actual completed forwarding attempt.

Attempt `payload_sha256` hashes serde compact JSON **params**, excluding HTTP
auth. `raw_sha256` hashes decoded signed transaction bytes. `response_sha256`
hashes compact JSON of the actual typed RpcOutcome before acceptance injection
or selected-consumer shape handling. No raw requests, auth, cookie contents,
or node error messages are copied into these records.

## Completeness rules

Filter effect attempts (`sendrawtransaction`, and bootstrap `submitblock` when
relevant) separately from read-only observation calls, which continue during
pause. Bind each stream to the sealed binary source and its session. Verify
all sequence/attempt/result pairs and watermarks from the run's complete
command log. Compare pause/resume watermark permit revisions and actor history
3 intent/barrier sequences. A lost result, truncated evidence, missing session,
or unmatched process incarnation is inconclusive, never proof of zero sends.
Attempts include failed connections and broker dispatch errors, even when the
node never observed the request. No retries occur inside this forwarding call.

The evidence covers every RPC dispatched through the traffic actor gate and
every controller AddConnection write. Canonical-probe and read-only controller
preflight/completion RPCs keep their own source-bound evidence; their records
cannot substitute for the effect stream. Bootstrap's actual node sends pass
through the same actor gate. This is an audit trail, not a wallet/money ledger.

## Acceptance scope

Unit/socket/owned-process fixtures verify adapter behavior only. T3 still needs
real contextual-T2 funded execution, actual common-chain proof and an exact
tested definitive-rejection catalog row before claiming those acceptance cases.
The adapter does not change the canonical contract, node, fee/maturity policy,
hash budget, definitive-rejection classifier or core admission algorithms.
