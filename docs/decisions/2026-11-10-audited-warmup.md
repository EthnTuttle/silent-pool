# 2026-11-10 - Audited empty and underfilled startup

## Authority

Parent-authorized clarification of existing SPEC §4.3. QA confirmed that
refusing all underfilled jobs deadlocks a fresh pool before its first real
shares. This does not authorize unverified bootstrap or change ladder, bits,
N or Q. Native/CPU activation and wallet-service code have separate owners.

## Decision

Admit legitimate underfill only with a complete independently sealed
source/economic checkpoint and explicit complete eligible-history evidence.
Use all eligible completed slices excluding the finding/open slice. Partial
windows use the existing slice formula with actual work/count; empty windows
use the existing front raw=0/pool=T adapter. Pending stays in full owed under
existing routing/revenue/error rules, never zeroed or donated for startup.

Recompute Q and derived actual_sum, shortfall, underfilled. Missing high water,
omitted accepted work, unproved pruning, missing history, false Q or coverage
reject. The same full-owned-output check applies to empty, partial and full
windows; current coinbase revenue, transaction/Merkle and witness checks run
even with no historical shares. Immutable witness reserved values are supplied
from authenticated job records when stripped coinbases omit that input.

## Shared API

`source_audit::verify_received_owned` is the common pure verifier.
`UnixSourceAuditor::from_keys` accepts borrowed already-loaded B_scan, public
B_spend, the pinned SP address/network, socket and distinct pool/collector keys.
It reads no secret file. Consuming `verify_received(self, ...)` binds the actual
native attempt/ReceivedJob and returns public Checkpoint/Consumed/Committed
evidence only after all checks and collector commit. Existing configuration
and translator wrappers delegate to this implementation.

The request-owned scan byte copy uses Zeroizing and explicit Drop cleanup.
Temporary secp SecretKey/Keypair wrappers call supported non_secure_erase on
every exit, including cancellation/error. These are best-effort supported
surfaces, not a claim that arbitrary compiler copies of Copy types are erased.
Key-file text is Zeroizing even on parse failure. No signer task is detached;
the exchange has a 10-second timeout and a pre-ACK deadline fence after CPU
verification. The existing wallet's long-lived online key remains its owner's.

Source-feed/2 adds mandatory WindowFacts to Consumed and signs them under
SPc/SourceConsumedV2. Commit binds that ACK digest; v1 activation frames cannot
act as warmup permits. ReceivedJob, trace, checkpoint and commit preimages are
otherwise unchanged. `verify_public_evidence` is the shared public verifier.
The separate native gate must still bind run/connection/epoch to the attempt.

Full measured acceptance must subsequently reach Q using actual native hashes
at reserved budgets. Source/regressions are authored; execution awaits ticket.

## Preservation

The 717-file predecessor remains
`0515a6a88686f75ccf4ed0ff768a0a0324c7a0b3ae4b95b5acd4a2c126a48a7d`.
Its 55 changed files (1,563,359 bytes) were archived before amendment at
`/tmp/opencode/ladder-warmup-predecessor-0515.tar.gz`, SHA256
`2d080c4ea9a6c760cb1aaf0de164622d38506d5fad19036fe5c014dd44b2c1bf`.
