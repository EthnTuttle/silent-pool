# Tested 419 trace hint and read-only Rule B archive

Consumed `/tmp/opencode/ladder-combined-tested-r1/handoff.json`, source
`419b5f061d47fe4f56c36e85fb8ee5e66ba18cb045f00800dfd3d06a8aedefa3`.
Its signed-feed/audit helpers match the retained edebd bytes. The trace and
closed SQL readers admit this explicit read-side compatibility; this does not
admit the component's older CPU as a wallet-gated native campaign.

## EconomicChanged is only a reload hint

The exact producer is `bins/spc-pool/src/front.rs:738`, source SHA256
`bfb6da174f43887f1beb881cda3a25f8267bf2fb454b42efc305471a849f12fb`:

```
economic_change_observed {
  economic_revision: canonical u64 decimal string,
  anchor_hash: 32-byte hex,
  coherent_snapshot_reload_required: true
}
```

The strict parser retains the event in the complete hash/signature chain. It
rejects extra/missing fields, malformed revision/hash and non-true flags. Other
unknown events remain unavailable. The hint never changes balances, creates a
checkpoint, proves funding or turns absent history into an empty window.

When a hint follows the current frozen job, ReplayProvider requires a fresh
closed SQL capture and coherent row/parent replay before preparing a bundle.
Already admitted historical snapshots are preserved. A changed economic view
needs a new coherent frozen generation; a hint arriving during replay prevents
release from the now-stale prefix. The hint's numeric fields are not substituted
for actual SQL or raw-chain state.

## Gross K is not a net coinbase floor

`economic_revenue.py` checks each row and the whole coinbase:

```
raw_i = paid_i + Ark_i + forfeit_i + deltaPending_i
K = T - sum(raw_i)
payP = T - sumPaid = K + Ark + forfeit + deltaPending
```

Old liability repayment may reduce payP below K. The independent settlement
helper therefore has no additional gross-fee UTXO floor. Empty, partial and full
windows preserve positive prior pending; empty fresh work can repay old pending
after the same history/owed/output checks. `sumPaid > T` rejects; there is no
trim, negative pool output or P=0 fallback. JD gross weights/O11 are unchanged.
The producer's correction and next executed consecutive-pending test are still
required; the preserved pre-fix failure is not relabelled.

## Independent archive findings

`ladder_ruleb_archive.py` reads the sealed owned
`/tmp/opencode/ladder-native-rule-b-run-r4` archive without RPC, SQL mutation,
producer execution or reconstructed corpus. It verifies:

- All 249 signed source events, including 11 EconomicChanged hints.
- The captured migration-0011 initial row bytes and snapshot digest
  `2c9609bad669a882bd786be07c6bf8f3650f202a2aba2bca04f07dbe9f4a43b6`;
  prior migration checksums are independently derived from frozen SQL sources.
- Initial six-event and final 162-event row-history replay, same-transaction
  booked-pending/ledger deltas and actual SQL/raw-block anchor joins.
- Eleven raw blocks: PoW, Merkle, witness/weight rules, ancestry from the known
  regtest genesis hash and coinbase revenue. All are coinbase-only: fees are 0.
- Seven issued full-Q windows, including Q=65/actual=67. Mined blocks 9-11 also
  join their raw outputs to calculated rewards and retained frozen ledgers.

The final row replay yields B=1,499,997 sats, R=0. Recomputing the failed
proposal with those actual balances and the last verified 67/65 window gives:

| Quantity | Sats |
|---|---:|
| T | 5,000,000,000 |
| sum(raw) | 4,949,999,999 |
| gross K | 50,000,001 |
| sumPaid | 4,950,500,000 |
| payP | 49,500,000 |
| deltaPending | -500,001 |

This is an explicitly labelled independent recomputation from final SQL
revision 29 and last source window revision 28, not a newly observed FrozenJob
or mined block. Actual consecutive-pending acceptance remains **failed before
the producer fix**. The eleven archived economic blocks have frozen prior=0
and consumed=0; no successful consecutive nonzero net-debit pair is inferred.

Full historical entitlement remains INCONCLUSIVE: this archive lacks actual
per-job coherent historical SQL snapshots for the underfilled eligible-history
path and a raw genesis/contextual full-chain observation. Stored FrozenJob
values are retained claims, not a substitute for those missing checkpoints.
The component uses the explicitly older CPU without the wallet gate; full
native-audit, funded Ark and whole-lab acceptance remain INCONCLUSIVE.
