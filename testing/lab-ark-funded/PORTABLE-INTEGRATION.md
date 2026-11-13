# Primary integration contract: portable AP5

Only primary commits/pushes. This packet changes owned AP5 files; it does not
edit the primary flake, node/probe recipes, QA code or Git index.

## Flake wiring

In the root flake's existing `let` (which already has pkgs, rust, src and
arkPackages), add the equivalent of:

```nix
ap5Packages = import ./testing/lab-ark-funded/nix/packages.nix {
  inherit pkgs rust src;
  revision = self.rev;
  labTools = self.packages.${system}.spc-lab-ark-tools;
  node = self.packages.${system}.rbitcoin-node-ark-lab;
  inherit (arkPackages) captaind bark;
  probe = import ./nix/lab/ark-probe.nix { inherit pkgs; };
};
```

Export from `packages`:

```nix
inherit (ap5Packages) ap5-tools ap5-vtxo-verifier ap5-qa-source
  spc-lab-ap5-qualified ap5-inputs;
```

The qualified derivation overrides the joined package and is an independent
application compilation; overrideAttrs does not reuse compiled target artifacts.
The generated runtime wrapper points directly to that qualified output and
does not add the unqualified application build as a dependency.
The verifier is a standalone pinned-lock package. Existing node/bark/probe
packages are parameters, so primary's published source recipes apply directly.

For consumer-only revisions, select only `ap5-tools` and `ap5-qa-source` from
the new release. Retain the already built runtime release's `outputs.json` and
use it as `portable assemble --outputs`. Do not reevaluate `ap5-inputs` against
the new consumer revision. Full admission derives the source root from the six
actual runtime executables and verifies that root's original source contract,
manifest and 15 executed gate receipts. It does not compare the runtime against
the consumer checkout's HEAD. QA's explicit source descriptor is separately
verified and retained by the shared runner. Thus consumer changes need no
runtime recompilation when runtime inputs are unchanged.

The independently released consumer parses both the legacy exact-deriver Nix
JSON map and the actual Nix 2.34 version-4 envelope. It verifies the sole exact
queried derivation, version and output basename before normalizing output paths.
Unknown versions, extra/foreign derivations and path escapes reject. This is a
metadata format correction, not a source or executable admission exception.

The node Git publication c1505776117b1d4d42489f309af33978bbe124cc preserves the
f159 source/t6LQ NAR and bba binary. The portable probe's root-independent runtime
mapping is 90d33070, with contract 754e1e61 and thin entry 833288 unchanged. These
are source identities, not build/admission receipts. Fresh actual Nix outputs
must supply all emitted manifests and program hashes; old local recipes or c5gh
paths cannot be used as production output bindings.

## Source policy and actual gate generation

1. Commit the complete reviewed application/QA/AP5 source union and the actual
   gate plan. No source-owner approval is invented here. The required store
   migrations 0007-0011, aggregate/economic/send-fence, native-gate/outbound,
   readiness and control paths must all be present.
2. Fill `aggregate-source-lineage.json` from the reviewed aggregate lineage:
   exact source snapshot SHA and path-to-hash rows for its actual core. The
   shipped null/empty file is deliberately unqualified. This identity is not
   replaced by a fresh hash of arbitrary files. R4 retains its exact source/code
   identity and the byte-identical public review in provenance/; different R4
   code needs its legitimate source-applicable revision, not a path waiver.
3. Generate `source-contract.json` from HEAD and the committed gate plan:

```sh
nix run .#ap5-tools -- portable source-contract --repo "$PWD" \
  --plan testing/lab-ark-funded/gate-plan.json --out /absolute/new/source-contract.json
```

   Primary installs/commits that generated file. The generator reads HEAD with
   `git ls-tree` and verifies Git blob identities; it never reads/writes the
   index. Dirty/missing tracked files reject. The contract binds every selected
   committed file except itself (avoiding a recursive digest). It is labelled
   source-bound-not-tested. Source/Nix filtering excludes build/runtime caches.
4. Build `ap5-inputs` under the assigned serial build budget. Before compilation
   the qualified package checks the source contract; in installCheckPhase it
   executes the entire exact plan. Failed steps prevent the Nix output. Actual
   case lists, logs, tool hashes, application hashes, source census and tested
   capability links are installed under share/ap5-qualification.
5. `portable assemble` reads only those immutable outputs and committed policies,
   generates current-machine canonical source/control/aggregate/probe/verifier
   bindings, recounts the logs, and calls existing `check_bundle(full=True)`.
   No admission receipt is emitted on failure. No historical private run is used.

The initial source-contract.json and gate-plan.example.json are NOT admission
data. Empty cases, boolean capabilities, unrelated-purpose capability links,
fake executables and workstation/store-instance paths in the plan reject.

### Gate plan shape

The closed plan has `checks` and `capabilities`. Each check has exactly:

```text
name, purpose, argv, cwd, env, runner, cases, ignored_cases
```

Use the actual completed source suite inventory, not a global count floor.
`cases` is the complete expected identity list; Rust includes runner-group
prefixes from rust_cases, Node uses node_cases, unittest uses the printed
`test_name (module.Class.test_name)` identities. Ignored Rust cases must be
explicitly inventoried and never counted as passed. Capabilities map to actual
check names and must cover the required matching purposes.

Runners: `rust`, `node`, `browser`, `unittest`, `build` (release Cargo build),
`command` (fmt/clippy only), `aux-build` (Cargo JSON compiler-artifact output,
not a test pass). Required purposes are the six Ark/UI purposes plus
readiness-tests, native-gate-tests, ladder-tests and qa-consumer-tests.
Native tests select spc-cpu-miner; readiness selects lab/txd; ladder selects
pool/store/accounting. No --list/--ignored/--no-run can stand in for a test.
Two additional mandatory `command` purposes, `verifier-fmt` and `verifier-clippy`,
run from `testing/lab-ark-funded/verifier` with exact argv:
`cargo fmt --all -- --check` and
`cargo clippy --offline --locked --all-targets -- -D warnings`.
These remain actual source-bound static gates; the six crypto cases come from
the verifier's actual Nix doCheck log.

Portable values use `@source`, `@build`, `@installed`, `@evidence`, or an entire
`@artifact/CHECK_NAME/TARGET_NAME` value resolved from an earlier actual Cargo
compiler-artifact event. Browser env supplies its built server/actor helpers;
the runner itself binds LIVE_UI_CLI_BIN to the installed lab and its output to
the per-check browser directory. CHROME and SPC_LAB_RBITCOIN_NODE come from Nix.
Required real readiness tests must retain their actual proof output in that
check's evidence directory. A merely env-skipped successful test is not live
propagation evidence.

## QA runtime source and verifier API

`source_package.py` hash-verifies the committed r21 `portable_source.py` and
invokes its actual `--repo REPO snapshot --out OUT` interface. The reviewed
manifest is 41dcb01b9f75ef6605773d5d2467c17024d768cf72b5605abdb135e0dca0494f;
the archive is 59a0f849d7697b915a940855093709df39aed2c7ed4ca8894b30aecc1f282f8c.
The 123-file / 1,456,511-byte manifest uses recipe `spc.qa.source-tree/1`, sorted
paths, Git modes and zero-time tar/gzip metadata. There is no source-root field.
The `qa-source.json` references are relative, confined to its directory and
fully hash-checked; modes and bytes are checked before read-only materialization.
The shared live runner requires `t3-controller-r21` and retains the same
audit_live / one final_drain / final_audit_after_drain / close / AP5 reap APIs.
Changed QA bytes or modes require a reviewed source profile, not an arbitrary
manifest rehash. Legacy r18-r20 intake remains only for historical replay.

Fresh verifier descriptor: `spc.lab.ap5.verifier-reproduction/v1`, with exactly
`schema,path,sha256,nix_binding,test_log`. Binding schema is
`spc.lab.ap5.verifier-nix-binding/v1`. AP5 inspects the actual Nix deriver/source,
requires doCheck, matches the exact nine-file QA source policy a9c36c67, verifies
the 5330 executable and ac24 installed/source lock, and retains actual `nix log`
bytes with all six exact unconditional crypto cases. Assembly executes two
public CLI checks; AP5 admission re-executes them and compares their full output.
QA's actual r21 `reproduced_verifier.admit` consumes this descriptor directly.
No parent descriptor or historical private receipt is copied. Source/feature
qualification must also include the actual formatter/Clippy gates.

The exact verifier `LOCK.md` is retained as a hash-bound historical source file.
Later successful-build notes are preserved in `provenance/verifier-development-lock.md`.
Neither note is a fresh-machine operational prerequisite. Metadata interoperability
was executed against the existing approved Nix output and actual Nix log, with
fresh CLI checks; no new compilation or full-lab acceptance is claimed.

QA also owns its per-run scenario/input provisioning interface. Use newly
generated private collector/runtime credentials and this build's binaries,
source manifest and `qa_path_env` from outputs.json. The public pool/collector
keys must match the real supported renderer; no fixture authority or private
old input file is a fresh setup substitute. AP5's canonical assembler does not
invent these domain configuration facts.

## Verification and limits

This agent ran only affected Python portability/metadata tests and Nix syntax
parsing. No heavy build, Git index operation, commit, push, node RPC or live
campaign was run. The exact integration file manifest and receipts are in the
immutable AP5 portability handoff. Root-flake exports, published-source recipe
integration, final source/gate inventory and fresh complete execution are
primary/owner integration work, not claims of an already qualified release.
