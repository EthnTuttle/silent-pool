# Explicit metadata dependency for the runtime probe

The fresh c03 failure exposed an environment dependency, not a node/protocol
incompatibility. The installed canonical probe writes reports, then calls bare
`git rev-parse HEAD` while building its source manifest. Runtime receives only
QA's explicit path_env; the outer AP5 tools wrapper's Git directory was absent.
The original failed run and its sealed inputs remain unchanged.

## Binding and prelaunch check

`prepare-shared --consumer-tools` requires an actual immutable AP5 tools wrapper.
The generator reads its declared Nix PATH prefix, resolves Git there (never the
ambient host PATH), verifies membership in the package's actual Nix closure and
checks the Git output against its actual deriver. It seals the native Git file,
deriver file and actual source archive plus the querying Nix tool identities.
Nix 2.34 structuredAttrs are read directly; nix-store's multicall invocation
name is preserved. An arbitrary hash/boolean does not provide this binding.

The new input recipe records `runtime_metadata`, adds a source-sealed `git`
binary entry and prepends only the verified Git bin directory to the existing
runtime PATH. Runtime's full application source, six binaries, c03 bundle,
probe contract and probe program/source manifests remain unchanged.

Preparation executes the installed `git_base_commit`, `build_manifest` and
`write_manifest` against a labelled public metadata-only fixture. It checks
all 38 actual installed probe source/fixture hashes. A subprocess audit hook
allows exactly one `git rev-parse HEAD` at the immutable probe root and rejects
network connections or listeners. Git correctly returns no repository HEAD;
the resulting base_commit is null and is never treated as approved source.

`validate_shared_binding` repeats the dependency/source/deriver and exact-PATH
writer checks after QA prerequisites and before QA.start. It checks the sealed
Git entry against runtime_metadata, then retains the actual metadata receipt
in both evidence streams. Missing Git or wrong/shadowed bindings fail before
any actor startup. The generated metadata manifest is explicitly a file-only
fixture, not a replacement for the actual run's later probe manifest.

## Inspected subprocess inventory

This census covers the installed canonical probe's production Python path and
the dependent AP5/QA launch/metadata path. Test/build-only exporters and fixture
monkeypatches do not become runtime tool requirements.

| Caller | Subprocess | Binding/context |
| --- | --- | --- |
| `arkrpc/reporting.py:244-250` | `git rev-parse HEAD` | The only bare metadata executable in the probe; now explicitly sealed in runtime PATH |
| `arkrpc/nodeproc.py:341` | actor-exec helper plus selected node argv | Actual absolute helper/node from Runtime handoff; PID/start/profile seals retained |
| `arkrpc/nodeproc.py:133` | caller-selected node | Legacy probe mode; not selected by this runtime |
| `arkrpc/cli.py` | delegates to nodeproc and reporting | No hostname/date command; report time uses Python datetime |
| QA `runtime_profile.py`, native/fullrail root resolvers | `nix-store --query --deriver`, `nix derivation show` | Controller's immutable AP5 wrapper PATH supplies Nix; separate from Runtime actor PATH |
| QA verifier readers | `nix-store`, `nix derivation show`, `nix log`, absolute verifier | Controller-side source checks and sealed verifier; not a Runtime metadata helper |
| QA `regtest-traffic-e2e.py` | absolute lab runtime/reaper | Sealed application path; runtime receives only declared path_env |
| QA `storage.py` | constructed absolute SQL/helper argv | Sealed PostgreSQL/namespace tools from binary bindings; no shell lookup |
| AP5 `ap5.py` | supplied absolute admitted argv | Canonical bundle/tool bindings, owned process recording and bounded wait |
| New `metadata_tools.py` | declared absolute Nix queries; absolute probe Python/helper | Read-only derivation/source queries and a bounded file-only metadata subprocess |

The probe's source clock, report formatting, source hashing and platform path
handling use Python libraries. No additional date, hostname, uname or host-bin
fallback is introduced. The new preflight catches an unexpected extra metadata
subprocess rather than extending an allowlist automatically.

## Verification and scope

Four focused tests execute the actual installed manifest writer, reject absent
Git before the writer starts, reject forged source/deriver/executable seals and
reject host PATH/store traversal. Existing coordinator tests retain the
preflight-before-start and single-finalization ordering. A separate file-only
check used the previous public report's exact node-description/exit fields and
exercised finalized-launch-profile manifest output, without filling the missing
manifest in the failed run or changing its verdict.

Fresh r22 prerequisites verified 18 binary seals, and the AP5 shared prelaunch
metadata check passed against the unchanged c03 application bundle. The new
private input/key tree and run ID are separate from the failed run. Execution
still requires the published updated Python consumer and the parent's selected
qualified runtime/input combination. The partial-provision reaper issue remains
separate until its assigned CLI fix/regression is tested; no nonzero reap exit
is ignored and global unreadable-PID coverage remains inconclusive.
