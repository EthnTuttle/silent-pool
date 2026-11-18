# 2026-11-18 - Testnet5 explorer deployment (dev tooling)

- "2026-11-18: opt-in Testnet5 explorer tooling beside the dev node; no SPEC
  section changes. The pinned dev node (`rbitcoin-org/rbitcoin @ 8af95b8`)
  already embeds the Esplora/Electrum servers in `rbitcoin-node`; serving them
  on testnet5 is per-run flags (`--esplora-listen`, `--electrum-listen`,
  `--sh-index`, optional `--rpc-client-version 310000` for a co-hosted bark
  captaind). Validated on a fresh genesis-only testnet5 datadir: every
  endpoint the bark fork's chain source uses (genesis check `/block-height/0`,
  tip height/hash, block raw/txids, tx json/raw/status/outspend,
  scripthash/address history and utxos, `/fee-estimates`, `POST /tx`,
  `POST /txs/package`), and a `bark-testnet5` wallet created and synced
  (`create`/`balance`/`onchain balance`) against `--esplora` on that node.
  New opt-in package `rbitcoin-space-testnet5` (`nix/dev/testnet5-explorer.nix`,
  pin `nix/dev/testnet5/explorer-source.json`): the rbitcoin.space explorer
  frontend+server (`smolcars/rbitcoin-space @ 77892d71`, MIT, discovered via
  the live site's own bundle), built offline (bun lockfile FOD + vite, cargo
  with Rust 1.99.1) with a loopback `run-testnet5-explorer` launcher serving
  `/testnet5` with `data-network=\"testnet5\"` against the node's
  RPC/Esplora/Electrum. UPSTREAM_DEPS.md gains the explorer pin row. Evidence:
  docs/analysis/2026-11-18-rbitcoin-space-testnet5.md."

Merge note (merge-session-owned file): `flake.nix` needs the
`rbitcoin-space-testnet5` package attribute; exact hunk in
`merge/wp-explorer-testnet5/flake-explorer.patch`. The branch tree leaves
`flake.nix` untouched per docs/MERGE-QUEUE.md, so the new attribute builds
only once the merge session applies the patch (the packet validated the build
and a loopback serve with the hunk applied locally).
