# Implemented one-process AP5/T3 coordinator

Entry: `shared_run.py`. Source loader: `shared_sources.py`.
Source descriptor: `qa-source-r20.json` (caller pins SHA256
`3328123104108f3eee5f2198d22ebc411f9de9c5d97e47ca14328bf258d414fb`).
Current evidence/source freeze: `/tmp/opencode/AP5-shared-coordinator-r4`.
The original 39-test r1 packet and qa-source-r18.json remain immutable.

## Actual owner APIs used

The complete 107-file QA r20 archive is verified against its pinned manifest
and extracted to a new private source directory. Python modules are compiled
from the verified byte map, with conflicting preloaded module names refused.
No imports come from the mutable QA worktree. Unknown descriptor/API versions,
substituted archives, missing/duplicate source files and hash mismatches reject.

The actual source exports used are:

- `regtest-traffic-e2e.py:Controller(args)`; `prerequisites()`, `start()`,
  `bootstrap()`, `measured()`, `reorg()`, `audit_live()`, `final_audit_after_drain()`,
  `check(name, callback)`, `result(name, state, detail)`,
  `receipt()`, `manifest()` and `audit_budget()`.
- The same Controller's `source_feed_runtime` object supplies `final_drain()`
  and `close()` from the actual `CollectorRuntime` implementation.
- `cleanup_proof.assess_cleanup`, `collect_resource_receipt` and
  `identity_observation` independently assess the single retained AP5 reap.

The coordinator uses the actual r20 Controller class directly. It no longer
subclasses audit dispatch or supplies r3's custom retention/proxy bridge. The
old bridge and its tests remain in the immutable r3 source packet and have been
removed from active imports, avoiding a collision with QA's real final_audit.py.
QA's independent cleanup loop is never invoked: one AP5 stop/reap remains.

## Execution and finalization

1. Full AP5 canonical bundle admission and the QA source descriptor are checked
   before actor construction. QA prerequisites then verify its actual inputs;
   the coordinator binds both consumers to identical scenario/run/source,
   executables and executed probe tuple, and the earlier shared deadline.
2. Only QA Controller.start creates a runtime. AP5 adopt_shared_runtime binds
   to the same in-process Popen object and fresh owned path. This is not old-run
   disk adoption. QA bootstrap observes Runtime's original phase, not an HTTP
   bootstrap mutation. AP5's observer-only startup performs no second launch.
3. QA's actual measured phase runs with a manual destination read from Alice's
   same-run native allocation. Its existing implementation drives the manual,
   witness and fault cases. The actual reorg phase follows. These phases run
   before AP5 funding/boarding so they cannot displace newly proved Ark anchors.
   AP5 funding, board and lifecycle observations then run, including ordinary
   and derived offboard. The actual qa.audit_live() collects required live facts
   with actors available. It does not call a drain or final audit.
4. The actual CollectorRuntime.final_drain runs once. Its returned complete
   capture result or exception is retained in both still-open evidence streams
   before teardown. A failure is never relabelled a drain pass.
5. Call the actual qa.final_audit_after_drain() BEFORE close. It consumes the
   exact sealed spc-lab-owned-economic-drain/2 boundary, retained raw export,
   source EOF, SQL/checkpoint journal and native prefix. It performs no fresh
   node RPC, SQL capture, drain or stop. The coordinator checks the closed
   returned status/scope/boundary/stable_key/checks envelope, exact boundary
   schema/run/hash, and sticky final_sealed_prefix state. A returned dict with
   FAILED or INCONCLUSIVE status is never promoted to success.
6. Actual collector close joins/quiesces its workers. AP5 performs owned stop,
   reap and census even after drain/reader/lifecycle failure. QA's resource
   assessor consumes that same reaper result rather than reaping a second time.
   The real r20 audit performs its before/after input checks while its collector
   finalization lock is held. Its failures remain sticky. There is no second
   audit using a custom proxy after the source-owned close.
7. Shared final observations are appended before streams close. QA writes its
   receipt; AP5 retains the receipt hash and seals its separate component
   verdict. Neither implies whole-lab or global cleanup acceptance.

Finalization is non-reentrant and idempotent. A repeated run call refuses a
second runtime. Concurrent/reentrant finalization never starts another drain
or teardown. Cancellation interrupts the execution phase; signals received
during finalization/receipt publication do not interrupt owned cleanup.

## Invocation

Use the admitted Python 3.14 interpreter with the frozen harness:

```sh
python3.14 -B shared_run.py \
  --bundle /absolute/qualified-full-bundle.json \
  --qa-inputs /absolute/qualified-QA-inputs.json \
  --scenario /absolute/shared-scenario.json \
  --qa-source /absolute/qa-source-r20.json \
  --qa-source-sha256 <externally-pinned-descriptor-sha256> \
  --out /absolute/fresh-shared-root --timeout-secs 7200
```

The coordinator selects operator_ui=True, allow_reorg=True and witness=True
for the explicitly requested full funded case flow. r19's witness phase
retains its own test-only census for the two fixture keys; those do not become
SP wallet, Ark delivery, funding or hashing authorities. The scenario and QA
inputs must carry real independently owned collector
bindings and the complete approved source union. No default/fabricated keys,
source policy, proof, historical SQL or success adapter is constructed here.

## Verified scope and remaining source integration

The preserved r1/r2/r3 packets retain their 39/21/27-test claims only for their
original sources. This r20 revision passed 23 affected tests, including actual
r20 final methods on signed synthetic QA fixtures. Controlled
hook fixtures verify one
Runtime/bootstrap observation, AP5-before-drain ordering, checkpoint/EOF/raw
SQL before successful teardown, failed drain before cleanup, actual timeout
and cancellation handling, reader/result-write failures, idempotence and no
second cleanup loop. Fixtures are unit sequencing evidence, not live proof.
The real frozen r20 module closure imports. Its actual final methods reject a
late invalid economic/source event and a late invalid native miner event; the
coordinator retains nonpassing results and performs one close/stop. Node fixture
call counters do not increase during final audit. An initial 30-second tool
timeout produced no completed test receipt; the bounded 120-second retry passed
all 23 tests in 38.708 seconds. The final-source check (including the boundary
byte cap) passed the same 23 affected cases in 39.884 seconds.

The QA r20 API guide is pinned to SHA256
`24c8693c749b4f9168453c98f5a4acf8a3ae47b414cb924b3543e610b3ab7bef`.
Its native outer-map parsing is source-only conformance against 8259f003; this
does not establish compiled native production behavior or live acceptance.

All traffic drivers are acknowledged paused before final drain; r20's already
acknowledged bootstrap-driver pause is reused. The independent ordinary driver
is not left broadcasting after the final history observation. The post-cleanup
hash-budget check has a separate case name, so it cannot overwrite the final
prefix's hash-budget result/detail.

The actual r19 export delta is hash-bound to
`b82e2b92474e01004d0e26f8e56f1692ce03aba9b2cf4bbf118a5fcb8d80aa94`.
Its operator-ui flag and runtime/private collector-root corrections are now
consumed directly; positive and mismatch cases passed. r18 remains available
only as its exact historical loader profile; the live entrypoint requires r20.
Same-run scenario comparison handles QA's Decimal JSON parser exactly, without
coercing strings/bools or hiding a changed decimal value. Combined exit success
requires both actual QA and AP5 verdicts plus cleanup, not Ark-only completion.

The v3 readiness source TIME correction, full Nix source union,
canonical bundle/QA input descriptors and durable historical producer fields
remain dependencies. r19's fullrail arithmetic/history adapter is implemented;
its PRODUCER-FIELDS-NEEDED.md specifies the still-missing SPc-side observations.
Native outer-retention source 8259f003 is supplied but uncompiled. The r20 parser
is tested source conformance, not runtime production proof. No qualified
new shared run has occurred. All failed network archives, v2 projection bytes,
r17/r18/old r5 evidence, successful reap and compiled verifier remain preserved.

The r19 dead-miner lookup limitation is resolved by consuming r20's actual
post-stop retained-frame/process API. Missing evidence still returns a nonpassing
native verdict; no prior live-prefix pass is reused or repaired.
