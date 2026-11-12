# Read-only original-board proof request, via parent

## Resolved by tested r3 handoff

The requested interface is implemented and consumed. Authoritative handoff:
`/tmp/opencode/pool-wallet-control-packet-r3/pool-wallet-control-proof-handoff-r3.json`.
Producer source snapshot is `dd9f0c37d7e79474a56132170272fcd6509eb3ebdc501aaa93372e8dd8a4102b`;
tested code is `8ec102f55081c58f2ff29aaadb5a78c56c9531a1245a74ffa77b484e077e0e42`.
`pool-control-proof-lineage.json` binds the frozen inputs and explicitly
separates this interface milestone from joined/funded acceptance.

Same executor/config/request file, action `board_proof_v1`, new read id and
unchanged `original_board_operation_id`. No money fields. The r3 envelope
retains full original bytes, anchor, owner/full policy, original request,
board facts, historical confirmation/checkpoint and terminal-context digest.
The current controller uses this interface for historical proof collection.
No current stock key-row lookup or remote GetVtxo reconstruction is required.

The remainder records the original request for audit history.

## Original request (historical)

The stable r1 result DTO has the original funding txid and nullable VTXO id,
but no retained full proof or public owner-policy binding. Please supply the
final tested producer's exact read-only same-live-wallet interface, including:

- Concrete executable/argv and request/reply schema/discriminator.
- Reference to the unchanged main board operation id and exact original VTXO.
- Retained full original VTXO bytes including genesis/signatures, with public
  owner and complete policy binding from that wallet's original board record.
- Original raw anchor identity and server/policy facts, correlated with the
  historical terminal receipt.
- Retention/retrieval behavior after automatic refresh/spend and stock-server
  pruning, plus the descendant history needed to relate that historical board
  to a separately observed current float.
- Typed missing/unavailable result; missing proof must remain inconclusive.
- Tests and immutable source hashes covering the actual interface and the
  original-spent/refreshed/pruned cases.

The board mutation is never resubmitted under a new id. This request adds no
server RPC and grants no second signer or wallet writer. AP5 will bind a
producer-supplied read-only contract only after its concrete tested handoff;
it will not guess a remote lookup after pruning. The current stock GetVtxo
path can only return a still-retained proof, and is not evidence that the
required historical read-only producer interface exists.
