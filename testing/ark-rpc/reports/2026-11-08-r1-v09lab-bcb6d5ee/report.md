# AP-1a ark-rpc probe report: rbitcoin-v0.9-lab-r1

- probe: `ap1a-ark-rpc-probe` v1.0.0 (report schema v2)
- generated (UTC): 2026-11-08T07:37:12Z
- run token: `AP1a-b1aeb688e017`
- target kind: owned-fresh-regtest-node
- profile: `bark-c419159` (server >= 310000, wallet >= 290000, network `regtest`)
- contract hash: `11bdb1007e130a749720f87482f287c1fb65d1e743c142fc4b1ca15a782d89e7`
- node binary: `/tmp/opencode/rbitcoin-v0.9-lab/target/dev/debug/rbitcoin-node` (source `rbitcoin-v0.9-lab`, sha256 bcb6d5eefe0dd0bd96e2ac79e173b4b99a91f67f7717dae6620c30786378a171)

## Verdict

**incompatible**

Reasons: `fail-version-gate`, `funded-unproven`

incompatible: the RPC version gate fails honestly (bark server needs >= 310000, wallet >= 290000); funded broadcast/mempool behavior additionally remains unproven (read-only mode)

- funded state: **unproven** (read-only mode cannot observe funded broadcast/mempool behavior)
- full acceptance: **False** (requires a separate funded packet; a version bump plus read-only passes can never grant it)

Unproven (unknown/not_run) checks: `getrawtransaction.verbosity0`, `getrawtransaction.verbosity1`, `getrawtransaction.bool-verbose`, `getrawtransaction.bool-verbose.async`, `getmempoolentry.recognized`, `getmempoolentry.shape.server`, `getmempoolentry.shape.wallet`, `estimatesmartfee.fee-data`, `testmempoolaccept.recognized`, `submitpackage.recognized`, `sendrawtransaction.recognized`

## Gap matrix

| Check | Method | Availability | Behavior | Detail |
|---|---|---|---|---|
| `getnetworkinfo.gate.server` | `getnetworkinfo` | passed | failed | version 190000 < server minimum 310000: the server gate fails honestly (bark refuses to start against this node) |
| `getnetworkinfo.gate.wallet` | `getnetworkinfo` | passed | failed | version 190000 < wallet minimum 290000: the wallet gate fails honestly (bark refuses to start against this node) |
| `getnetworkinfo.observe` | `getnetworkinfo` | passed | passed | observation set parsed (not a bark requirement; never gates the verdict) |
| `getnetworkinfo.version-shape` | `getnetworkinfo` | passed | passed | parsed against getnetworkinfo.required (bark c419159 server/src/bitcoind.rs:236-244 (require_version, Response{version})) |
| `getblockchaininfo.network` | `getblockchaininfo` | passed | passed | chain 'regtest' matches expected network |
| `getblockchaininfo.shape.async` | `getblockchaininfo` | passed | passed | parsed against getblockchaininfo.async (bitcoind-async-client 0.15.0 src/client/v30.rs:182-188 (network(): call::<GetBlockchainInfo>("getblockchaininfo") -> .chain)) |
| `getblockchaininfo.shape.sync` | `getblockchaininfo` | passed | passed | parsed against getblockchaininfo.sync (bitcoincore-rpc-json 0.19.0 src/lib.rs:1017-1057 (GetBlockchainInfoResult)) |
| `getindexinfo.txindex` | `getindexinfo` | passed | passed | txindex present and synced (best_block_height=0; the same field name Bitcoin Core and corepc emit) |
| `getblockcount` | `getblockcount` | passed | passed | integer tip height 0 |
| `getbestblockhash` | `getbestblockhash` | passed | passed | hash32 tip (async get_current_timestamp form) |
| `getblockhash.genesis` | `getblockhash` | passed | passed | genesis hash matches the Core regtest genesis |
| `getblock.singleparam` | `getblock` | passed | passed | single-param form accepted ([hash]; verbosity defaults to 1 server-side, async get_block_height form) |
| `getblock.verbosity0` | `getblock` | passed | passed | genesis block served as hex (verbosity 0, async form [hash, 0]) |
| `getblock.verbosity0.rehash` | `getblock` | passed | passed | double-SHA256 of the 80-byte header equals the requested block hash (integrity proven) |
| `getblock.verbosity1` | `getblock` | passed | passed | parsed against getblock.verbosity1 (bitcoincore-rpc-json 0.19.0 src/lib.rs:195-219 (GetBlockResult)) |
| `getblockheader.encoding.sync20` | `getblockheader` | passed | passed | sync form accepted ([hash, true], envelope 2.0, numeric id 3 echoed) |
| `getblockheader.nonverbose` | `getblockheader` | passed | passed | sync hex form accepted ([hash, false], envelope 2.0); 80-byte header re-hash matches |
| `getblockheader.verbose` | `getblockheader` | passed | passed | parsed against bitcoincore-rpc-json 0.19 GetBlockHeaderResult (bark server tx_status type; async form [hash, true]) |
| `getblockheader.verbose.genesis` | `getblockheader` | passed | passed | genesis header: hash/height/nTx consistent (nTx=1, confirmations=1) |
| `getrawtransaction.bool-verbose` | `getrawtransaction` | passed | unknown | node does not serve the genesis coinbase via getrawtransaction (code -5) -- Bitcoin Core behaves identically; sync exact form [txid, true, null] (bitcoincore-rpc 0.19 and bitcoin-ext get_raw_transaction) routed and reached the domain layer; shape unverified without a funded fixture |
| `getrawtransaction.bool-verbose.async` | `getrawtransaction` | passed | unknown | node does not serve the genesis coinbase via getrawtransaction (code -5) -- Bitcoin Core behaves identically; bark call_raw form [txid, true] routed and reached the domain layer; shape unverified without a funded fixture |
| `getrawtransaction.verbosity0` | `getrawtransaction` | passed | unknown | node does not serve the genesis coinbase via getrawtransaction (code -5) -- Bitcoin Core behaves identically; async form [txid, 0] routed and reached the domain layer; shape unverified without a funded fixture |
| `getrawtransaction.verbosity1` | `getrawtransaction` | passed | unknown | node does not serve the genesis coinbase via getrawtransaction (code -5) -- Bitcoin Core behaves identically; async form [txid, 1] routed and reached the domain layer; shape unverified without a funded fixture |
| `gettxout.mempool-false` | `gettxout` | passed | passed | null for the genesis coinbase -- Core-identical (the genesis coinbase output is not in the UTXO set on Bitcoin Core either; bark try_get_tx_out maps this to None). Note: the non-null GetTxOutResult shape remains unverified without a funded fixture |
| `gettxout.mempool-true` | `gettxout` | passed | passed | null for the genesis coinbase -- Core-identical (the genesis coinbase output is not in the UTXO set on Bitcoin Core either; bark try_get_tx_out maps this to None). Note: the non-null GetTxOutResult shape remains unverified without a funded fixture |
| `getrawmempool` | `getrawmempool` | passed | passed | array of txids (0 entries; async form []) |
| `getrawmempool.bool-form` | `getrawmempool` | passed | passed | sync form accepted ([false], envelope 2.0; bitcoin-ext get_mempool_spending_tx) |
| `getmempoolinfo` | `getmempoolinfo` | passed | failed | shape: maxdatacarriersize: expected JSON integer, got null |
| `getmempoolentry.recognized` | `getmempoolentry` | passed | unknown | txid not in mempool (expected on an unfunded node): method routed and returns the Core -5 code bark maps to None; entry shape unverified without a funded fixture |
| `getmempoolentry.shape.server` | `getmempoolentry` | passed | unknown | requires a funded mempool entry (separate opt-in packet); getmempoolentry.server shape unverified on an unfunded node |
| `getmempoolentry.shape.wallet` | `getmempoolentry` | passed | unknown | requires a funded mempool entry (separate opt-in packet); getmempoolentry.wallet shape unverified on an unfunded node |
| `estimatesmartfee.fee-data` | `estimatesmartfee` | passed | unknown | no feerate for targets [1, 3, 6] (empty mempool: the node returns errors + blocks only). Missing fee data is unknown, not pass: bark falls back to its configured fallback fee |
| `estimatesmartfee.t1.shape` | `estimatesmartfee` | passed | passed | parsed (blocks=1, feerate=None; mode literal "ECONOMICAL" as bark serializes it) |
| `estimatesmartfee.t3.shape` | `estimatesmartfee` | passed | passed | parsed (blocks=3, feerate=None; mode literal "ECONOMICAL" as bark serializes it) |
| `estimatesmartfee.t6.shape` | `estimatesmartfee` | passed | passed | parsed (blocks=6, feerate=None; mode literal "ECONOMICAL" as bark serializes it) |
| `protocol.envelope20.getnetworkinfo` | `getnetworkinfo` | passed | passed | sync-client form accepted (envelope 2.0, numeric id 1 echoed exactly, HTTP 200) |
| `protocol.id-echo` | `None` | passed | passed | all 29 exchanges echoed the numeric request id exactly |
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
- **mempoolinfo.bytes**: 0
- **mempoolinfo.fullrbf**: true
- **mempoolinfo.loaded**: true
- **mempoolinfo.maxmempool**: 75000000
- **mempoolinfo.size**: 0
- **mempoolinfo.usage**: 0
- **networkinfo_observed**: {"subversion": "/rbitcoin:0.9.0/", "protocolversion": 70016, "relayfee": 100, "incrementalfee": 100}
- **protocolversion**: 70016
- **rpc_version**: 190000
- **subversion**: /rbitcoin:0.9.0/
- **tip_height**: 0
- **txindex**: {"best_block_height": 0, "synced": true}
- **zmq_impact**: bark c419159 does not use ZMQ: it polls the node via getblockcount/getblockhash (server poll loop) and getrawmempool (bdk emitter), so ZMQ absence is not a compatibility gap for the pinned bark.

## Non-claims

- Funded broadcast paths (testmempoolaccept / submitpackage / sendrawtransaction with valid funded transactions) were deliberately NOT exercised; their behavior is `not_run`.
- No transaction funding, mining, or block generation was performed; the node stayed read-only.
- Fee/mempool data on an empty mempool is `unknown`, never a pass.
- Read-only mode offers no full-compat outcome: the best reachable result is `surface_compatible` with `funded-unproven`; full acceptance requires a separate funded packet.
- No Core version was faked, shimmed, or overridden; the version gate fails honestly on nodes reporting 190000.
- The genesis-coinbase `getrawtransaction` -5 and `gettxout` null outcomes are Core-identical behavior, not rbitcoin quirks.
