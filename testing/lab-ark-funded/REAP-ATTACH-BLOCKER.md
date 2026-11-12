# Post-stop reap failure, separate from global unreadable coverage

Run: `/tmp/opencode/AP5-corrected-native-live-r1`.
Bound CLI: `52e45eabfbcbc53921c670a19b36cb6940a7228f5d8b4bc15c43556bedce402b`.
Source: `/nix/store/11icfby7ni0185p093hxmbsahyl0p2dl-source`.

## Exact recorded error

```text
Error: traffic_launch_profile_mismatch: node ownership/argv/binary changed
```

Exit 1, not a timeout; stdout empty. The exact stderr bytes above, including
the newline, hash to
`ac9c338cb3beb77443d4b9be33c8ceaaaf232f774636277df2b00992be459d7e`, matching
`000008-tool-exit.json`. The original runner retained the digest rather than
the raw stderr. This text was recovered by exact hash matching, not by
rerunning reap. The recorded argv digest also matches the bound CLI's
`reap --run-dir /tmp/opencode/AP5-corrected-native-live-r1/run` invocation.

## Stage and cause

1. `bins/spc-lab/src/main.rs:422-425`: Reap first calls
   `RunEngine::attach(...)?`, then `engine.reap()`.
2. `crates/spc-lab/src/engine.rs:346-350`: attach selects `adopt=true`.
3. `engine.rs:519-523`: this path re-verifies live traffic binding before
   adopting processes or reconciling commands.
4. `engine/traffic_runtime.rs:1051-1065`: every node must be live
   `Ownership::Owned`, with matching argv and executable hash.
5. `supervisor.rs:507-521`: gone/zombie nodes yield Missing; reused PIDs yield
   PidReused. Neither can satisfy the live-adoption requirement.

The preserved manifest is Stopped. Both node records are Stopped, and their
stored argv and binary hashes still exactly match their recorded profiles:
node-a PID 2075188/start 61115028; node-b PID 2075219/start 61115049.
The independent census records those identities gone. The specific runtime
Ownership variant was not logged, so it is not reconstructed as an observed
enum value. The concrete structural failure is the post-stop cleanup command
being routed through an all-nodes-live adoption gate.

`RunEngine::reap` (`engine.rs:3057-3080`) was **not reached**. This failure is
not caused by the four unreadable same-UID user processes: this gate examines
the scenario's node records, not the global process census. Conversely, known
recorded processes being gone does not turn the failed reaper into success.
Both facts remain recorded separately. Global coverage stays INCONCLUSIVE.

## Required primary correction and regression

Provide a cleanup-purpose attachment/load path that verifies persisted source,
manifest and ownership bindings without requiring every recorded node to
still be alive. It must populate the supervisor with the recorded census,
avoid resuming/reconciling control work, and retain normal identity checks
before signalling any surviving process. Missing nodes can be reported as
gone; reused/unverified PIDs must not be signalled or silently accepted.
Live adoption/control must retain its existing strict traffic gate.

Simply switching to `attach_read_only` is not sufficient: that path does not
populate the supervisor through adopt_processes, and the current reap method
would otherwise operate on an empty record set and replace the manifest's
census. Do not implement an empty-census success or ignore the error.

Regression scope: a normally stopped traffic run with matching preserved
profiles must reach the real reaper; residual owned actors must be handled;
PID reuse, tampered argv/binary/profile bindings and unknown ownership must
remain fail-closed. This can accompany the primary's cleanup work before a
full cleanup-acceptance claim. No reap, service or process action was rerun
for this analysis.

Sanitized, source-hashed analysis:
`/tmp/opencode/AP5-verifier-admission-and-reap-r1/reap-analysis.json`.
