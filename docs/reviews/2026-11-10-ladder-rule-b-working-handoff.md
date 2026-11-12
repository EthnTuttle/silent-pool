# Rule-B and aggregate boundary working handoff

Status: **source authored, current verification awaiting the parent slot**. This is not a
tested freeze or a disposition of the independent audit as fixed.

Working tree: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-ladder-completion`.
Owner API/SQL mapping: `/tmp/opencode/ladder-completion-owner-api-r2.json`;
current follow-up contracts and source hashes are emitted in
`/tmp/opencode/ladder-followup-source-r3/handoff.json`.

## Source changes since the predecessor

- Rule-B frozen B/R/A, exact slice IDs and validated mined ledger, SPCB/3,
  coherent bootstrap, canonical economic history and descendant readiness.
- Frozen-ledger reuse for explicit cancelled-to-recanonical transitions;
  changed replay effects reject. Unknown history never initializes as zero.
- A1: independent payout-owned authorization dispatcher with its own Redis
  connection; actual main construction enables fresh authorization.
- A2: immutable full Ark server key and exact VTXO policy comparison.
- A3: aggregate-mode legacy rejection and Store's serialized reserved-member
  exclusion; no SPRP parse-error fallback to legacy decoding.
- A4: orphan/cancellation share one transactional per-identity reversal path.
  Reserved/sent rows survive; unrelated reversible rows are undone once.
- A5: durable bounded receipt dispositions before ACK and restart-persistent
  fair PEL traversal; unknowns stay reserved. Logs summarize each batch.

SPEC, WIRE, ARCHITECTURE and MULTI-REGION now document the Rule-B authority and
remove the superseded full-unbooked-paid deduction. The handoff names every
new SQL column, the exact bootstrap/SPCB/3 fields, and QA/AP3 responsibilities.

## Evidence and outstanding checks

The immutable predecessor remains source digest
`415d21fa88ad2124712cc1fe9a8400bc7fbdd951bd005f223f92f879068f2602`, archive
`/tmp/opencode/ladder-completion-rule-b-predecessor.tar.gz`, SHA256
`6e88fd6adaad879d87fb5a9a6faff1ad79733f090ea7a78370cbfac740606ef4`.
Its 260 pool tests, three PostgreSQL aggregate tests and native stock-Bitcoin
990-output proof are historical evidence only.

`/tmp/opencode/ladder-rule-b-compile2.log` passed an earlier all-targets no-run
compilation; later source edits and the new boundary suites are not covered.
The earlier disk block was 43.352 GiB. The parent has since reported recovery
to 92.36 GiB and assigned PRIMARY the next heavy slot. No heavy attempt was
made while waiting. Future Cargo uses CARGO_INCREMENTAL=0 and the existing
cache under the same 50/20 GiB nonblocking guard.

Authored cases include real quiet Redis reads, late grants after the worker's
real deadline, genuine-grant valid wrong-policy/server proofs, two-member
legacy exclusion followed by full proof, freeze/Sent serialization,
mixed-identity positive/negative and unreserved-Ark orphan/Sent/cancel races,
malformed and >64 unknown PEL entries across reconnect, and Rule-B booking,
orphan, replay and dependent-reservation cases.

Added source/fixtures include combined held-aggregate/parent-reservation
full confirmed recanonicalization, the immutable signed-order API, source-feed
ACK before activation, wallet grant fences and the economic initial seed.
Still required: compile/run those suites, affected pool suites/clippy, native consecutive-pending proof,
independent QA replay and funded stock-captaind/final-rbitcoin acceptance.
The next reviewed freeze must bind those actual receipts to the tested source.
