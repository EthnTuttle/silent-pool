# AP-1a ark-rpc probe report: l2-eb257af7

- probe: `ap1a-ark-rpc-probe` v1.0.0
- generated (UTC): 2026-11-08T05:19:29Z
- run token: `AP1a-58e665e4c9d9`
- target kind: owned-fresh-regtest-node
- profile: `bark-c419159` (server >= 310000, wallet >= 290000, network `regtest`)
- contract hash: `48894548fc322b2b54da4d2e30c4ca9c8eadb20c72e7607ca86e7a8b12a3a4b7`
- node binary: `/tmp/opencode/rbitcoin-lab-retarget-profile/target/debug/rbitcoin-node` (sha256 eb257af7c10cdf0ded584adfadf2ae8124466304b13d26c17f855e76f9320f7c)

## Verdict

**incompatible**

Reasons: `fail-version-gate`, `incomplete`

incompatible: the RPC version gate fails honestly (bark server needs >= 310000, wallet >= 290000); funded-path and live-mempool/fee behavior remain unproven (unknown/not_run); a version bump alone cannot yield full compat

Unproven (unknown/not_run) checks: `getrawtransaction.verbosity0`, `getrawtransaction.verbosity1`, `getmempoolentry.recognized`, `getmempoolentry.shape.server`, `getmempoolentry.shape.wallet`, `estimatesmartfee.fee-data`, `testmempoolaccept.recognized`, `submitpackage.recognized`, `sendrawtransaction.recognized`

## Gap matrix

| Check | Method | Availability | Behavior | Detail |
|---|---|---|---|---|
| `getnetworkinfo` | `getnetworkinfo` | passed | passed | parsed against getnetworkinfo (bark c419159 server/src/bitcoind.rs:236-244 (require_version, Response{version})) |
| `getnetworkinfo.gate.server` | `getnetworkinfo` | passed | failed | version 190000 < server minimum 310000: the server gate fails honestly (bark refuses to start against this node) |
| `getnetworkinfo.gate.wallet` | `getnetworkinfo` | passed | failed | version 190000 < wallet minimum 290000: the wallet gate fails honestly (bark refuses to start against this node) |
| `getblockchaininfo.network` | `getblockchaininfo` | passed | passed | chain 'regtest' matches expected network |
| `getblockchaininfo.shape.async` | `getblockchaininfo` | passed | passed | parsed against getblockchaininfo.async (bitcoind-async-client 0.15.0 src/client/v30.rs:182-188 (network(): call::<GetBlockchainInfo>("getblockchaininfo") -> .chain)) |
| `getblockchaininfo.shape.sync` | `getblockchaininfo` | passed | passed | parsed against getblockchaininfo.sync (bitcoincore-rpc-json 0.19.0 src/lib.rs:1017-1057 (GetBlockchainInfoResult)) |
| `getindexinfo.txindex` | `getindexinfo` | passed | passed | txindex present and synced (best_block_height=0) |
| `getblockcount` | `getblockcount` | passed | passed | integer tip height 0 |
| `getblockhash.genesis` | `getblockhash` | passed | passed | genesis hash matches the Core regtest genesis |
| `getblock.verbosity0` | `getblock` | passed | passed | genesis block served as hex (verbosity 0) |
| `getblock.verbosity0.rehash` | `getblock` | passed | passed | double-SHA256 of the 80-byte header equals the requested block hash (block data integrity proven) |
| `getblockheader.verbose` | `getblockheader` | passed | passed | parsed against bitcoincore-rpc-json 0.19 GetBlockHeaderResult (bark server tx_status type) |
| `getblockheader.verbose.genesis` | `getblockheader` | passed | passed | genesis header: hash/height/nTx consistent (nTx=1, confirmations=1) |
| `getrawtransaction.verbosity0` | `getrawtransaction` | passed | unknown | node does not serve the genesis coinbase via getrawtransaction (code -5); shape unverified without a funded fixture |
| `getrawtransaction.verbosity1` | `getrawtransaction` | passed | unknown | node does not serve the genesis coinbase via getrawtransaction (code -5); shape unverified without a funded fixture |
| `gettxout.mempool-false` | `gettxout` | passed | passed | null for the genesis coinbase (Core-compatible: the genesis coinbase output is not in the UTXO set; bark try_get_tx_out maps this to None). Note: the non-null GetTxOutResult shape remains unverified without a funded fixture |
| `gettxout.mempool-true` | `gettxout` | passed | passed | null for the genesis coinbase (Core-compatible: the genesis coinbase output is not in the UTXO set; bark try_get_tx_out maps this to None). Note: the non-null GetTxOutResult shape remains unverified without a funded fixture |
| `getrawmempool` | `getrawmempool` | passed | passed | array of txids (0 entries) |
| `getmempoolentry.recognized` | `getmempoolentry` | passed | unknown | txid not in mempool (expected on an unfunded node): method routed and returns the Core -5 code bark maps to None; entry shape unverified without a funded fixture |
| `getmempoolentry.shape.server` | `getmempoolentry` | passed | unknown | requires a funded mempool entry (separate opt-in packet); getmempoolentry.server shape unverified on an unfunded node |
| `getmempoolentry.shape.wallet` | `getmempoolentry` | passed | unknown | requires a funded mempool entry (separate opt-in packet); getmempoolentry.wallet shape unverified on an unfunded node |
| `estimatesmartfee.fee-data` | `estimatesmartfee` | passed | unknown | no feerate for targets [1, 3, 6] (empty mempool: the node returns errors + blocks only). Missing fee data is unknown, not pass: bark falls back to its configured fallback fee |
| `estimatesmartfee.t1.shape` | `estimatesmartfee` | passed | passed | parsed (blocks=1, feerate=None) |
| `estimatesmartfee.t3.shape` | `estimatesmartfee` | passed | passed | parsed (blocks=3, feerate=None) |
| `estimatesmartfee.t6.shape` | `estimatesmartfee` | passed | passed | parsed (blocks=6, feerate=None) |
| `testmempoolaccept.recognized` | `testmempoolaccept` | passed | not_run | method routed: rejected the invalid sample with server_error_-22: TX decode failed: 00 Make sure the tx has at least one input. (funded broadcast path deliberately not exercised; invalid samples prove recognition only) |
| `submitpackage.recognized` | `submitpackage` | passed | not_run | method routed: rejected the invalid sample with server_error_-22: TX decode failed: 00 Make sure the tx has at least one input. (funded broadcast path deliberately not exercised; invalid samples prove recognition only) |
| `sendrawtransaction.recognized` | `sendrawtransaction` | passed | not_run | method routed: rejected the invalid sample with server_error_-22: TX decode failed. Make sure the tx has at least one input. (funded broadcast path deliberately not exercised; invalid samples prove recognition only) |

## Observations

- **blockchaininfo.bestblockhash**: 0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206
- **blockchaininfo.bits**: 207fffff
- **blockchaininfo.blocks**: 0
- **blockchaininfo.headers**: 0
- **blockchaininfo.initialblockdownload**: true
- **blockchaininfo.pruned**: false
- **blockchaininfo.target**: 7fffff0000000000000000000000000000000000000000000000000000000000
- **chain**: regtest
- **getzmqnotifications**: method not implemented (no ZMQ support)
- **mempool_size**: 0
- **protocolversion**: 70016
- **rpc_version**: 190000
- **subversion**: /rbitcoin:0.8.99/
- **tip_height**: 0
- **txindex**: {"best_block_height": 0, "synced": true}
- **zmq_impact**: bark c419159 does not use ZMQ: it polls the node via getblockcount/getblockhash (server poll loop) and getrawmempool (bdk emitter), so ZMQ absence is not a compatibility gap for the pinned bark.

## Non-claims

- Funded broadcast paths (testmempoolaccept / submitpackage / sendrawtransaction with valid funded transactions) were deliberately NOT exercised; their behavior is `not_run`.
- No transaction funding, mining, or block generation was performed; the node stayed read-only.
- Fee/mempool data on an empty mempool is `unknown`, never a pass.
- A passing version gate alone can never yield `full_compat` (see the `incomplete` reason).
- No Core version was faked, shimmed, or overridden; the version gate fails honestly on nodes reporting 190000.
