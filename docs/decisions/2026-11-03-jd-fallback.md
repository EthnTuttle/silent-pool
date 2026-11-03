# 2026-11-03 - JD fallback: declines, bounded silence, full-template rebuild

**Trigger.** SPEC §3.5/§3.6 said what the JDS publishes and how the JDC checks
its own payout, but not what the JDC does when the JDS declines a
`DeclareMiningJob`, answers nothing, or when the prev-hash moves while a
declaration is outstanding. The implementation reflected the gap: the JDS
accepted or *silently dropped* declarations (a warning in its own log only),
and the JDC treated those cases as fatal errors propagated out of the role -
which in `spc-agent` also kills the translator task and stalls pool-template
mining. A miner could sit on stale work or stall outright.

## Decision

1. **The JDS always answers a parseable declaration.** Every
   `DeclareMiningJob` (JDP) and `SetCustomMiningJob` (the MP custom-job path
   the module also serves) gets `.Success` or `.Error`; `spc-jds` is silent
   only on frames that fail Noise/codec decode, where no `request_id` exists
   to answer. Error codes (`STR0_255`); the first four are project codes under
   the negotiated `0x0003` extension, in the spirit of PR #203 §7:

   | Condition | `error_code` | JDC action |
   |---|---|---|
   | Declaration malformed: undecodable coinbase, missing/duplicate `distribution_id` TLV | `invalid-job` | hard fallback |
   | No distribution published on this connection yet | `no-payout-distribution` | transient: wait for the first publish |
   | Id unknown, outside the grace slot, or height-straddling | `stale-payout-distribution` | transient: retry on the next publish/template |
   | Recomputed outputs mismatch the declaration | `invalid-payout-distribution` | hard fallback |
   | Pool-side internal inconsistency | `internal-error` (upstream code) | hard fallback |

   Coded declines are protocol messages, so they are distinguishable from
   transport errors by construction; transport failure is never a fallback
   trigger, it tears the role down (a supervisor restarts it).

2. **The silence window is config: `[jd] declare_timeout_secs`, default 10,
   fail-closed minimum 1.** A value of 0 (or a missing `[jd]` for a JD role)
   is a startup error, not a clamp. The same 10 s the roadmap already names
   for negotiation silence now also bounds the wait for the declaration
   answer; the two waits are separate windows (`JD_NEGOTIATION_TIMEOUT` for
   connect/negotiate/first-distribution, unchanged).

3. **Fallback = declare-bounded, then hold and alarm; there is no
   pool-template mode on the JDP session.** Concretely:

   - The JDC declares at most once per **declaration key**
     `(distribution_id, template)` - where "template" is the whole
     `JdTemplate`: height, `coinbase_tx_value_remaining`, extranonce
     length, and the transaction list (`wtxid_list`). It waits up to
     `declare_timeout_secs` for the matching `.Success`/`.Error`, retaining
     any distributions published meanwhile, then records the outcome.
   - On a hard decline (`invalid-job`, `invalid-payout-distribution`,
     `internal-error`, or an unknown code - fail closed on the unfamiliar) or
     on silence, the JDC latches the fallback: it stops declaring, alarms
     exactly once per episode (error level), and stays on the connection.
     Transient declines (`stale-payout-distribution`,
     `no-payout-distribution`) do the same hold at warn level; they are
     expected at height races.
   - Recovery is event-driven, never timer-driven: a *changed* declaration
     key (a new `SetPayoutDistribution` - which the pool owes on every
     `SetNewPrevHash`, on rebalance and on ledger apply - or a new template)
     triggers exactly one fresh declaration. There is no retry loop; against
     a permanently silent JDS the alarm fires once and nothing is re-sent
     until an input moves.
   - Mining continuity for an `spc-agent` that runs the translator role does
     not depend on any of this: pool-template jobs keep flowing on the MP
     connection per the base protocol. That path is the agent-level fallback
     and it is *unaffected*, not *entered* - the JDP session never carried
     mining jobs and this packet adds no such traffic to it.

4. **The fallback can never mine a job that cannot be paid.** The only job a
   latched JDC issues is none. Resumption runs the same §3.6 gate as the
   happy path: under `require_own_payout = "refuse"` a distribution whose
   coinbase omits our `P` (or whose carrier is missing, so the check is
   impossible) enters the same latched fallback **before** any
   `DeclareMiningJob` is sent - the miner stops rather than hash unpayable
   work. Under `"warn"` the published policy already allows absence (below
   threshold, dust-pruned, trimmed), so the declaration proceeds and the
   warning stands. Recovery clears the latch with one info-level line.

5. **`SetNewPrevHash` abandons and fully rebuilds.** A template move (the
   JDC's own TP delivering a new prev-hash/height) invalidates the
   outstanding declared job: the JDC abandons it with a logged reason
   (`new-prev-hash` when the height moved, `distribution-replaced` when the
   distribution id did), then rebuilds the *whole* template - coinbase from
   the retained distribution and the new template's transaction list into
   `wtxid_list` - and declares under the newest key. JDP has no cancel
   message, and none is needed: the JDS grace rule (§3.5, unchanged by this
   packet: previous id, same front, same connection, same height, never after
   a settlement republish) makes the abandoned job unacceptable to the pool,
   so nobody can keep hashing it and get paid. A declaration that races the
   republish draws the coded `stale-payout-distribution` decline, which the
   latch treats as transient - at most one wasted exchange per height move,
   no false alarm.

## Why

- **Fail closed, not fail dead.** The pre-packet behavior (propagate the
  error out of the JDC role) took the whole agent down with it, including
  the translator serving pool-template work: a JD hiccup became a mining
  outage. Latching inside the role keeps the failure scoped to JD.
- **Event-driven recovery matches the publication contract.** The pool must
  republish on every prev-hash, on material rebalances and on ledger apply,
  so a healthy JDS always produces a recovery trigger; a timer the JDC
  picks would only guess at that cadence and could busy-loop a dead peer.
- **Codes over silence** is PR #203 §7's own shape; without them the JDC
  cannot distinguish "your coinbase is wrong" (bug, alarm) from "you lost a
  height race" (normal). The split alarm level reflects that.
- **The own-payout gate on the fallback itself** removes the one way a
  "helpful" fallback could silently donate: re-deriving work from an
  unverified distribution. Refuse mode means the miner stalls loudly instead
  of hashing for outputs it will never scan.

## Alternatives rejected

- **Extend JDP with a pool-template job feed ("mine the pool-template job on
  the same JDP connection").** JDP defines no job-delivery messages; this
  would be a nonstandard protocol extension on top of the pinned PR #203
  surface, and it duplicates what the MP connection already does for every
  spc-agent that runs the translator role. Rejected as scope creep with a
  privacy cost (a second work source to keep consistent).
- **Retry on a timer (e.g. every `declare_timeout_secs`) while declined.**
  That is the busy loop the packet forbids; a silent JDS would be re-dialed
  and re-declared forever with no new information.
- **Treat `stale-payout-distribution` as hard.** Height boundaries make a
  same-instant race between the pool's republish and the JDC's template
  update normal; alarming on it trains operators to ignore the alarm.
- **Reconnect on silence instead of latching.** Adds re-handshake churn for
  no observability gain; the latch plus event-driven retry has the same
  recovery point (the next publication) without the traffic.

## Edits applied

- `docs/SPEC.md`: §3.5 "Declaration answers" and "JDC fallback" subsections,
  §3.6 fallback interaction sentence, §6.3 `[jd] declare_timeout_secs`,
  frontmatter changelog.
- `docs/WIRE-FORMATS.md`: `0x0003` row notes the decline-code set lives in
  SPEC §3.5.
- `config/spc-agent.example.toml`: the new key.
- `bins/spc-pool/src/jds.rs`, `bins/spc-pool/src/jds_server.rs`: coded
  `.Success`/`.Error` answers on both declaration paths.
- `bins/spc-agent/src/jdc.rs`, `bins/spc-agent/src/config.rs`: bounded
  declaration await over a never-cancelled read pump, coded-decline
  outcomes, latched fallback with event-driven recovery, declaration-key
  de-duplication, full-template (`wtxid_list`) rebuild and abandon-with-reason
  on template moves.
