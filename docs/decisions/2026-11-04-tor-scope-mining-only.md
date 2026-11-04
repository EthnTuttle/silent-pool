# 2026-11-04 - Tor covers mining connections only

**Trigger.** The 2026-11-02 Tor decision scoped `[net]` to both mining traffic
and the pool's Ark egress: `spc-agent`'s upstream SV2, JDP, Ark-server and
mailbox traffic, plus the `ark` role's dial to the third-party Ark server.
The merged packet then shipped the mining half only (the ark role's bark
transport dials the server directly, and the agent's Ark traffic is not
implemented). Leaving the spec, the config surface and the code disagreeing
would either promise a privacy control that does nothing or force a proxy
connector into bark for a bounded gain. The operator has decided the scope.

## Decision

1. **Inbound is unchanged.** `[reach]` publishes onion endpoints for the pool
   listeners, forces every onion-published listener onto a loopback bind, and
   `GET /api/pool` serves `endpoints` verbatim. SPEC §3.8 inbound stands.
2. **Outbound `[net]` is `spc-agent` only, for mining connections**: the
   upstream SV2 connection and the JDP connection to `spc-jds`. Remote name
   resolution (`socks5h://`), per-connection stream isolation and the
   `.onion`-without-proxy startup error all still apply to those.
3. **No Tor for Ark.** The `ark` role's dial to the Ark server and the agent's
   Ark-server/mailbox traffic are direct: no proxy, no `.onion` gate, no
   per-epoch circuit requirement. `spc-pool` no longer has a `[net]` section;
   a pool config that still carries one fails to parse.
4. **Accepted residual.** With a direct Ark connection, the Ark server and the
   mailbox endpoint see the network address of the pool host or miner. Ark
   delivery privacy remains bounded by the Ark server's view (SPEC §11.7), not
   by Tor. A miner who needs network separation uses an operator-level
   control; this is not a pool feature.

## Why

- The linkable peer that matters for this pool is the miner's IP tied to
  hashrate and reconnect timing at the pool. That is the inbound and
  mining-egress path, and it is covered.
- The Ark server already sees every destination, amount and timing, and (with
  denomination payouts) the exact remainder of every paid miner. Hiding the
  network peer from that server does not change the trust assumption the
  design already makes, while it does add a custom gRPC connector, mailbox
  circuit plumbing and a second failure surface.
- The pool host's IP is operator infrastructure, not a miner identity. There
  is no receiver-privacy gain from proxying it.
- A no-op `[net]` on the pool would be a silent privacy failure: the operator
  would believe Ark traffic is proxied when it is not. Removing the key makes
  the configuration honest and fails closed.

## Alternatives rejected

- **Proxy bark's gRPC via a custom connector** (the 2026-11-02 scope): a large
  client change for a bounded gain, and it would still not hide destinations
  or amounts from the Ark server.
- **Keep `[net]` on the pool but ignore it**: silently ineffective; rejected
  by the fail-closed rule.
- **Forbid `.onion` Ark servers outright**: over-broad. A direct `.onion`
  target is a runtime misconfiguration, but it is the operator's network
  problem, not a pool policy; the config no longer gates it.

## Edits applied

- `docs/SPEC.md`: §3.8 outbound rewritten as mining connections only, with an
  explicit Ark-egress-out-of-scope paragraph; §6.3 pool `[net]` removed and
  the agent block re-scoped; §11.3/§11.7/§11.10 Ark/mailbox egress clauses
  replaced with the direct-dial consequence; frontmatter changelog entry
  `2026-11-04`.
- `bins/spc-pool/src/config.rs`: `net` field, `ArkProxyUnsupported` and
  `OnionWithoutProxy` removed, with the Ark proxy/onion checks and their
  tests; a pool `[net]` is rejected by the unknown-field rule.
- `config/spc-pool.example.toml`: `[net]` removed with a pointer to this
  record; `config/spc-agent.example.toml`: re-scoped to mining connections.
- `docs/OPERATIONS.md`: reach/proxy rows and the "Tor reach" section.
