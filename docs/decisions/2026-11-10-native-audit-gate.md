# 2026-11-10 - Native per-candidate wallet audit gate

## Decision

The default full lab native SV2 path MUST obtain a current committed wallet
audit receipt before activating each candidate. Native pool connections and
their dedicated Noise readers remain native; no SV1 translator, extra wallet
process or CPU-held scan/master/SP spending key is introduced.

The existing wallet process owns the loaded-key auditor and signing authority.
It uses source-feed/2's shared verifier, including complete independent
history, actual owned-output checks and signed WindowFacts. A complete audited
underfilled window is admissible and remains labeled underfilled; missing
history is Unknown, never an unaudited warmup exception. Full campaign
acceptance independently requires Q reached.

## Candidate and lifecycle contract

- Every new job, prevhash, target or pool extranonce update clears old worker
  assignments and cancels pending approval. Session/candidate epochs and a
  fresh nonzero 128-bit nonce bind actual ReceivedJob bytes. No existing
  authorization is replayed on reconnect or candidate replacement.
- The private native request also binds actual SP identity, wallet-generated
  lifecycle instance ID, controller peer/config generation and request expiry.
  The actual wallet rejects an old instance before auditing/signing. The
  controller never invents wallet instance IDs or peer process identities.
- The shared public codec verifies source-feed/2 evidence using its existing
  public verifier, then verifies the new wallet activation signature. It
  admits no source-feed/1 fallback. CPU verification derives B_scan from the
  actual configured SP address and uses configured independent pool/collector
  public keys, never response-supplied authorities.
- One bounded asynchronous exchange per native connection, 10 seconds total.
  Noise receive and pause/stop remain responsive. Failed/unknown exchanges
  keep work inactive; a bounded one-second retry uses a new epoch and nonce.
  Expiry clears old work and requires renewed proof. Hash budgets never reset
  or debit for proof exchange. Worker revocation is fenced around individual
  real SHA256d calls, not delayed until a large hash batch ends.
- SourceAuditCommitted (including signed window facts) and JobActivated must
  enqueue successfully before worker publication. AuditFirstHash records the
  first actual hash afterward. Candidate/active/unknown and underfilled/full
  are distinct states; telemetry loss never fabricates a receipt.
- Explicit unaudited smoke profiles remain supported and labeled unaudited.
  Missing source_audit configuration is not an implicit fallback. Full lab
  adapters require an existing wallet service unless explicitly marked fixture
  or `spc.miner.native.unaudited`.

## Runtime peer authorization

Wallets publish their actual instance/socket before CPU launch. The controller
checks supervisor ownership, PID/UID/start ticks, boot ID, executable bytes,
run-token/config hashes and actual config arguments, then atomically publishes
the expected peer/config binding before every initial/dynamic AddConnection.
The required wallet authorizer compares SO_PEERCRED/proc facts with that closed
registry before signing. Missing/corrupt/mismatched metadata fails closed.
Attach validates existing processes/config/socket/registry without creating
wallets, resetting generations or minting allowances. Current scope is local
host wallets; namespace expansion requires an explicit follow-up.

The existing funding and fresh registration barriers remain prerequisites.
The native gate adds per-job authority; it does not replace either barrier.

## Wire, ownership and verification

Wire/preimages/types are in `NATIVE-JOB-GATE.md` and
`crates/spc-scan/src/native_activation.rs`. This is a lab/private-wallet
activation contract implementing SPEC §3.6's before-mining requirement. Pool
payout derivation, accounting, Ark protocol, share submission and ownership
boundaries are unchanged. AP3 owns the actual wallet service/authorizer; the
ladder owner owns full-source/underfill verification and collector evidence.

Tests distinguish signed fixture authorities from actual joined-run proof.
Source/authored checks alone never establish real audited native mining.
