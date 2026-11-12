# Parent Review Manifest — AP-1a ark-rpc probe packet, r2 revision (2026-11-08)

One-page freeze for the parent review. Full detail:
`docs/reviews/2026-11-08-ark-rpc-probe-r2.md`.

## Packet identity

- Worktree: `lab-ark-rpc-probe`, base commit `b0ab4ca34fda07c61a6335d05f598eaacc69678c`,
  all changes uncommitted (nothing was committed on the user's behalf).
- Scope owned: `testing/ark-rpc/**` + review docs. No node, pool, agent,
  primary-tree, or pin changes.
- Probe: `ap1a-ark-rpc-probe` (Python 3 stdlib only), report schema v2,
  manifest version 2.
- r2 revision applies the Fable r1 re-review REQUEST_CHANGES findings:
  C1, C2 (blocking), D1, D2, D3, D5 (required), D4, D6, D7
  (non-blocking). Dispositions cross-checked line by line against
  `/home/ethan/code/silent-pool/docs/reviews/2026-11-08-ark-rpc-probe-fable.md`
  (r1 re-review: lines 300-594).

## Canonical contract hash (r2 candidate; NOT yet approved)

```
f4fc3e043f420870675095ab24144ba14e0afba6cb9ad69d818d99b1bccb2f62
```

- Covers `CONTRACTS` + `ENCODINGS` + `CHECKS` + `VERDICT_RULES` via
  `arkrpc.contracts.canonical_contract_json()` (sha256).
- Embedded and verified matching in both r2 reports and both r2
  manifests. Regenerate/diff: `cd testing/ark-rpc && ./probe.py
  contract-show`.
- Supersedes (all unapproved, never frozen by T0): r1
  `11bdb1007e130a749720f87482f287c1fb65d1e743c142fc4b1ca15a782d89e7`,
  r1-draft `905bf4b8...`, v1-era `48894548...`.
- Awaiting the Fable re-review; T0 must not freeze it before approval.

## r2 evidence (run token `AP1a-1fd3d1813c33`, read-only, fresh nodes)

| artifact | sha256 |
|---|---|
| `testing/ark-rpc/reports/2026-11-08-r2-l2-eb257af7/manifest.json` | `c6802ceb70d567f0eaacafcf8dfd43c142c6efb5e01133be4b127a9fe909ccc4` |
| `testing/ark-rpc/reports/2026-11-08-r2-l2-eb257af7/report.json` | `e834cba45cf3417711e8c0084e5792b2b18dec044782b22c00ffa507282c6d04` |
| `testing/ark-rpc/reports/2026-11-08-r2-l2-eb257af7/raw_responses.json` | `155923fa5036e7a45be7e29239764e00c126d1bd568dde551219b31deea377a0` |
| `testing/ark-rpc/reports/2026-11-08-r2-v09lab-bcb6d5ee/manifest.json` | `359747081a61be77c19a6f36b90f942ba760e2b5c47e5528c9e93f90e3af07c2` |
| `testing/ark-rpc/reports/2026-11-08-r2-v09lab-bcb6d5ee/report.json` | `e00030b76fd0aa9c60d3b4daa4a194915678c7eaa9e277702dc627e5e62ebfe6` |
| `testing/ark-rpc/reports/2026-11-08-r2-v09lab-bcb6d5ee/raw_responses.json` | `e77ebdfecf08c28dbb450cf57049fc72970e98fb0464d7f4c7a797c6e5e4dbac` |
| `testing/ark-rpc/reports/gap-matrix.md` (regenerated from r2) | `8eac634d25f73410188a68a6710e03f975980128ffe6f21874757a35530905cd` |
| `docs/reviews/2026-11-08-ark-rpc-probe-r2.md` | `bb992dc5b27377c1b03e0d51a715f95d496bc3a274b5e8bee47479b6493f36df` |

| report | node binary sha256 | verdict | checks |
|---|---|---|---|
| `2026-11-08-r2-l2-eb257af7` | `eb257af7c10cdf0ded584adfadf2ae8124466304b13d26c17f855e76f9320f7c` | `incompatible` (`fail-version-gate`, `funded-unproven`) | 40 |
| `2026-11-08-r2-v09lab-bcb6d5ee` | `bcb6d5eefe0dd0bd96e2ac79e173b4b99a91f67f7717dae6620c30786378a171` | `incompatible` (`fail-version-gate`, `funded-unproven`) | 40 |

- 40 rows per node, identical across both binaries; honest verdicts,
  funded behavior permanently unproven (read-only mode).
- devcheck gate bound into both manifests (`dev_verification`: exit 0,
  12 vectors, source hashes).
- Hash-tree consistency: each manifest `freeze_recipe` == current
  `_hash_tree` (38 hashed files) == report `contract_hash` == canonical
  `contract_hash()`.
- Historical evidence preserved byte-unchanged: r1 dirs
  (`2026-11-08-r1-*`, token `AP1a-b1aeb688e017`, hash `11bdb100...`,
  validated schema-only by unit tests) and frozen v1 dirs
  (`l2-eb257af7`, `v09lab-bcb6d5ee`).

## Verification state (all green, post-fix, pre-freeze)

- Unit tests: 113 OK (`python3 -m unittest discover -s tests`).
- Self-test: 20 scenarios, 0 failed (`./probe.py self-test`).
- devcheck: 12/12 vectors PASS (`devcheck/run.sh`).
- `testing/ark-rpc/**` is FROZEN as of the r2 run: nothing under it may
  change without a new evidence run and a new hash.

## Non-claims (Fable R7, standing)

- The candidate hash is NOT approved by this packet; no `bark-v31`
  named profile, no Core RPC-version compatibility claim, no node patch
  or version override, no bark acceptance.
- No funded behavior exercised or proven.
- AP-1b, AP-2, AP-3 scopes are not touched.

## Prior packet versions

- r1 one-pager: `docs/reviews/2026-11-08-ark-rpc-probe-parent-manifest.md`
  (carries dated D1/D3 corrections; its hash `11bdb100...` was never
  approved).
- Full history: `docs/reviews/2026-11-08-ark-rpc-probe-r2.md` section 1.
