# Actual g105 diagnostic runtime blocker: asynchronous bootstrap phase

Observed with the parent-verified bundle
`/tmp/opencode/AP5-joined-bundle-r1/bundle.json`, SHA256
`f2c96b21ccb85300b76ba93aff218fa39861dbda531c0e424a8cb5a71e7171af`.
Actual source: `/nix/store/wr1ix2s3hnhjhhmi2dv7v5cfd0mb28gc-source`.
Canonical source/binary admission passed; this is a runtime execution blocker,
not the absent dd9 wallet-control R1 or a request for a new funding API.

## Reproduction and evidence

- `AP5-native-confirmed-live-r1`: wrapper probe launch refused. Fixed in the
  harness using the already supported `--traffic-probe` native interpreter
  plus `--traffic-probe-arg` frozen script. No assembly/source bypass.
- `AP5-native-confirmed-live-r2`: probe passed read-only surface checks, real
  actors started. Declared `ap5-native-bootstrap` returned `running`, detail
  `bootstrap durably in flight`; Runtime treated it as a phase failure. Its
  durable terminal is `cancelled` / `bootstrap cancelled`, same original id.
- `AP5-native-confirmed-live-r3`: attempting to move bootstrap to the
  authenticated controller while leaving phases empty fails before launching
  actors: `traffic_bootstrap_route_missing`. The required declaration has now
  been restored; no repeated identical run is useful before the producer fix.

All run roots are under `/tmp/opencode/` and remain preserved with event seals,
logs, commands and cleanup inventories. No user-payment command ran. Recorded
processes (12 / 41 / 1) are all gone; recorded listeners are released; no
run-file references or locks were found. Four unreadable same-UID processes
prevent an exhaustive global cleanup claim. Native funding, full Ark lifecycle,
native-audit and aggregate acceptance are all unclaimed.

## Exact source contradiction

`crates/spc-lab/src/engine/traffic_runtime.rs:144-185` selects the bootstrap
driver/node from `scenario.phases[*].commands`. Without that declaration,
pre-provisioning admission fails. The native executor at lines 1250-1275
durably starts an asynchronous thread and correctly returns `Running`.

`crates/spc-lab/src/runtime.rs:757` submits each declared phase command, then
lines 840-842 reject every outcome other than `Succeeded`. The same file has
the intended bootstrap wait at lines 582-587, but that branch only constructs
`StopRun`, so its bootstrap test is unreachable.

## Minimum producer fix (not applied to another owner's source)

Inside `execute_phases`, capture the operation id before moving `command`,
and wait on the existing bootstrap executor only for its asynchronous outcome:

```rust
let bootstrap = matches!(command.kind, CommandKind::BootstrapFund { .. });
let operation_id = command.command_id.clone();
let mut outcome = self.engine.submit(command);
if bootstrap && outcome.state == crate::command::OutcomeState::Running {
    outcome = self.engine.wait_traffic_bootstrap(&operation_id)?;
}
```

Use this before building the phase observation/checking for Succeeded. Remove
the misplaced bootstrap wait from the duration StopRun branch. Do not submit a
second bootstrap, allocate a new operation id, invoke generate, or synthesize
success. Add a regression exercising Runtime phase execution (not merely
RunEngine's native executor). The updated source/test/Nix binding must accompany
the corrected executable. No heavy build was attempted by the AP5 owner.
