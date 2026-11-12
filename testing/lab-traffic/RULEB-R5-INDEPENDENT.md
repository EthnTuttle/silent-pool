# Independent corrected native Rule B replay

Inputs: `/tmp/opencode/ladder-completion-final-r1/handoff.json`, source
`2314f6e2ec154d390b476e7656349819d53004bf2144b9220d39b9cf07d5d034`,
and the sealed `/tmp/opencode/ladder-native-rule-b-run-r5` archive.
All 723 source rows, four green gate-log seals, three binaries and 147 native
artifact seals are checked independently. The producer's invocation of older
QA helpers is retained history, not this independent verdict.

`ladder_final_archive.py` uses the current independent raw-chain, source,
audited-window, SQL/checkpoint, settlement and per-identity reservation helpers.
It executes no producer program and modifies no input archive or database.

## Verified component

- Raw genesis plus 23 blocks satisfy the captured node profile's contextual
  rules, PoW/Merkle/coinbase checks and ancestry; fees are zero.
- The full 428-event signed source prefix verifies, as does the 283-event
  final SQL history and the supplied read-only SQL snapshots.
- H1-H8 have sufficient fresh complete-history and exact checkpoint evidence
  for audited warmup: actual work is 0, 1, 2, 9, 10, 20, 22 and 49. Q is 9
  for H1's actual bits, then 65. No omitted history or fabricated Q work is used.
- H1-H18 entitlement is checked from actual same-revision/parent SQL snapshots,
  with recursive verification of prior raw block E. H16 and H17 are canonical
  in the captured chain and recorded booked.
- Each raw pool output is checked against the configured public tpub
  descriptor at that block's height. SPc tweaks are independently derived from
  the archive's existing scan authority and public spend keys and matched to
  every k-associated raw payout. No spend key, tweak or private scalar is
  emitted. This proves archived key/output linkage, not full-run entropy.
- Actual old-CPU submissions join the raw headers, direct SV2/Noise config,
  worker/identity and job fields. The pool-issued extranonce prefix is joined
  to the miner portion before comparison with the source's full extranonce.

## Per-identity pair arithmetic

| Height | P | Aggregate delta | Sum of per-identity c | Actual pool output |
|---|---:|---:|---:|---:|
| 16 | 1,999,996 | -1,500,000 | 1,500,000 | 48,500,000 |
| 17 | 499,996 | -1 | 260,870 | 50,000,000 |

For H17, the three deltas are +21,739, +239,130 and -260,870. Reservations
are computed per identity, so the two positive deltas cannot offset the third
identity's 260,870-sat debit. `max(0,-sum(delta))=1` is not the reservation.
Each raw net pool output satisfies `T-sumPaid = K+Ark+forfeit+sum(delta)`.

The actual H16/H17 checkpoints are revisions 41 and 43, watermarks 171 and
178. H17 still sees H16's per-identity reservations of 625,000, 375,000 and
500,000. Booking transactions 3787 and 3790 insert the exact verified ledger
rows, change B by each identity's delta, and retire that block's own c in the
same transaction. Surrounding later reservations remain recorded values;
this check does not certify missing later-block entitlement proofs.

Signed source chronology distinguishes FrozenJob from channel issuance:
H16 is frozen at 284, issued at 286-288, candidate at 318, publication at 319;
H17 is frozen at 321, issued at 323-325, candidate at 337, publication at 338.
The producer summary's `job_issued_seq` values 284/321 identify the frozen
events, not the actual `job_issued` events. Snapshot mtimes and source-log wall
times are retained as observed archive timing, not a wallet activation ACK.

## Exact gaps and scope

- H19-H22 lack actual exact frozen checkpoints at revisions 50, 52, 54 and 56.
  No later-state reconstruction or default-zero pending is used.
- H23 has raw block and signed publication evidence, but final SQL is still
  anchored at H22 and lacks H23's economic acceptance/consumption row and exact
  frozen checkpoint. Its output/derivation observations are not entitlement.
- A coinbase fingerprint may legitimately repeat across economic generations.
  The consumer now disambiguates only using the exact FrozenJob embedded in
  SPCB/3. Ambiguous legacy records still refuse; no newest-generation guess.
- The archive is a three-identity, single-front, configured-40-H/s component.
  Its CPU remains `415d21fa...` / binary `01ae77e0...`, without the wallet gate.
  The newer native-gate CPU, its separate REQUEST_CHANGES audit and pending
  corrections do not retrospectively change this archive's scope.
- Full-run entropy, current float, native wallet COMMITTED/first-hash/revocation/
  expiry proof, Ark lifecycle, complete campaign topology/census/budgets and
  cleanup remain required. Installed restricted-ACL audit approval is separate.

The result is an independent **partial component pass**, including the pair
and early warmup, with whole-lab/full native-audited acceptance INCONCLUSIVE.
All prior failed runs and frozen consumer packets remain unchanged.
