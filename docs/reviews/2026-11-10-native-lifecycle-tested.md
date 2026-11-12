# N1/N2 lifecycle verification

The assigned build turn executed the lifecycle revision whose initial full
source hash is `d8cb05e420f115ab9181682a646e458210817ec551fe885b05411bbddae90323`.
All production source from that revision is unchanged. The only correction
was removal of a redundant `.into()` in the new test frame builder, diagnosed
by strict clippy. Its failed receipt is retained. This addendum is the other
post-test source change.

## Actual results

22 distinct targeted tests passed:

| Scope | Count | Log stem under `/tmp/opencode/native-audit-build-turn` |
|---|---:|---|
| Partial-write revocation, multi-connection stop, bounded writer disposal | 3 | `n1-regressions-r2` |
| Original deadline, rollback, shorter proof lifetime, forward expiry | 5 | `n2-regressions-r1` |
| Existing native Noise/CPU audit and configuration cases | 11 | `lifecycle-native-r1` |
| Interruptible pacing/stop and budget rebase | 1 | `lifecycle-stop-control-r1` |
| Control idempotency and pool-switch lifecycle | 1 | `lifecycle-control-r1` |
| Asynchronous submission/batch-ack correlation | 1 | `lifecycle-writer-correlation-r1` |

CPU all-target check and strict clippy (`-D warnings`) pass. The N1 tests were
rerun after the test-only clippy correction; unchanged N2/native tests retain
their original source-bound receipts. The prior 56-test freeze and old CPU
are not relabelled as passes for this revision. Unchanged scanner/codec suites
were not broadly rerun.

N1 tests force a one-byte transport to partially write and return Pending.
They prove permit revocation and stable hash counts before releasing the
writer, reject late pending approvals, fence later connections while the first
close remains blocked, and observe only the partial byte after writer disposal.
The writer queue limit and overflow refusal are asserted. The production Noise
writer remains exclusively owned by the task discarded on cancellation.

N2 tests inject separate verification/completion samples. The mono10 admission
bound survives wall rollback after mono9 verification. At-boundary completion
rejects, shorter proof expiry stays binding, and an observed forward expiry
cannot be resurrected by a later rollback. No host clock was modified.

## Handoff and remaining scope

Final source, exact delta, all invocation receipts and the new CPU binary seal
are at `/tmp/opencode/native-lifecycle-tested-r1/handoff.json` for the same
auditor's recheck. Public codec `39c75d44...` and source-feed/2 are unchanged.

The wallet owner's existing joint fixture is a meaningful next component
check, but its frozen test explicitly asserts the old CPU hash `aee91a9...`.
The parent must bind that assertion to the newly sealed CPU for a separate
rerun; this packet does not edit the wallet owner's sources or relabel the old
joint success. Real controller lifecycle, independent full history, four-wallet
40 H/s full-Q and funded stock-Ark acceptance remain separate integration work.

Every Cargo invocation used the assigned ticket, nonblocking global flock,
existing T1a target, `CARGO_INCREMENTAL=0`, an archived pre-invocation source,
and actual target/store disk checks. No cleanup was performed.
