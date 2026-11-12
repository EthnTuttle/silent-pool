# Native lifecycle revision for N1/N2

Status: authored and formatted; **uncompiled, tests not run**. Wallet-service
owner holds the build ticket. This source response addresses
`/tmp/opencode/native-activation-audit-a0d1d04.md` and needs the same auditor's
recheck after the next assigned build turn.

The reviewed source `a0d1d04f4d49c6b7c1345ae7fe31bfa2084e43e9abb74f72014113b6218a67f7`,
its 56-test receipts and CPU
`aee91a918046dbe753f313bef3c05fb12608e359aec0d4b93df038fce607b6a1`
remain preserved at `/tmp/opencode/native-audit-tested-r1`. Those passes do not
validate this revision. The public native codec, AddConnection DTO and signing
preimages are unchanged.

## N1: revocation independent of outbound backpressure

- A dedicated writer task owns the Noise write half. The actor serializes
  bounded owned frames and uses `try_send` into a 64-frame queue; there is no
  socket-write or queue-capacity await in the active-work select branches.
- Writer failure/overflow tears down the connection. Every session exit fences
  work and aborts pending audits before any graceful I/O. Graceful close has a
  500 ms budget, after which the writer owner is aborted and joined. Its partial
  frame/socket/encryption state is dropped, never resumed on the same stream.
- `request_stop` stops/fences workers synchronously, before the actor is awaited.
  `stop_connections` delegates to a two-pass routine that signals every handle
  before awaiting the first close/join. Worker publication checks permanent
  stop while holding the assignment lock, so an already-stopped worker cannot
  accept a late publication.
- `clear_work` releases the assignment lock before revoking the old permit,
  and acquires the stats lock only after the per-hash fence. The hash lease
  contains only SHA256d, never socket I/O or assignment/stats acquisition.
- Submitted remains an unresolved transport attempt. A queued or interrupted
  attempt is marked transport-unknown on teardown; no ingress success is
  synthesized from queue admission.

New regressions are in `connection/lifecycle_tests.rs`. A one-byte Tokio duplex
transport reports actual `poll_write -> Pending` after writing a strict prefix
of a frame. The peer is held unread until revocation/stop assertions complete.
Tests drive the production protocol actor, bounded writer, audit verifier and
real hash workers. The wallet returns fixture-signed source-feed/2 responses.

1. With a share partially written, deliver a new job or target and assert the
   old permit is revoked within 250 ms while the writer is still blocked;
   hash counts stabilize. Supersede a pending audit and then stop with another
   audit pending; late replies cannot commit. Teardown exposes only the first
   byte to the peer, with no resumed write or subsequent CloseChannel.
2. Activate two audited connections with hard share targets, then stop-all.
   The first CloseChannel is provably blocked on the bounded transport while
   both permits and the later worker are already stopped. Counts stabilize
   before either writer is released.
3. Block a partial write, fill precisely the 64 queued slots, reject the next
   enqueue, and prove dropping the writer discards the partial transport.

These bounded-transport regressions complement the existing real pinned-Noise
native tests, which must be rerun for this actor change.

## N2: original admission and verification clock bounds

- Sample monotonic time before wall time at candidate admission and reuse that
  original deadline in `timeout_at` and the candidate context.
- Return the paired verification sample alongside CPU-local verified evidence.
  Permit creation takes the minimum of original admission, verification-derived
  proof expiry and completion-derived proof expiry. Completion at/after that
  bound fails closed. Consuming a successful reply never restarts the timeout.
- Once a permit observes expiry or a clock failure, it remains invalid even if
  the wall clock subsequently rolls back.

Five injected-sample tests in `native_gate/clock_tests.rs` cover mono9/wall109
verification followed by a nine-second rollback before activation, original
deadline boundaries, normal and rolled-back shorter proof expiry, forward
jumps at completion, and prevention of resurrection after observed expiry.
No test changes host time.

## Next assigned verification

Use the existing T1a target, own-ticket/nonblocking flock and disk guards:

- CPU library filters `connection::lifecycle_tests` and
  `native_gate::clock_tests`.
- Existing `live_session` native audit cases and the affected pacing/stop
  regression. Config/target cases as needed by actual failures.
- CPU all-target check, strict clippy, formatting, then build and seal a new CPU.
- Preserve every failure and source hash, and publish final receipts for the
  same auditor. Unchanged scanner/codec crypto test passes stay on their
  original receipt; this source revision does not relabel them.

Actual AP3 wallet/collector/controller joined acceptance remains separate.
