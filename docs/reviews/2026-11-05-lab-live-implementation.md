# LP-live independent real-node v2 and retarget implementation report

Date: 2026-11-06
State: reworked candidate for the same Fable re-review. No approval is claimed.

Base: `b0ab4ca34fda07c61a6335d05f598eaacc69678c`. This revision changes
only the five packet files in `lab-live-e2e`. There are no commits, source-tree
changes elsewhere, Cargo/lockfile/flake edits, or historical-artifact overwrites.

## Rework dispositions

| Finding | Resolution |
|---|---|
| B1 approval binding | Replaced global substring matching with full-hash, per-review-pass verdict associations. Any explicit rejection of the same hash is a veto even when a different pass approves it. Missing/multiple verdicts, multiple explicit snapshots, missing full snapshots, prefix collisions, and unrelated citations fail closed. LP-1 is additionally pinned to `c16032a7`; LP-2 and L2 remain pinned to `5e27e918` and `d2f37b91`. These constants are secondary checks, never substitutes for the actual verdict. |
| B2 descriptive overclaim | Removed causal framing from code, fixtures, and this report. Boundary equality predicates now describe only whether nbits repeat, not why. The record distinguishes boundary arithmetic and budget-command application from any inference about what determined block pace. Per-phase submitted/ACK/stale/block-qualified and canonical counts, worker attempt deltas, and 28 `share_outcome_unknown` events are retained. |
| Item 3 build provenance | Baseline-node association is precisely described below; the retarget rebuild is identified as an up-to-date/no-relink check, not a fresh build attestation. |
| Item 4 metadata | `run_created.retarget_claim`, preflight `profile`, and preflight `retarget` now come from the scenario rather than a hard-coded false value. |
| Item 5 effective profile | Retarget interval and both timespan clamps are derived from `getblockchaininfo.consensus`; an independent eight-block/400-second vector exercises non-default assumptions. |
| Item 6 compact encoding | Small targets now shift left, matching Core GetCompact. Regression: `0x1234 -> 0x02123400`. |
| Item 7 job identifiers | LP-1 IDs are parsed explicitly as eight lowercase hex characters, including all-digit IDs; approved miner IDs are parsed explicitly as unsigned decimal integers. No decimal/hex heuristic remains. |
| C1 canonical and interval counts | Removed fixed `24/23/23/3` constants and embedded reviewer phase counts. `canonical_blocks_by_phase` is derived from unique successful submission headers joined to observed node best-tip records, with duplicate/rejected/missing-parent/reorg-conflict records classified separately. Wall-clock block phase membership and height-based retarget intervals are separate. Tail best-tips outside the recorded evidence vector are retained instead of a fixed "other" constant. |
| N1 prefix consistency | Full hash matches also honor any short prefix in the verdict line. A bare-line full hash is accepted only in an explicit `(matches)` association with a matching prefix; unrelated full hashes cannot bind approval. The permanent-rejection veto is unchanged. |
| N2/N4 small correctness fixes | Miner JSONL is streamed rather than read as one giant string, boundary lookups are cached, and `require_timespan` documents that Core does not require divisibility by four. |
| Item 8 evidence disclosure | All 83 settled blocks across both frozen runs have `fees=0`; live fee coverage is not claimed. The unit vector exercises the fee term. Twenty-eight unknown-outcome events are retained in the derived record. Git-provenance logs are unique per run and component. |

The separate duty-cycle investigation belongs to LP-1/LP-2. This packet does
not assign it to a specific pipeline stage or silently change that code.

## Frozen inputs and actual executed binaries

All source-manifest files were verified before and after each live run.

| Input | Approved source | Executed binary SHA256 |
|---|---|---|
| LP-1 pool/scanner | `c16032a78902e88b0157efed6780bc8b424e942b1c3bf46893d8fb09ae4708ee` | pool `f3b359157e0e019d0f5742dbdc44a558502c94209f50ea4fa96ec703fc41b6a9`; scanner `462877fd82b7d7f82f27858fdee78f44e9e1a5f8c2b57c03d7011c98a755aa9c` |
| LP-2 native miner | `5e27e9180e765e8049cbcccbcadc38b24c85d402505108f8f20c320f73961250` | `7e02b0fdf7434edd081aa60ee421c7eef999418f56345a2f302a756cbab07169` |
| Baseline rbitcoin | root HEAD `e8cf9c9d61f7c63ae59cc11ab1179cace50ca142` | `0b3a0cba63cfaec1b52a888a809e39103b558b1d12d8b3e5d013dce9f789ddea` |
| L2 retarget rbitcoin | `d2f37b9113e8855245866289ed393043aed6e6153e8101375072cbb23624d499` | `eb257af7c10cdf0ded584adfadf2ae8124466304b13d26c17f855e76f9320f7c` |

Explicit approval association accepted by the revised parser:

- LP-1: top-level pass `# Third pass (revision r3)`, `## Verdict (r3)` contains
  `**APPROVE - bounded LP-1.**`, and `## r3.1 Snapshot` contains the full
  `c16032a7...` aggregate as the `(matches)` code-block value.
- LP-2: top-level pass `# Fourth pass: R3-B5 publication ordering`,
  `## Result` is `**APPROVE** (bounded LP-2), tied to snapshot` followed by
  full `5e27e918...`.
- L2: top-level pass `# Second verdict (revision 2)`, with full
  `snapshot_sha256` `d2f37b91...` and `## Verdict` approving that snapshot.

The parser's actual-record tests additionally require rejection of every
previous explicit rejection named in those multi-pass review files:
`81fa17d2`, `69ac72b6`, `af531370`, `bcb9b342`, `6e2eb2aa`, and `646656a8`.
Separate fixtures cover approval B/rejection A, approval then revocation of the
same hash, unrelated mention, prefix collision, missing verdict/snapshot, and
ambiguous verdict/snapshot association.

### Build association, precisely

- Pool/scanner and miner compile logs record actual compilation into the
  executed binaries. Source manifests were unchanged.
- The baseline node predates this packet and has no build log here. Its
  association is the clean root tree at `e8cf9c9d` plus binary mtime ordering.
  There is no fresh build attestation for it in this packet.
- `LP-live-build-approved-retarget-node.log` is an up-to-date/no-relink cargo
  check against the frozen L2 sources, not a new deterministic build proof.
  The binary predates that log. Cargo did not replace the binary during that
  check, preserving the source/binary association verified above.

Build commands used were `cargo build --locked` under Nix with
`CARGO_BUILD_JOBS=2`. Logs remain `/tmp/opencode/LP-live-build-approved-pool.log`,
`LP-live-build-approved-miner.log`, and
`LP-live-build-approved-retarget-node.log`.

## Reusable harness and independent evidence boundary

The orchestrator still uses a fresh regtest datadir, a private trailing-newline-
free cookie, SCRAM Postgres, password-protected Redis, unique recorded ports,
`0600` secret files, `0700` run directories, direct-child/start-token cleanup,
and bounded waits. P2P listen, seeds, discovery and manual peers are disabled;
the node observes zero peers. Fresh identities and matching Noise material
come from supported `spc-scan` secp tooling. No templates, injected work,
mocked credits, or injected canonical blocks exist.

The harness independently re-hashes headers; joins `job_issued`, `accounted`,
and sealed leaves using explicit encoding; recomputes per-generation m and Q;
checks finding-slice exclusion, high-water selection and whole-slice work;
re-parses canonical coinbase/header/Merkle evidence; validates exact subsidy
conservation; waits for actual configured settlement depth; and invokes
`spc-scan light` on independently built light proofs. Pool ingress replies alone
are never promoted to accounting or candidate evidence. Missing node/job
information is `unknown`, not invalid.

## Reused verified live evidence, unchanged

The original public summaries and identity-bearing artifacts were not edited.
They retain these original hashes and outcomes:

| Run | Public summary SHA256 | Verified outcome |
|---|---|---|
| `/tmp/opencode/LP-live-baseline-verified` | `f8f62fba124618285272c1b3da59b637ba8aa8339c6f66f9f00db0a14886111d` | 15 settled blocks; baseline Q=65 for selected height 11, actual work 68; two native identities and scanner outputs; no early settlement |
| `/tmp/opencode/LP-live-retarget-run6` | `93ddfabcc128f6cf4ec29874a5fe17016651f0b7103b978ddd133c8504484b31` | 68 settled blocks; all 73 headers match the effective-profile arithmetic; frozen Q and custody conservation by height; two scanner outputs; no early settlement |
| `/tmp/opencode/Fable-LP-live-baseline-rerun` | `23640b2a4caf35e41420b08c2129780d3a28f2500721682f22c8cc8f5b7aec4d` | Fable's independent new-chain repeat at height 9, Q=65, work 70, with scanner recovery for both identities |

Fable independently re-derived all 83 `slice`/snapshot/credit/settlement rows,
including all 15 baseline and 68 retarget frozen windows and every confirmation
boundary. Minimum recorded settlement depth was six in both original runs.
Header nbits, per-generation work, finding-slice exclusion, high-water,
coinbase conservation, and light proofs were independently checked. All 83
settled blocks have zero fees; fee-term proof remains synthetic-unit-only.

For the selected baseline candidate, height 11 is
`09bc94a1717aa131a027d59f95a026a8767dbb96b25cf247786a89d7cc6203a8`, with
`m=2` for its non-block work, actual Q=65, actual work 68, underfill=0,
pay_P=50000001, and two recovered outputs. Genesis/pow-limit templates use m=1
as issued rather than a hard-coded m=2 assumption. The selected retarget
candidate is height 8, Q=524289, actual work 528384, underfill=0, and two
recovered outputs.

## Retarget boundary outcomes, without causal attribution

The retarget run used an effective profile with 10-second spacing, 240-second
timespan, 24-block interval, minimum-difficulty exception off, no separate
`--regtest-pow-bits` overlay, and the `1e7fffff` floor/seed. There was no RPC
generation, mock time, or post-solution holding. Control commands at all three
boundaries are recorded as `pause_workers`, `set_worker_budget` where changed,
`snapshot`, and `resume_workers`.

| Boundary | Recorded elapsed value | Actual nbits | Independent result, with clamps derived from this profile |
|---|---:|---|---|
| 24 | 497257296 s, clamped to 960 s | `1e7fffff` | `1e7fffff`, genesis-anchored clamp |
| 48 | 223 s | `1e76eeee` | `1e76eeee`, tighter than floor |
| 72 | 263 s | `1e7fffff` | `1e7fffff`, clamped back to floor |

The tightening and floor-clamp directions are directly established by headers.
Budget command application is separately established. The evidence does not
attribute the 223/263 elapsed values to those commands. The old equality
predicates are retained only as observations that nbits differ, not as an
experiment establishing that a budget caused the change.

### Duty-cycle counts and achieved work rates

Requested values below are not a throughput claim. Attempt/elapse deltas come
from consecutive miner snapshots. In the first full period, each worker ran
about 273 kH/s over 223 seconds; in the second, about 191 kH/s over 263 seconds.
The review's reference phase groups are separately labeled in the derived file;
the harness also retains its explicit event-time grouping, so a different
boundary-assignment convention cannot be silently substituted.

| Phase group | submitted | ingress ACK | stale-share | block-qualified | observed canonical best-tips by wall time |
|---|---:|---:|---:|---:|---:|
| coverage, heights 1-24 | 49277 | 1387 | 34360 | 787 | 24 |
| first full period, heights 25-47 | 59049 | 1374 | 38272 | 926 | 24 |
| second full period, heights 49-71 | 49900 | 1640 | 44512 | 730 | 24 |
| other/outside the two declared full periods | 5583 | 176 | 2908 | 86 | 1 |

These are the driver-derived current values, not embedded reviewer figures.
Each 24-height retarget period contains 23 height differences (retarget
intervals); each resulting wall-clock group contains 24 observed accepted best-
tip records. The additional one is the observed tail block at height 73, not a
hard-coded catch-all. Phase membership uses observed phase-boundary wall times.
Interval spans instead use the effective interval and recorded boundary heights.


Whole-run observations from the raw miner JSONL are 163809 submitted, 4577
ingress ACKs, 120052 stale-share rejections (73%), and 2529 block-qualified
solutions with 73 canonical blocks (about 3%). At `1e7fffff`, a block expects
131072 hashes; at `1e76eeee`, about 141064. These observations are descriptive.
They are not used to claim that any budget or a particular pipeline stage
caused block pace. `pool.log` additionally contains 28
`share_outcome_unknown`; those remain unknown rather than accepted or invalid.

Final snapshot attempts were 162047734 and 161995472. Reported achieved rates
218488 and 218110 H/s are snapshot rates, not the attempt/elapse phase averages
above. No exact-ten-second block-time model is asserted.

## New derived artifacts, not replacement history

```sh
PYTHONDONTWRITEBYTECODE=1 ./testing/regtest-lab-v2-e2e.py \
  --derive-retarget-from /tmp/opencode/LP-live-retarget-run6 \
  --derived-output-dir /tmp/opencode/LP-live-r3-derived-final-v2
```

This creates, only in a new directory:

- `retarget-duty-cycle-derived.json`: input hashes, the current duty-cycle table
  above, unique joined best-tip counts, separately derived retarget interval
  spans, worker attempt/elapse deltas, and all unknown-outcome events.
- `retarget-oracle-replay-derived.json`: the original full header vector with
  identical expected bits `24=1e7fffff`, `48=1e76eeee`, `72=1e7fffff`, plus the
  independently re-hashed light proof, coinbase conservation and scanner/raw
  payout recovery checks.

Every derived file is labeled `derived_from_historical_artifacts=true` and
`no_original_artifact_modified=true`. It includes the generating script SHA256,
the original public-summary hash, and the hash of each raw input used. The
helper re-hashes every input afterward and fails if anything changed. Original
public summaries and historical private logs are not treated as revised-script
output. Compact/oracle changes preserve the original boundary outcomes; light
proof and frozen payout recovery confirm the original matching cases.

## Exact validation and preflight commands

```sh
PYTHONDONTWRITEBYTECODE=1 ./testing/regtest-lab-v2-e2e.py --check-scenario testing/lab-live/baseline.json
PYTHONDONTWRITEBYTECODE=1 ./testing/regtest-lab-v2-e2e.py --check-scenario testing/lab-live/retarget.json
CARGO_BUILD_JOBS=2 PYTHONDONTWRITEBYTECODE=1 nix develop -c python3 testing/regtest-lab-v2-e2e.py --self-test
```

The reusable self-test suite now includes all original checks plus the approval,
effective-clamp, compact-left-shift, and explicit job-encoding cases above.

Frozen-source retarget preflight used the same LP-1/LP-2/L2 approved inputs as
the original retarget acceptance command, with `--validate-only --acceptance`
and a new run directory:

```sh
cd /home/ethan/.local/share/opencode/worktree/ad8886/lab-live-e2e
PYTHONDONTWRITEBYTECODE=1 nix develop -c ./testing/regtest-lab-v2-e2e.py \
  --validate-only --acceptance --scenario testing/lab-live/retarget.json \
  --pool-binary /home/ethan/.local/share/opencode/worktree/ad8886/lab-share-integration/target/debug/spc-pool \
  --pool-source-dir /home/ethan/.local/share/opencode/worktree/ad8886/lab-share-integration \
  --pool-source-manifest /tmp/opencode/lab-lp1-review-manifest.json \
  --pool-approval-record /home/ethan/code/silent-pool/docs/reviews/2026-11-05-lab-lp1-fable-review.md \
  --miner-binary /home/ethan/.local/share/opencode/worktree/ad8886/lab-cpu-miner/target/debug/spc-cpu-miner \
  --miner-source-dir /home/ethan/.local/share/opencode/worktree/ad8886/lab-cpu-miner \
  --miner-source-manifest /tmp/opencode/lab-lp2-review-manifest.json \
  --miner-approval-record /home/ethan/code/silent-pool/docs/reviews/2026-11-05-lab-lp2-fable-review.md \
  --scanner-binary /home/ethan/.local/share/opencode/worktree/ad8886/lab-share-integration/target/debug/spc-scan \
  --scanner-source-dir /home/ethan/.local/share/opencode/worktree/ad8886/lab-share-integration \
  --scanner-source-manifest /tmp/opencode/lab-lp1-review-manifest.json \
  --node-binary /tmp/opencode/rbitcoin-lab-retarget-profile/target/debug/rbitcoin-node \
  --node-source-dir /tmp/opencode/rbitcoin-lab-retarget-profile \
  --node-source-manifest /tmp/opencode/lab-l2-source-review-manifest.json \
  --node-approval-record /home/ethan/code/silent-pool/docs/reviews/2026-11-05-lab-l2-fable-review.md \
  --run-dir /tmp/opencode/LP-live-r3-preflight-final
```

Actual outcomes: both scenario checks returned `status=validated`; all 17 self-tests
passed under Nix (`LP-live-r3-selftest-final-v2.log`); frozen-source retarget preflight
returned `status=validated` and `assertions.preflight_only=true`
(`/tmp/opencode/LP-live-r3-preflight-final.log`). The derivation and payout replay
returned `status=derived_ok` (`/tmp/opencode/LP-live-r3-derive-final-v2.log`).
No new mining run was instituted for this bounded rework.

## Exact scenario and artifact paths

- `testing/regtest-lab-v2-e2e.py`
- `testing/lab-live/oracle.py`
- `testing/lab-live/baseline.json`
- `testing/lab-live/retarget.json`
- `docs/reviews/2026-11-05-lab-live-implementation.md`
- Historical inputs: `/tmp/opencode/LP-live-baseline-verified/private`,
  `/tmp/opencode/LP-live-retarget-run6/private`, and Fable's independently
  verified baseline rerun.
- Derived input/output logs are named `/tmp/opencode/LP-live-r2-*`.

The manifest is `/tmp/opencode/lab-live-review-manifest.json`, canonical compact
JSON of `base` and sorted `path`/`sha256` pairs, with no generated bytecode.

## Nix merge notes and explicit remaining gates

No new Nix dependency is required. Keep Python stdlib and the existing
PostgreSQL and Redis packages. Inputs remain explicit binaries from frozen
source worktrees, not pool/miner/node source bundled into this packet.

Not claimed or completed:

1. Nonzero-fee live economics, denomination ladder, TDP/multi-transaction
   witness, Ark delivery, v2 type-32 auditing, or a combined merged-flake gate.
2. LP-4 multi-pool topology, independent storage per competing pool, sustained
   namespace/link faults, or no-bypass proof.
3. General SV2 interoperability, production/mainnet behavior, or a new
   deterministic node build attestation.
4. The separate rbitcoin pre-merge CI-stability adjudication as this packet's
   harness proof. Its separate outcome does not alter the retarget header proof.
5. A causal budget-to-block-pace experiment or exact-ten-second block-rate
   model. The evidence establishes actual work and correct node arithmetic only.
6. LP-1/LP-2 stale-job pipeline remediation. It is an observation for another
   bounded packet, not an edit hidden in this test harness.
