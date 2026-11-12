# Collector/campaign follow-up after the 218-test freeze

The immutable predecessor remains
`/tmp/opencode/T3-source-feed-checkpoint-AP5-r6/source.tar.gz`, SHA256
`0b7a7b515f823a879b7b8abadbb1d04f801cd085debf5a07846e5a3a53a24609`.
The audited-underfill/feed-v2 source contract has now been consumed; its producer
is still uncompiled. See [SOURCE-FEED-IMPLEMENTATION.md](SOURCE-FEED-IMPLEMENTATION.md)
for the exact pin, independent eligibility proof and signed WindowFacts.
The native outer r3 activation codec/hook has now been consumed as uncompiled
source. [NATIVE-AUDIT-CONSUMER.md](NATIVE-AUDIT-CONSUMER.md) records its observer
and remaining actual outer-frame/lifecycle evidence requirements. Q/node bits
and the 40 H/s campaign budget remain fixed.

## Implemented now, on existing contracts

- **Eventual full Q is mandatory.** `full_window_coverage` is in the funded
  campaign's source-owned required-case set. The actual collector checks every
  declared front independently, joining an issued generation, raw template and
  parent, complete selected source shares and original Q. Missing fronts or
  absent full windows remain INCONCLUSIVE. A valid full window is not a native
  activation proof or proof that every earlier window was complete.
- **Incomplete SQL capture does not poison recovery.** Every attempted snapshot
  remains an immutable file and is durably associated with its front/job and
  artifact hash. Only a coherent, exact-revision, parent-bound row checkpoint
  enters the historical cache. An early incomplete capture can therefore be
  followed by a fresh coherent capture of the same generation; an admitted
  historical snapshot is never replaced by current DB state. These are row
  checkpoints, not grants before actual-E and source checks.
- **History is ordered economically across reorgs.** Historical orphan branches
  may have equal or greater height than the current parent. Their independently
  checked frozen economic revisions must precede the current revision. Stored
  block effects must follow their frozen revision. Height alone no longer
  incorrectly rejects that history graph.
- **Avoid repeated expensive verification.** Successfully verified historical
  contexts are memoized within one proof exchange; failed/incomplete contexts
  are not carried into later exchanges. Exact BIP340 verification successes
  have a bounded cache keyed by digest, signature and public key. Source-prefix
  completeness, process/config binding, history, expiry and candidate freshness
  are always checked independently and are not cached as signature authority.
- **No silent settlement omission/default.** Every identity with positive fresh
  earnings or available prior pending must occur in the frozen settlement.
  Pending for an identity outside the current share window is still routed.
  Shaped sats must match the actual ladder. Unknown delivery states and missing
  registered destination evidence cannot silently become zero Ark due. Known
  undeliverable kept-miner remainders remain error-held and never forfeited.

The existing read-only stock-Bitcoin archive was checked through the same
full-window helper at `/tmp/opencode/T3-full-window-archive-r7/receipt.json`:
four issued windows establish Q=9 and selected work=9. This remains the existing
stock-Bitcoin component scope, not rbitcoin T3/T4b, an audited-underfill pass or
funded Ark acceptance.

## Exact provenance still needed for other settlement rails

No unsupported rail is promoted using its producer ledger label alone:

- Ark-only: historical committed registration and the frozen destination's
  provenance at the relevant build/settlement boundary.
- Error/Pending: actual registration propagation or failed derivation evidence
  for the identity and frozen job. A pool claim of failure or a current mutable
  registration row is not independent proof of historical E.
- Forfeit: complete historical no-registration evidence for the below-threshold
  obligation. Missing registration history is not proof of absence.

These cases now return separate typed component gaps. Concrete additional
producer inputs will be consumed when source-bound; no new record schema or
zero fallback has been invented.

## Waiting on the assigned native/wallet/ladder owners

- `lab-native-audit-gate` owns native CPU/public codec and AddConnection hunks.
  Its source-bound r3 `native_activation.rs` contract has been consumed. The QA
  parser checks candidate/commit digests, window facts and first-hash ordering.
- `lab-aggregate-wallet` owns the verifier service in the existing wallet
  process. Scan authority stays there, not in the CPU miner or collector.
- The ladder owner's r2 handoff supplies loaded-wallet-key source-auditor
  construction, consumed/committed public evidence and the dated underfill
  policy. The consumer implements it; compiled producer/PG/native execution
  receipts are still required.

With these source contracts consumed, QA must prove the actual direct native SV2
activation path, not merely a valid Unix exchange. Full N coverage, all payout/
witness/economic checks and the 40 H/s accounting remain required. Whole-lab
acceptance remains INCONCLUSIVE pending actual joined execution and AP5 proof.
The corrected Runtime base diagnostic bundle `4d3778e8...` is now independently
source/test/binary verified; historical g105 stays blocked. The native-audited
complete bundle is pending. R4 pool-control source approval is not Nix integration.
