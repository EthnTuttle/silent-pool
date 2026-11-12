# AP-1a ark-rpc gap matrix

Generated (UTC): 2026-11-08T09:08:18Z

| Check | rbitcoin-lab-retarget-profile-r2 | rbitcoin-v0.9-lab-r2 |
|---|---|---|
| `getnetworkinfo.gate.server` | passed / failed | passed / failed |
| `getnetworkinfo.gate.wallet` | passed / failed | passed / failed |
| `getnetworkinfo.observe` | passed / passed | passed / passed |
| `getnetworkinfo.version-shape` | passed / passed | passed / passed |
| `getblockchaininfo.network` | passed / passed | passed / passed |
| `getblockchaininfo.shape.async` | passed / passed | passed / passed |
| `getblockchaininfo.shape.sync` | passed / passed | passed / passed |
| `getindexinfo.txindex` | passed / passed | passed / passed |
| `getblockcount` | passed / passed | passed / passed |
| `getbestblockhash` | passed / passed | passed / passed |
| `getblockhash.genesis` | passed / passed | passed / passed |
| `getblock.singleparam` | passed / passed | passed / passed |
| `getblock.verbosity0` | passed / passed | passed / passed |
| `getblock.verbosity0.rehash` | passed / passed | passed / passed |
| `getblock.verbosity1` | passed / passed | passed / passed |
| `getblockheader.encoding.sync20` | passed / passed | passed / passed |
| `getblockheader.nonverbose` | passed / passed | passed / passed |
| `getblockheader.verbose` | passed / passed | passed / passed |
| `getblockheader.verbose.genesis` | passed / passed | passed / passed |
| `getrawtransaction.bool-verbose` | passed / unknown | passed / unknown |
| `getrawtransaction.bool-verbose.async` | passed / unknown | passed / unknown |
| `getrawtransaction.verbosity0` | passed / unknown | passed / unknown |
| `getrawtransaction.verbosity1` | passed / unknown | passed / unknown |
| `gettxout.mempool-false` | passed / passed | passed / passed |
| `gettxout.mempool-true` | passed / passed | passed / passed |
| `getrawmempool` | passed / passed | passed / passed |
| `getrawmempool.bool-form` | passed / passed | passed / passed |
| `getmempoolinfo` | passed / failed | passed / failed |
| `getmempoolentry.recognized` | passed / unknown | passed / unknown |
| `getmempoolentry.shape.server` | passed / unknown | passed / unknown |
| `getmempoolentry.shape.wallet` | passed / unknown | passed / unknown |
| `estimatesmartfee.fee-data` | passed / unknown | passed / unknown |
| `estimatesmartfee.t1.shape` | passed / passed | passed / passed |
| `estimatesmartfee.t3.shape` | passed / passed | passed / passed |
| `estimatesmartfee.t6.shape` | passed / passed | passed / passed |
| `protocol.envelope20.getnetworkinfo` | passed / passed | passed / passed |
| `protocol.id-echo` | passed / passed | passed / passed |
| `testmempoolaccept.recognized` | passed / not_run | passed / not_run |
| `submitpackage.recognized` | passed / not_run | passed / not_run |
| `sendrawtransaction.recognized` | passed / not_run | passed / not_run |

Cells are `availability / behavior`. Statuses: `passed`, `failed`, `unknown` (ran but unproven), `not_run` (deliberately not exercised).

## Verdicts

- **rbitcoin-lab-retarget-profile-r2**: incompatible (fail-version-gate, funded-unproven; funded unproven, full acceptance False)
- **rbitcoin-v0.9-lab-r2**: incompatible (fail-version-gate, funded-unproven; funded unproven, full acceptance False)
