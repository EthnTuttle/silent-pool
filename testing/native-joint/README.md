# Same-source native CPU / wallet component check

Source-only follow-up; primary must integrate and execute the gate below.
From a fresh **committed** checkout after integration, with Nix installed:

```sh
nix build --no-link --print-out-paths .#checks.x86_64-linux.wallet-native-joint
```

The check builds its CPU and wallet tests from the same filtered source, then
exports a receipt, exact CPU-generated capture bytes and artifact descriptor in
its output. No historical binary/archive, worktree or fixed temporary path is
an input. The repository's build-slot/disk rules apply when running locally.

## Descriptor and trust boundary

`descriptor.schema.json` is the closed v1 schema. Hashes are lowercase SHA256
hex; `source_nar_sha256` hashes the exact NAR stream of the filtered source;
`derivation_sha256` hashes raw `.drv` bytes; binary size is bytes. Maximum JSON
length is 8192 bytes. No optional fields or alternative artifact kinds exist.
The binary path must equal `output_path + /bin/spc-cpu-miner`.

The Nix hook invokes `artifact.py` with the actual CPU output, its actual
`drvPath`, and the wallet test's `common.src` / `SPC_BUILD_SOURCE_ID`. It parses
bounded classic input-addressed ATerm derivations and requires the single
`out` output, `env.out`, `env.src`, source input dependency and source ID to
agree. Unsupported derivation formats fail. It hashes the actual executable
and source NAR (`nix-store --dump`, no daemon/database query in the sandbox).
The generated descriptor and anchors become immutable Nix store outputs.

The test compiles in these anchors BEFORE Cargo compiles its test binary:

- `SPC_JOINT_DESCRIPTOR`, `SPC_JOINT_DESCRIPTOR_SHA256`
- `SPC_JOINT_CPU_DRV`, `SPC_JOINT_CPU_DRV_SHA256`, `SPC_JOINT_CPU_OUTPUT`
- `SPC_JOINT_SOURCE_NAR_SHA256`
- `SPC_JOINT_TEST_SOURCE` and `SPC_BUILD_SOURCE_ID`, independently from common

At runtime the test checks immutable canonical paths, closed JSON, descriptor
hash, source/derivation/output bindings, raw derivation hash and actual CPU
size/hash. The observed child executable hash must also match. These anchors
are trusted **build-system** inputs, not signatures against a malicious build
system. Setting runtime expected hashes, paths or fixture startup metadata
cannot authorize a CPU. The Nix store/source closure must remain available.

Only `SPC_NATIVE_JOINT_RESULTS` is runtime-configurable: an existing canonical
owner-only 0700 output parent. Each invocation creates its own random private
child. Without it, the standard test temporary directory is used. Success
preserves results and prints their location; this path supplies no authority.

## Minimal primary Nix wiring

1. Set `common.SPC_BUILD_SOURCE_ID` from the flake itself, never a shell variable:
   `if self ? rev then "git:${self.rev}" else "nix-snapshot"`. CPU and test must
   share that exact `common.src`. A snapshot carries its true NAR/path binding
   and must not claim a clean commit. Use the existing source exclusions for
   `.git`, targets, runtime datadirs and caches; measure source size first.
2. Import `nix/checks/wallet-native-joint.nix` with `pkgs`, `common`, and the
   same-source `spc-cpu-miner` derivation. No changed upstream pins/lockfile.
3. Merge its `preBuild` BEFORE any Cargo compilation and `preCheck` before
   execution. Append `installResults` to a directory-valued install output.
4. Apply those hooks to **every normal ark-agent/workspace test derivation**
   that runs this test too. Its ignore marker is removed: absent descriptor
   anchors/files fail, and there is no `--ignored` invocation or skip fallback.
5. Supply the approved `spc_scan::native_evidence` module in the merged source.
   Keep that module and all production wallet/native codec code unchanged.

Illustrative dedicated check (primary owns final flake integration):

```nix
let
  joint = import ./nix/checks/wallet-native-joint.nix {
    inherit pkgs common;
    cpu = self.packages.${system}.spc-cpu-miner;
  };
in pkgs.rustPlatform.buildRustPackage (common // {
  pname = "wallet-native-joint";
  version = "0.1.0";
  doCheck = true;
  inherit (joint) preBuild preCheck;
  buildPhase = ''
    runHook preBuild
    cargo test --offline --locked -p spc-agent --features ark --lib --no-run
    runHook postBuild
  '';
  checkPhase = ''
    runHook preCheck
    ${pkgs.python3}/bin/python3 -B testing/native-joint/test_artifact.py
    cargo test --offline --locked -p spc-agent --features ark --lib \
      wallet::native_audit::tests::joint_artifact::tests::
    cargo test --offline --locked -p spc-agent --features ark --lib \
      wallet::native_audit::tests::sealed_native_cpu_waits_for_actual_wallet_collector_commit_before_first_hash \
      -- --exact --nocapture
    runHook postCheck
  '';
  installPhase = joint.installResults;
})
```

Use consistent Cargo profile/options in both phases and existing target reuse.
The hook requires a classic single-output CPU derivation and a filtered source
store path (not a source derivation). `unsafeDiscardOutputDependency` retains
the actual CPU `.drv` file as an input, without requiring sandbox daemon access.
Source correlation is checked from its bytes, not from a descriptor caller's
claim about the deriver. The CPU store output is also an explicit build input.

## Pending gate and scope

Prepared regressions: seven Python derivation-correlation cases, seven Rust
artifact-rejection cases, and one actual CPU/wallet component test. They are
separate counts, all **unexecuted** in this follow-up. Rejections cover wrong
binary/path/digest, source/commit/NAR, output/derivation, unknown schema/fields,
descriptor digest and missing artifacts. The valid check must exercise the
actual Nix-generated descriptor, not a hand-written artifact fixture.

The component retains zero attempts/no retained pair before COMMITTED, actual
Snapshot locator/manifest, shared retained-pair decoding, exact wallet-response
bytes, and COMMITTED -> activation -> first hash ordering. Startup metadata,
run marker and complete-empty-history collector remain explicitly synthetic.
It does not establish controller restart/attach, full history, four-wallet/full-Q,
N1/N2, independent QA or funded Ark acceptance. Earlier counts retain their scopes.

Historical fbef source/commands/receipts remain frozen separately. To replay
that historical scenario use its frozen test source and wrapper together; it
is not a default input or an accepted alternate kind in this portable check.
