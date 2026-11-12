# Parent Review Manifest — AP-1a ark-rpc probe packet, r1 revision (2026-11-08)

One-page freeze for the parent review. Full detail:
`docs/reviews/2026-11-08-ark-rpc-probe-r1.md`.

## Packet identity

- Worktree: `lab-ark-rpc-probe`, base commit `b0ab4ca34fda07c61a6335d05f598eaacc69678c`,
  all changes uncommitted (nothing was committed on the user's behalf).
- Scope owned: `testing/ark-rpc/**` + review docs. No node, pool, agent,
  primary-tree, or pin changes.
- Probe: `ap1a-ark-rpc-probe` (Python 3 stdlib only), report schema v2,
  manifest version 2.

## Canonical contract hash (FINAL; the only approved value)

```
11bdb1007e130a749720f87482f287c1fb65d1e743c142fc4b1ca15a782d89e7
```

- Covers `CONTRACTS` + `ENCODINGS` + `CHECKS` + `VERDICT_RULES` via
  `arkrpc.contracts.canonical_contract_json()` (sha256).
- Embedded and verified matching in both r1 reports and both r1
  manifests. Regenerate/diff: `cd testing/ark-rpc && ./probe.py
  contract-show`.
- Supersedes (both unapproved, never frozen for T0): v1-era
  `48894548...` and r1-draft `905bf4b8...` (pre-getmempoolinfo).

## Evidence (final run, token `AP1a-b1aeb688e017`)

| dir | node binary sha256 | verdict | checks |
|---|---|---|---|
| `testing/ark-rpc/reports/2026-11-08-r1-l2-eb257af7/` | `eb257af7c10cdf0ded584adfadf2ae8124466304b13d26c17f855e76f9320f7c` | `incompatible` (`fail-version-gate`, `funded-unproven`) | 40 |
| `testing/ark-rpc/reports/2026-11-08-r1-v09lab-bcb6d5ee/` | `bcb6d5eefe0dd0bd96e2ac79e173b4b99a91f67f7717dae6620c30786378a171` | `incompatible` (`fail-version-gate`, `funded-unproven`) | 40 |

Both nodes identical on every check; version gate fails honestly
(190000 vs bark's 310000/290000 minimums); funded behavior permanently
`unproven`/`not_run` (no fabricated funded pass). Frozen v1 evidence
(`reports/l2-eb257af7`, `reports/v09lab-bcb6d5ee`) untouched, still
validates under `SCHEMA_V1`.

## T0 traffic-slice projection (for worker ses_e4604a9d0ffeHEKwnwhfH0S61J)

- Subset: `sendrawtransaction`, `getrawmempool`, `getrawtransaction`,
  `getmempoolinfo`, `getblock` — consumed from THIS lineage
  (`report.checks[]` rows, schema v2); no second probe/client/verdict.
- Exact check IDs, axes, statuses, and semantics: r1 review doc §5.
- No Bark version floor on the traffic projection; recognition/shape vs
  funded-behavior and `passed|failed|unknown|not_run` preserved.
- Key traffic fact: `getmempoolinfo` routed on both binaries, all 13
  corepc-v30 fields present, but `maxdatacarriersize` is JSON `null`
  (Core emits an integer) — the pinned client's v30 deserializer
  rejects it; the traffic check fails honestly and never gates the bark
  verdict (coverage class `traffic`).

## Verification performed (this revision)

- 87 unit tests OK; 15 self-test scenarios OK (incl. traffic
  non-gating both directions); 12/12 devcheck vectors vs the real
  pinned crate types OK; both r1 manifests' file-hash trees verified
  against the final tree; both report contract hashes match the final
  hash above.

## Artifact hashes

| file | sha256 |
|---|---|
| `docs/reviews/2026-11-07-ark-rpc-probe-implementation.md` (v1 review, corrected + superseded banner) | `47b00a3f434d94d2175ffea2534fb29e1cd7673cac7201d4fa58f81d3fa78f8c` |
| `docs/reviews/2026-11-08-ark-rpc-probe-r1.md` (this revision's review) | `15541e903a5ce2e6dbd0058bad117d909577e2d140f6db1ebf7b992095221603` |
| `testing/ark-rpc/reports/gap-matrix.md` | `1cb21982f47d844f4c8f529c9cfd29f271cf7c0ab3f5ead1f74ab0e0ff24dd1d` |
| `testing/ark-rpc/reports/2026-11-08-r1-l2-eb257af7/manifest.json` | `ea23275865d15e970fbeb6048998a13fc84757863d7de8835f4e4d78bb03f515` |
| `testing/ark-rpc/reports/2026-11-08-r1-v09lab-bcb6d5ee/manifest.json` | `d3d50059a1eb8d9830f1522185ff40f5e85c10e10fd030434e9db6007a270fbd` |

(The two r1 `manifest.json` files additionally freeze the sha256 of
every probe source file at probe time; those trees were verified
mismatch-free against the final tree.)

## Provenance disclosure

> **Correction (2026-11-08, r2 revision, Fable D1 + D3):** the
> paragraph below was wrong, and the v0.9 binary sha256 in the table
> above had a one-character typo (`...e173f4b99a...` for the real
> `...e173b4b99a...`). The Fable review exists at the primary absolute
> path
> `/home/ethan/code/silent-pool/docs/reviews/2026-11-08-ark-rpc-probe-fable.md`;
> dispositions in the r1 and r2 review docs were cross-checked against
> it.

The Fable review file was not found anywhere in the repo; findings
F1-F9 were received verbatim in the requester's message and are
dispositioned in r1 review doc §0/§2. *(Superseded by the correction
above.)*
