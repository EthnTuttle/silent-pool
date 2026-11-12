# LP-1 share-policy integration implementation

Date: 2026-11-05
State: uncommitted revised candidate for Fable re-review. No other worktree or
main-root source was edited, and no commit was made.

## Approved baseline

Base Git HEAD: `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.
The worktree was populated from all 38 approved files and verified byte-for-byte
before new LP-1 semantics:

- L1: 26 files, `fa532607fd20ce8f9dabcae1f552973e3c45c12b99ab7be1eb2f148e36056cd2`.
- LP1a: 5 files, `45df0eb346787828e4b58a2fec229e1fce82ded72d1730ce011e82463696379c`.
- L3: 7 files, `3edcd4e744149667b184f44bdf3833ba465e210671eb90912fb1db9b9b11ba25`.

Combined baseline: `/tmp/opencode/lab-lp1-baseline.json`,
`ed5a40f1629b23c58c0c23fc9618a023d869d33d05454a08c17eb1c3241bc16a`.
The previous review snapshots were
`81fa17d27ee80641167aaeb53e97f627ba8daa4688e56a227a1ef44c9776b329` and
`69ac72b6ed7ab514f1870cfc55832d8c8d202576a9c01bc27aaeb72d2a20da6d`.
The r2 snapshot resolved F1-F10. This bounded r3 revision addresses only the
new R2-1/R2-2 retention findings; it does not alter the approved input
manifests or the verified LP-1 core behavior.

## Delivered v2 behavior (retained from the prior checkpoint)

`SharePolicy` and immutable `JobWork` bind generation, nbits, exact target,
credited `m_issue`, version and reference fingerprint. V2 uses the approved
LP1a exact target/multiplier arithmetic, O(1) stepdown/up, checked slice/window/
payout/JD products, Q(T), and frozen settlement metadata. V1 keeps its integer
difficulty, N*d, saturating arithmetic, target behavior and retention policy.
SV2 target activation and the stable Noise authority/pinning and secret
redaction from L1 remain intact.

V2 remains regtest-only and rejects legacy `share_difficulty`, SV1,
`extensions.share_accounting = true`, invalid profile/rule/cap combinations and
nonzero/zero-sentinel ambiguity. Snapshot, slice and Bootstrap metadata carry
version/fingerprint/window rule; mismatched restores fail without conversion or
zero-fill. `spc:slices` appends version/fingerprint and `spc:blocks` appends
those plus rule. Migration remains `0005_lab_work_units.sql`; `sv2-tp` and
`ladder` still end at 0004 when re-checked before this report.

## Fable REQUEST_CHANGES dispositions

### F1 - actual type-32 refusal

`Sv2Server::with_type32_enabled` is wired from
`extensions.share_accounting`. Disabled type-32 requests are decoded only to
recover their request id, then answered with
`Type32Message::Error { code: "unsupported-extension" }`; no GetWindow,
GetShares or GetTransactionsInJob service path runs. Main passes the same flag
to the front. `roll_slice` additionally refuses every v2 slice regardless of
that flag, and only enabled v1 slices can enter `Type32Store`. HTTP capability
filtering follows the same flag but is not the enforcement boundary.

The main-process v2 peer sends all three request types and checks exact error
codes/request ids. The legacy `tests/e2e_type32.rs` was not edited and still
passes with GetWindowSuccess/proof verification.

### F2 - refresh failure atomicity

Deterministic work/Q planning is hoisted before roll/key/height/open/replay
transitions, removing the duplicate later `job_work` call. Every other error
from `Front::refresh`, including errors after those mutations, triggers
`invalidate_current_work`: job, policy, template, distribution, weights and
routing become unavailable. `main` clears the current SV1/SV2 generation on the
same error, so ingress cannot continue presenting old work while the front
rejects it. There is no secret-key clone or event-history rollback.

The regression reproduces probe 7's chain move/profile failure: the old
generation is removed, its old share gets `NoJob`, and no new-height slice can
receive it. Fable's unchanged old probe now fails at its former
`front.job().unwrap()` counterexample assertion, as expected.

### F3 - actual regtest pow limit

V2 requires exactly `pow_limit_bits = 0x207fffff`, Bitcoin regtest's consensus
pow limit. `0x1d00ffff` is not an operator-selectable effective authority. The
regression uses R = 2^254 - 1 with that wrong override and checks the named
config rejection. V1 `/api/pool.work_profile.pow_limit_bits` is null rather than
incorrectly attributing the regtest value to another network.

### F4/F5 - checked payout restoration and definite wire tables

Payout `slice_agg` keeps v1 saturation but dispatches v2 stored/credit work and
fee sums through checked helpers. V2 stored difficulty must equal the credit
sum; disagreement gets `ReconstructError::StoredDifficultyMismatch` with the
slice id and both sums. Synthetic mismatch and overflow regressions pass.

WIRE-FORMATS sections 3.1 and 3.6 now contain the actual appended fields in
their byte tables: version codes 1/2, v1 zero fingerprint sentinel/v2 nonzero
32-byte value, block rule codes 0/1, and `window_target` as N*d for rule 0 or
Q(T) in v2 units for rule 1.

### F6/F7 - fingerprint shape and complete oracle joins

Both SQL fingerprint columns require NULL or exactly 32 nonzero bytes, in
addition to the version/null and version/rule CHECKs. The real Postgres suite
exercises 31-byte and 32-zero-byte writes against both slice and snapshot
columns.

`accounted` now also includes:

- `share_hash`: hex of the raw SHA256d digest byte array, explicitly marked as
  not reversed RPC display order;
- `extranonce`: the full pool-prefix-then-miner extranonce in coinbase order;
- `share_target_byte_order`: big-endian unsigned 256-bit target;
- `share_hash_byte_order` and `extranonce_byte_order` describing those joins.

These are private SPc/front evidence only. No identity field was added to Ark
orders, receipts or streams. The main-process test requires hash and extranonce
evidence, and gives its child an explicit RUST_LOG so Nix's empty-RUST_LOG test
environment cannot hide those records.

### F8/F9 - exact metrics and error names

The metric test now creates its own migrated database, writes one fixture, and
asserts exact results: one confirmed block, one scheduled Ark payout, 250 due
sats and 40 forfeited sats. It drops that database afterward. There are no
`>= 1` assertions and no concurrency-unsafe global deltas.

`raw_payouts_v2` names the actual offending slice for profile mismatch, uses
`PayoutWindowEmpty` for its empty-window error, and maps per-identity payout fee
aggregation to `PayoutProductOverflow` rather than a slice-local error.

### F10 - bounded v2 retention amendment

The original 16-slice truncation affected both roll and Bootstrap. V1 remains
unchanged. The r2 amendment retained enough work for the current Q up to 4096
slices, and disclosed a 4096-height durable-v2 horizon. R2 found two flaws in
that new amendment: durable deletion selected the active payout's configured
version, and live RAM was trimmed against current/previous Q. R3 corrects both
without broadening this packet.

## R2 focused review dispositions

### R2-1 - stored row versions select both durable horizons

`ScrubConfig` has no work-unit-version selector. `Store::scrub` now takes both
horizons and gates deletion in SQL on each stored row:
`(v1 AND height < legacy_cutoff) OR (v2 AND height < v2_cutoff)`. Both cutoffs
use the same tip and are strict `<` comparisons. The v1 calculation remains the
legacy configured retain/confirmation/window guard; the v2 horizon is the
shared 4096-height constant. Consequently, a v1-configured payout cannot erase
v2 credits early, and a v2-configured payout cannot retain v1 attribution past
its legacy horizon. Unknown versions are not deleted.

The mixed-version real-Postgres fixture places v1 and v2 rows at 5903, 5904,
6000, 9983 and 9984 beneath tip 10000 (cutoffs 5904 and 9984). Repeated scrubs
with identical inputs produce identical survivors: v2 at 5904/6000/9983/9984,
only the legacy-boundary v1 at 9984. This covers both active-profile failure
modes, exact boundary eligibility and idempotence. The decision/changelog now
disclose the intentional v2-only SPc attribution-retention extension; no new
identity data crosses the Ark boundary, and v1 retention is not extended.

Failure-first evidence is `LP1-r3-red-mixed-scrub.log`; the completed mixed
row/boundary suite is in `LP1-r3-db-tests.log`.

### R2-2 - Q-independent live history preserves future-window headroom

Live v2 retention no longer sums work against current or previous Q. It keeps
every eligible slice up to 4096 and applies exactly the durable credit cutoff:
`slice.height >= template.height - 4096`. `window_target` is assigned before
the roll, but retention does not depend on that assignment. Eligibility/cap
pruning and current-Q window selection are separate: headroom is kept for a
future harder window, and underfill is still derived from the selected work.

New red/green regressions prove:

- 70 rolling d=1 slices at Q9 retain >=65 units and become full at Q65, rather
  than being cut to 16 (`LP1-r3-red-roll-retarget.log`,
  `LP1-r3-green-roll-retarget.log`);
- the same Q9 -> Q65 Bootstrap retarget is full
  (`LP1-r3-red-bootstrap-retarget.log`,
  `LP1-r3-green-bootstrap-retarget.log`);
- at tip 5000, 70 slices at height 100 are ineligible while one at 4999 remains,
  yielding truthful underfill rather than credit for SQL-expired history
  (`LP1-r3-red-age.log`, `LP1-r3-green-age.log`);
- 4097 rows are capped at exactly 4096, and selecting 4097 units reports
  underfill, proving a real bound rather than unbounded retention
  (`LP1-r3-red-cap.log`, `LP1-r3-green-cap.log`).

The independent r2 probe 10 now observes **70 retained at Q9, then 71 slices / 71 work at Q65 with `underfilled false`**. Its old
`assert!(window_underfilled())` counterexample assertion therefore fails, as
required. The same run still contains 7 passing older probes and four failing
outdated expectations: old exact-33-slice assertions in probes 9 (now 40 kept),
the old probe-10 defect assertion, and a restore-usage assertion for a row at
height 1 which is intentionally age-expired at tip 800000. Actual output is in
`LP1-r3-fable-probes.log`; this is not represented as an all-green legacy-probe
run. No matching-restore or work-unit validation logic was removed.

## LP2/LP-live configuration and evidence contract

The native CPU miner's approved snapshot (`5e27e918...1250`) and the separately
dispatched `lab-live-e2e` harness are integration inputs only. Their sources
were not read, copied or modified for this rework. The reusable real-rbitcoin
+ Postgres + Redis + two-miner proof remains a separate canonical gate and is
not claimed here.

The effective v2 profile is:

- `[pool] network = "regtest"`; native SV2 only (`[sv1] enabled = false`);
- `[extensions] share_accounting = false`;
- `[accounting] work_unit_version = 2`, `window_rule = "network_block_work"`
  (optional but only that value is accepted), `window_n > 0`.
- `work_reference_target` is required 32-byte big-endian hex for the exact R
  family; `share_work_multiplier` is the required power-of-two issuance value.
- `max_work_multiplier <= 2^32`, decimal-string
  `max_window_target_units <= 2^64`, `max_slice_shares <= 65000`, and exactly
  `pow_limit_bits = 0x207fffff`.
- Legacy `share_difficulty` is forbidden with v2. V2 live history uses the fixed
  4096-slice/4096-height bounds above, not current Q or a payout profile flag.

`job_issued` on target `spc_evidence` carries front/channel/job ids,
`generation` (decimal string), nbits, big-endian `share_target` and its explicit
byte-order label, `m_issue`, version/fingerprint, issuance time, activation and
prevhash. `accounted` retains all common fields and adds private-fixture
`sp_id`, exact booked `d`, nonce/ntime/version/share_index, raw-digest
`share_hash` (explicitly not reversed RPC display order), and the full
pool-prefix-then-miner `extranonce` in coinbase byte order. `share_hash <=
share_target` is the integer validation relation; no display-string comparison
or rounding is involved. `share_outcome_unknown` has front/channel/job/sequence
and reason; multiplier step events retain their immutable generation metadata.
None of these identity-bearing records enters Ark orders, receipts or streams.

## Verification

All referenced commands finished. Nix dev-shell commands used
`CARGO_BUILD_JOBS=2`; the Nix flake build additionally used `--cores 2`.

| Check | Completed outcome and log |
|---|---|
| `cargo test -p spc-pool --lib --test main_sv2_pipeline --test sv2_conformance --test e2e_type32` | **224 + 1 + 4 + 1 = 230 passed, 0 failed**. Includes F2's failed-refresh guard, all new live-retention retarget/cap/age regressions, the v2 type-32 refusal and unchanged v1 type-32 (`/tmp/opencode/LP1-r3-targeted-final.log`). |
| `cargo test --workspace` | **494 passed, 0 failed**, including 65 accounting and 224 pool library tests (`/tmp/opencode/LP1-r3-workspace-final.log`). |
| Fresh real Postgres: `cargo test -p spc-store --test postgres --test work_units` | **20 + 2 passed, 0 failed**. Includes exact isolated metrics, fingerprint CHECKs, and mixed stored-version scrub boundaries (`/tmp/opencode/LP1-r3-db-tests.log`). |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed (`/tmp/opencode/LP1-r3-clippy.log`). |
| `cargo fmt --all -- --check` | Passed (`/tmp/opencode/LP1-r3-fmt-check.log`). |
| `python3 scripts/check-lab-docs.py` | `OK: 6 files, ASCII clean, links resolve, vectors pass`; only expected merge-input notices (`/tmp/opencode/LP1-r3-doc-check.log`). |
| Legacy `testing/regtest-e2e.sh` | Real legacy SV1/Core smoke passed (`/tmp/opencode/LP1-r3-sv1-regtest.log`). This is not v2 canonical-node proof. |
| Combined Nix flake/vendor gate | All five checks passed, including offline workspace and rpc,ark tests, on the exact source copy (`/tmp/opencode/LP1-r3-flake-check.log`). |

The exact uncommitted code tree was copied to a temporary path for the flake
build because Nix's Git input ingestion omits untracked files. Source path is
recorded in `/tmp/opencode/LP1-r3-flake-source-path.txt`; as in r2, the report
itself is finalized after the build log. The successful r2
`fetchCargoVendor` hash remains valid because r3 changed no dependency or
lockfile. No `git add`, commit, staging mutation, upstream pin or dependency
version change occurred.

The previous F1-F10 dispositions above remain in force. In particular, refresh
failures invalidate front/ingress work, v2 regtest pow limit stays exact,
payout sums are checked, type-32 stays refused for v2, and evidence remains SPc
private. No canonical v2 block acceptance, live LP2 runner result or
lab-live-e2e result is inferred from any mock-template test.

## Exact change inventory and manifest

The final inventory contains **57 modified/untracked files** in the overall
HEAD diff: 34 modified and 23 untracked. Relative to the approved combined
38-file input, it is **28 unchanged, 10 modified and 19 added**. Approved-file
hash comparisons are retained per row in the manifests rather than inferred
from directory names.

- Final review manifest: `/tmp/opencode/lab-lp1-review-manifest.json`.
- Final per-file hashes: `/tmp/opencode/lab-lp1-final-file-hashes.json`.
- Combined baseline metadata and each row's `baseline_sha256` are preserved.
- Aggregate recipe: exact `base` plus only path/SHA256 pairs, `sort_keys=true`,
  compact separators, excluding annotations and summary fields.

## Remaining limits and integration gates

1. **No v2 type-32 claim.** It is refused at ingress and omitted from the store;
   a later documented job-generation/oracle mapping must precede support.
2. **No canonical v2 node-acceptance claim.** The main test runs the real main
   front, Noise/SV2 protocol and genuine SHA256d non-block/block-qualified
   shares at m=2, but uses `template.source = "mock"`. Canonical Core/rbitcoin
   acceptance, v2 Redis/Postgres settlement, live retargets, TDP/ladder and
   multi-transaction templates remain separate gates. The legacy SV1 Core smoke
   does not establish v2 SV2 node acceptance.
3. **Bounded timeout state remains unknown.** Inline front waits remain capped
   at 10 seconds; timeout/lost outcome emits `share_outcome_unknown` and closes
   the stream rather than asserting definitive invalid/accepted state. LP2/LP3
   reconciliation must handle it out of line.
4. **Profile and retention claims are bounded.** m <= 2^32, Q <= 2^64,
   max_slice_shares <= 65000, 4096 live slices and 4096 durable heights are the
   implemented scope. Underfilled warmup or exhausted bounded history is
   truthful; it is not full N-network-block coverage. Multi-front restore is not
   claimed.
5. **Merge work is explicit.** Apply the L1/LP1a/L3 fragments, then the LP-1
   fragment. Reconcile SPEC 3.4, 4.1-4.4, 5.1, 5.3 and 7.4 with frozen Q,
   fail-closed refresh and the bounded v2 retention amendment. OPERATIONS needs
   profile/deploy/retention restrictions and authority-secret storage;
   share-accounting-ext-deltas needs the v2 source note before any support.
   Apply TDP/ladder separately with L1's deepest-first Merkle behavior and
   re-check migration allocation at merge.
6. **No LP2/LP3 dependency was absorbed.** Their stable handoff is the precise
   protocol and evidence contract above; no source from their worktrees was
   copied. Combined Nix and production-node gates still apply after merge.

All work remains uncommitted in `lab-share-integration`. Approved L1, LP1a and
L3 trees remain frozen.
