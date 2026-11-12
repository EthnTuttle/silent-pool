# L3 lab contracts: implementation report

Date: 2026-11-05
Status: revised candidate for Fable re-review. Uncommitted work in
`/home/ethan/.local/share/opencode/worktree/ad8886/lab-contracts`, base
`b0ab4ca34fda07c61a6335d05f598eaacc69678c`.

This revision responds to Fable's second-pass REQUEST_CHANGES on snapshot
`a53e9a29611df3d79aac01d12272b853a18998441f943ba764c7c014bbeeff2f`
(`docs/reviews/2026-11-05-lab-l3-fable-review.md` in the main worktree); the
first pass was on `6a2a5967113417e6ed8c348f11a6498c98e24685b21f2548f0d34b62b0f935f0`.
The same seven candidate files are revised; no files were added to the packet.
No commit, push, merge, reset or stash was performed, and no main-worktree file
was edited.

## Scope delivered

This packet owns documentation and decisions only. It does not add a miner,
runner, node profile or browser, and it does not claim that the proposed tests
currently exist. The deliverable is a concrete implementation contract plus
two narrowly scoped, dated policy proposals.

Inputs used: the frozen proposal, the five-model synthesis (including its
corrections), the status board, the merge queue, the existing share-target and
settlement-target decisions, the Fable review, and source inspection of the
current pool. Raw reviewer records were not treated as standalone requirements;
their correction sections were read with the original findings.

## Decisions

### D1 - Reference-target work units (v2)

File: `docs/decisions/2026-11-05-lab-share-work-units.md`.

- Adds an opt-in v2 work profile restricted to the power-of-two family:
  `R + 1 = 2^k` (`1 <= k <= 255`) and `m = 2^j` (`0 <= j <= k - 1`).
- Profile validity requires `R >= effective pow limit`, rejects a non-power
  `R + 1` with `reference_not_power_of_two`, and rejects a non-power/invalid
  `m_cfg` with `multiplier_not_in_family`. The recommended regtest reference is
  `R = 2^255 - 1` (`k = 255`, `E(R) = 2`), which covers the `0x207fffff` pow
  limit. `R = 2^254 - 1` is retained as a negative `pow_limit` vector.
- `S(2^j) = 2^(k-j) - 1`, so `E(S)/E(R) = 2^j` with
  `E(T) = 2^256/(T + 1)`. Credit is `d = m_issue`.
- The configured `m_cfg = 2^j_cfg` is a ceiling. Issuance is O(1):
  `j_issue = min(j_cfg, k - ceil_log2(T+1))`, with exact-power-boundary
  handling; it records `multiplier_stepdown`/`multiplier_stepup`, leaves old
  generations untouched, and is not a clamp. There is no trial-divisor loop.
- Rejects zero, overflow, non-family targets/multipliers,
  `m_cfg > max_work_multiplier`, and targets harder than the network target
  with no feasible member (defensive only).
- `max_work_multiplier <= 2^32`; `max_window_target_units <= 2^64`; both are
  hard caps. Per-slice `sum(d)` and `sum(d*f)` have stated sufficient widths;
  v2 checks every accounting intermediate and uses exact wider multiply/divide
  for SPEC section 4.4 products. There is no universal `u128` claim for the
  window-level fee aggregates.
- Compact decode follows consensus `SetCompact`, including valid exponents
  33/34; no exponent-32 cutoff.
- Credits the job's recorded target, not the target current at submission.
  `job_issued` is the oracle's `job_id -> (share_target, m_issue)` source.
- Persists work-unit version/fingerprint on slices and distributions, with
  definite wire fields and DB CHECKs; restore rejects mixed or mismatched
  profiles. v2 type-32 auditing is explicitly out of scope and not claimed.
- Freezes v1 exactly; v1 keeps saturating arithmetic. No v1-to-v2 conversion.

### D2 - Window is network-block work (v2)

File: `docs/decisions/2026-11-05-window-network-block-work.md`.

- Confirms that the intended SLICE/TIDES window is network-block work, not
  `N` share submissions at the pool target.
- Defines `Q(T) = ceil(N * (R + 1)/(T + 1))` in v2 reference units. With
  `R = 2^255 - 1` and `N = 8`, `Q(0x200fffff) = 65` (floor 64) and
  `Q(0x207fffff) = 9` (floor 8). The ceiling error is below `E(R) = 2`;
  whole-slice overshoot is reported.
- `underfilled`, `actual_sum` and `overshoot` are derived from `high_water`
  and `window_target`; they are not new columns.
- Persists `window_rule` with definite codes: `0 = share_work_legacy`,
  `1 = network_block_work`, used by both the DB `SMALLINT` and the wire `U8`.
- Settlement uses the frozen row, never current bits. `Q` above the hard cap
  `max_window_target_units <= 2^64` fails with `window_target_too_large`;
  checked window sums fail with `window_work_overflow`.
- v1 keeps the named `share_work_legacy` `N * d` rule.

## Main artifact

`docs/REGTEST-LAB-IMPLEMENTATION.md` contains:

- scope, non-goals and in-flight L1/L2/`sv2-tp`/`ladder` inputs;
- ten lab invariants;
- versioned scenario and distinct entity types (network/node/pool/front/
  identity/worker/miner/connection);
- work-unit and window implementation checklist, including profile validity,
  stepdown/step-up, checked accounting and definite wire fields;
- native SV2 CPU miner contract (job/target state, real hash budgets, measured
  rates, submission correlation, reconnect);
- durable command IDs and outcome states;
- evidence definitions: `job_issued`, submitted, ingress ack/reject,
  accounted/reject, candidate, `canonical_at(node, hash, anchored tip, time)`,
  settled, delivered, unknown; no absence-equals-invalid;
- oracle inputs and generation correlation, with the v2 type-32 gap stated;
- reorg re-evaluation and historical evidence retention;
- observer cursors, nonblocking loss/gap behavior, snapshot/SSE/resync;
- evidence versus chart decimation and artifact privacy/retention rules;
- process adoption/reap, run manifest and no-kill-by-PID;
- unique per-pool Redis instances, Postgres databases and ports;
- a concrete netns/veth/nftables sustained link-fault backend with no-bypass
  preflight, reachable listeners, Ark stub links and an unprivileged
  user-namespace capability probe;
- capability, witness, halving, TDP and height-progress gates with pending L2
  semantic introspection fields;
- time policy, warmup and statistics declarations;
- five bounded packets with owned paths, dependencies, acceptance commands and
  Fable gates.

## Normative deltas proposed

No `docs/SPEC.md` frontmatter, `docs/STATUS.md`, config or source file was
edited. The following amendments are proposed for the merge session:

- `docs/SPEC.md` sections named in each decision: 3.4, 4.1, 4.2, 4.3, 4.4,
  5.1, 5.3, 7.4.
- `docs/WIRE-FORMATS.md` sections 3.1/3.6: definite `spc:slices` and
  `spc:blocks` fields, the v1 zero-fingerprint sentinel and the `window_rule`
  enum codes.
- `docs/share-accounting-ext-deltas.md`: merge note that v2 type-32 auditing
  needs a `job_id -> (share_target, m_issue)` source; not claimed here.
- Changelog fragments:
  - `docs/changelog/2026-11-05-lab-share-work-units.md`
  - `docs/changelog/2026-11-05-window-network-block-work.md`

The v1 policy is explicitly frozen, so this is not a silent retroactive change
to existing rows or distributions.

## Validation

Run from the worktree:

```sh
git diff --check
python3 scripts/check-lab-docs.py
LC_ALL=C grep -nP '[^\x00-\x7F]' docs/REGTEST-LAB-IMPLEMENTATION.md docs/decisions/2026-11-05-lab-share-work-units.md docs/decisions/2026-11-05-window-network-block-work.md docs/changelog/2026-11-05-lab-share-work-units.md docs/changelog/2026-11-05-window-network-block-work.md docs/reviews/2026-11-05-lab-l3-contracts.md scripts/check-lab-docs.py
grep -nE ' +$' docs/REGTEST-LAB-IMPLEMENTATION.md docs/decisions/2026-11-05-lab-share-work-units.md docs/decisions/2026-11-05-window-network-block-work.md docs/changelog/2026-11-05-lab-share-work-units.md docs/changelog/2026-11-05-window-network-block-work.md docs/reviews/2026-11-05-lab-l3-contracts.md scripts/check-lab-docs.py
```

`scripts/check-lab-docs.py` checks ASCII, relative links and independent
integer/Fraction vectors: the consensus compact decoder, the power-of-two
family for `R = 2^255 - 1` (`m = 1,2,4,8,16`), non-power rejection, pow-limit
validity, the O(1) `m_cfg = 8` stepdown to 1 and exact-power boundaries,
`Q = 65`/`9` with floor `64`/`8`, the `< E(R)` ceiling error, both
fingerprints, the stated per-slice widths, and prose anchors for the chosen
vectors and named errors.

Observed outcome:

```text
EXPECTED: docs/REGTEST-LAB-IMPLEMENTATION.md: expected merge input REGTEST-LAB.md
EXPECTED: docs/REGTEST-LAB-IMPLEMENTATION.md: expected merge input reviews/2026-11-05-regtest-lab-review-summary.md
EXPECTED: docs/REGTEST-LAB-IMPLEMENTATION.md: expected merge input REGTEST-LAB-STATUS.md
OK: 6 files, ASCII clean, links resolve, vectors pass
```

`git diff --check` exits 0 for the tracked tree; the seven untracked files were
also checked directly for trailing whitespace and non-ASCII characters. The
main-worktree proposal, status board and review synthesis are expected to be
absent at this base and are reported as merge inputs, not failures.

No Rust tests were run. This packet did not change Rust code, existing tests,
`SPEC.md`, `STATUS.md` or the frozen proposal/review records. The acceptance
commands in `REGTEST-LAB-IMPLEMENTATION.md` are contracts for the later packets,
not executed evidence for this one.

## Fable finding dispositions

| Finding | Disposition | Where |
|---|---|---|
| B1: family cannot issue at pow limit; missing stepdown | **Taken.** `R = 2^255 - 1`; profile validity `R >= T_powlimit` with `reference_below_pow_limit`; O(1) power-of-two stepdown and step-up at a new generation; old `R` is a negative vector; `Q` vectors now 65/9 (floor 64/8). | Work-unit decision sections 3-4, tests 1-3; implementation sections 4.1-4.3; window decision vectors. |
| B2: false `sum(d*f)` overflow claim; SPEC 4.2/4.4 not gated | **Taken.** `m_cfg <= 2^32`, `Q <= 2^64`; per-slice widths stated as sufficient limits, not universal; v2 checks `sum(d)`, `d*f`, `diff_fee_sum`, `D_win`, every 4.4 product and final conversions with named errors; exact wider mul/div for fee products; v1 saturation frozen; SPEC 4.2/4.4 added to edits and tests. | Work-unit decision section 5, tests 9/11, edits; implementation sections 4.2/4.5. |
| B3: compact decode cutoff | **Taken.** Consensus `SetCompact` rule, including exponent 33/34 and overflow/sign/zero vectors; no exponent-32 cutoff; node owns pow-limit enforcement. | Work-unit decision section 5.1, test 5; implementation section 4.2. |
| B4: no oracle source for `job_id -> target` | **Taken.** `job_issued` evidence record with generation/target/m; `accounted` carries job/generation; oracle input list; v2 type-32 audit explicitly out of scope with a merge note; LP-1/LP-3 oracle tests. | Implementation sections 4.3, 7.2.1; work-unit decision section 6; LP-1/LP-3 acceptance. |
| 5: cite `SetTarget` and future-job rule | **Taken.** Cites pinned sv2-spec `d8bf5b8` section 5.3.21; future job target is the channel target at `SetNewPrevHash` activation; no invented extension. | Work-unit decision section 6.2; implementation sections 4.3.6 and 5.3. |
| 6: LP-1 overlaps L1 files | **Taken.** LP-1 is dispatched after L1 for `sv2_server.rs` and target-lifecycle `front.rs`; announced-target plumbing is an L1 merge note; owned paths adjusted. | Implementation LP-1; report merge notes. |
| 7: underfill/overshoot storage | **Taken.** Derived from `high_water` and `window_target`; recorded in lab evidence/metrics, not columns; missing slices yield `unknown`. | Window decision sections 2/5; implementation section 4.4. |
| 8: wire change hedged | **Taken.** Definite fields and order: `spc:slices` version/fingerprint; `spc:blocks` version/fingerprint/rule; codes 0/1; v1 zero sentinel; DB CHECKs; atomic deploy; old events fail decode. | Work-unit decision section 7.5; window decision section 3.3; implementation section 4.4; WIRE merge note. |
| 9: multiplier name mismatch | **Taken.** Canonical name is `share_work_multiplier`; scenario example updated; `m_cfg` is the configured ceiling, `m_issue` the generation's actual credit. | Implementation scenario and sections 4.1-4.3; work-unit decision section 8. |
| 10: fixed-point/float claims | **Taken.** Fixed point is described as deterministic but biased/needing an error-bound contract; float objections are binary64/JS precision and cross-language rounding, not platform non-determinism. | Work-unit decision section 4, Why, Alternatives. |
| 11: artifact privacy | **Taken.** Shareable manifest vs identity-bearing fixture material; no `a_send`/shared secrets/tweaks/Ark secrets; Ark stream boundary; redaction is not production privacy evidence. | Implementation section 7.7, LP-3 acceptance. |
| 12: unprivileged netns and flake inputs | **Taken.** `unshare -Urn` capability probe, veth/nft/tc, `sch_netem`, relay downgrade; LP-4 merge note adds `iproute2`, `nftables`, `util-linux`. | Implementation sections 8.3 and LP-4. |
| 13: hardening | **Taken.** DB `CHECK ((work_unit_version = 1) = (work_unit_fingerprint IS NULL))`, enum checks, chosen and old fingerprint vectors. | Work-unit decision sections 7.1-7.2; checker vectors. |
| R1: unbounded issuance search | **Taken via option (a).** v2 is restricted to `R + 1 = 2^k`, `1 <= k <= 255`, and `m_cfg = 2^j_cfg`; `j_issue = min(j_cfg, k - ceil_log2(T+1))` is O(1) bit-length logic, no divisor scan. Preflight rejects non-power `R + 1` (`reference_not_power_of_two`) and non-power/out-of-range `m_cfg` (`multiplier_not_in_family`); tests cover `m_cfg = 3`, non-power `R + 1`, and exact power boundaries. | Work-unit decision sections 3-4, 5.6, 8, tests; implementation sections 4.1-4.2; checker vectors. |
| N1: `max_window_target_units` cap wording | **Taken.** It is a hard cap `<= 2^64` in both decisions; a v2 config rejects a larger value, and `Q` above it fails with `window_target_too_large`. | Work-unit decision sections 5.5/8; window decision section 5; changelog. |
| N2: stray `window_target_version` | **Taken.** Removed; the only persisted discriminators are `work_unit_version` and `window_rule`, plus the work-unit fingerprint. | Window decision section 5. |

No finding is deferred. Items 5-13 and R1/N1/N2 are folded into the same
revision rather than left as optional follow-ups.

## Potential shared-file merges

- `docs/SPEC.md` and its changelog: merge-session-owned; fragments only.
- `docs/WIRE-FORMATS.md`: definite event fields and enum, coordinate with L1
  and the settlement-target decision.
- `docs/share-accounting-ext-deltas.md`: v2 type-32 mapping note; explicitly
  not a v2 claim in this packet.
- `docs/REGTEST-LAB-STATUS.md`: add L3 review outcome after Fable.
- `docs/MERGE-QUEUE.md`: update the `lab-contracts` row.
- `docs/OPERATIONS.md`: optional later lab operator section; not edited.
- `config/spc-pool.example.toml`, `bins/spc-pool/src/config.rs`,
  `bins/spc-pool/src/main.rs`, `Cargo.toml`, `Cargo.lock`, `flake.nix`: future
  LP-1/LP-2/LP-3/LP-4 merge notes, not edited here.
- L1 owns `sv2_server.rs`/session/Merkle changes. LP-1 is sequenced after L1
  for `sv2_server.rs` and target-lifecycle `front.rs`; announced-target
  plumbing is an L1 merge note.
- L2 owns node flags/profile introspection; this document names semantic fields
  only and does not assume current flag names.
- The `sv2-tp` and `ladder` branches are inputs. Do not overwrite their files
  or re-derive their behavior in the lab.

## Review focus and known gaps

Fable should check the revised B1-B4 lines specifically:

1. Does `R = 2^255 - 1` plus `R >= T_powlimit` cover genesis, retarget clamps
   and minimum difficulty, with the stepdown/step-up rule unambiguous?
2. Are all SPEC section 4.2/4.4 intermediates checked with named failures, and
   is the absence of a universal window-level `u128` bound stated honestly?
3. Is the consensus compact decoder exact, including exponents 33/34?
4. Is `job_issued` sufficient for the oracle, and is the v2 type-32 scope
   explicit rather than silent?
5. Are the definite wire fields/DB constraints and LP-1-after-L1 sequencing
   implementable without a parallel-write conflict?
6. Is R1 closed by the power-of-two restriction, with no residual divisor
   search and exact-power-boundary handling, and are N1/N2 gone?

Explicitly not done: SV2 conformance fixes, rbitcoin retarget profile, CPU
miner, supervisor, browser, sustained-partition execution, retarget experiment,
v2 type-32 audit, witness/fee/Ark economic scenarios, and any production/
mainnet claim.

## Result

Candidate is ready for a new manifest and Fable's third review, confined to the
R1/N1/N2 diff. The reviewer remains the same Fable session. No commit, push,
merge, reset or stash was performed.
