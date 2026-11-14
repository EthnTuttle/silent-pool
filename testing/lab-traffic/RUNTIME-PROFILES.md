# Compiled-runtime profiles: r23 and historical r22

## r23: opaque-token and purpose-specific cleanup

The selected execution envelope is the durable 98f bundle with SHA256
`3cf18a8ee2611ad698e3002ca7862b2d5a9ec3d7382aab025ca211513c310ce0`.
The separate earlier temporary 9ecf assembly is not its selected reference.
The application tuple is CLI `bd5523ba...`, agent `ff27a7b7...`, files-only
manifest `85a0d7d0...`, compiler source `6hwxw1jm...`, and actual Nix deriver
`g2247ghg...`. All 1173 source files and all 15 declared gate logs are checked.
Runtime source remains `78c81ae2...`; it alone does not identify this release.

The hash-pinned `profiles/runtime-98f.json` includes the expanded security
closure and source-relative supplemental evidence. Original same-source Nix
startup logs prove the actual engine/Supervisor/loaded-agent/native-service
bridge for opaque traffic and literal markers, with stale-instance and wrong-
digest refusal. Fresh host logs separately cover the foreign namespace and
20 cleanup unit cases. The old readiness-lab 20 integration cases are distinct.
Supplemental records retain the actual source and test-harness bindings; they
do not claim these were full live campaigns or installed-binary executions.

`native_scope.py` accepts exactly the generated private NativeAudit fields,
including required canonical nonzero `run_token_hash`. It hashes the exact
Supervisor marker UTF8 bytes, including 64 hex characters as text; no decoding
or run-ID-prefix inference is allowed. Markers are bounded to 1024 bytes and
process environments to 1 MiB. Config, instance, observed process and complete
recorded marker census must agree. Existing PID/start, UID, boot, executable,
config, namespace, socket and complete PeerRegistry joins remain mandatory.
The field is not a user Scenario profile option or public wire field.

Runtime/locked-clock source rows are byte-identical to the historical reviewed
e4 versions. Earlier phase-test applicability is retained with its original
scope; the old raw temporary log was lost at restart and is not reconstructed
or relabelled as a fresh execution. Both pre-bootstrap failures retain their
historical failure classifications. Profile admission does not upgrade them.

## Historical r22: e4

The QA consumer and runtime applications are independently versioned artifacts.
This profile binds the already-built e4 application release; it does not claim
that the e4 compiler source contains r22 Python code.

| Binding | SHA256 / identity |
|---|---|
| Runtime source | `78c81ae25b7794d93ab88a1842ab2fe45a5e2a3c3e0c1ac978944cedea8283e0` |
| CLI | `ad07689dca3f0b90fca7fb8ffc44af136a664e7d3b1e0e18a6fed6b7ceb75d39` |
| Source manifest | `2085d16dc957451547d96d995a52d55302a6581e23fbd3af710f927d9e586524` |
| Compiler source | `/nix/store/x2n9ncavn4xzrwjph6px25ri4vyplfan-source` |
| Qualified derivation | `/nix/store/aqbz6alcpf6ivwdadcsfwj4q2vfykprg-spc-lab-ap5-qualified-0.1.0.drv` |

`profiles/runtime-e4.json` is the reviewed policy. Its hash is pinned in
`runtime_profile.py`. Admission verifies the actual native executable, Nix
deriver, source/output/install-check binding, all 1153 compiler-source rows,
52 explicitly enumerated critical rows, and the 15 immutable gate logs and
their source/binary/case inventories. Flat Nix derivation JSON and the observed
Nix version-4 format are supported strictly. The files-only source manifest is
never rewritten to add a root. No caller-supplied root or `passed` marker can
replace the compiler binding.

The e4 native/scan closure includes all 43 files matching the tested 4d14
source, including connection/outbound.rs and the retention writer/DTOs. The
private runtime adapter, evidence and native-binding files are also bound.
The old pre-N1/N2 source map is historical, not an eligible full-proof profile.

The three Runtime phase regressions have genuine earlier log passes on commit
82c4d7ca3fe0a1a61e88401915e8ab193427f048. Runtime, command-budget, native and
readiness code are byte-identical to that compiler source. Their scope
is the actual execute_phases path with a scripted actor/test bootstrap executor:
same original ID/principal/deadline, responsive cancellation, and no later phase
after failure. The original log/source references are recorded in the policy;
these are not relabelled as additional attempt-8 executions or funded terminals.
The same source-bound log also covers locked-clock start/equality/lock-wait
regressions. Current readiness raw-reader tests are in the qualified package.

The c03 and 8e58 canonical bundles share the runtime tuple but differ in local
artifact references. The selected envelope must be supplied explicitly:

```python
qa = Controller(args, runtime_bundle={"path": str(bundle_path), "sha256": bundle_sha256})
```

Alternatively, standalone QA inputs may carry `runtime_bundle:{path,sha256}`.
Admission checks the selected bundle bytes, app/source/check bindings and its
sealed pool-policy root, and returns that actual bundle SHA. The old 833-row
base profile retains its original bundle hash and rich-manifest handling;
g105 remains explicitly rejected.

Rendered public identities use only the actual Scenario serde shape
`address_source:{"address":...}`. Address parsing is bounded, ambiguity/legacy
field coercion is refused, and the collector joins the public address to the
observed wallet config. Private key files are not read by this adapter.

Profile admission proves compiled source/test provenance only. It does not
remove live readiness, native activation, full-Q, payout/Rule B, signed traffic,
reorg/witness, final-prefix or exhaustive-cleanup requirements. Releasing a new
QA archive must not rebuild the already-qualified applications or their inputs
aggregation merely to embed the new consumer revision.
