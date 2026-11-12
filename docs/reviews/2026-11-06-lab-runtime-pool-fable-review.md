# Lab runtime pool delta - Fable approval gate review

Date: 2026-11-06
Reviewer: Fable (normative + key/security gate), subagent
Worktree: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-runtime`
Base: `b0ab4ca34fda07c61a6335d05f598eaacc69678c`
Manifest: `/tmp/opencode/lab-runtime-review-manifest.json` (172 files, with
source/change annotations), aggregate SHA256
`c9dfa986c2f6ad0daca44912906a0a2ec94129c25d465eee246f19a43ccd543d`
Scope: the pool changes relative to the frozen approved LP-1 snapshot
`c16032a78902e88b0157efed6780bc8b424e942b1c3bf46893d8fb09ae4708ee`
(`lab-share-integration`). Runtime, config, UI, collectors, Cargo/Nix are
reviewed separately by another Fable instance and are not re-reviewed here.

## Verdict: APPROVE (bounded pool delta)

Fix A (independent Redis reader connection) and Fix B (job delivery while one
share outcome is pending) are implemented as the diagnostic specified, with
no change to validation, per-job credit, activation ordering, the ten-second
unknown-outcome policy or event durability ordering. The new regressions are
real (wire peer; real Redis) and pass independently here. Non-blocking
findings are listed in section 5. The approval binds to the exact aggregate
hash above; it does not inherit from, or extend to, the historical L1/LP-1
approvals, and it does not approve the runtime, UI, collectors or the
combined binary as a whole.

## 1. Snapshot verification

- Recomputed `sha256(json.dumps({'base': m['base'], 'files': m['files']}, sort_keys=True, separators=(',', ':')))`
  over the full file records (path, sha256, source, change):
  `c9dfa986...d543d`, equal to `snapshot_sha256`.
- All 172 per-file SHA256 values match the worktree contents.
- Pool-relevant records: `bins/spc-pool/src/main.rs`,
  `bins/spc-pool/src/sv2_server.rs`, `bins/spc-pool/tests/main_sv2_pipeline.rs`
  are `combined-or-edited-after-approved-input` (source `lab-lp1`);
  `bins/spc-pool/tests/sv2_outcome_preemption.rs`,
  `crates/spc-store/Cargo.toml`, `crates/spc-store/tests/redis.rs` are
  `added-by-runtime-integration`. `crates/spc-store/src/redis.rs`,
  `bins/spc-pool/Cargo.toml`, `e2e_sv2.rs`, `sv2_conformance.rs` are byte-identical
  to the frozen LP-1 worktree (checked with `diff -q`).
- The pool delta against `lab-share-integration` is exactly the six files the
  implementation report section 6 lists. No other pool or store source differs.

## 2. Fix A - `spawn_front_event_reader` owns its Redis connection

`bins/spc-pool/src/main.rs:367-378, 741-770, 1010-1026`.

Verified:

- The reader now takes `config.storage.redis.clone()` (the same URL the
  publisher used at line 249) and calls `Streams::connect(&redis_url)` inside
  its own task. `Streams::connect` (`crates/spc-store/src/redis.rs:24-28`) does
  `redis::Client::open` + `get_multiplexed_async_connection`, which is a new
  TCP client to Redis; `Streams::clone` only clones the
  `MultiplexedConnection` handle. So the reader is a genuinely independent
  physical client and `XREADGROUP ... BLOCK 1000` no longer holds the
  publisher's client.
- Connection failure logs `front event reader connection failed; live front
  events disabled` and returns, the same disable path already used for
  `ensure_group` failure. Group/consumer names (`spc-front-<front_id>`,
  consumer = group), `read_pending` replay of unacked entries, decode-failure
  ACK, bounded 1,024-entry channel, 1 s sleep on read error, and stop on
  receiver drop are unchanged from LP-1. ACK-after-apply still happens in the
  main loop on the publisher `Streams`; XACK is a non-blocking command and is
  unaffected by the split.
- `refresh_and_publish` still publishes pending slice/block events before
  `sv2.publish` and the found-block path still does `submit_block` ->
  `publish_events` -> refresh; durability ordering was not moved.
- `wiring_guards::front_event_reader_requires_an_owned_redis_url` is a
  compile-time signature pin only. It prevents the specific regression
  (passing a `Streams` clone) by making it a type error; it does not exercise
  Redis. See N2.

Real-Redis regressions (`crates/spc-store/tests/redis.rs`, gated on
`SPC_TEST_REDIS_URL` like the two pre-existing tests in that file):

- `isolated_reader_connection_does_not_block_publisher_xadd`: two
  `Streams::connect`, reader held in the production `read_group`
  (`BLOCK 1000`), MONITOR confirms the XREADGROUP is in flight before the
  publisher `add` is timed (< 250 ms), then asserts the reader receives the
  exact payload, the entry is pending until `ack`, and pending is empty after.
  This exercises the actual `Streams` helpers (`read_group`, `add`,
  `read_pending`, `ack`), not a stub.
- `one_raw_client_still_reproduces_blocking_xadd_serialization`: one raw
  client with `CLIENT SETNAME`, waits for `flags=b` in `CLIENT LIST`, then
  asserts a same-client `XADD` takes > 400 ms. This is the negative topology
  control; it proves the positive test is not passing because Redis changed.
- Recorded `/tmp/opencode/lab-runtime-redis-tests.log`: 4 passed, `finished
  in 1.10s` (a skipped run finishes in ~0 s; 1.1 s is consistent with the
  blocking read and the 1.1 s expiry sleep actually running).
- Independent re-run against a private `redis-server 8.10.3` on a random
  loopback port (`/tmp/opencode/Fable-runtime-pool-redis-tests.log`): 4 passed,
  0 failed, 1.10 s. The server was stopped afterwards.

## 3. Fix B - jobs delivered while one share outcome is pending

`bins/spc-pool/src/sv2_server.rs:459-540`.

Verified against the frozen LP-1 code:

- One `register` per accepted share, one pinned `Box::pin(outcome)`, one
  `deadline = now + 10 s` computed once. The nested `select!` has three arms
  while the job channel is open: outcome -> `break`, `sleep_until(deadline)`
  -> `abandon` + `share_outcome_unknown` evidence + `return Err("share outcome
  unknown")` (byte-for-byte the LP-1 timeout behaviour), and `jobs.recv()`.
- A received job goes through the unchanged `send_job(&mut write, ch, &job)`
  with the same `Channel` state and writer, so SetTarget-before-job,
  future-job marker, `(prev_hash, nbits)` activation and `SetNewPrevHash`
  ordering are the ones already approved. The pending share is not repriced:
  its outcome is produced by the front for the job it was validated against,
  and `new_shares_sum` is the credited work from that outcome. Exactly one
  response is written after `break`.
- Jobs do not extend the deadline (fixed `Instant`), and a stream of jobs
  cannot starve the outcome: each iteration awaits the job write and then
  re-polls the outcome future in the next `select!`.
- `Lagged` keeps the existing hard-error policy. `Closed` sets
  `jobs_open = false` and falls into a two-arm `select!` (outcome, deadline);
  no busy loop, and the original deadline still applies.
- No additional submissions, type-32 frames or other client messages are read
  while the outcome is pending; only job delivery was unblocked. Socket
  backpressure on `send_job` can still block, which the diagnostic explicitly
  scoped out.
- `main.rs` was not changed on the share-outcome path; the block-qualified
  and actual-`d` checks exercised by `main_sv2_pipeline.rs` are intact (the
  test still requires exactly one success for sequence 1 with
  `new_shares_sum == 2` and the `accounted`/`job_issued` evidence; it now only
  tolerates job frames arriving before that success, which is the intended
  new ordering).

Wire regression `bins/spc-pool/tests/sv2_outcome_preemption.rs`: independent
Noise/SV2 peer (pinned authority), real qualifying share received by the test
front and deliberately left unresolved, new-prevhash job published; asserts
`NewExtendedMiningJob{job_id: 2}` then `SetNewPrevHash{job_id: 2, prev_hash:
0x22.., nbits}` arrive within 2 s, then resolves the old share with
`credited_work: 2` and asserts one `SubmitSharesSuccess{last_sequence_number:
0, new_shares_sum: 2, new_submits_accepted_count: 1}` and no further frame in
300 ms. On the LP-1 code this test cannot pass: `jobs.recv()` is unreachable
until the outcome future resolves or the 10 s timeout closes the connection,
so the 2 s wait fails deterministically.

Independent runs (`/tmp/opencode/Fable-runtime-pool-sv2-tests.log`,
`Fable-runtime-pool-lib.log`, `Fable-runtime-pool-clippy.log`, Nix devshell,
`CARGO_BUILD_JOBS=2`): `sv2_outcome_preemption` 1/1, `main_sv2_pipeline` 1/1,
`e2e_sv2` 2/2, `sv2_conformance` 4/4, `e2e_sv1` 1/1, `sv2_server` unit tests
6/6, `cargo clippy -p spc-pool -p spc-store --all-targets -- -D warnings`
clean. Recorded `/tmp/opencode/lab-runtime-pool-tests.log` sums to 236 passed,
0 failed. The full workspace was not re-run here (recorded 604/0 is the
runtime reviewer's scope).

## 4. Normative check

No decision record is required for this delta and none was added: the
changes are connection ownership and message delivery sequencing. The
normative rules in force (L1: success only after correlated front
acceptance, latest-only stale policy, LE256 targets, `(prev_hash, nbits)`
activation; LP-1: per-job credited `d`, ten-second unknown-outcome evidence,
fail-closed revalidation) are unchanged in text and behaviour. SV2 permits a
`SubmitSharesSuccess` to follow later job messages because responses are
correlated by `sequence_number`; the independent `channels_sv2` conformance
tests still pass.

## 5. Findings (all non-blocking)

- N1 Report precision / missing fail-first artifacts. Section 6 says both
  new regressions "were run against the old topology/wait and failed first".
  No cargo output for those red runs is stored; `/tmp/opencode/lab-runtime-redis-failfirst/`
  contains only a `redis-server` log. The preemption test's failure on LP-1 is
  deterministic by code reading (above). The Redis isolated test, however,
  never touches `main.rs`; it can only have failed against a deliberately
  shared client inside the test itself, not against the old `main.rs` wiring.
  The report should say that, or store the red logs.
- N2 Wiring coverage. The only `main.rs`-level guard is a function-signature
  pin. A real-Redis test inside `main.rs`'s `#[cfg(test)]` module that calls
  the production `spawn_front_event_reader(url, group, tx)`, holds it in the
  blocking read, and times a separate publisher `add` would cover the actual
  wiring the diagnostic asked for. Recommended, not required for this gate.
- N3 Evidence gap on error paths while a share is pending
  (`sv2_server.rs:495-498` and the `send_job(...)?` inside the nested loop).
  If the broadcast lags or the job write fails while an outcome is pending,
  the handler returns without a `share_outcome_unknown` record for that
  share; the miner never gets a reply and the front may still credit it. LP-1
  had the same gap on connection loss during the wait; Fix B makes `Lagged`
  during the wait less likely (jobs are now drained), so this is not a
  regression, but emitting the unknown-outcome evidence on these exits would
  make the unknown category complete.
- N4 `crates/spc-store/Cargo.toml:25-26`: the `redis` dev-dependency
  duplicates the normal dependency with the same version/features; harmless.
  `futures-util 0.3.34` and `redis 0.27.6` were already in the LP-1 lock, so
  no new crate versions enter via this delta.
- N5 Deadline-fixed-under-updates is reasoned, not tested. A wall-clock test
  would need ~10 s; a paused-time variant is awkward through the Noise TCP
  path. Given the single fixed `Instant` it is not worth blocking on.
- N6 Live provenance. At review time `target/debug/spc-pool` hashes to
  `16e55ac8...9225`, while `live8/run-manifest.json` records
  `11a82a87...afbd` for the pool binary used in live8. The binary has been
  rebuilt since (test runs rebuild it); the source timestamps (sv2_server.rs
  and main.rs edited 08:46 UTC, live8 created 08:51 UTC) make it plausible
  that live8 ran this delta, but this reviewer did not re-establish the
  live8 binary -> source correspondence. That is the runtime reviewer's
  provenance item.

## 6. Live6 vs live8 evidence (qualitative only)

The report's table (tip -> first job issue p50 859 -> 13 ms, p95 2,012 -> 16 ms;
submitted/stale 410/281 -> 663/55; outcome-unknown 0 -> 0) is consistent
with what Fix A removes (a same-client wait of up to one read timeout before
publication) and Fix B removes (per-channel head-of-line delay). Checked in
the live8 pool logs: zero `share_outcome_unknown` events on either front. The
two runs are on different chains with different load and tip counts (17 vs
159), so this is directional evidence that the mechanism is gone, not a
causal hashrate-budget or stale-rate proof. The report labels it provisional
and non-causal; that labelling is correct and must stay.

## 7. Boundaries

- No candidate files were edited; no commits, pushes or subagents. The only
  file written is this review in the main checkout. A private `redis-server`
  was started on a random loopback port for the store tests and stopped.
- This approval covers only the six-file pool/store delta against
  `c16032a7`, as present in snapshot `c9dfa986...d543d`. The combined runtime,
  its config/UI/collectors, the Cargo/Nix changes and the live-run
  acceptance are gated by the other Fable review and the parent's integrated
  verdict.
- Not claimed: hashrate-budget validation, SV1 interoperability, node full
  suite, settlement, or any production claim.
