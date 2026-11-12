# devcheck

Dev-only Rust checker for the AP-1a ark-rpc probe packet. It validates
the Python contract golden vectors (the `#devcheck` tests in
`../tests/test_contracts.py`) against the **real pinned Rust crate
types** bark deserializes with, so the Python contract mini-language in
`../arkrpc/contracts.py` is not merely self-consistent.

It is NOT part of the probe runtime, NOT part of the packet manifest
hash (excluded in `../arkrpc/reporting.py` `_hash_tree`), and never
required to run the probe, the self-test, or the unit tests.

## What it proves

Each vector in `src/main.rs` mirrors a `#devcheck` Python test by name
(`v_amount_over_supply_accepted` <->
`AmountBtcTests::test_over_supply_accepted`, etc.):

- `bitcoin::amount::serde::as_btc` (bitcoin-units 0.1.3): the
  deserialization range is the u64 satoshi range, not the 21M BTC
  policy cap (30M BTC parses; 184467440738 BTC overflows).
- Two ScriptPubkeyType enums: bitcoincore-rpc-json 0.19.0 (gettxout
  consumer) has no Anchor variant and rejects `"anchor"`, while
  bark-bitcoin-ext's vendored copy (getrawtransaction-verbose
  consumer) accepts it.
- serde_hex (both copies): `""` deserializes as an empty `Vec<u8>`.
- `deserialize_hex_array_opt`: txinwitness items may be `""`.
- Absent-vs-null: `serde(default)` + custom `Option` deserializers
  (versionHex, coinbase, txinwitness, addresses) reject explicit null
  while accepting absence.
- getblockchaininfo shapes: corepc-types 0.13.0 v29 (async) requires
  bits/target/time; bitcoincore-rpc-json 0.19.0 (sync) has none of
  them and accepts a plain-string `warnings`.

## Pins

All registry dependencies are `=pinned` to the versions bark c419159
locks in its `Cargo.lock` (see the repo's `UPSTREAM_DEPS.md`):

| crate                 | version | note                              |
|-----------------------|---------|-----------------------------------|
| bitcoincore-rpc-json  | 0.19.0  | sync result types                 |
| bitcoincore-rpc       | 0.19.0  | (via bdk-bitcoind-rpc)            |
| corepc-types          | 0.13.0  | v29 async shape                   |
| bitcoin               | 0.32.9  | bark lock pin                     |
| bitcoin-units         | 0.1.3   | Amount + as_btc                   |
| bark-bitcoin-ext      | 0.7.1   | path: pinned checkout, `rpc` feat |

`bdk_bitcoind_rpc` resolves to 0.22.1 here (bark's lock pins 0.22.0).
It is only the re-export vehicle for bitcoincore-rpc (itself `=0.19.0`
and unified), so the types under test are unaffected; the difference
is recorded here deliberately.

## Running

```
./run.sh                 # or: cargo run --offline
cargo test --offline     # same vectors as #[test]
```

`run.sh` verifies the bark checkout is at commit c419159 and warns if
not. The build is fully offline against the local cargo registry
cache.

## Moving the bark checkout

`bark-bitcoin-ext` is a path dependency pointing at the checkout's
REAL path (`/tmp/opencode/bark-c419159/bitcoin-ext` in this
environment). Cargo resolves a dependency's workspace by walking UP
from the target directory, so a symlink inside `devcheck/` does not
work: the walk-up would find devcheck's own empty `[workspace]` table
and fail to inherit bark's workspace dependencies. If the checkout
moves, update the path in `Cargo.toml` and re-run.
