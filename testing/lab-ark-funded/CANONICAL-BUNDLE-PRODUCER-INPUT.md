# Stable canonical AP5 envelope for the serial producer

The executable contract remains `spc.lab.ap5.bundle/v1`. Readiness v3 is an
observation DTO within the run, not a replacement bundle format. The authority
for required fields/checks is `common.check_bundle`, `source_scope.py`,
`joined_checks.py`, `gate_inventory.py` and the verifier admission descriptor.
The existing canonical assembler is
`/tmp/opencode/network-reap-diagnostic-r1/assemble-bundle.py`; its old outputs
are historical examples of structure only, not reusable source/test approval.

## Envelope and executable aliases

- `schema: "spc.lab.ap5.bundle/v1"`; explicit scope `confirmed-funding` or `full`.
- `base_revision`: actual source lineage reference.
- `binaries`: artifact references `{path, sha256}` keyed by aliases **lab, txd,
  pool, agent, miner, scanner, node, captaind, grpcurl, probe, redis-server,
  redis-cli, psql**. Program basenames such as spc-lab are not these keys.
  Full scope additionally requires `bark` for the pinned public decoder.
- `joined_source_manifest`: `{path, sha256}` for the actual full source census.
- `joined_validation_receipt`: `{path, sha256}` for the actual completed checks.
- `postgres_bin_dir`: actual shared PostgreSQL bin directory, with initdb,
  postgres, createdb and psql; the psql artifact must resolve there.
- `bootstrap_and_workers_single_budget`, `host_peering_before_profile_binding`,
  `fresh_registration_before_workers`: true; `history_version: 3`.
- Keep the declared source-built `runtime_script` binding in the producer packet
  and its source/test receipts. The runtime invocation itself uses admitted lab.

For every application alias lab/txd/pool/agent/miner/scanner the consumer queries
the installed Nix package's actual deriver and `env.src` (including structured
`__json` derivations). All six must have the same actual `/nix/store/...-source`
input. The full manifest rows and applicable approved source rows are checked
against that input. A mutable snapshot directory or seven observer hashes alone
cannot establish this binding. Source-applicable approval and complete cumulative
union coverage are required; partial workspace success is not union coverage.

## Actual check receipt

`schema: "spc.lab.ap5.join-validation/v1"`, exact source_manifest_sha256,
six `binaries` alias-to-hash values, and nonempty completed `checks` records.
Every check has a genuine integer exit 0 and a hashed actual log. Preserve actual
name/purpose/argv/source/binary identities, not prose summaries. Required feature
purposes are agent-ark-tests, agent-ark-build, pool-ark-tests, pool-ark-build,
ui-build, ui-browser-tests. Ark test/build invocations explicitly select Ark
features; browser cases must match complete required/passed case lists and the
actual lab binary. `ui_source_files` binds the full required UI/runtime files.

For diagnostics, `diagnostic_source_policy` is the exact artifact defined by
DIAGNOSTIC-SOURCE-POLICY.md: actual source class/root/manifest, complete pool rows,
validation, app hashes, runtime bindings, fixed limits, maintenance scope and
the source-qualified joined inventory. The old 332 and 350 inventories remain
bound to their old exact sources. A changed suite total requires an actual new
completed per-suite/source/log qualification; never an >= floor or inherited pass.

If control code is present, even in a diagnostic, pool_control_source_policy
must bind that actual root/manifest and all pool files to the approved R4 source
and completed applicable checks. A no-control policy cannot hide control code.
Full scope also needs the actual aggregate_source_policy and its completed
source/check bindings; source-only or unrelated policies cannot be attached.

## Exact executed probe tuple

`probe_invocation` has the closed fields schema, wrapper, program, script,
prefix_args, runtime_subcommand, source_manifest. Schema is
`spc.lab.ap5.probe-invocation/v1`; wrapper equals binaries.probe. The program is
the actual native Python ELF, script the exact canonical probe.py, and prefix_args
contains that single script path. Runtime appends node-probe exactly once.
Bind the 14-file probe source manifest and all hashes. A wrapper path alone is
not the executed interpreter/script tuple.

## Fixed dependencies and full extras

Node bba4752298aff258adb6cce99c6db849b02f50736a8ec32766ed1710045051aa and
stock captaind 4cac675bccc017608378382129613ed5202dd90e7300a92c4ed799869abaef9d
remain fixed. Full scope adds the same-source Nix `pool_wallet_executor`
artifact; its actual deriver src must equal the six-app source. Reuse the
immutable admitted `vtxo_verifier` descriptor/artifact 5330c3ce... and existing
source/lock/check receipts. Bark source-revision must remain c419159fa54085572f8ca72d2ba1716556a9d3eb.

The full source union must actually contain approved Ladder/R4/native/wallet
and their required store/migration/control/native-gate/outbound modules. No
capability is inferred from an old CPU, fixture, ad-hoc schema or omitted files.
Uncompiled native8259 and new historical SPc observations need their own actual
compiled/tested integration before full claims. Those identity-bearing records
remain in SPc/QA, never the Ark role/streams.

The shared runner additionally consumes separate hash-bound QA source descriptor,
QA inputs and a same-run scenario. Those do not replace the AP5 canonical bundle.
Supply the completed canonical artifacts through the parent; the consumer will
run its actual admission and fresh execution automatically when the evidence is
valid. This inventory introduces no alternate envelope or approval flag.
