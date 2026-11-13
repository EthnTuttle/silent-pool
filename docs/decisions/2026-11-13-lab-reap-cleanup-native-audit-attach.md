# 2026-11-13 - Cleanup attach tolerates an unpublished native-audit lease

## Scope

Fixes the second partial-startup reaper bug in the cleanup-purpose attach
path (`RunEngine::attach_for_cleanup`, the `reap` CLI subcommand). After
the mode-factored traffic gate (2026-11-12 record) let a recorded-capability
run through, `attach_internal` still ran a post-match block that called the
LIVE `validate_native_audit_attach` for every non-read-only attach whose
scenario declared any native-audit wallet agent - overriding the purpose
factoring and demanding live wallet/lease readiness from cleanup. A wallet
that fails during startup (the failed AP5 Git shared run: the
wallet/alice token-contract mismatch) never reaches the miner bind that
publishes `private/native-audit-leases.json`, so `reap` exited 1 with
`native_audit_unavailable: native audit lease projection missing on adopt`
before the recorded census was ever loaded. Observed on the preserved run
`/tmp/opencode/AP5-c03-git-shared-r1`; counterexample
`/tmp/opencode/AP5-git-run-launch-r1/counterexample/handoff.json`
(traffic capability recorded, probe manifest present, wallet exited during
readiness, commands 0, heights 0). No change to `prepare`, the Live adopt
path, or read-only attach.

## Decisions

1. **Purpose factoring completed, no global relax**: the redundant
   post-match native validation is removed from `attach_internal`. The
   Live arm keeps its strict `validate_native_audit_attach` (live wallet
   socket, instance file, registry hash, ownership and evidence proofs -
   missing lease projection on a declared native-audit wallet still
   rejects a live adopt with exactly the old error). Cleanup validates
   only persisted native-audit bindings. Read-only validates nothing
   (unchanged).
2. **Missing lease is expected-not-launched**: a lease projection is
   published by the miner bind (`bind_native_audit`), not by wallet
   startup, so a wallet that failed before any miner bound has no lease
   to contradict. Cleanup treats the absent
   `private/native-audit-leases.json` as Ok - the absence is never
   fabricated into a lease, capability or command, and never a reason to
   block resource cleanup.
3. **A present lease is verified as persisted metadata, fail closed**:
   `validate_persisted_native_audit_bindings` (new, cleanup-purpose)
   requires the file to decode under the private-read invariants and the
   recorded run/schema to match, then cross-checks every lease against
   the PERSISTED supervisor records: wallet and miner peer process
   identity (pid, start-time ticks, executable hash, run-token hash), the
   single recorded `--config` argument, the config bytes' hash, the
   registry file's recorded hash, and the registry's wallet-instance/run
   binding. Foreign or rewritten projections reject
   (`native lease projection foreign`, `native wallet record replaced`,
   `native peer record replaced`, `native registry projection mismatch`,
   `native registry foreign`, `native record missing from manifest`).
4. **No live proof, no wallet authority in cleanup**: the cleanup
   validation never touches `/proc`, sockets, instance files or
   `supervisor.verify` (the live `native_process` ownership check stays
   Live-purpose), spawns no wallet, loads no keys, and reconciles,
   signs or hashes nothing. Cleanup only reports and signals processes
   the recorded census verifies as ours; a reused or foreign PID still
   reports `OrphanUnverified` and is never signalled.
5. **Post-attach checks audited by mode**: the mode match in
   `attach_internal` is now the complete post-attach check list - Live
   (traffic reverify, adopt, native live validation, command reconcile,
   state rebuild), Cleanup (persisted traffic bindings, persisted
   native-audit bindings, census load), ReadOnly (none). No cross-mode
   check remains after the match, so one partial-startup edge cannot
   reintroduce a purpose mismatch.

## Tests

`crates/spc-lab/src/engine/native_audit_cleanup_tests.rs` (new): the
actual configured native-audit traffic scenario (Ark-backed pool, wallet
agent with collector profile over well-known generator x-only pubkeys,
validated scenario, recorded traffic capability projected from a full
passing report over the approved contract). The AP5 Git run blocker
pinned end-to-end: partial wallet startup with a recorded capability
attaches for cleanup (previously `native audit lease projection missing
on adopt`), the Live native gate still refuses the same state, reap
reports the owned wallet record plus the never-launched node, no lease/
command/capability is invented, idempotent second cleanup. A foreign
lease projection (wrong run id, empty map) fails closed. A consistent
present lease (impossible PIDs, stopped records, no live socket or
instance file) is accepted as persisted metadata and reaped; rewritten
registry, replaced wallet record, replaced miner peer record and a
missing wallet record each reject. A live foreign process recorded in
the wallet record with a stale start-time tick reports
`OrphanUnverified` and survives untouched. The prior suites
(`reap_cleanup_tests`, `reap_cleanup`/`engine_restart`/`access_scope`
integration) stay green.
