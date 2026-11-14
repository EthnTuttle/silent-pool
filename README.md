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
The current tree also includes native funding/traffic, SP-aware wallet and
Ark lifecycle implementations and their source/audit interfaces. Implementation
and package qualification are not a full funded-campaign verdict. See the
source-pinned lab setup below and `docs/reviews/` for exact executed scopes.

## The two ideas

1. **SPc — silent payments in the coinbase.** BIP 352 cannot be used in a coinbase because the input has no public key. SPc supplies one: a per-height ephemeral key `A_send`, published in the coinbase itself (scriptSig pool tag in pool-template mode, `OP_RETURN` under Job Declaration), with the BIP 34 block height as the nonce. Each miner's output is `P = B_spend + hash_SPc/TierSpc(ecdh ‖ 0)·G`; a wallet finds it from the block header, the coinbase, and its scan key. The pool erases `a_send` once the coinbase is built. Public write-up: [delvingbitcoin t/2833](https://delvingbitcoin.org/t/silent-payments-coinbase/2833).
2. **A sub-threshold rail that is not the pool's own ledger.** Earnings under the coinbase threshold are paid to a per-block Ark key derived from the same ECDH, once the block has 6 confirmations, from the pool's Ark float on a **third-party** Ark server — deliberately not one the pool runs, because arkoor's only security assumption is "sender or server honest." The block's single pool output (a fresh P2TR under the pool's hot Ark key) refills the float when it matures. Miners who don't register Ark delivery forfeit their below-threshold earnings to the pool.

## Components

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

Use the source-pinned setup in [LAB-RUNTIME](docs/LAB-RUNTIME.md). The current
application reference is
`github:EthnTuttle/silent-pool/98f0e6caa2482646e3c7073949f6388cd39f200b`;
Nix obtains the node, stock Ark server and application tools from published,
immutable sources. A developer worktree or prebuilt local node is not required.

Choose the intended profile:

| Profile | What it supplies |
|---|---|
| `spc-lab-self-hosted-ark` wrapper | One node, two pools, four wallet agents/workers at 8 H/s total; no traffic population or full acceptance campaign. Requires an explicit fresh run directory and provenance arguments. |
| [Prepared shared campaign](testing/lab-ark-funded/README.md) | Two nodes, two pools, four SP wallets/workers, eight background and four payment wallets, 40 H/s aggregate and 0.2 ordinary tx/s; native funding and audited lifecycle checks through the admitted runtime and a separately pinned QA/AP5 consumer. |

The application pin and QA consumer release are independent. Supply a published,
reviewed compatible consumer revision for the full campaign; a future consumer
revision is not guessed by these docs. Build/qualification, source admission and
a full funded campaign verdict are distinct results. Full funded acceptance is
not asserted here.

An operational controller started with `--listen 127.0.0.1:0` prints its actual
origin after startup. `/` provides live controls and user wallet views; `/lab`
is the operator directory. Expanded runs require the run-local token from the
appropriate `private/access/*.json` claim file. See LAB-RUNTIME sections 5-6
for exact token handling, live API stop, recorded review and ownership-bound
recovery. Headless `runtime` means omitting `--listen`, usually with a finite
`--duration-secs`; there is no `runtime --headless` switch. `nix run .` selects
the pool package, not a lab.

The separate design-feedback/mockup utility, often served on loopback port
18481, is not an operational lab UI or proof that a simulation is running.
It is not installed by this checkout's lab launcher. Preserve its feedback and
lifecycle independently of lab run cleanup.
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
