# 2026-11-06 - Native scenario convergence v1

## Scope

This is a bounded lab-scenario and evidence decision, not a pool accounting,
payout, SPc/Ark or production-network decision. The approved namespace-runtime
code `2d6c752d` and the earlier 969/913 no-heal diagnostic remain in scope.

## Decisions

1. A phase MAY declare one native `convergence` action with
   `version = 1`. An absent `convergence` field leaves existing scenarios and
   non-convergence runs unchanged. In particular, the runtime no longer infers
   mining merely from a previous `heal_link` or a run duration. One action per
   scenario is accepted.
2. The action has an explicit `timeout_secs`, `max_observations`,
   `allow_resume`, and `max_resume_attempts`. Version 1 accepts only
   `1..=90` for the first two. Resume is disabled by default; a zero resume
   budget is required when `allow_resume` is false, and at most one resume per
   miner is allowed when enabled. No implicit post-phase hash loop exists.
3. Resume/pause use the existing typed durable `resume_workers` and
   `pause_workers` commands, journaled before effect. Any common atomic
   (height, hash) observation is only a candidate: workers are paused and both
   nodes are re-read before a `converged` result is published. Pause is not
   hashing and is allowed regardless of `allow_resume`.
4. A timeout or observation limit is `not_converged`. Stop during an action is
   `unknown` with a reason, not a convergence claim. Invalid JSON, versions or
   combinations fail closed during validation. Resume-only mode without
   declared budget is rejected.
5. `topology_phase` and `topology_convergence` are versioned typed Evidence
   payloads (schema version 1). Raw broker JSON stays bounded and nested with
   explicit success/error answers instead of stringification. Every atomic view
   retains its actual `previousblockhash`, while phase/convergence records
   preserve the pre-partition common anchor. Height differences alone are not
   accepted as a fork or a shared tip. Old recorded
   `unknown { topic: "topology_phase", detail: <string> }` records retain their
   prior meaning; the generic SSE `message` handler needs no special case.
6. The network-namespace actor boundary is not a filesystem or security
   sandbox. Builders, workload child processes and credentials share the
   intended host filesystem/run-directory trust domain.

## Consequences

Real work needed to break a tied fork is explicitly enabled and bounded in
scenario authors' native DSL. Existing finite/interactive/host phases without a
convergence declaration do not receive an implicit headless `resume_workers`.
Stop, convergence failure and ordinary completion still follow the existing
supervisor lifetime. Approved files outside this bounded decision and its guide
updates are not silently re-authorised.
