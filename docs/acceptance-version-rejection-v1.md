# QA API: early holds and production before-sign version rejection

## Retained coinbase hold

With `traffic.lab_acceptance=true` and `expanded_access=true`, the existing
operator `traffic_acceptance` arm/release/cancel commands are now admitted in
provisioned and bootstrapping as well as active. Other stage gates are unchanged.
Pause the original native bootstrap before distribution, then arm the original
first distribution target:

`bootstrap/<lowercase SHA256(original bootstrap command id UTF-8)>/00000000`

The hook deadline starts at that late arm (100..120000 ms). The native
operation's original monotonic deadline and charged hashes are unchanged.
Resume native work, await durable SignedTx/held evidence, roll back below the
retained coinbase's maturity, release the exact raw hash, and require no intent
or forward while the core withholds. Restore maturity for same-byte recovery.
User payments of already distributed outputs are not this coinbase case.

## Version-rejection constructor (new closed command)

Only after the joined native bootstrap has funded and independently verified
the actual common chain, use the existing authenticated commands endpoint with
the OPERATOR claim, a fresh original command ID, and:

```json
{
  "type": "acceptance_version_rejection_v1",
  "user": "id-alice",
  "destination_address": "<actual standard regtest destination>",
  "amount_sats": 100000,
  "fee_rate_sats_vb": 3
}
```

The source `LabCommand` envelope supplies `run_id`, operator-bound `actor`,
`command_id` and timestamp exactly as normal commands. The lab flag must be on;
native funding/active stage must be ready. User and controller principals
cannot use this operator-only constructor. No `version` field is accepted.
The private actor operation is `acceptance_version_rejection_v1`, carrying the
same closed payment fields as `send_user_payment`, including server-derived
driver mapping. It chooses version 3 BEFORE the existing builder signs.

This selection is intrinsic to the original durable command, not a volatile
hold hook. It is stored in the core CommandBegin request as:

```json
{"construction_effect":{"schema":"spc-lab-version-rejection/v1"}}
```

The existing request digest covers it; normal requests omit the field. Changing
normal/selected semantics under one ID conflicts, including after SignedTx.
Same selected command/payload replays stored raw bytes and does not resend.
Reserved recovery uses the stored effect, or refuses if acceptance is disabled.
Losing an optional hold/loss hook cannot regenerate version 2. The existing
`traffic_acceptance` modes remain delay/loss only, not version selectors.

## Exact release binding and expected evidence

- Source full: `f1592c935e29b0b1fdf6429ad1b84e9ae925e9a9fcccd66b04f3d090680ff20f`.
- Release binary: `bba4752298aff258adb6cce99c6db849b02f50736a8ec32766ed1710045051aa`.
- Actual consensus: `4634ab3e2ca66aa9d319b35d89131f7ad1fc48c3b315435652282fe9e20b79d3`.
- Actor `node_profile_binding` contains those exact three strings on a fresh
  applicable launch. A conflicting explicit source disables selection. Debug
  36e is not enabled just because it shares the source/profile.
- Real request: `sendrawtransaction [<exact production-signed raw hex>]`.
- Exact raw node error: `{"code":-26,"message":"version"}`.
- Typed outcome preimage for `traffic_audit/1` response SHA256:
  `{"outcome":"node_rejected","code":-26,"message":"version"}`.
- Core code: `rpc_tx_version_nonstandard`; actor result status `rejected`.
  The outer command state is `failed`, with that actor result serialized in
  `detail`. QA treats this exact correlated refusal as the expected case result,
  not as transaction acceptance or funding success. Hook-control `recorded`
  results, in contrast, map to `succeeded` only for `traffic_acceptance`.

Before claiming a valid fixture, independently decode the FINAL raw bytes,
verify version 3, txid, every SIGHASH_ALL signature against published P2PKH
keys and actual mature canonical prevouts, exact standard outputs/fee/vbytes,
and the complete forwarding attempt/result/watermark correlation. Do not mutate
the version after signing. Require candidate absence from mempool/raw-node
publication, and Available input release only for a fresh never-uncertain send.

For the prior-unknown regression, the existing one-response-loss hook may target
this same fresh constructor command. Its real node result is captured but the
actor sees Unknown; a later same-byte retry/exact rejection must leave inputs
PossiblySpent. No annotation grants spendability. Pause/barrier evidence still
comes from the same controller log and core history.

## Scope of this handoff

The earlier actual version-after-sign diagnostic is route evidence only and
retains `valid_signature_or_funded_admission_proven=false`. Source tests use
the production before-sign builder and independent signature verifier, with
explicit offline prevout fixtures. Fresh native-funded execution requires the
joined T2 installer/QA runner, preserving its 40 H/s and all original deadlines,
maturity and debit rules. This packet does not claim that unrun live case.
