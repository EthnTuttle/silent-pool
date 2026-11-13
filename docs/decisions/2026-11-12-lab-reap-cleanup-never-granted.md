# 2026-11-12 - Cleanup attach tolerates a never-granted traffic capability

## Scope

Fixes the cleanup-purpose attach path (`RunEngine::attach_for_cleanup`,
used by the `reap` CLI subcommand) that the
`docs/decisions/2026-11-08-lab-traffic-contract-t0.md` F1 gate wiring
had accidentally made stricter than its purpose: the gate ran
unconditionally on every non-read-only attach, so a traffic run whose
probe stopped BEFORE granting the capability (FAILED/INCOMPLETE
provisioning - no capability record, no launch profiles, no processes)
could never be reaped; `reap` exited 1 with
`traffic_capability_unrecorded` before the recorded census was ever
loaded. Observed as the second cause of the failed AP5 r22 shared run
(preserved at `/tmp/opencode/AP5-c03-shared-r1`; counterexample
`/tmp/opencode/AP5-r22-runtime-launch-r1/counterexample/handoff.json`).
No change to `prepare`, the Live adopt path, or read-only attach. No T1
or T3 semantics, no capability granting, no command/hash/resume
enablement.

## Decisions

1. **Purpose-factored gate, not a global relax**: `attach_internal`
   factors the traffic capability gate by attach mode. Live verifies
   the recorded projection exactly as before (missing, unapproved,
   tampered or unrecognized still rejects with
   `traffic_capability_unrecorded`/lineage codes after preflight and
   before the owner lock). Cleanup verifies a record that IS present
   exactly as strictly as Live - rewritten history still rejects - and
   tolerates only the never-granted ABSENCE. Read-only never runs the
   gate (unchanged).
2. **Cleanup grants nothing**: cleanup attach never grants or
   exercises a traffic capability and never enables commands, hashing
   or resume; it validates the persisted manifest/config/profiles and
   only stops what the recorded census verifies as ours. The absence
   of a capability record because the probe stopped before the grant
   is not permission to block resource cleanup.
3. **Persisted binding order**: `validate_persisted_traffic_bindings`
   checks the node process record FIRST. A scenario node with no
   recorded process was never launched: its missing launch profile is
   the same expected-not-launched state, reported by `reap`, not a
   contradiction. A recorded process without a profile still fails
   closed (`recorded node process without recorded profile`), and
   changed argv/binary/driver-config bindings still reject.
4. **Expected-not-launched reporting**: for traffic runs, `reap`
   reports every scenario node and profiled node with no process
   record as `Exited` (expected-not-launched) instead of silently
   vanishing from the cleanup report. Non-traffic runs report only
   their recorded processes (unchanged).
5. **Ownership unchanged**: `reap` still signals only processes the
   recorded census verifies as ours (start-time tick, run token,
   executable path and hash); a reused or foreign PID reports
   `OrphanUnverified` and is never signalled.

## Tests

`crates/spc-lab/src/engine/reap_cleanup_tests.rs`: the AP5 blocker
pinned end-to-end at the engine boundary (cleanup accepts a
never-granted prepare-state run, Live refuses with
`traffic_capability_unrecorded`, no capability/command/process
mutation, idempotent second cleanup); never-launched and partial
provisioning; recorded-process-without-profile fail-closed; a live
foreign PID recorded with a stale start-time tick is never signalled
and survives; a present-but-invalid capability record still rejects
cleanup. Prior stop/reap suites (`reap_cleanup`, `engine_restart`,
`access_scope`) stay green.
