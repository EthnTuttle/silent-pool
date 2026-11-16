# Pinned Testnet5 development node

This package implements a separate source-pinned, node-only build and fixture.
It installs no service and starts no P2P process. Deployment remains gated on
the owner's satisfaction with the current regtest lab and separate approval.

## Source/build contract

- Root non-flake input `testnet5-node-src` is **Git**, revision
  `8af95b811d81b4797afb69a655fc100202d57ca6`, without submodules. `source.json`
  records the verified NAR and Cargo.lock hashes. Updating the package pin,
  this record, root flake lock and UPSTREAM_DEPS is one atomic integration.
- This adds `rbitcoin-node-testnet5-dev`; the c150577 lab node remains a
  separate unchanged input/package. No lab profile facts are inherited.
- This package builds only node/CLI, using the primary flake's locked
  nixpkgs/rust-overlay, Rust **1.95.0**, default generic CPU target and native
  C compiler for mimalloc/zstd. No `target-cpu=native`, host cargo, unlocked
  dependency update or shared target-copy is used. Cargo.lock has registry
  dependencies only; Nix imports their locked checksums.
- Linux x86_64/aarch64 only. This is **Nix-store-linked glibc**, not the
  upstream portable musl release and not byte-identical to it. The reviewed
  upstream musl recipe has separate build/host linker flags and crane layers;
  this package does not claim to reuse those compiled artifacts.
- Git source is 1350 blobs / 22353155 bytes before filtering. Product input
  keeps root Cargo files and crates/fuzz fixture trees; it excludes git,
  targets, datadirs, logs, caches, corpus/artifacts and generated outputs.
  Upstream `third_party/bitcoin` is a gitlink, not downloaded; product build
  and the listed self-contained BIP54/95 vectors do not need that corpus.
  Do not point this input at an unfiltered local worktree.
- Install checks run version/help and offline Python negative controls only.
  They do not start the node. The package's status stays unqualified until
  the separately scheduled build/consensus/runtime evidence is recorded.

Authoritative source at this revision:
<https://github.com/rbitcoin-org/rbitcoin/tree/8af95b811d81b4797afb69a655fc100202d57ca6>.
See `changelog.d/testnet5.md`, `COMPAT.md:106-107`,
`crates/rbitcoin-consensus/src/params.rs:206-223,705-723`,
`crates/rbitcoin-net/src/seeds.rs:78-94`, and node config `apply_kv`.

## Lightweight checks (no node or network endpoint)

From the integrated source root, using the already fetched Git source path:

```sh
python3 -B nix/dev/testnet5/test-validator.py
python3 -B nix/dev/testnet5/validate-node.py --capture nix/dev/testnet5/rpc-fixture.json
python3 -B nix/dev/testnet5/check-source.py --source /nix/store/ACTUAL-PINNED-source
nix eval --offline --no-write-lock-file --raw .#packages.x86_64-linux.rbitcoin-node-testnet5-dev.drvPath
```

The source checker hashes the reviewed source files and verifies that fixture
genesis, ports, magic and config keys correspond to them. This is not a run of
the Rust config parser or a proof of consensus correctness. The validator's
tests use mocked responses only: no listening socket or node is needed.
`rpc-fixture.json` is synthetic input for this offline CLI test, not a node
observation or evidence that the public chain is at height zero.

## Heavy checks: parent allocation required, not run in this packet

Acquire `/tmp/opencode/silent-pool-heavy-build.lock` with nonblocking flock
for the **entire** invocation, respecting the priority reservation JSON.
After acquiring, check actual `/nix/store` and build-temp filesystems have
at least 50 GiB available and estimated peak growth leaves at least 20 GiB.
Use the existing assigned target/cache; no new target per review revision.

Build command after the parent assigns the slot:

```sh
nix build --no-link --no-write-lock-file .#rbitcoin-node-testnet5-dev
```

Retain derivation/output paths, build log and installed
`share/testnet5-dev/{source.json,Cargo.lock,binaries.sha256,version.txt}`.
This command has not been run. Same-source two-build byte identity has not
been measured; do not repeat heavy builds just to broaden a passing check.

Targeted upstream tests, in a later assigned Rust-1.95 Nix shell using the
same source and an approved existing target directory (commands unrun):

```sh
cargo test --locked --offline -p rbitcoin-consensus --test suite testnet5
cargo test --locked --offline -p rbitcoin-consensus --test suite bip54
cargo test --locked --offline -p rbitcoin-primitives --test suite
```

The `testnet5` filter selects genesis, parameters, minimum-difficulty refusal
and pow-limit/retarget cases; `bip54` includes the local official coinbase,
timestamp, tx-size and sigop vectors. These are source tests, not a public
network launch. Separately schedule a real node config-parser smoke check and
cross-implementation block validation; no node binary was launched here.

## Future node-only commissioning (after deployment approval)

1. Allocate a fresh independent private datadir, service identity and ports;
   render every `@DATADIR@` in `node.conf.in` to that absolute path. Never use
   the current lab's data, cookies, sockets, keys, peer file or service units.
   The fixture uses recognized pinned rbitcoin `key=value` syntax. CLI
   overrides can change it: record exact argv/config and revalidate them.
2. Recheck published bootstrap peers. No DNS/fixed seeds ship at this pin;
   `lapis.erisian.com.au:18353` is a maintainer-published peer, not a liveness
   guarantee. Public network P2P/RPC defaults are 18353/18352. This fixture
   has no inbound listener and private loopback RPC; it uses ordinary P2P
   outbound sync, full scripts (`milestone=0`) and normal mempool relay.
   Add peer diversity once independently established.
3. An approved operator may start the exact package with its rendered config.
   No launcher is supplied here. Preserve provenance and actual parser/startup
   results. Do not use `trust-peer`, difficulty overrides, `generatetoaddress`,
   archived block imports, private premine, manual chain credit or RPC identity
   rewriting. The public chain is not owned by the deployment operator.
4. Run the installed **read-only** probe explicitly against that owned node:

   ```sh
   testnet5-validate-node --rpc-url http://127.0.0.1:18352 --cookie /OWNED/FRESH/DATADIR/.cookie
   ```

   It reads only getblockchaininfo, getblockhash(0), getblockheader(genesis,
   false), getnetworkinfo. It refuses wrong chain/genesis/header, missing sync
   state, mismatched RPC IDs, RPC errors, duplicates, oversize JSON, redirects,
   proxies and non-loopback origins. Credentials are read from an owned private
   regular file, not argv. The first wrong-chain response stops the probe.
   `--capture` accepts an offline JSON map keyed by those four method names
   and always labels it `offline_capture`, never live proof.
5. A successful identity result is **not readiness**: verify completed IBD,
   chainwork/tip agreement with an independent Testnet5 implementation,
   ordinary P2P propagation, BIP54/retarget enforcement and exact binary/config
   provenance. The probe's `deployment_ready` and consensus-proof flags stay
   false by construction. Status-reported software version does not attest a
   source SHA, network magic, full validation or RPC policy compatibility.
6. Node-only rollback stops only the newly owned node and preserves its new
   datadir/evidence. Never downgrade a store schema in place or roll back the
   public chain. The existing lab is unrelated.

## Handoff to pool/network/Ark lanes

`network.json` is the common public-network constant set: genesis
`00000000000008c1637e7d3252dc1c333d4546fd6fff6a474b200d674a91d497`,
wire bytes `46 49 56 45`, Testnet5 string, normal 2016-block retarget,
600-second spacing, 210000-block subsidy interval and 100-block maturity.
There is **no 20-minute difficulty exception**. BIP54 applies from block 1:
the pool must request `consensuscleanup` in GBT and produce height-minus-one
coinbase locktime / non-final sequence. These pool changes are not in this lane.

Stock Bark/captaind c419159 cannot represent Testnet5. This package does not
change it, impersonate another network, select `bark-v31`, or assert Ark
compatibility. New upstream's optional RPC version-report knob is not selected
and is not a substitute for a real stock-network/policy compatibility test.
