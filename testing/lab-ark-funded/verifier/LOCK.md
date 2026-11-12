# Frozen verifier dependencies and deferred crypto gate

The checked-in Cargo.lock is generated and validated offline, not compiled.

- Parent workspace lock SHA256:
  `307a71fe5d279320a74ad06310d0bac79d9c7b4d4cb1a0b14af7e0940553d0c5`.
- Verifier lock SHA256:
  `ac24ee3b09c7c2dd8731417672b9d7b072aa6e9e04127b9c96e8ad80ae842833`.
- Actual `ark-lib`: **0.7.1**, Git revision
  `c419159fa54085572f8ca72d2ba1716556a9d3eb`.
- Direct registry dependencies: anyhow 1.0.104, bitcoin 0.32.102, hex 0.4.3,
  serde 1.0.229 and serde_json 1.0.154, exactly as already locked by the
  current project. All checksums/source identities are retained.
- Metadata tool: Nix Rust 1.96.1, Cargo executable SHA256
  `adf659cdb5bc344f523bd88df7acb8b1e7a5b6a995df1c89940a2c6bebd8fe30`.

Evidence is `/tmp/opencode/AP5-verifier-lock-r1/receipt.json`, with the frozen
parent lock, vendor-input identities, complete metadata output and commands.
The final packet includes these inputs. No project lock or upstream pin changed.

## Reproduction without resolution to new versions

`lock_seed.py seed` walks the exact parent-lock dependency graph from the six
direct dependencies. It copies that closure, not an old ark-lib entry or an
unrelated workspace root. There are 184 packages in the conservative seed.
Cargo metadata prunes unused feature edges/packages to a 91-package lock
(including the verifier); it does not add new versions.

The metadata operation used a private Cargo home with offline mode and
directory-source replacement. Its vendor directory contains read-only links
only to packages in the copied parent closure, from the existing vendor cache.
Neither an online registry nor a shared Cargo cache is written. The local Nix
Cargo/rustc paths are explicit. No build target directory was created.

```sh
python3 -B lock_seed.py seed --workspace-lock PARENT_LOCK \
  --workspace-sha256 307a71fe5d279320a74ad06310d0bac79d9c7b4d4cb1a0b14af7e0940553d0c5 \
  --lock FRESH_VERIFIER_COPY/Cargo.lock
# In the private, closure-only offline Cargo home:
cargo metadata --offline --format-version 1 --manifest-path FRESH_VERIFIER_COPY/Cargo.toml
python3 -B lock_seed.py audit --workspace-lock PARENT_LOCK \
  --workspace-sha256 307a71fe5d279320a74ad06310d0bac79d9c7b4d4cb1a0b14af7e0940553d0c5 \
  --lock FRESH_VERIFIER_COPY/Cargo.lock
cargo metadata --offline --locked --format-version 1 --manifest-path FRESH_VERIFIER_COPY/Cargo.toml
```

The audit rejects any new package/source/checksum or dependency edge. A second
fresh seed/normalization was actually run and produced byte-identical output.
The checked-in final lock passed `cargo metadata --offline --locked`.

## Public crypto vectors and later build slot

`freeze_stock_vectors.py` reads the exact c419159 Git object
`lib/src/test_util/vectors.rs`, SHA256
`7926e40e613fb9817d3710f78b34ae5d19eff6dc2aced25be25a9f37831417fa`.
It copies only eight public fields: six complete VTXOs and their two anchors.
It never copies the source fixture's private keys or invokes its generator.
Pinned Git, existing self-hosted source and vendored vector/validation bytes
were compared; they match. The corpus is `fixtures/stock-public.json`.

Six prepared Rust tests exercise:

1. An unconditional embedded signed-board positive with exact frozen public
   owner/server, full Pubkey policy, VTXO/anchor IDs and raw hashes. It requires
   nonempty signed transitions and production full signature validation.
2. Full signature validation for board, two rounds and two arkoor proofs.
3. A cryptographically valid HTLC whose U matches the expected owner but whose
   complete policy is not Pubkey.
4. Wrong expected owner/server.
5. First and terminal transition signature mutations that remain canonically
   decodable and pass `validate_unsigned`, but must fail full `validate` and
   the production verifier. This prevents a decoder-only test from passing.
6. Changed anchor input/output identities and truncated proof bytes.

These tests call the same production `verify()` function. They contain no
signing code, secret inputs, RPC calls or new proof generation. They have not
been compiled or run. `package.nix` now uses the frozen lock and has `doCheck`.
The parent must assign the later heavy gate before any Cargo/Nix compilation
or Rust test execution; the shared build lock, priority and disk budget apply.
Use `CARGO_INCREMENTAL=0` and the parent's existing topic target/cache when
assigned; do not create a new target for this source revision. The mandatory
test invocation is `cargo test --offline --locked --bin ap5-vtxo-verifier` in
this crate (or the equivalent Nix doCheck). None of these tests is ignored,
environment-gated, or dependent on an external archive.

The available project prototype fixture pins derivations, and AP3a's recovery
fixture explicitly contains synthetic IDs rather than full proofs. Neither
is used as a full crypto proof. The embedded stock corpus is labelled
`synthetic_signed_conformance`: its signatures are the frozen stock test
signatures, while its anchors are synthetic, not funded chain evidence.
All eight public byte strings remain identical to c419159 and the previous
AP5 freeze; only corpus metadata and test assertions were extended.

**A preserved live corpus is optional additional coverage, never a compilation
or offline crypto-test prerequisite.** The mandatory signed-stock fixtures
are sufficient to compile and run every conformance test. The first actual
AP5 board will provide the live original full proof, anchor and public
owner-policy binding through the producer's required retention/read-only
interface. AP5 must validate that proof before funded acceptance. No live
receipt is claimed here and no protected run was queried to create one.
