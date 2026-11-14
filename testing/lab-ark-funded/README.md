# AP5 native-funded Ark controller

AP5 observes the real runtime, pool/wallet lifecycle and public proofs. It does
not supply another wallet signer, miner, Ark server or historical oracle. One
shared owner performs the live work, final audit/drain and stop/reap sequence.
Full acceptance requires the actual native-funded campaign verdict; a build,
source contract, prepared plan or served UI is not that verdict.

## Fresh-machine packages and release selection

Use x86_64 Linux, Nix with flakes/network access, Git and Python 3, and an
ordinary non-root user for PostgreSQL. Follow `AGENTS.md`'s build disk budget
(>=50 GiB available, projected >=20 GiB residual, one heavy build at a time).

The application reference documented here is the published immutable commit
`98f0e6caa2482646e3c7073949f6388cd39f200b`. Its runtime artifacts remain independent
of the separately released QA/AP5 Python consumer. Select a published, reviewed
compatible consumer commit and its matching QA descriptor. The published
reviewed compatible consumer is `ce7b703ac0c68e6738e5f4510f056cb1209fa982`:
its shipped shared runner expects exactly the `t3-controller-r23` QA
descriptor, packaged as `ap5-qa-source-r23`. The application pin remains
`98f0e6caa2482646e3c7073949f6388cd39f200b`; updating the consumer does not
rebuild or reselect the application.

| Application package | Contents |
|---|---|
| `spc-lab-ap5-qualified` | Application build with its committed exact gate plan executed against installed programs. |
| `ap5-inputs` | Actual Nix output paths for canonical bundle assembly, including the qualified application, node, stock Ark server, probe and verifier. |
| `ap5-vtxo-verifier` | Source/lock-pinned public-proof verifier and unconditional crypto tests. |

| Consumer package | Contents |
|---|---|
| `ap5-tools` | Immutable Python entrypoints: `portable`, `ap5`, `shared`, `replay`. |
| `ap5-qa-source` | The matching relocatable, hash-bound QA descriptor/source archive. |

The committed `source-contract.json` describes source and gates, not an already
executed pass. Nix's qualification step executes the declared checks; bundle
admission checks their actual source/program/log bindings. A qualified bundle
still needs the live campaign. See [PORTABLE-INTEGRATION.md](PORTABLE-INTEGRATION.md).

## Build and admit from immutable published sources

Use a fresh HOME-local workspace. The following commands work from a fresh
checkout or any directory; the flake URLs, rather than ambient checkout state,
select the sources. If using the workspace from LAB-RUNTIME section 2, keep its
`APP_REV`, `APP_REF` and `WORK` and start at the consumer selection below.

```sh
APP_REV=98f0e6caa2482646e3c7073949f6388cd39f200b
APP_REF="github:EthnTuttle/silent-pool/$APP_REV"
umask 077
WORK="$(cd "$HOME" && pwd -P)/.spc-lab/$(date -u +%Y%m%dT%H%M%SZ)-$$"
mkdir -p "$(dirname "$WORK")"
mkdir "$WORK"
export WORK
```

Use a short canonical path. Preparation rejects a collector Unix-socket path of
108 bytes or more; with an unusually long HOME, choose a shorter canonical
user-owned workspace before running it. Preserve old run roots and use a new
workspace for a new experiment.
The corrected preflight also checks native-wallet, pool-PostgreSQL and
Ark-PostgreSQL socket paths, including PostgreSQL's appended port suffix.
See [N14A-ARK-POSTGRES-PATH-COUNTEREXAMPLE.md](N14A-ARK-POSTGRES-PATH-COUNTEREXAMPLE.md).
Assembly receipts may use a longer durable path than the actual run root.

Set `CONSUMER_REV` to the published 40-hex commit chosen for this campaign.
It is intentionally required rather than silently selecting a moving branch,
the application commit, or an unpublished future QA release. For the
`t3-controller-r23` descriptor that commit is
`ce7b703ac0c68e6738e5f4510f056cb1209fa982`.

```sh
: "${CONSUMER_REV:?Set the published reviewed compatible QA/AP5 consumer commit}"
[[ "$CONSUMER_REV" =~ ^[0-9a-f]{40}$ ]] || { echo 'CONSUMER_REV must be a full commit' >&2; exit 1; }
CONSUMER_REF="github:EthnTuttle/silent-pool/$CONSUMER_REV"
nix build --max-jobs 1 "$APP_REF#ap5-inputs" --out-link "$WORK/runtime-inputs"
nix build --max-jobs 1 "$CONSUMER_REF#ap5-tools" --out-link "$WORK/consumer-tools"
nix build "$CONSUMER_REF#ap5-qa-source" --out-link "$WORK/qa-source"
nix run "$CONSUMER_REF#ap5-tools" -- portable assemble \
  --outputs "$WORK/runtime-inputs/outputs.json" --out "$WORK/qualified"
QA_SOURCE="$WORK/qa-source/qa-source.json"
QA_SHA=$(nix hash file --type sha256 --base16 "$QA_SOURCE")
```

Continue only when qualification and assembly succeed. Assembly emits
`$WORK/qualified/bundle.json` using actual Nix derivers, committed source rows,
executed checks, source policies and probe/verifier bindings. It does not use
developer worktrees, prebuilt local binaries or historical private receipts.
The node Git source is pinned in the application flake; archive export-subst is
not a replacement for that Git-fetch recipe. Reuse already realized outputs for
the same application pin when updating only the compatible consumer release.

## Prepare the two-node native/traffic profile

Choose a unique run ID and integer seed, then generate fresh inputs:

```sh
RUN_ID="lab-$(date -u +%Y%m%dT%H%M%SZ)-$$"
RUN_SEED=$(date -u +%s)
nix run "$CONSUMER_REF#ap5-tools" -- portable prepare-shared \
  --consumer-tools "$WORK/consumer-tools/bin/ap5-tools" \
  --bundle "$WORK/qualified/bundle.json" \
  --outputs "$WORK/runtime-inputs/outputs.json" \
  --qa-source "$QA_SOURCE" --qa-source-sha256 "$QA_SHA" \
  --out "$WORK/fresh-inputs" --run-out "$WORK/shared-run" \
  --run-id "$RUN_ID" --seed "$RUN_SEED"
```

Preparation writes `fresh-inputs/scenario.json`, `qa-inputs.json` and
`execution-plan.json`. It validates the scenario with the actual admitted CLI,
creates this run's private source-attestation inputs and binds immutable tools,
SQL checksums and public identity profiles. It does not start mining/Ark actors,
copy developer wallets, or claim a live result. Preserve any failed preparation
for diagnosis and choose a new workspace instead of overwriting private state.

The prepared profile is two nodes/two pools/four SP wallets/four CPU miners,
eight background and four distinct user payment wallets, 40 H/s aggregate and
0.2 ordinary tx/s. Funding uses the native bootstrap, canonical maturity and
allocation checks. Public witness fixtures remain explicitly test-only. The
one-node, 8 H/s `spc-lab-self-hosted-ark` wrapper has no traffic population and
is not this full profile; no second `--scenario` override is needed or supported
on that fixed wrapper.

## Run, open the operational UI and stop

```sh
nix run "$CONSUMER_REF#ap5-tools" -- shared \
  --bundle "$WORK/qualified/bundle.json" \
  --qa-source "$QA_SOURCE" --qa-source-sha256 "$QA_SHA" \
  --qa-inputs "$WORK/fresh-inputs/qa-inputs.json" \
  --scenario "$WORK/fresh-inputs/scenario.json" \
  --out "$WORK/shared-run" --timeout-secs 7200
```

This is a bounded campaign, not a keep-alive daemon command. The shared owner
starts one actual runtime, supplies its explicit source/private inputs and
performs the finalization sequence. It owns the operational URL/run directory;
AP5 records them in its `shared-runtime-binding` evidence. Standalone AP5 also
records `spc-lab-runtime: http=...` in `controller.log`. Use those actual paths
and URL rather than assuming `<out>/run` for shared mode.

Use fresh persistent directories for long runs, for example beneath
`${XDG_STATE_HOME:-$HOME/.local/state}/spc-ap5`, and keep the path short enough
for the collector Unix sockets. Temporary test directories are not a durable
archive across server restarts. An absent formerly used run path is not a new
run: retain its historical outcome and choose a new run ID and output root.

After successful startup the shared runner prints a `spc.lab.ap5.run-info/v1`
JSON record and writes `run-info.json` in the selected output root. Runtime
identity, listen argv, run-bound API snapshot and the actual controls/lab HTML
routes are checked before URLs are published. Listener ports come from the
actual manifest. The record gives the private operator-claim file location,
never its token. It is a startup observation, not a promise that those ports
remain live after teardown. Final `run-result.json` and its printed JSON report
the actual QA/component/cleanup statuses and combined exit code.

Open the controller's `/` for worker/wallet controls or `/lab` for the operator
directory. It is live only after startup succeeds and only while that owner is
running. Enter the appropriate token from that run's local 0600
`private/access/operator.json` or user claim file; see
[LAB-RUNTIME sections 5-6](../../docs/LAB-RUNTIME.md) for exact handling and
live-controller API stop. Pool operator pages are separately linked when served.
The design-review/mockup utility on port 18481 is unrelated to this runtime and
must not be stopped or treated as campaign evidence.

Normal campaign completion performs live audit, one source-feed final drain,
final audit after drain, close, and one AP5 stop/reap plus independent cleanup
assessment. SIGINT/SIGTERM request the coordinator's cancellation/finalization
path; wait for its result. Manual API stop or interruption is not a successful
full-campaign verdict. A concurrent `spc-lab command --run-dir` cannot take a
live controller's ownership lock. Recovery `reap` applies after the owner exits
and only to that run's persisted process/resource identities.

The QA source descriptor is relocatable and hash-bound to its selected release.
Its actual manifest determines the file inventory; historical r21 file counts
are not a requirement for a newer compatible descriptor. Check the execution
and final cleanup receipts. Full funded acceptance remains unclaimed until the
complete real campaign produces its verdict. Neither this guide nor
`docs/lab-build-report.html` substitutes for it.

[HISTORICAL-README.md](HISTORICAL-README.md) and dated reports retain earlier
developer evidence as history, not fresh-machine prerequisites.
