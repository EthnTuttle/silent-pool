# 2026-11-06 - Pool reader/outcome latency fixes (bounded)

Input: main-worktree `docs/reviews/2026-11-06-lab-stale-work-diagnostic.md`
(the read-only diagnosis is outside this frozen input set; this record is the
integration decision, not an approval of a new binary).

## Evidence

The diagnostic reproduced two pool-side head-of-line delays without changing
mining, validation or accounting:

1. One physical Redis client served both the front's blocking
   `XREADGROUP ... BLOCK 1000` reader and inline slice/block `XADD` publication.
   Redis blocks the client for the read, so an unrelated publish could wait
   about one second. A fresh-Redis probe measured ~641 ms same-client versus
   ~0.58 ms isolated.
2. An accepted share awaited its front outcome before the same SV2 connection
   could process a new job. A pending outcome therefore delayed
   `NewExtendedMiningJob`/`SetNewPrevHash` for that channel.

Run6 measured canonical tip to first activation p50 3.591 s / p95 11.011 s,
while exact job issue to activation was p50 4 ms / p95 15 ms. Of all
submissions, 73% were stale and many block-qualified shares were never
canonical; the diagnostic correctly rejected a causal hash-budget claim.

## Decisions

1. **Reader owns an independent connection.** `spawn_front_event_reader`
   accepts the Redis URL, connects itself, and never receives a `Streams`
   clone. `Streams` clones share one physical client, so the type-level change
   is also the future-wiring guard. Group/consumer names, pending replay, ACK
   after apply, bounded channels, and slice-before-job publication order are
   unchanged.
2. **Pinned outcome with nested job delivery.** The SV2 accepted-share wait
   keeps one outcome future and one fixed ten-second deadline while a nested
   `select!` delivers jobs through the existing `send_job`. A new job does not
   reprice the share, restart its timeout, create a second response, or relax
   stale/validation rules. Lagged publication uses the existing error policy;
   a closed job channel stops job delivery but does not extend the deadline.
3. **No accounting or protocol shortcut.** No old work is accepted, no
   share difficulty is lowered, no `submitblock`/consensus step is skipped,
   no event is dropped or reordered, and no per-share writer tasks are added.

## Verification

- `crates/spc-store/tests/redis.rs` real-Redis tests: isolated publisher XADD
  completes in <250 ms while the reader is blocked; the raw shared-client
  topology still reproduces >400 ms serialization; the published entry is
  pending until ACK.
- `bins/spc-pool/tests/sv2_outcome_preemption.rs`: a new-prevhash job and
  activation arrive while an old share outcome is unresolved; the old share
  then resolves once with its original sequence and credited work.
- Both regressions were run against the old topology/wait and failed first
  (Redis prompt assertion; SV2 new-job timeout), then passed after the fix.
- `main_sv2_pipeline` now tolerates a legitimate new job arriving before a
  found-block share's success, while still requiring exactly one success with
  original work.

The host runtime replay after the fix is provisional: the combined runtime and
pool binary are materially changed and are not covered by the LP-1/LP-2/L2
approvals until Fable reviews a new manifest. No causal hashrate-budget claim
is derived from the replay.
