# Self-hosted Ark infrastructure

Build from this repository with Nix:

```sh
nix build .#captaind .#spc-lab-self-hosted-ark
nix run .#spc-lab-self-hosted-ark -- \
  --run-dir /tmp/opencode/ark-lab-1 \
  --base-revision <source-revision> --dirty-tree-hash <frozen-source-hash> \
  --listen 127.0.0.1:0 --operator-ui --duration-secs 120
```

Use a fresh run directory. The Nix launcher supplies the stock pinned
captaind, protobuf-aware grpcurl, Ark-enabled pool, lab, scanner, miner tooling,
Postgres and Redis, plus the separately packaged opt-in `rbitcoin-node-ark-lab`.
The node package binds the 2106-file frozen AP1b plus metadata source with its
actual NAR hash, checks its RPC/CLI tests, and never replaces the default L2
node. This unpublished source snapshot must be available in the lab source
cache described by `nix/lab/ark-node.nix`. Consult the implementation receipt
for its actual build and canonical same-chain profile-binding results; an
evaluated derivation is not a tested binary. No separate Core chain is used.

The initial scenario intentionally has no miners or user wallets. It initializes
the provider and two independent pool wallets. Real wallet funding destinations
and observations are private artifacts; `115000000` is each pool's float target,
not its balance. Native funding, boarding and AP-3b registration/receive/refresh/
offboard must be integrated before claiming the four-user funded profile.

The hub remains operator-only for this expanded-access scenario. The run's
operator claim is under `private/access/`; never publish its token. Ark metadata
in inventory is explicitly a startup observation, independent of current process
state. Normal duration/stop and `spc-lab reap --run-dir ...` use owned identities.

## Lifecycle test

The test driver uses Python 3.11 or newer (stdlib `tomllib`); use a Nix Python
on hosts whose system Python is older.

`infrastructure.py` runs actual initialization, graceful duration-stop, an
intentional pool-process startup failure, and controller SIGKILL/reap. It never
funds wallets or generates blocks. Supply a JSON map with exact executable paths:

`lab`, `node`, `pool`, `miner`, `scanner`, `captaind`, `grpcurl`,
`postgres-bin-dir`, `redis-server`, `redis-cli`.

```sh
python3 testing/lab-ark/infrastructure.py \
  --binaries /tmp/opencode/ark-infrastructure-binaries.json \
  --scenario crates/spc-lab/scenarios/lab-ark-infrastructure.json \
  --output /tmp/opencode/ark-proof-1 \
  --base-revision <source-revision> --source-hash <frozen-source-hash>
```

Coordinate the shared heavy-build lock and disk budget before builds/tests.
Keep test output paths short enough for PostgreSQL Unix-domain sockets.
Receipts and private logs are retained for diagnosis. Consult
`docs/reviews/2026-11-08-self-hosted-ark-infrastructure.md` for the actual tested
paths, results, source hashes and remaining dependencies.

## Packaged release verification

`release.py` invokes the hash-closed canonical r4 probe package for default and
explicit bark-v31 profiles, then runs the actual all-store-path launcher once
with a fresh bounded run. `--canonical-from` can reuse completed reports only
when their node/helper/contract bindings match. It records actual node argv,
ENV, executable and RPC facts; it preserves canonical rejection of the existing
engine's colon-containing ownership token rather than pretending a J2 engine
gate exists. The full consumer-model null diagnostic and funded-unproven state
are retained.

The completed immutable launcher and exact replay expression, including all
outputs and hashes, are listed in the final completion section of the receipt.
`/tmp/opencode/self-hosted-ark-packet/launch-bindings.nix` reuses the tested lab
tools and reproduces the tested launcher without recompiling unchanged Rust.
