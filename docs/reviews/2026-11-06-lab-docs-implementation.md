# 2026-11-06 - Lab documentation integration

Status: documentation-only integration candidate for Fable review. This packet
does not edit Rust, Cargo, flake, scripts, the main progress board or review
histories, and does not mark the entire lab complete. The host runtime remains
approved only at its prior aggregate; this packet changes documents, including
the normative SPEC body, so the resulting documentation aggregate needs its own
review.

## 1. Inputs and baseline

| Input | Snapshot / evidence |
|---|---|
| Base | `b0ab4ca34fda07c61a6335d05f598eaacc69678c` |
| Approved host runtime docs | `/tmp/opencode/lab-runtime-review-manifest.json`, aggregate `e891f34c69c70c2a6748be1ca2a77a71aba4a63377eda55a43afeb8613b68043`, tested code `d75ab391b0ca1c8ab5b03af27dd5cf45a8a384ef8ac056a1ceeb2b2f456d8684` |
| Approved LP-1 docs | `c16032a78902e88b0157efed6780bc8b424e942b1c3bf46893d8fb09ae4708ee` |
| LP-live docs | `aa3b37b67960071ec6bd4f09c7cd0ac83e1b526404231994c42dc331dc44ded2` |

25 approved documentation files were copied byte-for-byte from the frozen
runtime worktree and their hashes checked against the approved manifest:
the lab implementation contract, SPEC and WIRE splices, the four 2026-11-05
lab decision/changelog pairs, the 2026-11-06 runtime/pool-latency
decision/changelog pairs, and the approved implementation reports. The Fable
review histories were **not** copied; they remain read-only context in the main
tree.

Baseline hashes for the files this packet edits, the full copied-input list and
the merge order are recorded both in
[`2026-11-06-lab-docs-baseline.md`](2026-11-06-lab-docs-baseline.md) and in
`/tmp/opencode/lab-docs-baseline.json`.

## 2. Owned files and delta

Edited:

- `README.md` - status and host-lab quick start; links to the operator guide
  and to the type-32 deltas.
- `docs/SPEC.md` - v1/v2 version matrix, exact work family and issuance,
  network-work window `Q(T)`, checked arithmetic, per-row scrub/retention,
  RAM cutoff, type-32 refusal, settlement immutability, wire fields, Noise
  authority config, and the v2-only retention/privacy boundary.
- `docs/WIRE-FORMATS.md` - v2 metadata in `spc:blocks`/`spc:slices`, zero
  sentinel/nonzero-fingerprint rule, v1-only type-32 bootstrap.
- `docs/OPERATIONS.md` - v2 profile and regtest restriction, type-32 refusal,
  retention/scrub, Noise secret handling, migration/rollback notes, host-lab
  pointer.
- `docs/share-accounting-ext-deltas.md` - v2 scope refusal and v1-only window
  source.
- `docs/REGTEST-LAB-IMPLEMENTATION.md` - merge-note status updated for the
  documentation packet; runtime appendix retained.
- `docs/decisions/2026-11-02-share-target-policy.md` - prescribed
  "Superseded in part (2026-11-05)" banner only; historical body/date and v1
  policy are unchanged.

Added:

- `docs/LAB-RUNTIME.md` - operator guide for the approved host slice: Nix
  package commands, explicit node exception, startup/headless/browser commands,
  private artifact paths, idempotent controls, cleanup, evidence scope and
  capability limits.
- `docs/reviews/2026-11-06-lab-docs-implementation.md` - this report.
- `docs/reviews/2026-11-06-lab-docs-baseline.md` - owned-file base/final
  hashes, copied-input hashes and merge order.

No Rust/Cargo/flake/script/source file was edited. The F1 malformed share-hash
merge fix remains assigned to the topology owner and was not touched.

## 3. Normative changes and clarifications

These are applications of the approved 2026-11-05 lab decisions, not a new
payout policy:

1. **Version matrix.** v1 is frozen: legacy `share_difficulty`, `N * d` and
   saturating arithmetic retain their existing meaning and rows. v2 is opt-in,
   `pool.network = "regtest"` only, and never silently reinterprets v1.
2. **Exact family.** v2 uses `R = 2^k - 1`, `m_cfg = 2^j`, issued
   `S(2^j_issue) = 2^(k-j_issue) - 1`, credited `d = 2^j_issue`. The reference
   must be a power of two minus one and at or above the effective pow limit;
   multipliers are bounded by `2^32`; stepdown/stepup allocates a new
   generation and never reprices an issued job.
3. **Window `Q(T)`.** v2 selects by
   `Q(T) = ceil(N * (R + 1) / (T + 1))` network-reference units, not `N * d`.
   `Q` is persisted; underfill/overshoot are derived; `Q` above
   `max_window_target_units` fails; bounded history below Q is explicit
   `window_underfilled`, never N-block coverage.
4. **Checked arithmetic and frozen settlement.** v1 keeps its saturating path;
   v2 fails with named errors on overflow/division. Settlement uses the
   snapshot's persisted version/fingerprint/rule/target and the finding job's
   immutable `nbits`; a later retarget cannot rewrite it.
5. **Persistence and scrub.** SQL carries version/fingerprint checks with
   legacy defaults. Durable deletion is selected per stored row version: v1
   keeps the legacy horizon; v2 uses `height < tip - 4096` (row at the cutoff
   retained). The live v2 RAM tail is capped at 4096 slices and uses template
   height `tip + 1`, one block stricter than SQL. This v2-only attribution
   retention extension adds no identity field to the Ark boundary.
6. **Type-32.** v1 behavior is unchanged. v2 does not advertise or accept
   `0x0020`; requests receive `unsupported-extension` and no v2 slice populates
   the type-32 store.
7. **Noise authority.** `authority_public_key` is the public pin;
   `authority_secret_key` is private, generated/provisioned together, mode
   0600, never logged or published. A mismatched pair fails startup.
8. **Wire.** `spc:blocks` carries definite work-unit version/fingerprint and
   `window_rule`; `spc:slices` carries version/fingerprint. v1 uses the 32-byte
   zero sentinel on the wire and NULL in SQL; v2 requires a nonzero 32-byte
   fingerprint. Bootstrap `type32_blobs` are identity-free and MAY be present
   for v2 tail rows because `payout` exports every row's blob; the v2 front
   ignores them and does not serve type-32.
9. **No tool-only SPEC entry.** The 2026-11-06 host-runtime and pool-latency
   lab records remain as decision/changelog documents; they are not in the
   normative SPEC frontmatter. The merge session decides the final
   SPEC/OPERATIONS splice.
10. **Payout policy.** The lab remains exact-`owed` pre-ladder scope. The
    denomination ladder, `DESIGN.md` derivation/tiers/Ark delivery and
    production payout policy are unchanged; no lab capability implicitly
    amends them.

### D1-D3 disposition

- **D1 fixed.** `type32_blobs` is documented as the identity-free export of
  every `window_tail` row; a v2 payload MAY contain blobs for v2 slices. The
  v2 front ignores them and never populates `Type32Store`; the service refusal
  remains v1-only. No wire format or code changes were made.
- **D2 fixed.** `LAB-RUNTIME.md` section 7 and section 6 below disclose that
  the approved host `e891f34c`/`d75ab391` silently skips a malformed or
  non-32-byte `share_hash`, so such a record can still count as `consistent`.
  Missing hashes remain `unknown`. F1's `malformed_share_hashes` /
  `inconsistent` fix is accepted independently in the topology packet but is
  pending merge and a new combined approval; the approved host is not claimed
  fully fail-closed.
- **D3 fixed.** `docs/decisions/2026-11-02-share-target-policy.md` now carries
  the prescribed "Superseded in part (2026-11-05)" banner pointing to the two
  v2 decisions and the LP-1 integration record. Its historical body/date and
  v1 policy are unchanged. The `REGTEST-LAB-IMPLEMENTATION.md` "applied"
  statement is therefore true within this packet.
- **Nits fixed.** SPEC `updated: 2026-11-06`; the OPERATIONS authority-secret
  cell says inline TOML, mode 0600 operator convention not enforced; SPEC's
  `pow_limit_bits` is the pinned `0x207fffff`, not a dynamic consensus query.
  The pre-existing absence of a generic `RequestExtensions` handler is now
  scoped in the type-32 deltas instead of being invented.

## 4. Operator guide scope

`docs/LAB-RUNTIME.md` documents the actual approved CLI and browser/headless
surface:

- `spc-lab runtime` with explicit `--node`, `--pool`, `--miner`, `--scanner`,
  storage binaries, `--base-revision` and `--dirty-tree-hash`;
- Nix-built `spc-lab`/`spc-pool`/`spc-cpu-miner`/`spc-scan` store paths, with
  the pinned L2 rbitcoin binary as an explicit non-Nix exception;
- fresh private directories (`private/` 0700, files 0600), logs and shareable
  artifacts;
- browser `/catalog` and durable `add_miner`, `switch_pool`,
  `set_worker_budget`, `pause_workers`, `resume_workers`, `stop_run`;
- snapshot/events/manifest/command/reap inspection and clean-stop behavior;
- the scenario assertions, `accounted_target_consistency` scope, tip-sampler
  (not full canonical coverage) and scanner retry semantics;
- explicit non-claims: no causal hash-budget/retarget response, no full
  header/payout oracle or DB credit-sum verification in the runtime, no
  fee/witness/Ark/ladder/SV1/general-interoperability claim, no namespace
  placement, and only `sv2_tp_bootstrap` 6/6 on the node side with L2
  CLEARED_WITH_RECORDED_BASELINE_LIMITATIONS.

The namespace work is linked as a pending handoff, not given a fictional CLI.

## 5. Checks

- SPEC frontmatter parses as YAML with 26 changelog entries; the approved
  LP-1 fragments are present and no lab tool entry was duplicated.
- Relative links in the edited/new documents resolve, except three expected
  merge inputs in `docs/REGTEST-LAB-IMPLEMENTATION.md` that exist only on main
  (`REGTEST-LAB.md`, `REGTEST-LAB-STATUS.md`, the review summary). They are
  recorded as expected-missing, not silently ignored.
- No U+2010/U+2011/U+2012 lookalike punctuation was introduced by the edited
  prose or the new guide; the repository's pre-existing em dashes are
  unchanged.
- Command examples were checked against the approved runtime CLI/static and
  the approved LP-live implementation report. No Rust build is required for
  this packet; no production binary was rebuilt.
- D1 checked against `Store::bootstrap` (`postgres.rs:1151-1154`) and the
  front bootstrap consumer: `type32_blobs` is the identity-free export of every
  `window_tail` row, while `Type32Store` inserts only v1 slices. D2 checked
  against approved `runtime.rs`: malformed/non-32-byte hashes are skipped and
  can remain `consistent`; missing hashes are `unknown`. D3 is the prescribed
  banner only and leaves the 2026-11-02 body/date and v1 policy unchanged.
- The existing LP-live driver self-test (17/17) and scenario validation were
  already recorded by the approved runtime packet; this packet does not rerun
  or alter them.

## 6. Remaining namespace/operator handoff

The following are explicitly pending and are **not** solved by documentation:

- namespace placement, address publication, controller RPC/stdio transport and
  the PostgreSQL-userns/daemonising limitations (LP-4). The currently reviewed
  helpers are a precursor: the host runtime does not invoke them, the existing
  two-node artifact used RPC `generate` plus shell `nsenter` rather than native
  SV2/CPU mining and the Rust supervisor handshake, and the storage boundary
  (Unix Redis socket versus namespace TCP) is undecided. Host loopback is not a
  cross-namespace promise. No namespace CLI or operator mode is claimed here;
- F1 malformed share-hash handling (topology owner, separate code fix). In the
  approved host `e891f34c`/`d75ab391` a malformed or non-32-byte `share_hash`
  is skipped and the record can still count as `consistent`; F1's
  `malformed_share_hashes`/`inconsistent` fix is accepted independently but
  pending merge and a new combined approval. A `consistent` result from the
  approved host must not be read as a hash check over every record. Missing
  hashes remain `unknown`; the approved host is not fully fail-closed here;
- network-tool Nix outputs externally blocked; absence is not a core package
  failure;
- causal retarget/hash-budget response, full-window/payout/header oracle,
  fee/witness coverage, ladder/JD payout convention, SV1, Ark settlement and
  general SV2 interoperability.

The docs packet prepares clear merge hooks for the namespace owner but does not
claim their implementation.

## 7. Merge delta

Apply only the owned documentation delta above; the copied approved docs are
the already-reviewed inputs and should not be re-authored. Merge order and
baseline hashes are in `/tmp/opencode/lab-docs-baseline.json`. The final
documentation manifest is `/tmp/opencode/lab-docs-review-manifest.json`.
