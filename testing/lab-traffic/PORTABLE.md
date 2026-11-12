# Fresh-checkout QA reproduction

Supported execution: Linux x86_64, Nix, Python 3.11 or newer (the qualified
development environment currently supplies Python 3.14), procfs and the
existing host IPv4 lab profile. No macOS/Windows or namespace readiness claim.
The node, stock Ark server and native tools are built from the repository's
pinned sources. A new machine needs no historical reviewer home directory,
private run, copied SQL database, frozen `/tmp` packet or old check log.

## Source checks and portable QA descriptor

Keep build/run/check outputs outside the checkout. For example:

```sh
export WORK="$HOME/.cache/silent-pool-qa"
mkdir -p "$WORK"
chmod 700 "$WORK"
export TMPDIR="$WORK/tmp"
mkdir -p "$TMPDIR"
chmod 700 "$TMPDIR"
export CARGO_TARGET_DIR="$WORK/target"

nix develop --command python3 -B testing/lab-traffic/portable_source.py \
  record-check --out "$WORK/python-check" --purpose independent-QA -- \
  python3 -B -m unittest discover -s testing/lab-traffic -p 'test_*.py' -v

nix develop --command python3 -B testing/lab-traffic/portable_source.py \
  snapshot --out "$WORK/qa-source"
```

Output directories must be fresh. Tests use `TMPDIR`; public fixture paths are
relative to the checked-out test files. Historical archive replayers require
an explicitly supplied archive and are optional, not startup prerequisites.

`portable_source.py` creates `source.tar.gz`, `manifest.json`, `qa-source.json`.
The descriptor has schema `spc.lab.ap5.qa-source/v1`, API `t3-controller-r21`;
its artifact paths resolve relative to the descriptor directory. Its printed
SHA256 is the external descriptor pin supplied to AP5. The versioned snapshot
recipe sorts all source paths, validates regular files/bytes/SHA256, preserves
Git's executable bit, normalizes private/read-only filesystem modes to Git
0644/0755, and writes zero-mtime/zero-owner deterministic tar/gzip metadata.
It excludes build/cache/private directories. Archive modes/bytes/hashes must
be checked by the AP5 loader; relative paths are not permission to traverse out
of the descriptor root. Source modules must be loaded from this one bound tree.

## Produce actual build/check inventory

Apply `AGENTS.md`'s disk budget and one-heavy-build lock before heavy commands.
Use the shared lock selected by the current build owner, not another parallel
build tree. These commands are for the fresh-machine operator's build turn:

```sh
nix build .#spc-lab-ark-tools --out-link "$WORK/lab-tools"
nix develop --command python3 -B testing/lab-traffic/portable_source.py \
  record-check --out "$WORK/pool-ark-check" --purpose pool-ark-tests -- \
  cargo test --locked -p spc-pool --features ark --all-targets
```

Use the producer's required selectors/environment for the remaining suites,
including real PG/Redis, native lifecycle/joint, readiness, UI build/browser,
node profile and stock-Ark checks. A harness-reported pass that skipped an
environment-gated case is not an executed case. Record each actual command and
log at its source state; do not invent a fixed total or reuse another run's pass.
`record-check` records the executable hash, exit/timeout, complete log hash and
source before/after equality. It is an execution receipt, NOT an approval tool.

The primary/AP5-owned canonical bundle assembler must consume those new
receipts, query each installed package's real Nix deriver/source, inventory the
actual binaries and interpreter/probe tuple, and emit
`spc.lab.ap5.bundle/v1` plus the QA input JSON. The same six application aliases
must bind the tested joined source. The old review-manifest script and old
private packet paths are not a fresh-build recipe. Porting/publishing this
canonical assembler and final merged source policy remains a primary-owned
integration gate; this QA intake does not replace it with a mirror assembler.

For the existing approved VTXO verifier, `CompiledVerifier` additionally accepts
`spc.lab.ap5.verifier-reproduction/v1` with closed fields `schema`, `path`,
`sha256`, `nix_binding:{path,sha256}`, `test_log:{path,sha256}`. Rebuild the AP5
owner's pinned nine-file verifier source package using its Nix recipe. Capture
`nix log <actual-deriver>` into the supplied test_log file. This profile checks
the same approved binary SHA256 and exact public source policy, actual deriver,
source/output binding and the six real crypto cases against the derivation log.
It needs no old machine-specific receipts. A different binary/source still needs
explicit review; a `compiled` or `passed` boolean is insufficient.

## One shared funded run

After the complete joined bundle and source policies are actually qualified,
use the AP5 owner's portable `shared_run.py` entrypoint with explicit artifacts:

```sh
python3 -B testing/lab-ark-funded/shared_run.py \
  --bundle "$WORK/bundle.json" --qa-inputs "$WORK/qa-inputs.json" \
  --scenario "$WORK/shared-scenario.json" \
  --qa-source "$WORK/qa-source/qa-source.json" \
  --qa-source-sha256 "<printed descriptor SHA256>" \
  --out "$WORK/fresh-shared-run" --timeout-secs 7200
```

The shared scenario keeps two nodes/two pools/four actual wallet/native miners,
eight network/four payment wallets, aggregate 40 H/s and ordinary 0.2 tx/s. Use
the runner's real manual destination, witness/fault/reorg options; no case may
be omitted. Auxiliary bootstrap-stop/maturity cases remain fresh serial cases.
If separate T3 execution is intended, `campaign.py` is the serial three-case
entrypoint; do not run it concurrently with another funded coordinator.

The AP5 coordinator calls ONE `Controller.start()` and observes Runtime's
original bootstrap. Required T3/AP5 live work precedes `qa.audit_live()`, ONE
`source_feed_runtime.final_drain()`, and `qa.final_audit_after_drain()` before
collector close and the one AP5 stop/reap. The QA final audit issues no RPC,
SQL recapture, writer stop or additional hash authority. Cleanup and whole-lab
acceptance remain separately evaluated. The campaign lock defaults to a per-UID
file in XDG_RUNTIME_DIR or the system temp directory; all cooperating runners
must use the same `SPC_LAB_CAMPAIGN_LOCK` override when one is assigned.

Collector/scan secrets are generated/provisioned per new private run by the
owning runtime/wallet adapters. Never use public fixture keys. Keep files mode
0600 and parents 0700. Rendered collector sockets live under runtime/private,
with existing private parents. Provide `source_feed.sql_contract` equal to
`e5283a85ddf91f9b44bd9fe1a678afe9b84a6f35ce7d468356605338e0f79803`
only with that reviewed decision API source; the SourceFeed2 contract and
signature preimages remain unchanged. No current registration or later SQL
snapshot substitutes for an original historical decision/checkpoint.

## Remaining live gates

This source intake is not a funded pass. Final merged Runtime/native/wallet
source applicability and the portable canonical bundle recipe still need the
primary's completed tests/Nix binding. Readiness v3 requires the corrected
locked pre-HashStart transition and strict deadline equality checks; its
structural fixture is not admission. AP5 owns the one v3 parser/coordinator.
Actual SPCA receipt/escape captures, all-output wallet/Ark policy proofs,
current float, final prefix and exhaustive cleanup still need a qualified run.
Missing/truncated/unsupported evidence remains INCONCLUSIVE.
