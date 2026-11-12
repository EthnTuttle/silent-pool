# 2026-11-10 - Early acceptance hooks and pre-sign version rejection fixture

## Authority and source evidence

The parent/user authorized the concrete QA producer-gap fixes. UI-v2 completed
19 focused tests plus clippy/fmt at
`/tmp/opencode/traffic-runtime-r2-gate-20261110T043945Z/`; its frozen handoff is
preserved. This follow-up is a separate delta on that source.

Node-owner proof: `/tmp/opencode/node-definitive-rejection-source/REPORT.md`,
`source-hashes.json` and `review-binding-evidence.json`. All 2106 source rows
bind full source `f1592c935e29b0b1fdf6429ad1b84e9ae925e9a9fcccd66b04f3d090680ff20f`.
The only enabled executable is release
`bba4752298aff258adb6cce99c6db849b02f50736a8ec32766ed1710045051aa`, with actual
consensus `4634ab3e2ca66aa9d319b35d89131f7ad1fc48c3b315435652282fe9e20b79d3`.
Debug 36e is independently bound history and is NOT enabled implicitly.

The exact `(-26, "version")` guard is before admission, orphan parking,
script checks, commit or publish in both synchronous and HTTP paths:
`rbitcoin-rpc/src/methods/mempool.rs:892-895,979-1011,1015-1065` and
`methods/mod.rs:72-89`. Whole-file hashes are respectively
`771ea11a43b3c1e7bcd5328d7af3ff9925d5e4c88afa04d1c0796c2ed3c5b11f`
and `aa86fde9262c486c1fb0d2f0c73beaee5cf2cd2fb13cdcf0b128f93b602dccbf`.
Actual unchanged release-node route evidence is retained at
`/tmp/opencode/traffic-runtime-version-rejection-preflight-v1/receipt.json`.
That version-after-sign mutation proves the route ONLY, not valid signatures
or funded admission. Its explicit false claim remains unchanged.

## Decisions

1. `traffic_acceptance` reaches the existing handler in provisioned and
   bootstrapping stages. The handler's default-off lab flag, authenticated
   operator scope, incarnation and exact hook binding remain mandatory.
   Normal commands keep their stage gates. The paused native operation's
   original deadline and cumulative debit are never reset by an arm/release.
2. A separate closed command, `acceptance_version_rejection_v1`, selects ONLY
   version 3 before construction. It requires expanded access, the explicit
   lab flag and an operator-owned durable original command. It is active-stage
   only. Normal/manual/ordinary/bootstrap construction remains version 2.
   There is no arbitrary version integer and no post-sign byte mutation.
3. The production wallet uses its same validation, deterministic selection,
   reservation, builder, fee convergence and legacy SIGHASH_ALL signer. The
   chosen version is passed to every assembly, including each fee re-sign.
4. `RequestRecord.construction_effect` is an optional closed tagged value:
   `{"schema":"spc-lab-version-rejection/v1"}`. It is covered by the existing
   request digest and durable CommandBegin before reservation/construction.
   None is omitted, preserving all normal history-3 bytes and golden vectors.
   Old binaries refuse a selected record's unknown field rather than silently
   interpreting it as version 2. Control major 1/history 3/derivation 2 remain.
5. Reopen/recovery retains the recorded effect. Disabled acceptance refuses
   to construct/send selected bytes; it never falls back to version 2. Normal
   versus selected requests under one ID conflict. A late selection cannot
   rewrite SignedTx. Delay/loss hooks remain independent; their disappearance
   cannot change the constructor's immutable selection.
6. Journal SignedTx validation requires version 3 exactly for the selected
   user-payment effect, and standard version 1/2 otherwise. New external raw
   version-3 admission is refused; only exact replay of an already selected
   SignedTx is allowed through the existing raw admission path. Witness input,
   fee, output, reservation and maturity guards remain unchanged.
7. Only the exact reviewed release/source/consensus tuple and exact version
   diagnostic on version-3 bytes is definitive. Unlisted errors, wrong tuple,
   standard raw version, mismatched messages and transport unknowns stay
   Unknown. Fresh applicable actor configs receive the reviewed tuple from
   actual bound binary/profile facts; explicit conflicting source config
   disables it. Existing live runs/configs are not rewritten.
8. The node proof is about THIS invocation. Existing core rules preserve
   PossiblySpent inputs after any prior unknown/accepted/reorg send. Only a
   fresh never-uncertain transaction can release inputs on this exact reject.

## Verification and live boundary

Source tests cover unchanged v2 bytes, independently verified final v3 legacy
signatures, fee re-signing, after-sign mutation failure, selector conflicts,
replay and reserved recovery, lost enablement, fresh versus prior-unknown
rejection, real HTTP stage/scope/default-off gates and unchanged hash budgets.
They do not fabricate a live funded proof. The unrun external re-signing /
archive-replay test was preserved and removed from the source candidate.

This T1 tree intentionally has no native installer. The fresh 40 H/s native
funded case belongs to the joined T2/QA runtime and must use the exact operator
API in `docs/acceptance-version-rejection-v1.md`. No node/server build, hash
faucet, generate RPC, mock credit or live-inspection modification is authorized.
