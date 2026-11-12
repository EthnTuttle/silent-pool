# v0.9 release validation contract

## Ownership and result semantics

This directory is an independent release-validation packet. It does not modify
rbitcoin, the pool, miner, scanner, namespace runtime, or either integration
worktree. The default entry point is `v09_release.py`.

There are two input modes:

- `--release-check` requires exact, pass-local approval bindings for the new
  v0.9 source aggregate **and** the namespace full aggregate, in addition to
  the older approved components. It never emits a deployment approval.
- Without `--release-check`, a diagnostic may collect evidence with those two
  approvals absent. Its output remains provisional.

`release_checks_passed` means the selected live predicates passed with all
required inputs bound. `provisional_pass` is diagnostic evidence only.
`incomplete` has at least one explicitly unrun check. `unknown` means a timeout
or other inconclusive observation. A missing field or record is never turned
into a pass or a product failure without an actual counterexample.

The LP-live driver remains unmodified and is invoked as a child process. Its
internal retarget acceptance pin belongs to the **old** L2 source, so every new
node run deliberately uses LP-live's `--provisional` mode. The outer gate, not a
parser override, verifies the new source/binary/approval tuple before and after
execution. Baseline LP-live can additionally consume the new node manifest
itself; retarget source verification remains in the outer gate because of the
old pin. No global `APPROVE` substring search or old-approval inheritance is
used. Every APPROVE pass has exactly one unambiguous full candidate hash (the
pre-existing runtime code/full pair is treated only as its explicitly recorded
compound binding). Conflicting declarations fail closed, fenced verdicts are
ignored, and REQUEST_CHANGES remains a broad permanent veto.

## Fixed input and coverage contract

`contract.json` binds:

- official release `9614b27ecd832a04070a5226da26c8844284f2c4`;
- old L2 reference `d2f37b9113e8855245866289ed393043aed6e6153e8101375072cbb23624d499`,
  used only as an arithmetic/semantic reference;
- approved LP-live `aa3b37b67960071ec6bd4f09c7cd0ac83e1b526404231994c42dc331dc44ded2`;
- exact default and retarget `getblockchaininfo.consensus` values;
- all seven negative profile cases and the old real-clock min-difficulty
  exception at height 1;
- namespace full/code aggregates and all four Nix binary hashes;
- static stock source excerpts, their hashes and the source-only schema-26
  observation; and
- fee-zero, pre-ladder, Ark, witness, interoperability, causal-rate and other
  non-claims.

`candidate.example.json` is an intentionally incomplete input template. The
parent handoff must supply the exact frozen source manifest, source aggregate,
node executable/hash, build/log association and approval record. A separate
stock-node executable may also be supplied. If it is absent, stock source
absence checks still run, but the stock CLI/RPC probe is explicitly `not_run`
and the profile stage is `incomplete`.

The four stages are `profile`, `baseline`, `retarget`, and `namespace`.
Baseline and retarget use the approved LP-live driver and oracle. Retarget also
replays the fresh raw artifacts into a separate derived directory. Namespace
uses the frozen runtime scenario and immutable Nix project binaries. Every
source and binary is re-hashed after the run.

## Exact commands used

Run from `/home/ethan/.local/share/opencode/worktree/ad8886/lab-v09-validation`
with `PYTHONDONTWRITEBYTECODE=1` and a private umask:

```sh
umask 077
python3 testing/lab-release/v09_release.py --check-contract
python3 testing/lab-release/v09_release.py --static-audit /tmp/opencode/v09-audit
python3 testing/lab-release/test_release.py
python3 testing/lab-release/v09_release.py \
  --inputs /tmp/opencode/LP-v09-candidate-input.json \
  --release-check --check-input
```

Successful v0.9 profile probes (overall `incomplete` only because no separate
stock executable was supplied):

```sh
python3 testing/lab-release/v09_release.py \
  --inputs /tmp/opencode/LP-v09-candidate-input.json \
  --release-check --stage profile \
  --run-dir /tmp/opencode/LP-v09-live-profile-r3 \
  > /tmp/opencode/LP-v09-live-profile-r3.console.log 2>&1
```

Successful real native-SV2 baseline:

```sh
python3 testing/lab-release/v09_release.py \
  --inputs /tmp/opencode/LP-v09-candidate-input.json \
  --release-check --stage baseline \
  --run-dir /tmp/opencode/LP-v09-live-baseline-r2 \
  > /tmp/opencode/LP-v09-live-baseline-r2.console.log 2>&1
```

Successful three-boundary real-work retarget and independent replay:

```sh
python3 testing/lab-release/v09_release.py \
  --inputs /tmp/opencode/LP-v09-candidate-input.json \
  --release-check --stage retarget \
  --run-dir /tmp/opencode/LP-v09-live-retarget-r2 \
  > /tmp/opencode/LP-v09-live-retarget-r2.console.log 2>&1
```

Successful real two-node/two-pool namespace partition and heal:

```sh
python3 testing/lab-release/v09_release.py \
  --inputs /tmp/opencode/LP-v09-candidate-input.json \
  --release-check --stage namespace \
  --run-dir /tmp/opencode/LP-v09-live-namespace-r2 \
  > /tmp/opencode/LP-v09-live-namespace-r2.console.log 2>&1
```

For a new reproduction, allocate a new empty `/tmp/opencode/LP-v09-live-*` or
`/tmp/opencode/LP-v09-r2-*` directory instead of reusing any path above. The
driver refuses nonempty run directories. Approved-input sources and executables
must remain frozen.

## R2 read-only replay and stock/profile predicate checks

No baseline, retarget, or namespace live stage is re-mined for the Fable G1/G2
follow-up. The exact new-node chain replay runs without a live process:

```sh
python3 testing/lab-release/namespace_replay.py   --run-dir /tmp/opencode/LP-v09-live-namespace-r2-rt   --output /tmp/opencode/LP-v09-r2-namespace-replay/evidence.json   > /tmp/opencode/LP-v09-r2-namespace-replay.log 2>&1
```

This requires paused common ancestry, raw-header parent descent on each node's
own partition-window chain, a divergent hash at the common pre-heal height, and
a post-heal displacement anchored on the surviving chain. Missing ancestry or
raw observations return `unknown`, not success. The self-contained structural
fixture is `testing/lab-release/fixtures/namespace-v09-structure.json`.

Only the stock/profile path was rerun after the N7 refusal-predicate change:

```sh
python3 testing/lab-release/v09_release.py   --inputs /tmp/opencode/LP-v09-r2-profile-stock-input.json   --release-check --stage profile   --run-dir /tmp/opencode/LP-v09-r2-profile-stock-r1   > /tmp/opencode/LP-v09-r2-profile-stock-r1.log 2>&1
```

The stock profile now requires refusal *and* stock help omission of
`--retarget-regtest` plus a recognized parser reason. The successful r2 summary
and derived replay include input hashes; earlier summaries and raw run trees
were not overwritten.

Final static checks use:

```sh
PYTHONDONTWRITEBYTECODE=1 nix develop -c python3 testing/lab-release/test_release.py
python3 testing/lab-release/v09_release.py --check-contract
python3 testing/lab-release/v09_release.py --static-audit /tmp/opencode/v09-audit
python3 testing/lab-release/v09_release.py --inputs /tmp/opencode/LP-v09-candidate-input.json --release-check --check-input
```

## Artifact and cleanup rules

- Fresh datadirs for every run; no prior-version datadir compatibility claim.
- Run directories `0700`, files `0600`, private generated credentials/cookies.
- Unique outer `/tmp/opencode/LP-v09-live-*` or r2-scoped `/tmp/opencode/LP-v09-r2-*` paths. Approved LP-live and runtime
  destinations use short sibling paths so PostgreSQL's 107-byte Unix socket
  limit is respected.
- Direct children carry a random run token. Cleanup requires both the kernel
  PID start-time token and that run token, tolerating normal exec transitions.
  No process is signalled by PID alone.
- Runtime `reap` is scoped to the newly owned runtime run directory. Namespace
  fault evidence is host-tool evidence; project binaries and store paths are
  still immutable Nix inputs.
- Measured mining rejects every `setmocktime` or `generate*` node API call.
- Failed, partial and successful artifacts are retained. Temporary unit-test
  fixtures are the only automatically removed directories.

See `docs/reviews/2026-11-06-lab-v09-validation.md` for actual observations,
hashes and limitations.
