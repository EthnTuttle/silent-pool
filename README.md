# silent-pool

A Stratum (SV1 and SV2) mining pool that pays miners to **BIP 352 silent-payment addresses**: one-time, unlinkable P2TR outputs **directly in the coinbase** for earnings above a threshold, and **Ark (bark) out-of-round payments** below it. Accounting is **SLICE / PPLNS-JD**, so miners build their own block templates via Job Declaration and are paid for both hashrate and the fees they selected.

**Status: implementation phase, bounded lab slice.** The core is running:
`spc-crypto`, `spc-scan`, `spc-accounting`, `spc-store`, `spc-ext` and
`spc-share-ext` are implemented and tested; `spc-pool` serves native SV1 and
SV2, builds the SPc coinbase, assembles and submits blocks, settles through the
payout role, and serves the read-only API; `spc-agent` translates SV1 miners to
SV2. The opt-in v2 `network_block_work` work-unit profile is implemented for
regtest only; type-32 remains v1-only and is refused, not advertised, in v2.
A real-bitcoind regtest run (`testing/regtest-e2e.sh`) mines blocks and recovers
a miner's coinbase output with `spc-scan`. The approved host lab adds a real
two-pool runtime with rbitcoin, Redis and PostgreSQL (`docs/LAB-RUNTIME.md`).
Ark settlement, the denomination ladder and full-window payout auditing remain
separate gates. An isolated two-node/two-pool native-SV2 namespace lab has an
implementation report under review; it does not inherit the host approval. See
`docs/OPERATIONS.md` and `docs/reviews/` for exact scopes.

## The two ideas

1. **SPc — silent payments in the coinbase.** BIP 352 cannot be used in a coinbase because the input has no public key. SPc supplies one: a per-height ephemeral key `A_send`, published in the coinbase itself (scriptSig pool tag in pool-template mode, `OP_RETURN` under Job Declaration), with the BIP 34 block height as the nonce. Each miner's output is `P = B_spend + hash_SPc/TierSpc(ecdh ‖ 0)·G`; a wallet finds it from the block header, the coinbase, and its scan key. The pool erases `a_send` once the coinbase is built. Public write-up: [delvingbitcoin t/2833](https://delvingbitcoin.org/t/silent-payments-coinbase/2833).
2. **A sub-threshold rail that is not the pool's own ledger.** Earnings under the coinbase threshold are paid to a per-block Ark key derived from the same ECDH, once the block has 6 confirmations, from the pool's Ark float on a **third-party** Ark server — deliberately not one the pool runs, because arkoor's only security assumption is "sender or server honest." The block's single pool output (a fresh P2TR under the pool's hot Ark key) refills the float when it matures. Miners who don't register Ark delivery forfeit their below-threshold earnings to the pool.

## Components (planned)

| Binary | Role |
|---|---|
| `spc-pool` | Stratum pool; roles `front` / `payout` / `api` / `ark`; native SV1 **and** SV2 listeners; SLICE accounting; SPc coinbase; Ark settlement. `front` includes the `spc-jds` module — Job Declarator Server implementing sv2-spec PR #203 (`0x0003` Coinbase Transaction Payouts), always on |
| `spc-agent` | Miner-side: SV1/SV2 translator + JDC + payout verifier + Ark wallet, one binary, one seed (optional when the miner connects to the pool directly over SV1 or SV2) |
| `spc-scan` | Reference scanner / wallet helper for SPc outputs |

Miner identity is the silent-payment address, full stop: it is the only thing that keys a miner's account, and both payout tiers are derived from it per block. Separately, a miner MAY register a private **delivery capability** for the Ark tier (`B_ark` plus mailbox blinding data, signed with the scan key) over a small project-defined SV2 extension; it changes where the miner can be paid, never who they are, and is never published.

## Build and run

Nix is the only supported build path:

```sh
nix develop
just check                 # fmt + clippy + tests
nix flake check            # what CI runs
nix build .#spc-pool .#spc-agent .#spc-scan

# real-regtest end to end (bitcoind + pool front + two SV1 shares)
./testing/regtest-e2e.sh
```

`config/spc-pool.example.toml` and `config/spc-agent.example.toml` are validated
starting points. The pool's `[template] source` is `"rpc"` for the regtest
harness, `"ipc"`/`"sv2-tp"` for the production TDP paths, and `"mock"` for a
smoke run.

## Lab quick start

The approved host slice is an operator/lab tool, not a production mode. From
the approved host-runtime source:

```sh
nix build \
  .#packages.x86_64-linux.spc-lab \
  .#packages.x86_64-linux.spc-pool \
  .#packages.x86_64-linux.spc-cpu-miner \
  .#packages.x86_64-linux.spc-scan \
  --print-out-paths

/nix/store/...-spc-lab-0.1.0/bin/spc-lab runtime \
  --scenario crates/spc-lab/scenarios/lab-live-v2-two-pools.json \
  --run-dir /tmp/spc-lab-run-1 \
  --node /path/to/rbitcoin-node \
  --pool /nix/store/...-spc-pool-0.1.0/bin/spc-pool \
  --miner /nix/store/...-spc-cpu-miner-0.1.0/bin/spc-cpu-miner \
  --scanner /nix/store/...-spc-scan-0.1.0/bin/spc-scan \
  --postgres-bin-dir /path/to/postgresql/bin \
  --redis-server /path/to/redis-server \
  --redis-cli /path/to/redis-cli \
  --base-revision b0ab4ca34fda07c61a6335d05f598eaacc69678c \
  --dirty-tree-hash <frozen-code-snapshot-sha256> \
  --listen 127.0.0.1:0 --duration-secs 120
```

The printed URL serves the browser controls at `/` and the read-only role /
process hub at `/lab`; `spc-lab serve` exposes both for recorded read-only
review. The hub links only actual routes and does not claim wallet or Ark
services are running. See [`docs/LAB-ROLES.md`](docs/LAB-ROLES.md) for the
current role map, missing role-native UIs and required next Ark/wallet
integrations. A headless finite run uses
`--duration-secs` without `--listen`. Private identities, credentials, node
cookie and generated configs live under `<run_dir>/private` (`0700` dir,
`0600` files); inspect with `spc-lab snapshot|events|manifest|command` and stop
with `stop_run`/duration/`interactive`; `spc-lab reap` only uses persisted
ownership records. The node is an explicit externally supplied pinned L2 build,
not a Nix package. See [`docs/LAB-RUNTIME.md`](docs/LAB-RUNTIME.md) for the full
command set, idempotency rules and known limits (no causal hashrate claim,
runtime header/payout oracle `inconclusive`, fee/witness/Ark/ladder/SV1 not
covered). Its namespace section documents actual actor/storage/control
placement and the separate partition/heal/crash-cleanup proof path.

## Documents

- [`DESIGN.md`](DESIGN.md) — the one-identity derivation: Ark-tier destination derived from the same per-block ECDH as the coinbase output (authoritative for the Ark-tier key scheme; spec §11.8 defers to it); prototype in [`spc_ark/`](spc_ark/)
- [`docs/SPEC.md`](docs/SPEC.md) — technical specification v0.1 (architecture, SPc protocol, SV2 surface, SLICE accounting, data model, interfaces, tests, deployment, §11 Ark rail)
- [`docs/ROADMAP.md`](docs/ROADMAP.md) — phased build plan with architecture decisions and risks
- [`docs/REGTEST-LAB.md`](docs/REGTEST-LAB.md) - review proposal for a headless/browser-controlled rbitcoin network with real SV2 CPU miners, competing pools, and network scenarios
- [`Regtest lab review synthesis`](docs/reviews/2026-11-05-regtest-lab-review-summary.md) - independent Fable, Opus, Kimi K3, DeepSeek and GLM feedback, with checked findings and corrections
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — component map, interfaces and scoping for implementation planning
- [`docs/MULTI-REGION.md`](docs/MULTI-REGION.md) — multi-region deployment design (its §10 deltas are mostly adopted into the spec; the spec wins on conflict)
- [`docs/WIRE-FORMATS.md`](docs/WIRE-FORMATS.md) — byte-level formats of the project's own Redis streams, `spc:arkreg`, `spc:ark:*` and the front bootstrap RPC
- [`docs/OPERATIONS.md`](docs/OPERATIONS.md) — build, deploy, v2 profile/retention, migrations and harnesses
- [`docs/LAB-RUNTIME.md`](docs/LAB-RUNTIME.md) — approved host-lab startup, browser/headless controls, artifacts and limits
- [`docs/LAB-ROLES.md`](docs/LAB-ROLES.md) — actual lab role/process map, `/lab` and `/` UI availability, namespace links and required next Ark/wallet integrations
- [`docs/share-accounting-ext-deltas.md`](docs/share-accounting-ext-deltas.md) — intentional type-32 deltas and the v2 refusal
- [`docs/reviews/`](docs/reviews/) — implementation-readiness review and its resolutions
- [`docs/decisions/`](docs/decisions/) — dated decision records; first entry: the 2026-10-12 assessment of sv2-spec PR #203's reference implementation
- [`UPSTREAM_DEPS.md`](UPSTREAM_DEPS.md) — exact commits this design is pinned to, and why

## Provenance

These documents were produced from a research wiki (`bitcoin-mining-payout-schemas`) that ingested the primary sources — BIP 352, the Stratum V2 spec and PR #203, the SRI/sv2-apps and bark source trees, Blitzpool, DMND's SLICE material, OCEAN's docs, and the relevant Delving/Optech threads. Every article and raw source the spec or roadmap cites is snapshotted under [`docs/research/`](docs/research/) with the wiki's layout preserved, so the documents stand alone; each raw file's frontmatter carries its original URL, ingest date and credibility rating.

## License

MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`), matching SRI, bark, `extensions_sv2` and the `spc_ark` prototype. Blitzpool (AGPL) is a design reference only: its ledger, trim and autoscaler patterns are re-implemented from the spec, no code is ported (spec §7.6).
