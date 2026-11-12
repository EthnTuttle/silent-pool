# 2026-11-06 - LP-runtime host integration

This record covers the lab runtime, not pool/payout/Ark protocol behavior. The
approved LP-1, LP-2, LP-3 and LP-4a worktrees remain the authority for their
components; this integration combines their artifacts and adds a built-in real
host runner.

## Decisions

1. **The runtime renders configs, it does not merge operator fragments.** Node
   arguments, pool TOML, miner TOML, Redis config and PostgreSQL cluster setup
   are produced from the versioned scenario plus allocated resources. The only
   generated pool authority material is the LP-3 private file; there is no
   external handwritten TOML or fragment merge path.
2. **Identity persistence is write-once.** Each declared identity slot gets a
   private mode-0600 artifact under `private/identity-<id>.json`. Re-preparing
   a run directory loads the existing artifact and refuses to parse/regenerate
   a damaged one. Browser add-miner reuses an unused declared identity; it never
   mints a replacement after a partial crash.
3. **Add-miner is the existing typed command.** The browser builds `AddMiner`
   from a read-only `/catalog` projection and posts it through the durable
   command log. Pool switching uses the existing `SwitchPool` command. No
   command meaning is changed.
4. **Front API listener.** `FrontListen` gains an optional `api` endpoint so a
   rendered pool TOML can bind the real `/api/pool` listener. Existing scenarios
   omit it and continue to validate.
5. **Redis ownership exception.** `redis-server` clears its environment and
   rewrites its process title. For the `redis` role only, ownership verification
   accepts a missing run token when the kernel start-time tick, executable path
   and executable hash still match. Every other role requires the run token
   exactly. The residual risk is that an unrelated same-binary redis process
   started at the same kernel tick would be indistinguishable; the other three
   checks make that boundary narrow and explicit.
6. **Evidence is collector-fed, never derived from command success.** The pool
   log is tailed from its beginning into the LP-1 `spc_evidence` adapter. Miner
   ACKs are not treated as accounting. PostgreSQL and Redis are read with their
   authenticated clients; a failed read is recorded as an error observation.
7. **Requested and measured rates stay separate.** `runtime_metrics` reports
   the durable requested budget and the miner v1 `Snapshot` attempt/hash-rate
   counters, plus submitted/ACK/rejected/unknown/block-qualified/stale and
   canonical-observation counters. A block-qualified share is not a canonical
   block, and no causal hashrate-response claim is made from budget commands.
8. **No namespace claim.** The host runner runs real processes in the host
   namespace. Network-namespace placement, address publication and controller
   RPC transport are declared unsupported and handed to the LP-4 integration
   gate; the primitives themselves are unchanged.

## Consequences

The real host slice proves process/config/storage/evidence integration. It does
not prove the full rbitcoin suite, controlled retarget response, TDP ladder, Ark,
or namespace placement. `runtime_metrics` and the report keep those claims
separate.
