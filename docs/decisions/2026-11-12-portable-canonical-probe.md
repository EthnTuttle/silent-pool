# 2026-11-12 - Repository-owned portable canonical probe packaging

## Authority and provenance

Portability packet based on committed source
`471c4a02a63ad6b0228aa62f40d52f67f350f38f`. All 14 installed canonical files are
already present in `testing/ark-rpc`. Thirteen byte-match the deployed r4
package and original frozen producer. The committed lifecycle fixture matches
the frozen producer, while the deployed fixture additionally selects a source
root/label and supports the literal `--compat` option.

Historical installed 14-file authority: `spc.lab.ap5.probe-source/v1` for
`spc-ark-canonical-probe-r4`, recorded by the T3 runtime-admission r13 packet.
Historical original producer: `traffic-probe-profile-r4-manifest-full.json`,
contract `754e1e6131955f9e5bbce7ef81b6f9aabaa2f1379ec6409f1c3a7a9e88be91f9`.
Those source manifests and installed source bytes are preserved by hash in the
portability handoff. Historical reports are not rewritten as fresh evidence.

## Decision

- Default Nix source is repository-relative `../../testing/ark-rpc`, explicitly
  filtered to the 14 canonical files and 24 public JSON fixtures. No private
  staging path, absolute store output, cache or runtime data is a default input.
- Carry the deployed fixture's `--compat` hunk and resolve the canonical root
  from `Path(__file__).resolve().parent`. After the descriptive node-label
  substitution, this differs from the prior installed lifecycle fixture only
  in the root expression. Its parent identity and strict profile checks stay.
- Keep the wrapper/native ELF Python/script split. No wrapper injects
  `node-probe`; the runtime remains the sole appender for that invocation.
- Generate source/fixture manifests and the program tuple from actual build
  outputs. A fresh derivation requires fresh admission evidence even though
  the thin launcher still hashes to `833288d7...`. There is no contract-hash
  bump: the RPC contract, normalizer, schemas and 13 other runtime files are
  byte-unchanged. This is a source/deployment lineage change, not a new RPC
  approval or funded-compatibility result.
- Test temporary directories honor `TMPDIR` without a developer directory.
  Source-only devcheck references and historical public reports remain test
  inputs only; no Rust devcheck/node/server build is implied.

## Checks and scope

The packet verifies bounded Python unit/scenario tests, the exact contract
hash, local import closure and fixture paths from a fresh small copy with an
empty HOME and private developer paths forbidden. Lightweight Nix parsing and
evaluation of the source filter/install recipe are separate from a real Nix
build. A local recipe execution checks generated actual-output manifests and
native-Python bindings; it is not labeled a Nix derivation build.

The primary writer performs the later combined build/admission using those
actual output paths/hashes. Node, stock Ark server, T1 profile flags, runtime
subcommand handling and upstream pins are unchanged.
