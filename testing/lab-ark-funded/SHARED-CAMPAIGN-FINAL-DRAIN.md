# Shared full-campaign capture and final shutdown coordination

Integration input: `/tmp/opencode/T3-checkpoint-drain-r17/handoff.json`.
QA contract: `testing/lab-traffic/CHECKPOINT-CAPTURE-DRAIN.md` in the
`lab-funded-acceptance` worktree, source SHA256
`c266ffe8da968205b56f3f1bf55063a5b828ab9750c5d858684d428b0731b107`.
The handoff records 95 affected consumer tests and 98 archived source rows,
parent-verified. It records no live joined capture/drain execution.

This is an integration requirement for the eventual full shared campaign.
Implementation update (2026-11-11): `shared_run.py` now provides the one-process
coordinator using source-pinned QA r18 hooks. See `SHARED-RUN-ENTRYPOINT.md` for
the tested call order, exact API and remaining qualified launch dependencies.

The current base confirmed-funding diagnostic remains blocked on fresh
exact-profile IBD readiness and the first real common block. It does not
acquire a full-QA final-drain prerequisite from this note. The new shared
shutdown sequence is implemented separately; no live shared run is claimed.

## One shutdown coordinator, ordered capture boundaries

Before a shared run starts, the campaign owner and AP5 owner must bind both
consumers to the same run/actor identities and designate one coordinator for
final shutdown. Independent harness stop/finally paths must not race to stop
pool writers, SQL or source readers. Use the existing QA capture/drain consumer
and oracle; do not introduce a second historical oracle or fabricate state.

The successful final sequence is:

1. Drain the declared mining processes with existing controller commands,
   verify their original process identities exited, and quiesce feed grants.
   Keep source/checkpoint readers, pool writers and SQL available. Issue no
   extra mining authority to complete settlement.
2. Require retained exact candidate checkpoints for every frozen/advertised
   job, bound to the original revision, parent, anchor, front, history watermark
   and digest through the existing retention/oracle path. Preserve original
   captures and observations. A current-view read is not a missing historical
   revision; notifications schedule reads but confer no admission authority.
3. Require canonical raw/economic ancestry and economic acceptance/consumption
   for locally mined pool blocks, plus due-ledger/publication and signed-source
   convergence. Native-bootstrap and other pools' blocks require canonical
   chain records, not invented local ledger rows. Preserve shallow canonical
   liabilities unbooked; no forced booking, deletion or target changes.
4. Require the QA consumer's three stable tip/revision/history-watermark/
   source-head samples. Only then stop owned pool writers using PID-bound
   handles after run/process/start/executable/config identity checks. Keep
   SQL and readers available for the final capture. Writer EOF is not proof
   of successful process reaping.
5. Drain the source watcher through EOF and recheck raw tip, original SQL and
   source. Retain the final raw SQL capture. Changed bytes, incomplete tails,
   changed stable keys, missing exact checkpoints, reader/journal/queue errors
   or deadline failure prevent a converged/certified final seal.
6. Retain the QA final boundary and exact-job coverage, then allow the shared
   coordinator to stop the remaining roles and perform AP5 owned-resource
   cleanup. Seal final evidence/verdicts after the appropriate captures and
   cleanup observations have been retained. A failure path must retain its
   missing/failed boundary rather than label an early teardown converged.

The final drain is a capture/termination gate, not an entitlement, activation,
revocation, current-float or Ark lifecycle proof. Exact checkpoint coverage
can fail even when a scoped convergence observation is valid.

## Separate cleanup and acceptance facts

AP5 reaping, recorded-owned resource release and exhaustive global cleanup
remain independently evaluated. QA source/checkpoint thread shutdown failures
also keep cleanup inconclusive; a closed writer or EOF cannot override that.
The four preexisting unreadable processes remain untouched and cannot be
ignored to manufacture a global cleanup pass. No serial campaign proceeds on
a false cleanup success.

Old r5 missing-H23 economic acceptance/consumption and all its seals remain
failed. Later captures cannot supply its missing revisions or upgrade a raw
block/publication to economic acceptance. Preserve every failed network run,
the 2484 diagnostic, successful reap and compiled-verifier evidence.

This coordination note changes no public wire format, proof schema, admission
registry or executable. Actual full-campaign integration still requires the
tested source union and live shared capture/drain evidence.
