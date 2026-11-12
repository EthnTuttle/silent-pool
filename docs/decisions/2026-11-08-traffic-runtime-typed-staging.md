# 2026-11-08 - Typed traffic RPC and staged startup

## Scope

Runtime plan v5 sections 1, 2 and 3.3. Source candidate; compilation and
native funded acceptance are pending.

## Decisions

- The traffic-only host and namespace routes preserve parsed numeric node
  rejections separately from transport, broker, timeout, protocol and shape
  uncertainty. Response ids are checked before interpreting node errors.
  Gate closure is a distinct local result. Decimal RPC money is not consumed.
- A concrete Arc node registry is retained before NodeControl erasure. The
  actor receives only declared node ids, never endpoints or credentials.
- Fresh declared traffic preparation records pending preflight. Actual finalized
  node argv/environment are captured with inheritance disabled for those nodes.
  The controller's Rust profile consumer follows the canonical r4 grammar,
  including literal RPC compatibility policy tokens and six-field comparison.
- The canonical probe runs through the existing parent-death helper. Required
  live consensus facts, binary, genesis and normalized profile must match every
  actual node; the existing lineage gate remains authoritative and unpromoted.
- Miner processes may start after preflight, but AddConnection is deferred in
  traffic runs. The T2 executor remains unavailable by default. The tracked
  operation releases the command mutex and bypasses unconditional finish;
  cancellation and failure are terminal only when handled by that worker.
- No-traffic optional fields are omitted and the legacy transport path remains
  selected. No server or Ark-provider implementation is part of this packet.

## Remaining source joins

The implementation report explicitly lists the incomplete scheduler/observer
pump, completion verifier, complete restart reconciliation and pause metadata
join. No capability promotion or full runtime readiness follows from these
types. Source-bound definitive rejection rows remain disabled.

## Sections edited

Actor/control, traffic RPC/consumer/profile/runtime modules, namespace helper,
engine, configuration, supervisor launch, manifest, runtime and CLI. SPEC
frontmatter is supplied as a fragment for the serial owner.
