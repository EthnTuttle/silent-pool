# rbitcoin v0.9.0 + L2 independent live validation

Date: 2026-11-06 (live observations crossed midnight into 2026-11-07 UTC)
Author: independent TFF release validator
Worktree: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-v09-validation`
Base: `b0ab4ca34fda07c61a6335d05f598eaacc69678c`

## 1. Result and adoption boundary

**Chronology matters: three original stages passed; the original profile was
incomplete.** Baseline mining/settlement/scanner recovery, three real automatic
retarget boundaries, and the namespaced two-node/two-pool partition/fork/heal
returned `release_checks_passed` against the exact new node. Original
`profile-r3` was `incomplete` because no separately frozen **unpatched stock**
executable was then supplied. Pinned stock source inspection still established
that the lab flag and `consensus` RPC were absent; it was not a live probe.

The later, separately frozen stock supplement completed the comparison. With
that supplement all four functional stages have live evidence, but the old
`profile-r3` artifact remains unchanged and is not retroactively relabelled.
After Fable's bounded G1/G2 review, this revision strengthens replay and parser
predicates without re-mining any stage. A fresh non-mining N7 stock/profile
rerun also passed; it is identified separately below.

Both the new source/port review and the namespace review were independently
Fable-approved before the original successful release-check runs. This does not
inherit old-L2 behavior as proof: the evidence below is from new processes on
new datadirs. Broad workspace green and quiet A/B stability were **not**
established by the porter/Fable records and were not independently rerun here.
Those CI-readiness limitations remain separate from these functional results.
This report is evidence for the same independent reviewer, not an upgrade
approval by this agent.

## 2. Frozen candidate and approved reusable inputs

### New node and its build association

| Item | Exact value |
|---|---|
| Official base | `9614b27ecd832a04070a5226da26c8844284f2c4` (`v0.9.0`) |
| Source files | 19 files in `/tmp/opencode/lab-v09-source-manifest.json` |
| Corrected manifest file SHA-256 | `a1abd17039dd952f54074f5a7b75a5f57d1355dd95bc5836281414d9e6b0cc92` |
| Source aggregate | `0e037d358c057d382adcca8d6c094bcf4a46eb1e02758777d20f5d70eb4ddcda` |
| Node executable | `/tmp/opencode/rbitcoin-v0.9-lab/target/dev/debug/rbitcoin-node` |
| Node executable SHA-256 | `bcb6d5eefe0dd0bd96e2ac79e173b4b99a91f67f7717dae6620c30786378a171` |
| Version output | `rbitcoin-node 0.9.0` |
| Build log | `/tmp/opencode/v09-node-build.log`, SHA-256 `4b78f22be06e5d7d69d07a9421981bf0fae057c2416af9cd05463773940c2c79` |
| Mutable port report | SHA-256 `041d301a9ebec3087e9cacf995f0a5fb7973c1b50fee2d84eceba004ea6b3bd7` |
| Parent handoff record | `/tmp/opencode/LP-v09-node-handoff.json`, SHA-256 `de284c0656b8ea5888a88038e581a60129d9a4d6dbbaa4dc92600f0d50e39893` |
| Node Fable record | `/home/ethan/code/silent-pool/docs/reviews/2026-11-06-lab-v09-port-fable-review.md`, SHA-256 `a266df838b2dda15b08933e2a460821257ea6f390a7dc148886d3e4f4e51ab65` |

The manifest prose correction from 17 to 16 self-version entries was consumed
as metadata only: base, all 19 file hashes, aggregate and binary remained
unchanged. The corrected manifest file hash above is the one bound at runtime.
The build was associated by the parent handoff; this validator did not rebuild
or modify the port tree.

### Approved reference and executable inputs

- Old L2 arithmetic/semantic reference only:
  `d2f37b9113e8855245866289ed393043aed6e6153e8101375072cbb23624d499`.
  Its approval does not approve v0.9.
- LP-live driver/oracle: `aa3b37b67960071ec6bd4f09c7cd0ac83e1b526404231994c42dc331dc44ded2`;
  manifest file hash `4aa10e605e8815358cddfe61ece7e05adb939802fca76ae8f197547ee99326d0`.
- Baseline/retarget pool binary:
  `f3b359157e0e019d0f5742dbdc44a558502c94209f50ea4fa96ec703fc41b6a9`.
- Baseline/retarget native miner:
  `7e02b0fdf7434edd081aa60ee421c7eef999418f56345a2f302a756cbab07169`.
- Baseline/retarget scanner:
  `462877fd82b7d7f82f27858fdee78f44e9e1a5f8c2b57c03d7011c98a755aa9c`.
- Old comparison node used only for the fresh default-profile reference:
  `eb257af7c10cdf0ded584adfadf2ae8124466304b13d26c17f855e76f9320f7c`.
- Runtime full aggregate:
  `553cd4e8bb40d34be514a4ebd55d466904e1357f174b2b184b5dcf8724aa5692`;
  code aggregate `2d6c752d24d4f4cc9ea30f6d5ced8b507dfd860acb8721ded85c758992705098`.
- Runtime approval record:
  `/home/ethan/code/silent-pool/docs/reviews/2026-11-06-lab-network-runtime-fable-review.md`,
  SHA-256 `568c2c8e8fd514d7f1afb0058c8958b695fb48bf3475951198492615c000e3ea`.

Namespace project binaries and Nix outputs:

| Component | Binary SHA-256 | Immutable output |
|---|---|---|
| `spc-lab` | `6a3b61f51a8f60a89ff2a70078dd3109fad77a6a55d7caa9078f7e3712bf95d2` | `/nix/store/ac546m1g3z90fsp67vv3mrp4q31wrv4z-spc-lab-0.1.0` |
| `spc-pool` | `aac1278f870fbdd504f9c5846b5d38ebf52335853dcbc471932afb8b9c6c481d` | `/nix/store/39rsjr4drdfw1y1xwcdzcpzpiy3y5x9v-spc-pool-0.1.0` |
| `spc-cpu-miner` | `443de4380b50b48626556bfff3596e621723d54eea82f96d656af39959ffa472` | `/nix/store/kfiw6pylnrkk14vmbvk677vx58932wpp-spc-cpu-miner-0.1.0` |
| `spc-scan` | `f8000b43b3c7f12030be4d2eebb6fb5073cf0ae4b86830f595f24cad6bf14417` | `/nix/store/0fdnzpnlcwkkl1mpgmjl01qyvmmc339q-spc-scan-0.1.0` |

`nix-store --verify-path` and exact deriver checks passed for all four. The
actual runtime used both nodes at the new `bcb6d5...` hash, not the old node
that the original namespace approval used as its baseline. Namespace N1-N7
integration edits were deliberately not chased.

PostgreSQL used `/nix/store/vync24csmxy5saql5ah77vkn59yg2x5h-postgresql-17.11/bin`;
Redis used `/nix/store/nql1lir6650xigvf43n11zz4hh0lbw9z-redis-8.10.3/bin`.
The actual, fully bound input file is `/tmp/opencode/LP-v09-candidate-input.json`,
SHA-256 `55fcdb02ed3fe9c47d3354f7431d0a9acb9799ab33172bc9be481491e99e67a5`.
The contract used by every successful live stage has SHA-256
`adcba5c9cb4b3124cd98a3122d4cd16bc33ad681f78672d09598ac8a331152d1`.

## 3. Independent checker and static/negative results

The gate is in `testing/lab-release/`; its usage and exact commands are in
`testing/lab-release/review_contract.md`.

- `--check-contract`: passed.
- `--static-audit /tmp/opencode/v09-audit`: passed on 21 hash-pinned stock
  excerpts plus the compare and old-patch records. Stock lacks
  `--retarget-regtest` and `getblockchaininfo.consensus`; old L2 contains the
  expected schedule/floor/RPC extension. `SCHEMA_VERSION = 26` is a source-only
  observation, not an old-datadir compatibility claim.
- Self-tests: **15/15 passed**, including synthetic global/prefix approval
  refusals, permanent veto, old-L2/new-candidate separation, source drift and
  symlink rejection, missing-profile failures, static-reference tamper,
  no-fork/lag/baseline-displacement counterexamples, missing evidence, private
  modes and owned cleanup. The committed new-node structural fixture requires
  no external files.
- Real approval-record tests accepted only their exact LP-1, LP-2, L2, LP-live,
  v0.9 and namespace bindings. No parser was loosened to turn pending reviews
  into approvals.
- Final exact input check with `--release-check`: passed, including new node
  and namespace approvals and immutable Nix store paths.

### Actual new-node CLI/RPC profile observations

All seven negative cases exited nonzero with the required explicit reason:

1. retarget on signet;
2. PoW overlay on signet;
3. retarget overlay easier than the floor;
4. overflowing compact target;
5. negative compact target;
6. zero compact target; and
7. malformed bits.

Positive checks passed on independent fresh datadirs:

- Default regtest: every declared `consensus` field matched the old semantics.
  Initial GBT was height 1, bits `207fffff`, the documented real-clock
  min-difficulty exception (`200fffff` was also accepted by the independent
  predicate). A fresh old-node reference returned the same result. Old and new
  `observed-profile.json` files were byte-identical, both SHA-256
  `53927377ecfe2694338f775a5b4f54ae4113e899a52278415fe6d6e7b86d051c`.
- Retarget regtest: exact 10 s spacing, 240 s timespan, 24-block interval,
  floor/overlay `1e7fffff`, retarget enabled, min-difficulty and BIP94 off.
  Initial GBT was height 1, bits `1e7fffff`.
- Live unpatched-stock comparison was **not run**: no separate frozen stock
  executable was supplied. Static absence evidence is not relabelled as a CLI
  probe. This is the only reason profile-r3 is `incomplete`.

Profile summary:
`/tmp/opencode/LP-v09-live-profile-r3/public/summary.json`, SHA-256
`8039ec886d4b57a79b74dc9452169dc4431817a05876039cd0b841617181c453`.

## 4. Real native-SV2 baseline result

Outer result: `release_checks_passed`.
Summary `/tmp/opencode/LP-v09-live-baseline-r2/public/summary.json`, SHA-256
`47eedf950603c56f35613f3de4e940e90462b7310878843c69b206b913c6be44`.
The unmodified LP-live child reported `passed` and explicitly `provisional`
(its internal old acceptance pin was not overridden). Child summary:
`/tmp/opencode/LP-v09-live-baseline-r2-lp/public/summary.json`, SHA-256
`f53333de85703bcebfd486b9bbdbe0bee68254a77f49374d5d730533fec73c72`.

All nine LP-live assertions passed: real native SV2 work, nonblock share credit
at its job multiplier, two contributing identities, actual coinbase/header
binding, configured-depth settlement, v2 work fingerprint, exact conservation,
scanner recovery, and unchanged source inputs.

Observed selected candidate (not an inferred benchmark):

- height 7, canonical block hash
  `0f45ce611b638bffabe2ab2bd9afa826f67241e1ffa4e28ad5f6271c69d90418`;
- seven confirmations at check, 13 slice IDs, two independently scanned
  outputs;
- fees `0`, subsidy `5000000000`, `total_paid = 4949999999`,
  pool amount `50000001`, identities `2`;
- expected Q 65, actual window work 68, overshoot 3, underfill 0;
- a separate non-block-share record for Bob credited at its issued `M = 2`,
  with independently declared share/network targets.

A node-API scan found no `generate*` or `setmocktime` call. The pre-ladder
payout evidence is exact for this run only; its zero fee term adds no live
fee-workload proof.

## 5. Real automatic retarget result

Outer result: `release_checks_passed`.
Summary `/tmp/opencode/LP-v09-live-retarget-r2/public/summary.json`, SHA-256
`170264cd392602902a0aae1f087af5b60413189042d60411bc23015d3154d549`.
Child summary SHA-256:
`c6fc811b9b69c5189d3df9ddb65ddb9d64c4666ff5716ff7fb41a96e22f9701c`.

All 73 headers (genesis plus 72 mined headers) matched the independently
recomputed chain and retarget oracle. No RPC generation, mock time or held
real solution was used.

| Boundary | Parent bits | Actual bits | Independently expected | Elapsed -> effective clamp (seconds) |
|---:|---|---|---|---:|
| 24 | `1e7fffff` | `1e7fffff` | `1e7fffff` | 497327831 -> 960 |
| 48 | `1e7fffff` | `1e608887` | `1e608887` | 181 -> 181 |
| 72 | `1e608887` | `1e5ae6f6` | `1e5ae6f6` | 226 -> 226 |

The genesis-anchored first boundary is therefore distinguished from the two
measured full periods. Applied budget commands and actual work counters are
reported by the driver; no causal budget-to-pace claim is made. Observations
ran from `2026-11-07T01:49:38.045334Z` to `02:02:07.292912Z`, but that wall time
is not a benchmark or CI-stability result.

Fresh derivation into `private/stages/retarget/independent-replay` produced:

- `retarget-duty-cycle-derived.json`, SHA-256
  `21959f91a80d6ed7b0e1a9d08c5b1d21efc736443ba293618e9aaa517267da21`;
- `retarget-oracle-replay-derived.json`, SHA-256
  `20ceade4132b66267b200bbc1b5b64f003485710a894bd5e9eac91a0ad45f7f9`.

Derived wall-time groups contained 24/24/24/0 unique observed canonical
best-tip submissions; each full height period contains 23 timestamp intervals.
There was no extra tail submission in this run. Unknown pool outcomes are
reported at two distinct observation times: **23** in the live
`retarget_evidence.duty_cycle`, read from `pool.log` before pool shutdown, and
**27** in the final derived replay after shutdown. Fable independently confirmed
27 in the final log. These are scoped counts, not a contradiction or a fabricated
accepted/rejected classification.

## 6. Real two-node/two-pool namespace result

Outer result: `release_checks_passed`.
Summary `/tmp/opencode/LP-v09-live-namespace-r2/public/summary.json`, SHA-256
`dd5b253261d4206018160923ecf31c710ecdd3a99ccc3fc7be2bbc8dcc8420e0`.
Runtime result SHA-256:
`225b7130146eefe4426b9ca7a13f6b2dfa7ad53acf3722206b79b18559aaeeda`.

Both nodes used new executable `bcb6d5...`. Nix pool/miner/scanner and the
approved frozen runtime scenario were used with real P2P link partitioning.

Observed pre-heal, worker-paused divergent tips:

- node-a height 15, hash
  `388c545a4631e4246bdc3784311c045f06c67d9b3915cae4594d04186c655ecd`;
- node-b height 14, hash
  `22ff609226483861448696a57b4b758ee61e19fcb4bd98df4bff65841a0d1904`.

After the real heal and resumed native work, the paused atomic final view was
both nodes at height 47 with identical hash
`47bd16f98dec6fa9755a58f8f94f11c407f5596f87b8f7734056ce8ac7555e80`.
A raw tip-displacement record was present; convergence was not inferred from a
momentary equal height. All four runtime assertions passed and evidence gaps
were zero.

### G1: genuine fork/heal replay added after Fable review

`testing/lab-release/namespace_replay.py` independently derives the per-node
chains instead of accepting unequal tip hashes. The proof now requires:

- a paused common pre-partition block at height 7, hash
  `7548ae23eaf21676a7b9d80c54b1179139b877f38cddafe71aea5171dc74d89e`;
- raw-header parent-linked descent to both paused pre-heal tips within the
  link-partition window;
- different hashes at common height 14 (node-a `4e4cf648...bf1c`, node-b
  `22ff6092...1904`), not merely different tip heights;
- post-heal re-announcement of node-a heights 8-15 on node-b; and
- the post-heal event-724 displacement of node-b's height-13 losing block
  `61551bcd...403a`, anchored to node-a's surviving height-15 block
  `388c545a...5ecd` inside the partition window.

This is the observed seven-block losing segment followed by adoption of the
winning tip; the replay list also includes that height-15 tip. No parent links
are invented from height alone. A missing ancestor/header/observed branch yields
qualified `unknown`, never a pass. A same-chain lag returns
`namespace_no_real_fork`; a baseline-only displacement returns
`namespace_reorg_evidence`. Old-runtime displacement anchors may descend from
later winning blocks, and replay validates those parent links explicitly too.

New derived replay: `/tmp/opencode/LP-v09-r2-namespace-replay/evidence.json`,
SHA-256 `a5322a67ca8ada3e24b8c00940259ad2d821fe9e92d54ba8506204d2cd1c2480`.
Fable's new-node lag mutation and a baseline-only displacement mutation were
rejected with the required codes and preserved with input hashes in
`/tmp/opencode/LP-v09-r2-fable-counterexamples.json`, SHA-256
`08bd240f37b9ebb19dc00d13ac1cd90407e8356c6fb458d866f5bcfab79c95fe`. No original
raw artifact was modified and no stage was re-mined.

Independent scoped checks found:

- 276 no-bypass probes, zero violations, with actual `link-p2p-nodes`
  partition/heal commands;
- 71/71 accounted target/credit records consistent with their typed
  front/job/generation evidence, 80 scoped job keys and zero unknown records;
- no forbidden generation/mock-time method in either node API log (1433 and
  1021 parsed calls respectively);
- all ten actor records terminal; no process matching this run survived and
  `/run/netns` was empty after the run.

This is host-tool namespace evidence, separate from the still-unrealizable
pinned Nix network tools. Its accounting check is scoped and is **not** a full
header-family-target, database-credit-sum, payout, settlement or sub-block-share
proof. Native per-job/settlement evidence comes from the separate baseline run.
No sub-block conclusion is drawn from the namespace run.

## 6a. Fable r2 dispositions

| Finding | Resolution and actual evidence |
|---|---|
| G1 namespace predicate | Replaced unequal-tip acceptance with the paused-anchor, per-node raw-header chain and partition-window displacement proof above. The exact Fable same-chain lag fails `namespace_no_real_fork`; baseline-only displacement fails `namespace_reorg_evidence`. Missing ancestor, cross-node confusion, phase-window and chain-hole negatives are covered; absence of proof is qualified unknown. |
| G2 approval multi-binding | Each APPROVE pass now computes one declaration set and admits exactly one unambiguous full hash with matching verdict prefixes/suffixes. Conflicting historical/under-review hashes invalidate the whole pass. Fenced verdicts are ignored, unrelated bare hashes are not associations, and REQUEST_CHANGES permanent vetoes remain broad. Both Fable fixtures approve neither hash; actual input approvals and all six earlier-rejected hashes retain their intended outcomes. |
| N1 unknown counts | The report now distinguishes 23 mid-run unknowns from 27 final derived unknowns, with log read timing explained. |
| N2 chronology | The opening now says three original passes plus original profile incomplete, then the stock supplement all four functional stages. No overwriting chronology. |
| N3 new-node unit fixture | Added `fixtures/namespace-v09-structure.json`, a 49 KB structural/redacted projection of the new-node run (block hashes, headers and times only). It exercises the 7-block fork point and displacement with no credentials, identities or private paths. Full old/new raw replays remain optional extras, not sole unit coverage. |
| N4 fenced verdicts | Verdict parsing skips fenced lines, including when fenced examples are inside a Result section. Manifest `(matches)` declarations used by real reviews remain supported. |
| N6 initial GBT semantics | The default run actually returned `207fffff`, not the requested `200fffff` overlay. Both are allowed only at the documented real-clock min-difficulty gap; effective consensus/overlay fields are checked separately. Retarget GBT remained exactly `1e7fffff`. |
| N7 stock refusal | `_stock_refusal` now also requires stock help to contain `--regtest-pow-bits` and omit `--retarget-regtest`, then requires a recognized parser error rather than relying on the echoed flag substring. A fresh non-mining comparison passed. |

The r2 stock/profile comparison did not re-mine. Its summary is
`/tmp/opencode/LP-v09-r2-profile-stock-r1/public/summary.json`, SHA-256
`d7a9931f2975b021f9ff5cd2d58e85d2bc2b0e83fa12d1b4594a24cc14f9b4b8`.
Its new input is `/tmp/opencode/LP-v09-r2-profile-stock-input.json`, SHA-256
`1e0178d71bb7b055bd756ac05eea6e8d02da64ef136022531c30e29bddba1898`.
The stock help hash is `fcd654c61b3be32da49e9395322a497b7b086edd242e553d9b1a9fcfb46e6d83`;
the observed parser reason is `requires a value`. Previous supplement summary
`a96fa067da9428d733a096616ea25bac14df6ba870a7d4364b889a5d4b05ef24` and all
previous artifacts remain unchanged.

The final read-only r2 evidence validation also returned `passed` for baseline,
retarget, namespace and both stock profile predicates, with both required
counterexamples rejected: `/tmp/opencode/LP-v09-r2-evidence-validation.json`,
SHA-256 `7b840f21a1edf45b264ad1096d1aaa20389a024564811aebb7d5bb9338654eec`.

## 7. Failed/inconclusive attempts retained

These were harness or environment issues, not silently discarded results and
not attributed to the v0.9 node:

| Attempt | Actual outcome | Disposition |
|---|---|---|
| `profile-r1` | Malformed-bits case failed as expected, but harness expected `regtest-pow-bits` instead of actual clear `conf regtest_pow_bits` diagnostic | Fixed expected diagnostic; r1 retained |
| `profile-r2` | Harness incorrectly required overlay bits at real-clock height 1; effective profile matched | Old-node comparison confirmed documented min-gap `207fffff`; predicate corrected |
| `baseline-r1` | PostgreSQL refused an over-107-byte Unix socket path; no mining occurred | Shortened artifact paths; failed run retained |
| `retarget-r1` | Inconclusive at height 68/72 due short outer timeout; no third boundary or retarget result | Corrected per-phase bound and exec identity; r1 partial artifacts retained |
| `namespace-r1` | Wrapper path-type error before any actor launched | Path conversion fixed; no live result inferred |

The retarget-r1 ownership check initially confused a normal exec transition;
only the PID start-time and random token are now required for signalling.
Owned cleanup succeeded and no run-matching process survived. No production
source was patched and no runtime defect was found.

## 8. Provenance, permissions and reproducibility

Every new run verified all source manifests, approval records, executable
hashes and Nix bindings before and after execution. The driver writes an outer
summary containing its exact script hash:

| Stage | Driver script SHA-256 |
|---|---|
| profile-r3 | `0981344746ee4fdd12fffc177a850e20df0cb75dd894963ccdb41d4f5ffbf49c` |
| baseline-r2 | `eeadd623887492ea2742bc523c02e6ee3c60b8048d36b4d35d2a1ded8822a80c` |
| retarget-r2 | `e5c924adf938ccf2d4a9db1c4d2aede98293d3b18e49770a531e4f2d892b2412` |
| namespace-r2 | `23c5c8261e47a14117cec02b051ce0c87fd6f4931201ae97e13ae0983a34102a` |
| post-lint original gate, used for read-only revalidation | `b2dd6cbc74a1b00fa6d0c5d46439043b05c3a355fb799a3fdd22262441829afd` |
| r2 G1/G2/N7 profile driver | `14469af5964f2a059ac65c415dbfd3ea9c8d8613dcaf95c52bf92e3bc9080f8d` |
| r2 namespace replayer | `7d16def46fddcf81b766d1355a5e23edad41b134e881ace9a760cc7983d5239e` |
| r2 approval parser (`release_lib.py`) | `923d3d71a16afa4578f48d288be598355f748ecc90fdf7e5ecafef06d32c1faa` |

Final read-only revalidation returned `passed` independently for baseline,
retarget and namespace artifacts; `/tmp/opencode/LP-v09-live-final-evidence.json`
has SHA-256 `628af0d331e5de334780c99f8d4c4252c745c4fe88affd3ecccf3f7150d04d72`.

The differences are the documented harness fixes above; node/pool/miner/
runtime sources and binaries did not change. A second full mining repetition
was not performed merely to unify these driver hashes. The raw per-stage hashes
and old versions in `/tmp/opencode` preserve that history, and the final checker
revalidates the preserved artifacts. `/tmp/opencode` had 2.9 GB free at report
time; retaining another roughly 700 MB retarget run was unnecessary.

Successful-run artifact trees are files `0600`, directories `0700`; outer
console/diagnostic logs were likewise normalized to `0600`. Each LP and
namespace actor root is under a unique `/tmp/opencode/LP-v09-live-*` name. No
existing running node was hot-swapped and no datadir was reused.

The preserved approved LP-live historical evidence remained unchanged:

- old baseline summary `f8f62fba124618285272c1b3da59b637ba8aa8339c6f66f9f00db0a14886111d`;
- old retarget summary `93ddfabcc128f6cf4ec29874a5fe17016651f0b7103b978ddd133c8504484b31`.

Exact commands are in `testing/lab-release/review_contract.md`. No build,
commit, push, reset, stash, production edit, or other-worktree write was made.

## 9. Explicit non-claims and remaining limits

- No causal hash-budget/block-rate, benchmark, or CI-stability conclusion.
- No broad-workspace-green or quiet A/B baseline claim; porter shared-load
  failures were not independently rerun.
- Original `profile-r3` remains incomplete by design. The later stock
  supplement and the r2 non-mining predicate rerun are separate live evidence;
  no historical artifact was relabelled.
- Pre-ladder exact-owed behavior only; no ladder/tier-conformance claim.
- Selected payout candidates had fee zero; no nonzero-fee workload or fee-term
  proof was added.
- No Ark, mailbox, privacy-boundary, witness, SV1, type-32, or general SV2
  interoperability claim.
- Namespace accounting is scoped; no full payout/settlement/header-family
  oracle or sub-block claim is taken from it.
- Schema 26 was checked only in stock source excerpts. All live chains were
  fresh; prior-version datadirs and migration/opening were intentionally not
  tested.
- No deployment/production upgrade approval is issued by this report. The
  candidate, gate code, raw evidence and remaining limits go to independent
  Fable review.
