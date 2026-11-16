# 2026-11-15 - Explicit chain identity and fail-closed bindings

Status: DRAFT for dedicated future Testnet5 integration, not adopted into the
lab. Lab approval and separate deployment authorization remain prerequisites.

## Decision proposed

1. Separate consensus `ChainNetwork` from historical SP/BIP32 encoding
   `Network`. Recognize public BIP95 Testnet5 with genesis
   `00000000000008c1637e7d3252dc1c333d4546fd6fff6a474b200d674a91d497`.
   Shared `tsp`, `tb`, Base58 and tpub/tprv do not identify test chains.
   Preserve address/descriptor/key bytes, derivation paths and SPc/Id preimage.
2. Keep codes 0 mainnet, 1 testnet4, 2 signet, 3 regtest. Add 4 testnet5 and
   5 testnet3. Preserve historical aliases at their original boundary only:
   pool/scanner generation `testnet` means testnet4; old agent `testnet` means
   testnet3. Canonical chain parsing has no ambiguous `testnet` alias.
3. Check actual `getblockhash(0)` against the known genesis before endpoint
   use. Settlement/scanner reads recheck through their own client/view. The
   network wrapper checks the same configured authenticated template endpoint
   before/after GBT and before submit, and requires the BIP54 packet's private
   active marker on Testnet5. No RPC label rewriting. Genesis is identity, not
   a full validation, best-work or malicious-server authentication proof.
4. Add independently versioned chain metadata, exact canonical
   `(version=1,network,network_code,genesis_hash)`, with display-order lowercase
   genesis hex. Old SQL and bootstrap have no global chain identity; SPc/Id
   and address prefixes cannot supply it. Unknown or missing metadata fails.
5. Add singleton `pool_chain_identity` via additive migration 0013. Payout
   initializes only fresh empty chain state. Existing unbound history requires
   a separately reviewed provenance migration, never assignment from config
   or HRP. Check any present binding before migrations, and require it before
   state reads/writes. Mismatch never wipes, rewrites or converts data. Existing
   migration checksums, monetary rows, descriptor bytes and codes stay intact.
6. A dedicated SPc Redis DB has a non-expiring `spc:chain-identity`, initialized
   atomically by payout only into an empty DB. Readers require exact metadata
   before groups/listeners. Unbound nonempty state requires explicit migration.
   Stream bytes stay unchanged; producers/readers upgrade coherently. This
   key remains SPc-side, never an Ark input or an extension of its ACL.
7. New-chain remote bootstrap is `{chain,snapshot}` with the old snapshot
   nested unchanged. Consumers validate metadata and identity row codes before
   apply. The actual binary currently uses direct SQL bootstrap with a required
   database binding; a future remote server must supply the same envelope.
8. Stock Bark c419159/rust-bitcoin 0.32.9 cannot select Testnet5. Ark enabled,
   effective Ark role and explicit Testnet5 wallet requests fail before state
   or RPC effects. No encoding alias, dependency bump, RPC spoof, server fork
   or self-host exception. Agent Testnet5 execution remains unavailable until
   wallet and authenticated job/source contracts carry chain identity. Its
   explicit Testnet4 wallet also refuses rather than selecting legacy Testnet3.
9. Scanner RPC find requires canonical `--network`. Unknown names reject.
   Offline recovery keeps its byte format without a chain ancestry claim;
   `find --blocks --network` refuses to certify sparse data from a label.

## Why independent versioned metadata

A new enum alone cannot bind existing SQL/Redis history. Silently inserting
required fields into the old unversioned bootstrap risks old readers ignoring
them. A separate exact envelope avoids rewriting payout snapshots and Ark
messages. Unknown legacy provenance needs its own migration decision; assigning
it from whichever endpoint config currently selects would bless wrong-chain
reopen. This source packet intentionally holds such old data rather than
claiming a tested migration. Startup changes require dedicated integration.

## Preserved policies and prerequisites

SPEC §3.4/§4.1/§9 v2 remains regtest-only. N, fees, denominations, fail-closed
parsing, no donation fallback, registration and privacy boundaries stand.
SPEC §11.3 self-hosting remains regtest/signet only. BIP54 and public work/audit
policy are separate packets. No deployment or complete-suite readiness claim.

## Sections proposed

SPEC frontmatter, §2.1, §5.1, §5.3, §6.1-§6.3, §9, §11.5/§11.10;
WIRE §3.2 codes and §3.5 envelope. No pin or frozen-vector changes.

## References

- BIP95 `bitcoin/bips@f82c4612768827ea26e7b08cc895696db61bf073/bip-0095.md`;
  Core `ee505609676a10c389da5962b25a2c0080b9df57`,
  `src/kernel/chainparams.cpp::CTestNet5Params`.
- UPSTREAM_DEPS.md; binding decisions 2026-11-02-bark-adapter,
  2026-11-08-agent-wallet-foundation, 2026-11-05-lab-lp1-integration and
  2026-11-08-self-hosted-lab-ark.
- Durable deployment-research Testnet5 REPORT.md, source index N3/N4/A1/P1.
