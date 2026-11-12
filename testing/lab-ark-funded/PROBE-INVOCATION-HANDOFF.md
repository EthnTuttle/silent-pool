# Exact probe invocation handoff to primary, via parent

Status: extracted from the successful r2 probe's retained artifacts. No probe,
node, runtime or compiler was rerun for this handoff. The 69-test source packet,
three canonical checks and three actual run archives remain immutable.

## Resolved tuple

Wrapper reference (provenance only, not the executable passed to supervisor):

```text
/nix/store/c5gh85rs220kc6g0yg9v2yk2nqd7szkx-spc-ark-canonical-probe-r4/bin/ark-rpc-probe
sha256 3d43a195999bf2cabe5af030559cae04cfa83089e3a24cc6c186ee9ca0e58d57
```

Actual native ELF interpreter:

```text
/nix/store/xahxrjs30p3mw0a42bqg9ikgkikskw8m-python3-3.14.8/bin/python3.14
sha256 5c4a05f602b2e901adf6065899af4659229fb7e1290d6da3c3eb8f07e9a1f4a5
```

Script (the only prefix argument):

```text
/nix/store/c5gh85rs220kc6g0yg9v2yk2nqd7szkx-spc-ark-canonical-probe-r4/share/ark-rpc/probe.py
sha256 833288d7b4a9491ff9cfa4d7186328f38f04e7fe7e5ef1c135496b5f9a62754a
```

The supported runtime flags are exactly:

```text
--traffic-probe /nix/store/xahxrjs30p3mw0a42bqg9ikgkikskw8m-python3-3.14.8/bin/python3.14
--traffic-probe-arg /nix/store/c5gh85rs220kc6g0yg9v2yk2nqd7szkx-spc-ark-canonical-probe-r4/share/ark-rpc/probe.py
```

Do **not** put `node-probe` in the prefix list: `engine/traffic_runtime.rs`
appends it, followed by binary/source/profile/owned-endpoint/actor-exec/parent
identity/output flags. Exact observed argv is in the retained baseline receipt.
`node-probe --launch-profile` dispatches `cmd_profile_node_probe`, which calls
`launchprofile.normalize()` before starting its owned node. There is no separate
strict-normalizer executable argument to add. `ark-lab-profile-check` is a
different wrapper for `lifecycle_fixture.py`; it is not this probe.

The first attempt failed with: `adapter program .../ark-rpc-probe is an
executable wrapper script; refusing to launch an unverified exec target`.
Adding a subcommand to that shell wrapper would not fix the refusal. Using the
supported native interpreter/prefix interface preserves the supervisor check.

## Minimal Nix launcher / assembler correction

The g105 launcher does not pass any traffic-probe flags at all (it only accepts
caller arguments via `"$@"`). Its `nix/lab/ark-runtime.nix` argument set also has
no probe parameter. The minimal source delta is supplied in
`probe-launcher-correction.patch`; it is a proposal, not an edit to primary.
It imports the existing pinned `ark-probe.nix` with the same `pkgs`, passes that
output to the launcher, and uses `${pkgs.python3}/bin/python3` plus the frozen
script path. Runtime canonicalizes the interpreter symlink to the ELF above;
the assembler must record that resolved target. No new probe implementation,
normalizer mode, native hashing helper or server API is needed.

Keep `binaries.probe` as the hash-bound wrapper reference if needed for existing
bundle consumers, and add a **required** `probe_invocation` object:

```text
schema: spc.lab.ap5.probe-invocation/v1
wrapper: {path, sha256}              # equals binaries.probe
program: {path, sha256}              # canonical native ELF path
script: {path, sha256}               # canonical same-package probe.py
prefix_args: [script.path]           # no extra arguments
runtime_subcommand: node-probe
source_manifest: {path, sha256}      # schema/root/complete source-file map
```

Exact ready-to-copy object:
`/tmp/opencode/AP5-probe-and-pid-handoff-r1/probe-invocation.json`, SHA256
`d01b43be03626bd6e7750c126919650df306b59de82d07b409ff1e2179af017c`.
Its source manifest has schema `spc.lab.ap5.probe-source/v1`, the immutable
`share/ark-rpc` root and all 14 source file hashes copied from the recorded
probe manifest and checked against the installed source. The assembler may
copy that manifest into the new bundle directory and update its artifact path;
its bytes/hash stay unchanged.

The harness now requires this tuple in canonical admission, verifies native
target/script/source hashes and exact prefix semantics, emits the same tuple
in `probe-invocation`, and launches only those declared args. Archive replay
compares bundle, event and launch argv. A wrapper-only new bundle is refused;
the historical g105 bundle and earlier successful admission/replay receipts
are preserved under their original 69-test source packet, not rewritten.

## Retained positive baseline and scope

Baseline: `/tmp/opencode/AP5-native-confirmed-live-r2/run/private/traffic/probe`.
Contract hash:
`754e1e6131955f9e5bbce7ef81b6f9aabaa2f1379ec6409f1c3a7a9e88be91f9`.
Comparison profile hash:
`0d36f9e3ad4e9d341abfbe7a674d585cfa38ad9e296cb3049a137fadcc46c739`.
Node exited 0 and was reaped. The actor-exec helper hash is `70d775b3...`, with
two parent-death levels and finalized-no-inheritance environment.

`probe-baseline-receipt.json` in the handoff retains artifact hashes, the exact
argv and profile facts. This was a read-only Bark surface-compatible result,
not funded or native-audit acceptance. The retained report also records a
consumer-model shape failure for `getmempoolinfo.maxdatacarriersize` (null where
the consumer model expects an integer); that fact is preserved, not hidden or
retested. It does not change the recorded probe exit or upgrade its scope.
