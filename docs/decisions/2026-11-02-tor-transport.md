# 2026-11-02 - Tor as a supported transport (both directions)

**Trigger.** The spec closes address linkability but still exposes the transport
peer: the pool sees source IP, ASN and reconnect co-timing (leak vector 6 in
`docs/research/wiki/concepts/payout-attribution-privacy.md`), and the Ark server
and mailbox endpoint see the miner's IP (residual leak (b) in
`2026-10-14-mailbox-default-epoch.md`; `MAILBOX.md` §4 already asks for Tor
stream isolation as the mitigation but never made it configurable). `front` was
also only reachable on a clearnet interface, which excludes miners who cannot
accept inbound connections or do not want their hashrate tied to a hosting
provider.

## Decision

1. **Tor is a transport option, not a protocol mode.** No message, encoding,
   accounting rule or key changes. The SV2 Noise session, the SV1 session and the
   JDP session are carried unchanged; Tor is a byte pipe. `[reach]` and `[net]`
   are the only additions to the config shape.

2. **Inbound: onion services for `front` and `api`.** The Tor daemon owns the
   onion key and forwards to a loopback listener. The binary MUST NOT create or
   manage onion services. `[reach] onion` maps each listener to its advertised
   `<id>.onion:<port>`, and `[reach] advertise` is published verbatim as
   `endpoints` at `GET /api/pool`.

3. **Loopback binding is enforced, not documented.** If a listener has a
   non-empty `[reach] onion` entry, `spc-pool` MUST refuse to start unless that
   listener binds a loopback address. An onion-published listener that is still
   reachable on a clearnet interface is a silent bypass of the privacy it is
   meant to provide, and operator discipline is not a control.

4. **Outbound: `[net] proxy` for `spc-agent` and `ark`.** Upstream SV2, JDP,
   Ark server gRPC and mailbox reads use one remote-resolution SOCKS5 proxy.
   Three rules are normative:
   - remote name resolution only (`socks5h://…` or equivalent). Local DNS
     resolution leaks the query before the proxy sees it;
   - per-connection stream isolation (distinct SOCKS credentials, Tor
     `IsolateSOCKSAuth`) whenever `stream_isolation = true`, which is the
     default when a proxy is set. Mailbox reads additionally open a new
     connection per epoch;
   - a `.onion` target with no proxy is a startup error. There is no direct
     route and no clearnet fallback.

5. **Scope of the proxy.** `[net] proxy` applies to miner-facing and Ark traffic
   only. It MUST NOT apply to Postgres, Redis or the template source: those are
   inside the deployment boundary, and proxying them adds latency and failure
   modes while hiding a misconfiguration.

6. **The downstream SV1 listener is unaffected.** ASIC firmware does not speak
   Tor. `spc-agent` is the anonymity boundary for its own ASICs; the local
   downstream stays plain TCP.

7. **Peer-IP controls are replaced, not weakened.** Onion clients arrive from
   the daemon's loopback address. Connection concurrency caps and per-`sp_id`
   share-rate limits are the replacement for IP reputation, and they also bound
   `§3.1`'s keep-open-on-`invalid-user-identity` behaviour.

8. **Multi-region needs an explicit choice.** One onion service terminates on
   one `front`, so a single address defeats the geo split. Deployments MUST
   either publish one address per front in `advertise` (agent rotates on
   failure) or put OnionBalance in front of the set. The onion key is a front
   secret and MUST be backed up with the front's other secrets.

## Why

- **It closes a named leak with no cryptographic change.** Leak vector 6 is a
  property of the transport peer, so it is exactly the class of leak a transport
  option can remove. Nothing in §2-§5 has to move.
- **It is cheap to state and cheap to test.** `socks5h` vs `socks5`, loopback
  binding, and `.onion` without a proxy are all startup assertions, not runtime
  heuristics. They fit the project's fail-closed style.
- **It serves reach as well as privacy.** A miner behind CGNAT or on a
  residential link can reach an onion `front` without any inbound port, which is
  the same class of reach argument that justified native SV1.
- **It is honest about what it buys.** Tor hides the network peer. It does not
  hide share timing, hashrate signature, amounts, or anything from the pool's
  validation. The spec says so in `§3.8`, so neither operators nor miners can
  overclaim.

## Alternatives rejected

- **Protocol-level Tor / onion-only addresses as a new identity type.** No
  benefit: identity is already the SP address and the wire protocols are
  transport-agnostic. It would add a mode without adding a guarantee.
- **Embedding an Arti client in the binaries.** Removes the system-Tor
  dependency but adds a large, security-critical dependency and a second
  networking stack, while removing the operator's ability to run one hardened
  Tor daemon for the host. Deferred; `[net] proxy` keeps the door open because
  the SOCKS interface is the same.
- **`socks5` with local DNS resolution.** Rejected as a leak, not as a
  preference: the DNS query reveals the target even when the TCP connection is
  proxied. `spc-pool`/`spc-agent`/`ark` reject it at startup.
- **Requiring Tor for every connection.** Rejected: it would put a
  latency-sensitive JD role behind a circuit by default, exclude miners who
  prefer a direct path, and make the pool dependent on one anonymity network.
- **Letting the binary manage onion services.** Rejected: it would mean holding
  onion keys in the pool process, duplicating a mature daemon, and putting a
  long-lived service key on the same host as `a_send`. The daemon owns the key.
- **IP-based rate limiting preserved for clearnet with a Tor exception.**
  Rejected: it produces two control planes and makes onion clients
  second-class. Per-connection and per-`sp_id` limits cover both.

## Consequences

- Two config blocks (`[reach]`, `[net]`) and one startup validation each on
  `spc-pool`, `spc-agent` and the `ark` role.
- `GET /api/pool` gains `endpoints`; it is public data (an onion address is
  meant to be published).
- Operators gain Tor daemon setup (NixOS snippet in `docs/OPERATIONS.md` §2),
  onion-key backup, and a decision between per-front addresses and OnionBalance.
- Reconnect transients get more common on Tor (new `extranonce1`/`extranonce2`,
  VarDiff ramp). This is a documented cost, not a spec change; the accounting
  path already treats a reconnect as a new session.
- The `ark` role's proxied egress hides the pool host's IP from the ASP but not
  the wallet, destinations, amounts or timing. The spec says this explicitly so
  it is not sold as wallet privacy.
- Tests: startup rejections for non-loopback onion listeners, `.onion` without
  proxy, and local-resolution proxy URLs; an onion-reached miner mines and is
  paid identically to a clearnet one (ROADMAP Phase 8).

## Edits applied

- `docs/SPEC.md`: frontmatter changelog; `§3.8` added; `§6.1` `endpoints`;
  `§6.3` `[reach]` and `[net]`; `§9` per-front onion services; `§11.3` `ark`
  egress; `§11.7` mailbox network residue; `§11.10` note.
- `docs/ARCHITECTURE.md`: `§5.1` transport note.
- `docs/OPERATIONS.md`: `§2` ports table and "Tor reach" subsection.
- `docs/ROADMAP.md`: Phase 8 task and acceptance.
- `config/spc-pool.example.toml`, `config/spc-agent.example.toml`: `[reach]`,
  `[net]`.
