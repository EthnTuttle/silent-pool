# 2026-11-05 - SV2 TP keeps [template] rpc_url as its settlement chain view

**Trigger.** `[template] source = "sv2-tp"` now delivers templates over the
SV2 Template Distribution Protocol (TDP) from rbitcoin-node's Template
Provider and submits found blocks with `SubmitSolution`. TDP deliberately
has no confirmation query: the payout role still needs tip heights, active
chain block checks, the found coinbase's outputs and `getblockstats`
economics for settlement and orphan detection. A chain view had to be chosen
without pretending TDP provides it.

## Decision

1. **Templates and solutions are TDP only.** With `source = "sv2-tp"`, the
   pool gets templates exclusively from `[template] sv2_tp_address` under
   Noise_NX pinned to `[template] sv2_tp_authority_pubkey`, and submits
   exclusively with `SubmitSolution`. It never calls `getblocktemplate` or
   `submitblock` on the node RPC in this mode.
2. **Keep `[template] rpc_url` as the chain view, explicitly not templates.**
   It supplies confirmations/orphan checks (`getblockcount`, `getblockhash`),
   coinbase outputs (`getblock`) and block economics (`getblockstats`) for
   the payout role. Both a bitcoind cookie and rbitcoin-node's Core-format
   cookie are accepted by the existing RPC client.
3. **No `[chain]` section.** A separate section would duplicate URL and auth
   for the overwhelmingly common deployment where rbitcoin-node (or
   bitcoind) serves the TP and the RPC on the same daemon. Introducing one
   later remains possible if deployments genuinely split the two services;
   the config would gain a new optional key, not reinterpret this one.
4. **Startup refuses an unusable chain view.** `sv2-tp` with the `payout`
   role requires a non-empty `template.rpc_url` and an `rpc`-feature build;
   a payout role that would settle without active-chain and orphan checks
   fails closed rather than starting. A front-only TDP deployment keeps the
   existing regtest/signet dev escape (`accounting.allow_unchecked_chain`)
   because a front does not settle anything.
5. **Everything else is unchanged.** The existing `source = "rpc"` retains
   its combined template-plus-chain-view use of `rpc_url`; found-block
   events, scanning and confirmation accounting continue as before.

## Why

- The chain view is a different responsibility from the template source,
  and conflating them let the original config silently suggest that an
  sv2-tp deployment needed no RPC at all. The smallest honest statement is
  one existing key with its role made explicit.
- TDP's `SubmitSolution` also has no acceptance acknowledgement, so the
  client treats the TP's ensuing tip push as the outcome and reports a
  competing tip as a rejection. The RPC chain view is what later confirms
  the result for settlement; neither channel substitutes for the other.
- A payout role without confirmations/orphan checks can book an orphaned
  block as payable. Refusing at startup is safer than discovering the
  mistake when an API shows an invalid confirmation.
- New sections are config surface forever: operators must understand and
  keep them in sync. The common one-node deployment does not justify them.
