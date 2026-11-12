# 2026-11-09 - Private traffic forwarding evidence and acceptance interposition

## Authority and scope

The user authorized these controller-owned adapters for approved T3/T4b
acceptance requirements. This is an additive, separately versioned delta on
the tested J1/T1 packet, not a new acceptance of funded traffic. The contextual
difficulty amendment remains owned by T2. Node binaries, canonical contract
754e1e61, control major 1, history 3 and derivation 2 remain unchanged.

## Decision

1. Every actor-to-node RPC handoff uses the closed `rpc_forward` envelope. The
   controller appends `traffic_audit` version 1 records to its existing strict
   private command log, synchronously flushing and syncing before dispatch.
   Session rows bind run, driver, incarnation and actual controller/actor binary
   hashes. Attempts bind RPC id, target, method, send sequence, params hash and
   actor core command/journal/chain/intent revisions. The latter are evidence,
   never grants. Raw bytes, cookies and authorization headers are not emitted.
2. Each attempted dispatch, including transport failure, gets a correlated
   result and a complete watermark. Missing end records remain unresolved on
   replay. A complete watermark cannot be emitted over an unresolved attempt.
   Pause drains the existing gate mutex and records closed watermarks around
   its durable actor acknowledgment; resume records before reopening. A failed
   log write poisons the same command log and prevents further forwarding.
3. The measured boundary is also appended to that log. Actual AddConnection
   pipe writes have durable before-write records with exact wire-command ids
   and binary bindings, followed by result records. They require a recorded
   measured boundary. Configuration declarations do not count as forwarding.
4. `traffic.lab_acceptance` defaults false and is bound into the private run
   manifest and actor configuration. Opt-in requires expanded operator access.
   The closed `traffic_acceptance` command selects a driver incarnation and
   durable target command. It may arm a bounded signed-before-intent hold or a
   one-response loss, release the exact captured raw hash, or cancel. It never
   provides arbitrary RPC, PoW, funds, accepted/rejected claims or admission.
5. While held, the actor's original command remains pending. The actor observes
   real chain changes on the same single-owner loop. Release re-observes before
   normal core intent creation and immediate forward revalidation. A retained
   coinbase becoming immature therefore still prevents sending. Signed bytes,
   reservations and change selection are not rewritten or released by a hook.
6. Hook transitions are separate typed evidence, stored in the same command
   log. Timeout/cancel prevents that target from forwarding; stale incarnations
   cancel conservatively. One-response loss records the actual transport
   outcome hash first, marks one exact correlated response discarded, and
   returns Unknown to the actor. The core retains normal Unknown semantics.

## Verification and limitations

Focused tests cover durable replay, unresolved/duplicate ends, incomplete
watermarks, failed-connect attempts, logging failure before connection,
AddConnection-before-boundary refusal, hook enablement, exact release binding,
restart conservatism, one real socket response loss and closed-gate forwarding.
An owned txd process test supplies explicitly synthetic raw chain data to
exercise Signed -> held -> retained coinbase rollback -> release -> no intent.
Those are adapter tests, not funded acceptance. Actual funded tests require the
contextual T2 join and real nodes. Definitive rejection activation remains
separate and source-bound. No rejection code family is guessed or broadened.

## Integration

The QA contract is `docs/traffic-acceptance-adapter-v1.md`. The matching SPEC
frontmatter fragment is for the serial integrator. Before/after row manifests,
separate patch and actual test receipts bind this delta to the tested T1 base.
