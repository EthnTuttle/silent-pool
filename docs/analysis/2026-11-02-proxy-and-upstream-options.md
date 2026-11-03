---
title: "Proxy pools, upstream pools, and p2poolv2: options for spc-pool"
type: analysis
format: markdown
date: 2026-11-02
status: closed
supersedes: none
summary: "spc-pool cannot front another pool as a proxy and cannot be a p2poolv2 node: both move coinbase and accounting authority away from the pool, which is exactly what the SPc and Ark payout rails depend on. Verdict: not a mode of spc-pool."
---

# Proxy and upstream pool options

## Question

Two adjacent architectures were considered for v0.1 and beyond:

1. Could `spc-pool` act as a **proxy pool** with another pool **upstream** - miners connect to silent-pool, silent-pool forwards their work to a third-party pool, and payouts flow back through silent-pool?
2. Could `spc-pool` **be a p2poolv2 node** (https://github.com/p2poolv2/p2poolv2) instead of, or in addition to, being its own pool?

Note on terminology: in this repo "upstream" already has a miner-side meaning - the pool an `spc-agent` connects to (`[upstream]` in `config/spc-agent.example.toml`). Neither question above is about that.

## Short answers

- **Proxy pool with an upstream pool: no.** The pool is the payout authority by construction. It builds the coinbase that carries `A_send` and every miner's `P_i`, and it settles from the coinbase it built. Delegating the coinbase to an upstream pool destroys the non-custodial rail and replaces it with a custodial ledger the design exists to avoid.
- **p2poolv2 node: no, not inside `spc-pool`.** p2poolv2 is a competing implementation of the same layer (`stratum` front, template handling, block submit, PPLNS ledger), not an upstream. Adopting it means replacing `front`/`payout`/`spc-accounting` and giving up SPc, JD and the Ark tier, not adding a role.
- **The one legitimate adjacency:** run a p2poolv2 node as a **separate service** on separate ports with its own bitcoind view, as an independent miner. No code, config or state is shared, and `spc-agent` cannot point at it (§3.6 aborts when the coinbase carries no `push32(A_send)`).

## Why: the pool is the payout authority

Every payout tier is derived from data the pool itself generates for a block it builds:

- **§2.2** - `a_send[H]` is a fresh CSPRNG draw held only by `front`, published in the coinbase and zeroised on height advance. It authorises the whole payee set for that height.
- **§3.3** - `front` constructs the coinbase from `Distribution(H, T, A_send)` and only then issues jobs. Under JD the miner declares the job, but `spc-jds` validates it against the pool's own distribution (§3.5).
- **§4.7** - the distribution builder turns the live SLICE window into `P2TR(P_i)` outputs and per-block Ark destinations (`U_i`, `dest_id`) while `a_send[H]` still exists.
- **§4.6** - settlement reads the **mined coinbase** to compute `claim(id)` and `pending'(id)`, driven by the winning distribution's `entries` in `spc:blocks`.
- **§11** - the Ark tier derives `U_i` from the same per-block ECDH and sends from the pool's float six blocks later; without `a_send` there is no Ark rail at all.
- **§3.6** - the miner's own-payout check exists because the coinbase is inspectable; a relayed upstream coinbase cannot be checked for `P_i` at all.

A relay keeps the miner-facing surface but loses all six. What remains is a normal custodial PPLNS pool with extra steps.

## What a relay product would require

If "hashrate forward" were ever wanted, it is a separate product with its own record. Minimum scope:

- **Upstream job ingestion** - map upstream `job_id`, prev-hash, target and version mask into the local share pipeline; §3.4 replay keys and slice sealing assume local job ids.
- **An accounting source** - either trust the upstream's per-worker ledger (custodial) or validate forwarded shares locally, which cannot prove they were accepted upstream.
- **A liability ledger** - upstream pays one account; per-miner balances, carry-forward and withdrawal become the operator's obligation, reported at `/api/pool/liability`.
- **A payout rail** - no per-miner coinbase outputs, so on-chain batching or Lightning replaces both the coinbase tier and the Ark tier. This is the FPPS-shaped system DESIGN.md argues against.
- **A privacy statement** - the upstream learns the proxy's connection metadata and share timing; miners learn nothing verifiable about the upstream's accounting.
- **An SPc-aware upstream or no SPc** - only a pool implementing §2-§4 can carry `P_i`; no third-party pool does, and this codebase is currently the only implementation.

Do not present any of this as a `--roles` value or a `[template] source`; it is a distinct binary with a distinct trust model.

## p2poolv2 comparison

Checked against p2poolv2 `main` on 2026-11-02 (README, `spec/`, `p2poolv2_lib/src/{shares,stratum,accounting}`).

| Axis | silent-pool | p2poolv2 |
|---|---|---|
| Share unit | SV1/SV2 mining shares in fee-banded slices; no per-share rows, no share-chain PoW | Share-chain blocks with their own PoW; uncles weighted 9/10 plus a 1/10 nephew bonus |
| Accounting window | `N x d` slice window; `trim_to_budget`; pending carry-forward; SLICE scores | Fixed PPLNS window of 120,960 shares (about two weeks at a 10 s share target), weighted by share difficulty |
| Payout authority | `payout` role writes Postgres; settlement from the mined coinbase (`spc:blocks` entries) | Share chain determines the payout set; peers rebuild the coinbase and verify a midstate plus commitment hash |
| Template source | Local bitcoind over `rpc`/`ipc`/`sv2-tp`; JD via `0x0003` with `spc-jds` in `front` | Its own bitcoind over RPC plus ZMQ; coinbase built per template, payout outputs sized for up to ~500 miners |
| Coinbase control | Pool builds it; carries `A_send` (scriptSig or `OP_RETURN`) and `P2TR(P_i)` per miner | Payout set fixed by the share chain; coinbase is deterministic enough for peers to validate; carries a share-commitment `OP_RETURN` |
| Payout identity | SP address; `sp_id` never on the wire; fresh `P_i` per `(miner, height)` | Plain Bitcoin address in the stratum username, committed publicly as `miner_bitcoin_address` |
| Attribution privacy | Hidden from third-party chain observers, visible to the pool (stated in §0) | Published to every peer by design; "strictly worse than a centralized pool on this axis" (prior-art survey) |
| Sub-threshold rail | Ark tier: 6-block confirmation, third-party server, hot float, 28-day trustodial | Many-output coinbase now; tradable shares sold to market makers later |
| Miner protocol | SV2 (Noise) plus native SV1; JD `0x0003`; type-32 share accounting | SV1 only; username is the payout address, `p2p=` password owns the share; `hydrapool` mode is standalone PPLNS with no share chain |
| License | MIT OR Apache-2.0 | MIT OR Apache-2.0 (reuse is legal; the blocker is architecture) |

`hydrapool` mode is the closest analogue to silent-pool - a standalone PPLNS node over its own bitcoind - and is a direct substitute, not a component.

## The real blocker for any SPc/p2poolv2 synthesis

p2poolv2's `PayoutDistribution` trait (`simple_pplns`, `sharechain_pplns`) is the natural seam, but it returns `Vec<OutputPair{Address, Amount}>`; carrying `P2TR(P_i)` and `A_send` means generalising it to script plus carrier. Worse, `a_send` has no owner in a peer-to-peer share chain: every node must build the same coinbase, so `A_send` would have to be committed in the share block, derived from the tip, or DKG'd. Each option either breaks §2.2's "fresh draw, never derived" rule or puts the sender key in a public consensus object. That is a research question, not wiring, and it is the only path worth writing a decision record for if decentralization of the pool function becomes a goal.

## Recommendation

- **Closed: not a mode of `spc-pool`.** Do not add proxy, upstream-pool or share-chain backends to the v0.1 binary or config.
- Keep the pool as the sole payout authority. The non-custodial property is not a feature that survives delegation.
- If a relay product is ever wanted, spec it as a separate binary with its own liability and payout rail, and state the custody change before writing code.
- If decentralization is the goal, the honest fork is p2poolv2 (transparent, no Ark, no SPc) **or** silent-pool (operator, private against chain observers, SPc plus Ark), not both in one process.
- The single legitimate adjacency today is operational: a p2poolv2 node as a separate service, with no shared state and no shared miner-facing listener.
