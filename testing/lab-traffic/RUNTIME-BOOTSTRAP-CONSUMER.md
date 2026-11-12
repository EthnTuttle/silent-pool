# Runtime bootstrap ownership and corrected base CLI admission

The corrected Runtime is now source/test/binary verified at **base diagnostic
scope**: `/tmp/opencode/runtime-fix-diagnostic-r1/bundle.json`, SHA256
`4d3778e8d4c65cffa95af483828fe1013a1d4307da32b3469801ce1624f036fd`.
The CLI is `52e45eabfbcbc53921c670a19b36cb6940a7228f5d8b4bc15c43556bedce402b`,
with Runtime source `e4c57fcd9b2f8bcf35d7cb2b60afd3da7f5327ca5d9084990cbce01ac3938478`
inside `/nix/store/11icfby7ni0185p093hxmbsahyl0p2dl-source`.
The full native-audited/Ladder/Rule B/control bundle remains pending.
The subsequent actual run proved the phase wait but failed the host P2P
boundary. `52e45eab...` is therefore blocked from another funding launch;
see [BASE-NETWORK-BOUNDARY.md](BASE-NETWORK-BOUNDARY.md). Its verified phase
semantics remain recorded, and a new tested network-fixed bundle is required.

The launcher uses `spc-lab runtime`, which starts HTTP and then automatically
executes `scenario.phases[*].commands`. The actual g105 source is
`/nix/store/wr1ix2s3hnhjhhmi2dv7v5cfd0mb28gc-source`; its Runtime SHA256 is
`fb0a5ccc9ad0c16ed3f104de1cd1bf7b7d16f1414c4257279aec8aab0579ce65`.
It requires a declared bootstrap route, but rejects that asynchronous command's
Running result instead of waiting. The source's intended wait is misplaced in
the StopRun duration branch. The corrected source moves the wait into the
actual phase path before observation, retaining the original operation ID.

The AP5 blocker record is preserved at
`lab-ark-funded/testing/lab-ark-funded/RUNTIME-BOOTSTRAP-BLOCKER.md` in its
owner's worktree. No g105 rerun or producer change is made by this consumer.

## Launcher correction

The previous consumer both required empty phases and submitted an explicit
HTTP bootstrap with a new UUID. That conflicts with this Runtime contract.
`runtime_bootstrap.py` now requires exactly one declared BootstrapFund command,
with an explicit stable original ID, actual funding host/node and run binding.
The default scenario supplies that declaration. No additional automatic phase
command or implicit bootstrap route is admitted.

Startup observes provisioned, bootstrapping or active states; it does not depend
on seeing the potentially transient provisioned state. If Runtime has already
mined before the first raw sample, the earlier genesis-observation requirement
remains INCONCLUSIVE rather than being fabricated from the current tip.

Bootstrap, stop-during-bootstrap and maturity cases all observe/control the
original declared operation. `prepare_command` refuses BootstrapFund, so even
accidental use cannot allocate another ID or replay through HTTP. The supplied
g105 `run` entrypoint also executes declared phases; no older manual-bootstrap
entrypoint is inferred or used without a separate actual source contract.

## Principal, outcome and budget binding

Runtime's in-process submit uses principal `controller` when expanded access
is enabled, with the actual declared actor or default `runtime-phase`. HTTP
uses the authenticated operator principal. `RuntimeBootstrap.observe` checks
the exact original Begin, `expanded_v1` principal/run/kind hash (or the actual
legacy kind-only form), command ID, actor, driver and node.

Polling reads retained private CommandLog evidence. It never obtains status by
resubmitting the original command as operator. RejectedConflict or a denied
cross-principal replay is not the original bootstrap execution outcome.
Successful bootstrap requires its original durable terminal and the matching
Runtime phase observation; all cases require that ownership/phase join in the
final receipt. The inherited non-wallet `finish` keeps `journal_seq=0` inside
CommandOutcome; the outer Finish row carries the actual journal sequence.

Exactly one HashStart must follow the original Begin. Every reservation belongs
to that operation, and its first captured boot ID/deadline/rate/cap remains
immutable. Reservations remain charged across unresolved batches and actor
incarnations. Separate operator pause/resume/cancel commands bind their own
principal and target the original operation; they create no new hash budget.

The maturity hook target is exactly:

```
bootstrap/<sha256(DECLARED_PHASE_command_id)>/00000000
```

The existing authenticated pause and acceptance hooks operate against that
target while Runtime waits. Missing or late evidence remains inconclusive;
neither original deadline nor cancellation grace is reset to get another try.

## Admission and test scope

Known g105 Runtime source stays explicitly blocked. The corrected Runtime/CLI/
source-manifest triple is admitted after independent verification of all 833
actual Nix source rows, 13 binary hashes, 17 passing check-log seals, the six-app
one-source derivation, the exact ELF probe/script/source/prefix tuple and the
sealed canonical confirmed-funding admission. The lib 77/0 log contains all
three actual execute_phases regressions: same-ID success, operator stop, and
failed terminal. Producer tests were inspected, not rerun by QA.

Admission rechecks actual Runtime and executable bytes as well as the exact
tested manifest digest. A novel hash or caller `fixed` flag cannot authorize it.
This proves base CLI readiness; the separate native source/capability gates
still prevent this pre-Ladder/Rule B/native-audit/control bundle from passing
the full campaign. The AP5 owner owns the fresh confirmed-funding run.

Orchestration fixtures use the actual Runtime command/phase/principal/hash
formats, including same-ID observation, independent operator controls,
Running/terminal/conflict distinctions and unchanged hash debit. They are
consumer tests, not a duplicate producer wait implementation or native funding.

## AP5 status

The g105 base diagnostic bundle exists. Its canonical admission passed, but
the actual Runtime run exposed the phase bug. Parent reports all three AP5
runs archived, zero user payments and all recorded actors gone. Four unreadable
same-UID processes leave global cleanup INCONCLUSIVE. No cleanup pass is inferred.

Parent reports AP5's 69-test diagnostic admission policy moved to `0d35cb...`.
That source-policy change does not alter this consumer's retained v2 raw-proof
schema/hash lineage. Full crypto, current float/lifecycle and native-audited
acceptance remain gated on their actual proofs and tested joined bundle.
