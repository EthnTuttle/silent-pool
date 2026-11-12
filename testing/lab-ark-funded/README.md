# AP5 native-funded Ark controller

AP5 observes the real runtime, pool/wallet lifecycle and raw public proofs. It
does not supply another wallet signer, miner, Ark server or historical oracle.
The shared runner uses one QA Controller/runtime and one final drain/stop/reap
sequence. Full acceptance requires actual native funding, independent history,
current float, receipt/refresh/automatic second renewal/offboard and cleanup.

## Fresh-machine packages

The primary flake integrates [nix/packages.nix](nix/packages.nix), exposing:

| Attribute | Contents |
| --- | --- |
| `ap5-tools` | Nix Python entrypoints: `portable`, `ap5`, `shared`, `replay` |
| `ap5-vtxo-verifier` | Exact nine-file Rust verifier and public fixtures; six unconditional Nix crypto tests; assembly executes public CLI checks |
| `ap5-qa-source` | Relative, hash-pinned QA runtime source package generated from repository files |
| `spc-lab-ap5-qualified` | One joined application build with the committed exact gate plan executed against its installed programs |
| `ap5-inputs` | Machine-generated Nix output paths for canonical bundle assembly |

Supported production artifact target: **x86_64-linux**. Nix with flakes and
network access to the pinned public source repositories is required. Runtime
requires Linux `/proc`, PID-bound signaling and an ordinary non-root user for
PostgreSQL. A build must start with at least 50 GiB free and have a peak estimate
leaving at least 20 GiB. Use one heavy build at a time; do not garbage-collect
another run's referenced artifacts.

The verifier builds entirely from committed `verifier/Cargo.lock`, Rust sources,
`verifier/fixtures/stock-public.json` and the public pinned Bark dependency. No
live corpus or historical private receipt is needed. Its crypto code/lock and
expected executable hash remain checked. Changed binary bytes require review;
portability is not permission to accept a different verifier.

## Release preparation (primary/source owners)

The source contract is intentionally **pending** until the complete reviewed
source union and actual gate inventory are committed. It cannot grant a pass.
See [PORTABLE-INTEGRATION.md](PORTABLE-INTEGRATION.md) for exact generation and
flake integration. A source-bound contract is still not a tested capability:
the Nix install-check executes every declared command/case, rejects omissions,
and records the actual source, programs, logs and capability-to-check links.

Once the release preparation is complete, these are the minimal build/admission
commands. Replace `COMMIT` with the primary's published release commit:

```sh
REF=github:EthnTuttle/silent-pool/COMMIT
mkdir -m 700 .ap5
nix build --max-jobs 1 "$REF#ap5-inputs" --out-link .ap5/inputs
nix run "$REF#ap5-tools" -- portable assemble \
  --outputs "$PWD/.ap5/inputs/outputs.json" --out "$PWD/.ap5/qualified"
```

Assembly emits the existing `spc.lab.ap5.bundle/v1` envelope and invokes the
canonical admission validator. It derives source roots from actual Nix derivers,
checks every committed source row, executed gate/case, Ark/UI feature selection,
R4/aggregate lineage, actual node/server hashes and the probe's ELF/script and
14-runtime/24-fixture manifests. It does not reuse a developer's store path.

The current node source is published through Git at
`https://github.com/EthnTuttle/rbitcoin`, commit
`c1505776117b1d4d42489f309af33978bbe124cc`; use the primary's Git-fetch Nix recipe,
not an archive that applies export-subst. The verified source NAR and node binary
identity remain unchanged. The portable canonical probe is repository-owned;
its fresh installed program/source/fixture manifests are required.

## Run the shared campaign

QA supplies a **fresh per-run** scenario and input file using its supported
provisioning interface. Its binary/source references and immutable `path_env`
must come from these new Nix outputs, not a historical run. Collector authorities
and private files must be generated for that run; do not copy developer wallets,
seeds, cookies, grants or private captures. Runtime generates the local node,
server, wallet and access credentials in its new private tree.

```sh
QA_SOURCE=$(nix eval --raw "$REF#packages.x86_64-linux.ap5-qa-source.outPath")/qa-source.json
QA_SHA=$(nix hash file --type sha256 --base16 "$QA_SOURCE")
nix run "$REF#ap5-tools" -- shared \
  --bundle "$PWD/.ap5/qualified/bundle.json" \
  --qa-source "$QA_SOURCE" --qa-source-sha256 "$QA_SHA" \
  --qa-inputs /absolute/fresh-plan/qa-inputs.json \
  --scenario /absolute/fresh-plan/scenario.json \
  --out "$PWD/.ap5/shared-run" --timeout-secs 7200
```

The r21 source package is relocatable: `qa-source.json` refers to relative
`source.tar.gz` and `manifest.json`. QA's committed `portable_source.py snapshot`
generates the deterministic archive. Intake checks all 123 files / 1,456,511
bytes, Git modes, hashes and zero-time archive metadata before importing.
The roster stays two nodes/two pools/four SP wallets/four CPU
miners, eight background/four payment wallets, 40 H/s aggregate and 0.2 ordinary
tx/s. QA's explicitly requested witness fixture remains test-only, not a new
Ark wallet or hash authority.

Required order is live work, `qa.audit_live()`, one
`qa.source_feed_runtime.final_drain()`, `qa.final_audit_after_drain()` before
close, then one AP5 stop/reap and independent cleanup assessment. Missing or
ambiguous evidence remains failed/inconclusive. Global cleanup is not inferred
from a successful reaper.

## Current verification status

The portability code has lightweight source/relocation/admission-negative tests;
the new Nix recipes have **not been built by this agent**. The primary owns
commit/push and the actual full-source builds/gates. Actual r21 QA accepts the
`spc.lab.ap5.verifier-reproduction/v1` descriptor generated by AP5. Compatibility
was checked using the existing approved Nix binary, actual derivation/log and
new public CLI executions; this was not a fresh build. Release assembly must
generate its own descriptor from its actual output. Full funded execution
remains pending qualified complete inputs.

[HISTORICAL-README.md](HISTORICAL-README.md) and the dated reports preserve old
developer paths and receipts as history. They are not fresh-machine prerequisites.
