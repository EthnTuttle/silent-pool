# Funded acceptance consumer review

Date: 2026-11-10
Disposition: REQUEST_CHANGES - four bounded false-pass cases.

## Binding and method

Independent reviewer: `ses_e3c1035fbffeK2eSIkSHL5nwGw`.
Packet: `/tmp/opencode/T3-T4b-acceptance-packet-v3/manifest.json`, digest
`8a020e71642687de96ae46c650dcd0e8ac1d219a0ec35b635cb38a5f30a671a5`.
All 25 source rows matched before and after the read-only review. The reviewer
ran no tests, builds or services and made no edits. Counterexamples below are
source-derived; their execution is assigned to the consumer author.

Paths are relative to the frozen `lab-funded-acceptance` worktree, not the
primary checkout. The parent inspected the forwarding, payout-window and
cleanup paths and forwarded all four findings to the author.

## A1 - P1: actual send outcome is not reconciled with journal classification

`testing/lab-traffic/traffic_audit.py:175-185,248-267,386-403` validates
forward-result shape and correlation, but not every ordinary result against
the journal's outcome. `journal.py:234-274` accepts a first-send rejection and
releases reservations when earlier journal records do not establish uncertainty.

A valid `signed_tx -> broadcast_sent -> rejected` journal can accompany a
matching complete audit whose actual result is `unknown`. Each checker accepts
its own input. The rejection-specific checker skips non-`node_rejected` results.
This can accept available funds that may already have been spent.

Required correction: reconcile every actual send with its exact durable intent
and journal outcome. Unknown or suppressed responses cannot justify definitive
no-effect release. Preserve legitimate later confirmation and reorg handling.

## A2 - P1: a durable send authorization can be reused by actual forwards

`traffic_audit.py:165-174,248-267` verifies the referenced intent but does not
consume it. Two attempts with distinct increasing RPC request IDs can share
one `intent_seq` and `send_sequence`. Correct result records and a watermark
do not reveal the reuse. Retry accounting counts journal intents, while
`regtest-traffic-e2e.py:899-921` permits excess observed node sends.

Required correction: one actual forward per durable send authorization,
including across actor incarnations. Legitimate retries require fresh durable
authorization and their retry debit. Reconcile all actual effect channels,
including the explicitly declared witness fixture, without blanket excess-send
allowances.

## A3 - P2: an omitted high-water front is not checked

`payout.py:75-98` checks a front's high-water value only after encountering that
front before Q is reached. Adding a nonzero front-1 high-water entry to the
passing front-0 fixture, without supplying any front-1 slice, leaves the payout
accepted.

Required correction: verify completeness and ordering of the immutable
high-water/window evidence before accepting arithmetic (SPEC §4.3). Distinguish
the retained tail from selected slices; an out-of-window front needs evidence
for exclusion, rather than silently disappearing. Bind stream IDs to slice IDs
using the actual producer contract.

## A4 - P2: failed reaping can be reported as successful cleanup

`regtest-traffic-e2e.py:1129-1137` records `reap`'s exit code but computes cleanup
success solely from matching PID/start-token survivors. A nonzero reaper exit
with no remaining recorded process can therefore produce `cleanup: PASSED`.

Required correction: require successful reaping and the applicable resource
cleanup evidence. Missing or failed cleanup evidence must remain failed or
inconclusive. Ensure timeout/error paths still produce the final receipt.

## Evidence unaffected by this review

The sealed packet records 97 passing offline tests; this review did not rerun
them. The real T2 archive's independent raw-chain checks cover 105 mined blocks,
3,492 sats in distribution fees, 80 allocation outputs and 1,809 measured /
1,810 charged hashes. Its full T3/Ark result is correctly INCONCLUSIVE:
storage/front/miner actors were inert, external payments were unconfirmed and
historical node forwarding logs were truncated.

The four findings require new regressions and a frozen-source recheck before
the joined run relies on these consumer verdicts. They do not substitute for
the separately assigned bootstrap-hook, rejection-catalog and ladder-bundle
producer work.

## Same-auditor confined recheck - followup r2

Date: 2026-11-10.
Disposition: REQUEST_CHANGES for the remaining A1 classification counterexample
below. A2, A3 and A4 are APPROVED within their bounded consumer scope. The
original A1 unsafe input-release counterexample is fixed.

### Frozen binding and method

- Packet: `/tmp/opencode/T3-consumer-followup-r2/manifest.json`, packet digest
  `4266d78465424b231bcd3059e0adc961acfd221e9750af8dc1a4c80045ad28f2`.
- Manifest SHA256:
  `166100f1d1e5d1b03afc81978950998cccaa70dfda7be2167ac846318e21c071`.
- Immutable source archive: `/tmp/opencode/T3-consumer-followup-r2/source.tar.gz`,
  SHA256 `bb78cf43e82e7f36296ef774aae2cbf0763f869fc6602a900d89e5b76e2b17c9`.
  All 41 archived content rows matched the manifest. All 41 current source
  rows also matched before and after inspection. Citations below refer to
  these frozen bytes, with paths relative to the packet's source root.
- Inspected the final source and regression logic, not just the 20-path delta
  from intermediate A1-A4 r1 or the test names. The sealed log reports 151
  passing consumer tests; its checked SHA256 is
  `cf407b1b544265b82bcbabdfa04f38b7a81eeb0f2a2d968607f48fdd44b711a6`.
  No tests, builds, services or subagents were run. Only this review appendix
  was written; previous findings and evidence remain intact.

### A1 - unsafe release fixed; P2 late RPC-acceptance claim still unchecked

`testing/lab-traffic/traffic_audit.py:283-372` now reconciles actual outcomes,
requires exact reviewed rejection preimages/profile bindings, and records
prior uncertainty. `journal.py:247-253,284-291` requires the history-prefix-bound
cross-stream proof before a rejection can release inputs. The original actual
Unknown -> first-send Rejected counterexample is therefore closed.

However, reconciliation selects only the first local response with `next(...)`
at `traffic_audit.py:300-302`. It scans the whole interval for later Rejected
records, but does not do the same for later AcceptedByRpc records. The Unknown
branch at lines 334-336 checks only that first local response. Meanwhile,
`journal.py:254-263,281-283` permits Unknown -> AcceptedByRpc.

Minimal source-derived counterexample, using the existing fully funded signed
fixture in `test_audit_regressions.py:28-124`:

1. Use `history(["unknown_outcome"])`: its send intent is sequence 6 and its
   local Unknown is sequence 7.
2. Append sequence 8: `accepted_by_rpc`, command `pay`, the same transaction
   ID, and node `node-a`. Add no new send intent or actual RPC result.
3. Supply the one actual forwarding result `("unknown", False)`, with its
   valid transport-Unknown response digest and complete watermark.

`TrafficAudit` accepts the first Unknown and overlooks the later acceptance
claim. `Journal(..., audit.outcome_proofs)` accepts both transitions and ends
in state `accepted`. Thus the history/forwarding consumer checks can report
success despite an RPC acceptance unsupported by, and contradictory to, the
only actual forwarding result. Inputs remain reserved in this counterexample;
this is a narrower classification false-pass, not recurrence of the unsafe
release. The existing positive Unknown -> raw Confirmed -> Reorged regression
at `test_audit_regressions.py:156-177` should remain valid.

Minimal correction: reconcile every AcceptedByRpc record in each intent's
interval, including its node identity, against a supporting actual
unsuppressed successful response. Reject the later acceptance above; retain
independently proved canonical confirmation/reorg and fresh-intent retries.
Add this negative regression using the existing signed mature-input fixture.
The current negative tests at lines 126-137 exercise only a first local
Rejected response, so they do not cover this suffix.

### A2 - APPROVED, bounded send-authorization and exact-census fix

`traffic_audit.py:175-185` consumes both durable intent and send-sequence keys
without including incarnation. `effect_counts.py:7-72` compares exact
node/raw-byte multiplicities and admits witness sends only through correlated
intent/RPC/completion references. `regtest-traffic-e2e.py:956-961` calls that
checker, and `all_sent_raw_observations` is a required case. Regression logic
at `test_audit_regressions.py:186-223` covers same/new-incarnation duplication,
a legitimate fresh retry, duplicate node effects, and declared versus
untraced witness effects. No greater-than count allowance remains.

### A3 - APPROVED, bounded complete-vector fix

`payout.py:55-91,114-139` checks every nonzero high-water mark before Q
selection, binds numeric slice IDs to their front, checks retained ordering,
and compares selected membership with the complete vector. Older retained
rows are excluded after the crossing slice; raw Redis marks and unsupported
multi-front ordering remain explicitly inconclusive. The new actual-source
adapter also compares complete selected marks and single-front ordering at
`pool_source.py:244-258`. Regression logic at `test_payout.py:86-155` covers
the original missing-front case, missing latest slice, front mismatch, raw
stream marks, explicit zero marks, retained-tail exclusion, reorder and
underfill. No new producer publication contract is required for this closure.

### A4 - APPROVED, bounded reaper/resource-outcome fix

`regtest-traffic-e2e.py:1170-1198` now preserves nonzero reaper exit as FAILED
before checking survivors; timeout or absent manifests remain INCONCLUSIVE.
`cleanup_proof.py:143-188` additionally checks exhaustive reaper tuples,
ownership census and resource evidence, including contradictory survivor
lists despite a claimed PASSED status. `finalize_controller` at
`regtest-traffic-e2e.py:1243-1261` preserves failures and closes the capture.
The regressions at `test_audit_regressions.py:226-294` exercise nonzero exit
with no survivors, successful complete evidence, actual survivors, missing
evidence, misleading resource status, and receipt finalization on
timeout/missing-manifest/exception paths.

### Scope retained

The before-sign version-3 path checks the closed construction effect and
actual final SIGHASH_ALL signatures against raw canonical prevouts
(`journal.py:164-189`, `signature_oracle.py:76-122`,
`regtest-traffic-e2e.py:550-570`); a node version-policy rejection does not
substitute for signature proof. The RULEB module remains independent arithmetic
with its producer contract outstanding; `accounting_history.py:33-57,76-97`
refuses missing ancestry and explicitly does not claim past-job reservation
completeness. Those assigned integration gaps are not new findings here.

The 151-test count is offline consumer evidence only. The prior real T2
3,492-sat fee and 1,809/1,810 hash evidence remains intact. The separately
rechecked stock-Bitcoin ladder component is not a joined rbitcoin or funded
Ark run. Full joined T3/T4b/Ark acceptance remains a separate INCONCLUSIVE
scope, not a condition added to this confined A1-A4 recheck.

After the final matching-source check, the author's live controller changed
again. The concluding REQUIRED/census call-site check was therefore read
directly from `source.tar.gz` without extraction. The disposition and all
line citations above remain bound to packet r2, not the newer live controller.

## Same-auditor final A1 recheck - late-acceptance fix r4

Date: 2026-11-10.
Disposition: APPROVE the bounded A1 correction. The remaining P2 late
AcceptedByRpc counterexample is closed. Prior bounded A2-A4 approvals stand;
this appendix supersedes the outstanding A1 disposition without changing the
original findings or evidence.

### Exact frozen input and verification

- Handoff: `/tmp/opencode/T3-A1-late-acceptance-fix-r4/handoff.json`.
- Packet: `f645217443d56977fe6162b256e15d5a09662536099b7df610cdcc73aa9d3e29`.
- Manifest SHA256:
  `8a452ad09cde7657aa40bc24139b9f5150bdff096e394ff554f227511fde0786`.
- Archive: `/tmp/opencode/T3-A1-late-acceptance-fix-r4/source.tar.gz`, SHA256
  `064780e29e5d92b344ea79d9b5e4ac6f7b24903c13da564b1d090f88db8171e4`.
  Read source directly from `source/` archive members without extraction;
  all 52 archived content rows matched the manifest. Citations below use
  those frozen bytes, not the moving worktree or the subsequent RULEB packet.
- Checked test-log SHA256:
  `6be4f2d45b3a42d16434056fbae4dfffeac666a37a9feb8392d805f4dcbeac73`.
  It records 177 passing consumer tests. The handoff records the original
  counterexample failing before the fix. Inspected regression logic and
  retained results; no tests, builds, services, commits or subagents were run.
  Only this review-owned appendix was written.

### Closure reasoning

`testing/lab-traffic/traffic_audit.py:297-348` now collects every
AcceptedByRpc record in each command's interval between durable send intents.
Each requires that interval's actual unsuppressed `ok` result, the exact
signed-txid response digest, matching transaction and node, and a journal
sequence after the forward context. Thus the original actual Unknown ->
journal Unknown -> AcceptedByRpc suffix is rejected at lines 337-339, even
though the first local response is Unknown. A prior intent's success cannot
authorize a later interval; orphan acceptances are refused at lines 381-386.

`send_proofs.py:16-26` binds each accepted classification to the run, driver,
record sequence, transaction, node and complete history-prefix hash.
`journal.py:248-252` checks this proof during cross-stream replay. The live
controller supplies the proofs at `regtest-traffic-e2e.py:610,676,864,898`.
Standalone journal state parsing remains a local consistency operation, not
an alternate live RPC-acceptance proof.

Inspected the six new regressions at `test_audit_regressions.py:159-213`:
the exact suffix, suppressed/rejected/unknown/gate-closed results, borrowed
old-intent success, wrong node/transaction, orphan acceptance and changed
history are refused. Positive recovery with local Unknown plus retained
actual unsuppressed success and fresh-intent restart/retry success remains
valid. The existing raw Confirmed -> Reorged positive at lines 215-236 still
uses independent chain evidence and does not manufacture RPC acceptance.
No remaining counterexample was found within this confined A1 correction.

### Scope

This closes the review's bounded A1-A4 consumer findings. It is not approval
of the later RULEB parser or a joined live run; the later 196-test count is
not attributed to this r4 review. Full T3/T4b/Ark remains separately
INCONCLUSIVE. The prior real T2 raw fee/hash evidence is unaffected.
