# V3 raw readiness: consumer implementation and source recheck

Input: `/tmp/opencode/ibd-readiness-owned-socket-source-r2/HANDOFF.md`.
All seven hashes at lines 98-104 matched the primary files when read. Exact
copies were frozen before further union changes at
`/tmp/opencode/AP5-readiness-v3-consumer-r1/producer-source`; its manifest binds
365987 source bytes. This is a seven-file observer review, not the full union.

## Implemented consumer

`readiness_v3.py` implements the actual v3 DirectedDialProof, raw brackets,
process/namespace/fd/TCP rows, dual peer samples, listener evidence, process
binding, acquisition record and bounds. It rejects v2 and missing/extra/duplicate
fields. The exact fixture SHA256 is
`ec4a81f78fa5a921489ed91cd6625a06ca8a54cfe19c5ea4e2f590d854decdf8`.
Its null operation ID is accepted only as a structural directed fixture; a
runtime report requires the independently supplied original operation/run.

The consumer recomputes both owned reversed ESTABLISHED joins in both brackets,
checks raw fd membership and listener inodes before/after, observed host/node
namespace identity, actual exe against the supplied manifest and pinned node
hash, both peer identity tuples and nondecreasing counters, complete directed
coverage and common sampling intervals. Summary booleans cannot replace raw
evidence. Acquisition bounds are exact source constants, with the whole deadline
derived from the original operation deadline and the source's 90000 ms cap.
Trusted success/HashStart observations must remain inside that deadline; those
context values cannot be reconstructed from missing report fields.

Fourteen targeted Python tests passed against the frozen directed fixture and
explicitly synthetic complete reports. They cover changed raw tuples/fds,
listener summaries without rows, namespace/executable/process substitution,
dual-peer/time changes, null operations, v2, altered bounds, duplicate dials,
unproved additional P2P transport, late success/HashStart and rejection of the
ad-hoc bundle envelope. They are not a live
gate or propagation pass. `source_admission_conferred` remains false.

## BRACKET improvements verified in actual code

In frozen `crates/spc-lab-txd/src/net_readiness.rs`, lines 342-363 read actual
namespace dev/inode and exe; 893-927 capture process/boot/host data before
stamping; 952-1024 serialize raw owned rows/fd sets; 1032-1072 hold dual peer
samples and checked ordering; 1331-1599 validate stable peer/process/namespace
identities and both raw joins. In frozen engine `traffic_runtime.rs`,
1512-1605 checks the before bracket/listeners, 1611-1707 obtains both peer
samples with after-read stamps, and 1726-1830 checks after-listener ownership,
inode stability and supervisor identity. Report fields at 1879-1900 retain
the listener/process observations. These address the prior missing raw fields.

## TIME remains blocked: late current time is still accepted

Frozen engine hash:
`b115d4dfdfa404864881e371d4786b5cbe78e252b9d55567942d9fbff3ec8572`.

The gate now captures a never-reset start/deadline and checks successful
completion, but `success_within_deadline` at 2635-2646 uses
`completed.unwrap_or(now)` and compares only that value to the deadline.
When completed is Some, now is ignored. Its test at 2814 explicitly accepts
`Some(10000), now=10600, deadline=10000`.

The call at 1313-1322 is before persistence. After the gate returns, lines
2060-2074 acquire the traffic lock and write HashStart with no fresh deadline
check. The separately inspected command/traffic_budget.rs start_hash_budget
requires a nonzero deadline but does not compare it to current time; only later
reserve_hashes does. Thus on-time completion can still produce a late ready
record/HashStart after a stall. This does not prove a hash debit happened, but
violates the requested pre-HashStart success/deadline boundary.

Required producer correction: check completed AND fresh current time, preserve
monotonic order, and recheck the original operation/acquisition deadline at the
actual transition before HashStart after persistence/locking. Reject and retain
the failed outcome; do not create a new operation or deadline. The consumer
already rejects the late-now/late-start cases; no weaker source approval is
created by those tests.

## Supplied gate log is not the claimed green result

The retained v3-affected-tests.log is a 25-line failed tail: 111 passed,
1 failed, 1 ignored. The failure is
`net_readiness::tests::node_netns_differs_from_host_controller_refuses`.
It does not establish the handoff's later claim of 112 passed or all integration
suites. A later completed green log may exist, but it was not in this supplied
file. Complete final logs/source bindings and the actual v3 live regression
remain necessary; no producer Rust test/build was run by this consumer.

Readiness version 3 is not a bundle-envelope version. The generated
spc.lab.readiness.bundle/v3 shape is not accepted. See
`CANONICAL-BUNDLE-PRODUCER-INPUT.md` for the actual AP5 envelope/inventory.
The partial source union, old CPU0712 and uncompiled native8259/historical
observation additions confer no full capabilities. No live run was started.

## 2026-11-12 source-only follow-up

Producer engine b5da9192 corrects the late-now helper and adds a check after
ready persistence/traffic locking. The exact recheck and remaining command-log
lock/original-deadline-equality findings are in
`TIME-FIX-SOURCE-RECHECK-20261112.md`. That source is still uncompiled/unadmitted;
the withdrawn green claim and all original fixture/test scopes remain unchanged.
