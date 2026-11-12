# AP-1a ark-rpc probe tooling: primary integration

Date: 2026-11-08
Owner: serial primary code integration owner (approved AP-1a r2 tooling packet only)
Status: complete; transferred packet frozen after the checks in section 4
Candidate: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-ark-rpc-probe`
(read-only throughout), r2 review manifest
`/tmp/opencode/lab-ark-rpc-probe-r2-review-manifest.json`
(sorted-key compact JSON `{base, files}` reproduces
`f0aa504aa6f247474eb247b4c25d12f286bb3f88abb1d42e77c3ba643f65ce12`),
74 packet paths, base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.
Approval: Fable re-review r2 (final section of
`docs/reviews/2026-11-08-ark-rpc-probe-fable.md`, primary copy preserved
byte-identical, SHA-256 `240bb301f8539f242645c8c97f6dbd0f264ab228c43eacc28ada5d77d3e615b2`):
APPROVE - the bounded read-only probe and the exact contract
`f4fc3e043f420870675095ab24144ba14e0afba6cb9ad69d818d99b1bccb2f62`, with
approval scope exactly `testing/ark-rpc/**` as frozen by the r2 review
manifest plus the canonical contract JSON as the AP-1a lineage T0 may bind
to after the recorded decision.

This document is the sole unapproved evidence record of the transfer. It is
not a behavior change and adds no workspace member, node profile, pin or
runtime adoption.

## 1. Pre-edit verification (all before any write to primary)

- Manifest recipe reproduced from the manifest's own rows: full snapshot
  `f0aa504a...` matches the approval.
- All 74 records re-hashed in the candidate worktree (hash and mode):
  0 mismatches.
- Primary pre-state: every one of the 74 destination paths was absent
  (clean add); 0 collisions with the approved 370-file baseline and 0
  collisions with pre-existing user work. The primary Fable review
  `docs/reviews/2026-11-08-ark-rpc-probe-fable.md` is **not** a packet path
  and was never touched.
- Primary index hash before any edit: `git write-tree` =
  `44f21a926d4d065ac6937fb0f81b698c7b2d0627` (nothing staged; unchanged
  throughout - section 5).

## 2. Transfer (add-only, exact bytes and modes)

The 74 packet paths were copied from the candidate with per-file
re-verification: destination re-checked absent immediately before each
copy, source re-hashed against the frozen manifest, then the copied file
re-hashed and its mode re-checked (executable bits on
`testing/ark-rpc/devcheck/run.sh`, `testing/ark-rpc/probe.py` and
`testing/ark-rpc/run_two_nodes.sh` preserved as `100755`). All 74 paths
verify `(path, sha256, mode)`-exact against the frozen r2 review manifest
(full per-file list with hashes in
`/tmp/opencode/AP1a-integration-transfer.log` and
`/tmp/opencode/AP1a-primary-integration-manifest.json`).

Packet shape: 69 files under `testing/ark-rpc/` (probe package
`arkrpc/`, entry points, fixtures incl. 20 scenario files, report schema,
six historical evidence report directories, devcheck crate, shell helpers,
unit tests) + 5 implementation/review documents under `docs/reviews/`.

Nothing else changed: no `Cargo.toml`, `Cargo.lock`, `flake.nix`, SPEC,
node profile or pin is part of the packet; the audit run reports keep
their historical source paths truthfully (byte-exact copies - the
`reports/` manifests and `report.json` files still name the candidate
worktree paths they were actually produced in; no path was rewritten).

## 3. Non-interference

- Index hash before == after:
  `44f21a926d4d065ac6937fb0f81b698c7b2d0627`.
- `git status` delta vs the pre-transfer snapshot is exactly 7 added `??`
  lines: the five packet `docs/reviews/2026-11-0*-ark-rpc-probe-*.md`
  documents, `testing/ark-rpc/`, and this session's
  `docs/reviews/2026-11-08-agent-wallet-primary-integration.md` (the AP-3a
  receipt, written after that snapshot). Nothing removed or modified.
- No node was started, no profile adopted, no cache touched; the probe is
  read-only tooling and the checks below run entirely offline against the
  frozen fixtures.

## 4. Checks (transferred tree)

| Command (in `testing/ark-rpc/`) | Exit | Result | Log SHA-256 |
|---|---|---|---|
| `python3 -m unittest discover -s tests` | 0 | Ran 113 tests, OK (0 failures) | `3018bb16afd0df8197f5bb875b4c6611ed26bcc022d91a2e66e69c624f7484c1` |
| `./probe.py self-test` | 0 | 20 scenarios, 0 failed | `c70af649e7396464642cf13041823d4a8ea4a8368a640323249c5e5d95ad8cd5` |

These match the reviewer's r2 receipts exactly (113 OK; 20/20). The
devcheck cargo crate and the two-node run script were **not** executed:
they are node-facing evidence tooling, and a fresh node run or dev rebuild
is out of scope for a tooling-only transfer (the r2 `dev_verification`
receipts embedded in the frozen reports remain the standing evidence).

## 5. Scope and limits

- The approval covers the read-only probe and the contract `f4fc3e04...`
  only. No node today satisfies `spc.node.regtest.mempool_rpc.v1`: both
  approved binaries project `recognized = false` with the `getmempoolinfo`
  row `passed / failed` for the selected modelling consumer
  (corepc-types 0.13.0 `v30::GetMempoolInfo`) - the shape failure is
  recorded, not fixed, and is not a universal Core compatibility verdict.
- No T0/T1 traffic runtime, signer, actor, funded behavior, node patch,
  version override or Ark readiness is established by this transfer.
- The AP-1a lineage binding into T0 (the allowlist entry and the recorded
  decision) lands with the T0 packet, not here.
- No commit was made; the transferred work sits in the working tree
  alongside the pre-existing primary user work.

## 6. Freeze

The transferred packet (74 paths) is frozen as of the completion of the
section 4 checks. Receipts: `/tmp/opencode/AP1a-primary-integration-manifest.json`
and the `/tmp/opencode/AP1a-integration-*` files (transfer log with
per-file hashes, check logs, git-status snapshots).
