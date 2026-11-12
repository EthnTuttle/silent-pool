# Existing-wallet native service and aggregate consumer: build turn

Status: component verification passed; ark-enabled binary built and sealed.

Final packet: `/tmp/opencode/aggregate-wallet-service-tested-r1/handoff.json`.

All work stayed in the assigned wallet worktree and external receipt paths.
The 0b0/e97 protected source trees, 708/717/726 source archives and failed
invocations are retained. No commits, subagents, funding or node-server changes.

## Actual verification

- Default agent all-target check passed; default library tests: 48 passed.
- Ark agent all-target check passed; ark library tests: 170 passed.
- Native public codec: 6 passed; corrected source/conservation audit: 9 passed.
- Frozen-order raw codec: 2 passed; wallet bridge integration: 3 passed.
- Runtime wallet binding/renderer helpers: 7 passed in this owned closure.
  The other owner's stale-pool runtime test blocker did not occur here. This
  does not establish the external current-primary CPU/runtime union.
- Default and ark agent clippy, scanner clippy, runtime-library clippy and final
  changed-file formatting checks passed. Each retains its actual source receipt.

Counts are per invocation/feature scope, not a sum of unique new service tests.
Earlier upstream helper test counts are dependency evidence only.

## Failures retained and corrections

1. --locked refused the manually added optional ark-lib serde_json edge in
   this wallet feature graph. Offline workspace resolution removed only that
   edge; package names/versions/sources/checksums remained identical.
2. The genuine arkoor negative fixture needed the actual builder's
   cosign_response() result. No proof/policy check was removed.
3. Collector adapter fixtures needed an explicit 0700 temporary root. The
   production path/ownership check remained strict.
4. The Python CPU peer fixture needed an explicit READY signal before /proc
   observation, so an executable wrapper could finish first. PID/config/start
   checks remained intact.
5. Internal large evidence storage was boxed and a boolean expression and
   runtime string borrow simplified for clippy. Public codec bytes stayed
   unchanged. Native-owner tested fixture/lint hunks were selectively imported.
6. The first joint test supplied a full proof job ID to the SV2 fixture's u32
   wire-ID field. The fixture now sends 17 while retaining full proof ID
   4294967313 and asserting the actual received wire value.

## Actual native CPU / wallet-service joint result

The sealed aee91a91 native CPU binary used real Noise/SV2 and the actual typed
wallet service, consuming auditor, COMMITTED verification and loaded-wallet
activation signing. The collector fixture held COMMITTED after verifying the
wallet's SourceFeed v2 Consumed signature. At that point the CPU snapshot had
attempted="0", active_job=null, source_audit_ready=false and state=pending.

After release, native event order was SourceAuditCommitted #5, JobActivated #6,
AuditFirstHash #7, with matching request digests. Receipt:
/tmp/opencode/aggregate-wallet-joint-r2.json.

The collector used independent fixture keys and synthetic complete empty
history (actual_sum=0, Q=2). Wallet startup metadata was fixture-controlled;
the service/verifier/signatures and native CPU binary ran for real. This is
component ordering evidence, not actual full CLI/controller bootstrap,
independent QA history approval, full-Q campaign or funded Ark acceptance.

## Remaining integration

Primary/QA must union the captured-current runtime hooks and native CPU packet,
then validate actual four-wallet lifecycle publication/restart/attach, initial
and dynamic controller peer publication, real history/full-Q, and funded
send/receive/refresh/offboard. Exact private command/status/UI interfaces and
native instance/peer/socket filenames remain as in the typed handoff.

## Binary

`spc-agent --features ark` source code aggregate:
`2a022552286d077a567535d156d93d93ee57d55541601866220b8a7e7ac00520`.

Sealed binary (400,668,496 bytes, mode 0555):
`/tmp/opencode/aggregate-wallet-service-sealed/aee146fb76cb8ac3716f700bdbdc356adf7ed7d539385b74ca5f7140cddfe2e8/spc-agent`.
SHA256 `aee146fb76cb8ac3716f700bdbdc356adf7ed7d539385b74ca5f7140cddfe2e8`.
Build and seal occurred under the global nonblocking lock with before/after
source binding, CARGO_INCREMENTAL=0, assigned T1a target and the 50/20-GiB guard.
