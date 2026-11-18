# 2026-11-18 - rbitcoin.space on Testnet5: frontend provenance, Esplora surface, bark requirements

Supporting analysis for the `wp-explorer-testnet5` packet (non-normative).
It records the evidence behind `nix/dev/testnet5-explorer.nix` and the pin in
`nix/dev/testnet5/explorer-source.json`.

## 1. Where the rbitcoin.space frontend lives

**Verdict: public, MIT.** The live site `https://rbitcoin.space/` (CSP
`default-src 'self'`, `data-network="mainnet"`, onion-location header) serves
a Vite-built bundle; its application chunk `/assets/app-o0NPwaJl.js` contains
the string `github.com/smolcars/rbitcoin-space` (fetched 2026-11-18). That
repository is public, MIT-licensed, default branch `master`, head
`77892d714387a2c8b1248f2c47b070489c4f2f12` (2026-11-17), described as "A fast
block explorer for rbitcoin: live mempool, blocks, mining, lightning and Ark."
Negative checks that preceded the find: `rbitcoin-org` hosts only `rbitcoin`,
`rbitcoin.org` (marketing site) and `.github`; no branch of
`rbitcoin-org/rbitcoin` (ls-remote 2026-11-18) carries a ui/web/space frontend;
the rbitcoin workspace at both master `e8cf9c9d6` and the testnet5 pin
`8af95b8` has no frontend crate. GitHub search also surfaces
`remcoros/rbitcoin-space-startos` (a StartOS wrapper, further evidence the
explorer repo is the deployed one).

Shape: one Rust workspace (`crates/server` -> binary `rbitcoin-space`, plus
`engine`, `node`, `ln`, `bark`, `arkade`, `card`, `logging`) and a Solid/Vite
frontend in `web/` built with bun. The server serves the built frontend
(`WEB_DIR`), an Esplora-compatible public API under `/api` (`docs/API.md`),
WebSockets, and talks to a stock rbitcoin node over Esplora + Electrum +
JSON-RPC. Networks are env-configured (`{NET}_RPC_HOST/PORT/USER/PASSWORD`,
`{NET}_ESPLORA_URL`, `{NET}_ELECTRUM_ADDR` for `MAINNET`, `SIGNET`,
`TESTNET4`, `TESTNET5`, `REGTEST`); mainnet is served at `/`, testnet5 under
`/testnet5`. `web/public/boot.js` sets `data-network` from the URL path at
page load, so `/testnet5/...` pages carry `data-network="testnet5"` with the
stock bundle. Testnet5 requires an rbitcoin newer than v0.9.2 - specifically
master `94dce31b` (2026-11-14) or later (`docs/DEPLOY.md#testnet-5`); the
project's node pin `8af95b8` (2026-11-15) contains `94dce31b` as an ancestor
(verified with `git merge-base --is-ancestor`). LDK has no Testnet5, so
Lightning is off there by design; Ark pages are optional per-network env.

## 2. What serves the Esplora API on Testnet5

`rbitcoin-esplora` is a library crate ("Esplora-compatible REST, electrs HTTP
drop-in except address-prefix") compiled into `rbitcoin-node` at the `8af95b8`
pin; there is no separate esplora binary. The existing
`rbitcoin-node-testnet5-dev` package (build_packages `rbitcoin-node` +
`rbitcoin-cli`) therefore already contains the server; it is enabled per run:

- `--esplora-listen ADDR` - the Esplora HTTP listener;
- `--electrum-listen ADDR` - the Electrum listener (the explorer needs both);
- `--sh-index` - scripthash (Class B) index; without it address/scripthash
  methods fail closed;
- `--rpc-cookie-file PATH` - opts TCP JSON-RPC into Core cookie HTTP Basic
  (the file is operator-created `user:password` bytes; the node never writes
  it), which is what the explorer's `{NET}_RPC_USER/PASSWORD` speak;
- `--rpc-client-version 310000` - only when a bark `captaind` shares the
  node: `getnetworkinfo.version` defaults to the 190000 RPC shape floor and
  captaind refuses anything below 310000 ("upgrade to v31"). The explorer
  itself does not need the override.

The network is the node's own: with `--network testnet5` every endpoint
serves testnet5 facts (verified: tip hash = BIP95 genesis
`00000000000008c1637e7d3252dc1c333d4546fd6fff6a474b200d674a91d497`, `tb1p...`
addresses parse).

## 3. What bark needs from that Esplora endpoint

From the adopted fork at `8c38e07` (`bark/src/chain.rs`,
`bark/src/onchain/bdk.rs`; HTTP client `esplora-client` 0.12.3 + `bdk_esplora`
0.22.1):

| Use | Endpoint |
|---|---|
| chain-identity check at client construction | `GET /block-height/0` (must return the network's genesis hash) |
| tip height / tip hash | `GET /blocks/tip/height`, `GET /blocks/tip/hash` |
| header/hash by height | `GET /block-height/{h}` |
| block fetch | `GET /block/{hash}/raw` |
| tx fetch / info / status | `GET /tx/{txid}` (JSON), `GET /tx/{txid}/raw`, `GET /tx/{txid}/status` |
| spentness | `GET /tx/{txid}/outspend/{vout}` |
| BDK full_scan/sync (onchain wallet) | `GET /scripthash/{sha256(spk)}/txs[/chain/{last_seen}]` (needs `--sh-index`) |
| fee estimation | `GET /fee-estimates` |
| broadcast | `POST /tx` |
| package broadcast (CPFP exits) | `POST /txs/package` |

All of these exist in `rbitcoin-esplora` at `8af95b8` and answered correctly
on a genesis-only testnet5 datadir (curl transcripts in the packet report;
`POST` shapes verified by error responses to garbage payloads, and implicitly
by `bark create`/`balance`/`onchain balance` succeeding against the endpoint).

## 4. Deployment story packaged here

`nix/dev/testnet5-explorer.nix` (opt-in, keeps `testnet5-node.nix` untouched):

- `webDeps`: `bun install --frozen-lockfile --ignore-scripts` from the pinned
  lockfile as a fixed-output derivation (pinned nixpkgs bun is 1.4.2; the
  repo's `packageManager` says bun@1.4.4 - the lockfile parses and installs
  identically, recorded here in case a future pin bump changes that);
- `web`: `vite build` run offline through bun (`bun node_modules/vite/bin/
  vite.js build` - the `.bin` shim's `#!/usr/bin/env node` does not resolve
  in the sandbox);
- `server`: `cargo build -p rbitcoin-space` with the workspace's Rust 1.99.1,
  registry-only `Cargo.lock`;
- `run-testnet5-explorer`: loopback launcher exporting exactly the six
  `TESTNET5_*` variables against node defaults (RPC `127.0.0.1:18352` with
  credentials read from the node's cookie file, Esplora
  `http://127.0.0.1:3003`, Electrum `127.0.0.1:50001`), `LISTEN_ADDR`
  defaulting to `127.0.0.1:8080`, `WEB_DIR` pointing at the built frontend.
  `DATA_DIR` (SQLite for mining/mempool-history/audits) passes through when
  the operator sets it. Behind-proxy/TLS posture is upstream `docs/DEPLOY.md`;
  nothing here publishes a port beyond loopback.

Not done, deliberately: no NixOS service/module (the dev-node fixture family
is explicitly "not a service or launcher"); no Lightning (none on testnet5);
no Ark env wiring to the explorer (optional, operator-set); no public
deployment claim.
