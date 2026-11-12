# Serial T3/T4b campaign

`campaign.py` executes the actual production-controller harness in three
fresh owned roots, serially. It has no signer, faucet, header search, build or
existing-run adoption path. The application binary closure is reused; chain,
storage, journals and run IDs are fresh for every case. The declared phase
command ID is preserved within each fresh run's independent command namespace.

```sh
python3 -B testing/lab-traffic/campaign.py \
  --inputs /private/actual-joined-inputs.json \
  --scenario testing/lab-traffic/scenario.json \
  --out /tmp/opencode/NEW-campaign \
  --destination <explicit-regtest-payment-address> \
  --case-timeout 900 --timeout 3300
```

This campaign explicitly includes owned-node invalidate/reconsider operations
and the existing two public test-only witness keys. It invokes the production
native bootstrap and miners at 40 aggregate H/s, with 0.2 ordinary tx/s.
`/tmp/opencode/silent-pool-acceptance-campaign.lock` is held nonblockingly for
the campaign lifetime. A second campaign does not wait, poll or start actors.
There is one active case, never one faucet per concurrent case. Each case keeps
its original production hash-attempt cap and deadline; the sum of the three
fresh caps is reported explicitly. Cleanup receives a separate 120-second
wall budget, which grants no hashing authority. A watchdog enters cleanup on
deadline. Unknown/failed cleanup withholds all later cases.

## Required cases

`case_contract.py` is the source-owned list. The caller cannot remove gates.

| Serial case | Required observations |
|---|---|
| `bootstrap-stop` | Actual bounded cancellation during native bootstrap |
| `bootstrap-maturity` | Signed retained coinbase, rollback below maturity, actual core refusal with no send intent or forward |
| `funded` | Normal traffic, pause, manual payment, raw fees/chain/history, both witness spends and mixed block, witness job refresh/stale outcome, parent/change-child reorg, original-block restoration, separate displaced same-byte retry, fresh signed v3 rejection, prior-Unknown retention, full source/payout/pre-mining proof gates |
| Every case | Fresh common genesis, input/config/entrypoint binding, original Runtime phase ownership/outcome, native hash-budget audit, raw canonical export, exhaustive cleanup |

The funded leg additionally requires `full_window_coverage` for **every declared
front**. The collector verifies an actual issued generation, raw template/parent
and selected-share work at its original Q. One full pool cannot cover another
pool's missing/underfilled source. Earlier windows which do not establish full
coverage remain recorded as such; a future audited-warmup permission cannot
satisfy this eventual full-N gate by itself. Native pre-hash activation and
economic/payout proof remain separate required gates.

`economic_final_drain` is also mandatory: stop/drain mining, retain three stable
raw/source/SQL samples, stop the owned pool writers, observe source EOF and
recheck the final boundary. Missing exact frozen/advertised-job checkpoints
still block certification. See [CHECKPOINT-CAPTURE-DRAIN.md](CHECKPOINT-CAPTURE-DRAIN.md).

`final_sealed_prefix` is mandatory after that drain. The final read-only audit
binds the final source/native prefix, raw export and original SQL checkpoint
watermarks. Pre-drain payout/window observations cannot cover later appended
blocks or events. Shared runners call `audit_live()` followed by their single
drain and `final_audit_after_drain()` before closing collectors or actors.

The funded leg does not impersonate the two auxiliary legs. Its receipt is
`campaign-case-only`; only the campaign can combine the three obligations.
Supplemental failed or inconclusive checks also block success. An earlier
failure/unknown result cannot be overwritten by a later pass.

## Bindings and receipts

The campaign copies its explicit input file and base scenario into the new
root. Per-case scenarios change `run.id` and any matching declared command
`run_id`; an exact semantic scenario hash
binds everything else, preserving JSON decimal rates without float rounding.
Each case receipt binds the input/scenario hashes, semantic config, joined
source snapshot, actual binary hashes and fresh run/root. Runtime config
files (including process `--config` paths), launch-argument digests and resolved
traffic/worker/pool policies are captured. The campaign checks their retained
hashes and independently replays each exported raw canonical observation.
Consumer source hashes are checked before and after execution.

## Runtime owns the declared bootstrap

The selected CLI entrypoint is `runtime`. Its actual source automatically
executes declared phases after starting HTTP. Exactly one bootstrap phase with
an explicit original `command_id` is required; an empty phase list fails the
producer's route preflight. `scenario.json` now declares `t3-native-bootstrap`.
See [RUNTIME-BOOTSTRAP-CONSUMER.md](RUNTIME-BOOTSTRAP-CONSUMER.md).

The harness observes that operation's private CommandLog and phase evidence.
It does not submit another bootstrap UUID or POST the original command under
the authenticated operator principal. Pause/resume/cancel and acceptance hooks
use separate operator commands targeting the original operation. Its original
deadline and hash debit remain unchanged.

`<case>-intent.json` precedes each invocation; `<case>-completion.json` retains
its verdict and child receipt hash. The campaign always lists all three cases
in `campaign.json`, including withheld/unexecuted cases as INCONCLUSIVE.

- `T3_T4b_status` is the conjunction of all three required case receipts.
- `AP5_status` comes from actual, independently sealed AP5 evidence.
- `whole_lab_status` is the conjunction of T3/T4b and AP5 acceptance.
- `T3_T4b_pass` and `whole_lab_pass` are derived from those statuses, not fixed
  success/failure booleans. There is no average, majority or partial-credit pass.
- Exit status reflects whole-lab status: 0 PASSED, 1 FAILED, 2 INCONCLUSIVE.
  A proved T3/T4b campaign can therefore still exit 2 when AP5 is unavailable.

## Actual AP5 archive interface

The optional `--ap5-evidence` file is a parent/collector descriptor:

```json
{"root":"/absolute/actual/AP5/archive","events_sha256":"<external SHA256>",
 "last_seq":123,"last_digest":"<external terminal record_sha256>"}
```

`ap5_observation.py` reads the actual AP5 `events.jsonl` hash chain, every
referenced file, `spc.lab.ap5.bundle/v1` and `spc.lab.ap5.result/v1`. It checks
shared joined binaries/source manifests, all four users' required lifecycle
labels, native funding, both pool floats and cleanup. Funding-only,
prepare/check-bundle, missing tails, missing user renewal, foreign binaries and
cleanup failures cannot establish full AP5 acceptance. This is read-only; AP5
retains ownership of its executor and wallets.

The supplied AP5 producer currently ends with
`funded-lifecycle-observed-awaiting-independent-oracles`. It has no final
independent raw proof/full-policy/server verdict contract or exhaustive
resource cleanup receipt. The adapter records successful lifecycle observation
when supported, but returns INCONCLUSIVE for final AP5 acceptance with these
named missing components. It does not invent a new accepted producer schema.
A new final verdict requires its source-owned sealed contract.

The separate five pool aggregate findings in
`docs/reviews/2026-11-10-ladder-aggregate-review.md` belong to the ladder owner.
The old `415d21fa...` source packet is not funded aggregate approval. Its
990-output raw component proof remains valid within that component's scope.
