# 2026-11-06 - Fable review: lab documentation integration (`lab-docs`)

Reviewer: Fable (subagent). Role: gate review only. No candidate edits, no
commits, no pushes, no subagents, no Rust build. This file is the only
artifact written into any tree.

## Verdict

**REQUEST_CHANGES - bounded to three precise documentation items (D1-D3).**
Everything else in the packet is accurate against the approved code
(`lab-share-integration` LP-1 `c16032a7...`, host runtime `e891f34c...` /
code `d75ab391...`), applies the binding L1/L3/LP-1 decisions without
rewriting legacy history, does not weaken `DESIGN.md` payout policy, keeps
the Ark privacy boundary intact, and invents no namespace CLI. The three
items are each a sentence or two; nothing requires a code change. Re-review is
confined to them plus any file whose hash moves.

## Snapshot reviewed

Worktree `/home/ethan/.local/share/opencode/worktree/ad8886/lab-docs`, HEAD
`b0ab4ca34fda07c61a6335d05f598eaacc69678c` (detached). Manifest
`/tmp/opencode/lab-docs-review-manifest.json` (file SHA-256
`cc31a35b82664454c7b9c7bf8694c2c2169625b4be8431c75f76ceb43cb6250b`), 30
files. Aggregate recomputed over `base` plus `path`/`sha256` only
(`sort_keys`, compact separators):

```
ff6909cd9a99e6bc57377e18394a84887a7f514e6a5b39d7eadb4c62fd885570   (matches)
```

All 30 files re-hashed from disk: 0 mismatches; `git status --porcelain
-uall` shows no path outside the manifest; no Rust, Cargo, flake, script or
`.sql` file is modified. Baseline `/tmp/opencode/lab-docs-baseline.json`
(SHA-256 `13529be4...7c99`): 22 of the 25 copied approved docs are byte-equal
to the approved runtime manifest (`e891f34c`), and the three that differ are
exactly the owned edits (`docs/SPEC.md`, `docs/WIRE-FORMATS.md`,
`docs/REGTEST-LAB-IMPLEMENTATION.md`); their approved pre-edit copies in the
frozen `lab-runtime` worktree match the runtime manifest, so the diffs below
are against approved text. For the eight owned files every `base_sha256`
equals `git show HEAD:<path>` (or is null for new files) and every
`final_sha256` equals the disk hash. The approved L3 decisions and changelog
fragments are byte-identical to their approved inputs.

Precedence applied: `AGENTS.md`; `DESIGN.md` > `SPEC.md` > `WIRE-FORMATS.md`
/ `OPERATIONS.md`; binding decisions `2026-11-05-lab-share-work-units`,
`2026-11-05-window-network-block-work`, `2026-11-05-sv2-native-conformance`,
`2026-11-05-lab-lp1-integration` (r3 text) and the two 2026-11-06 lab
records. Code was read in the frozen `lab-runtime` tree (which carries the
LP-1 r3 pool sources) to check every behavioural statement; the `spc-lab` CLI
was checked from `bins/spc-lab/src/main.rs`, `crates/spc-lab/src/{command,
http,runtime,oracle,secrets,artifacts}.rs` and `flake.nix` without building.

## 1. Verified accurate (normative consolidation)

`docs/SPEC.md` (diff vs approved runtime copy, body only; frontmatter is
byte-identical to the approved copy and parses as YAML with 26 changelog
entries, the four approved 2026-11-05 fragments present in L1/L3/LP-1 order):

- Section 3.2 extension table, 3.4 versioned share handling, 3.7 type-32 v1
  only with `Type32Message::Error{unsupported-extension}` in v2: match
  `config.rs` (v2 rejects SV1, `share_accounting`, `share_difficulty`,
  non-regtest, non-power-of-two family, `pow_limit_bits != 0x207fffff`) and
  `sv2_server.rs:550-566`.
- Section 3.3 job lifecycle paragraph: one future job + one `SetNewPrevHash`
  on a `(prev_hash, nbits)` change, non-future job otherwise, curtime
  advancement does not republish, latest-only acceptance with
  `stale-share`, `clean_jobs = true`, and "delivering a new job while an
  outcome is pending is a scheduling detail" - matches `send_job`,
  `submission_error`, `front.rs` refresh and the pool-latency decision. No
  hidden normative delta: the previous text never required outcome-before-job.
- Section 4.1 parameters: `work_unit_version`, `work_reference_target`,
  `share_work_multiplier` (`m_issue = min(m_cfg, 2^(k - ceil_log2(T+1)))`),
  `max_work_multiplier <= 2^32`, `max_window_target_units <= 2^64` decimal
  string, `window_rule` pairing, `retain_windows` never applied to v2 rows,
  `scrub_after_blocks` note - all match `config.rs`/`work_unit.rs`/`main.rs`.
- Section 4.3: stored `window_rule` selects the rule; v1 frozen `N * d` with
  saturating arithmetic; v2 `Q(T) = ceil(N * (R + 1) / (T + 1))`, vectors 65
  / 9, finding slice excluded, crossing slice included, Q persisted and never
  recomputed, `window_target_too_large`, derived underfill/overshoot, RAM
  tail every eligible slice up to 4096 using template height `tip + 1` (one
  block stricter than SQL). Matches `front.rs:1106-1119`,
  `select_window_v2`, `window_target_network_work`.
- Section 4.4: v1 saturating frozen, v2 checked with named errors, mixed
  profile restore/build fails closed, frozen distribution settles with its own
  profile; explicit statement that exact `owed` is the audit authority and the
  ladder / `DESIGN.md` are unchanged. This is the approved decision's intent
  ("v1 keeps its existing saturating functions unchanged, version-gated"),
  stated as normative freezing of the legacy path, not a new behaviour.
- Section 5.1 schema and scrub: columns/CHECKs are semantically identical to
  migration 0005 (`IN (1,2)`, `(version = 1) = (fingerprint IS NULL)`, 32-byte
  nonzero shape, rule `IN (0,1)`, version/rule pairing); per-row-version
  deletion horizons, v2 `height < tip - 4096` strict cutoff, unknown versions
  undeleted, RAM one block stricter, v2-only retention/privacy disclosure,
  Ark boundary unchanged - match `postgres.rs:585-608`, `payout.rs:1890-1902`,
  `models.rs:10`.
- Section 5.2/5.3, 6 (config example incl. `authority_secret_key`, v2 keys
  commented, `[sv1] enabled` and `share_accounting` MUST be false in v2),
  7.4 frozen settlement inputs, 8.3 v2 test vectors, 9 migration 0005 and
  "v2 is a lab profile, not a production rollout", 11.3 boundary note.
- Nothing in the retained 2026-11-02 share-target text or changelog (d) was
  rewritten; v1 history stands.

`docs/WIRE-FORMATS.md`: 3.1/3.6 tables (from LP-1 r3) retained; new 3.6
paragraph (no `window_rule` on `spc:slices`, mixed/unknown profile fails) and
3.5 `type32_blobs` annotation - see D1 for the one overclaim.

`docs/OPERATIONS.md`: 1.1 v2 profile TOML and restrictions, Q and underfill,
retention/scrub, SV1 rejected in v2, Postgres row shapes, independent reader
connection, Noise authority pair inline in TOML with matching-pair startup
error, migration 0005 note, backup horizon note, host-lab pointer, items 5-6
(namespace pending, v2 lab profile). All match code or the approved reviews.

`docs/share-accounting-ext-deltas.md`: v1-only scope row and v2 refusal -
accurate except the `type32_blobs` phrase covered by D1.

`README.md`: status paragraph, quick start (flags exist on `spc-lab runtime`;
`--scanner` optional; `--listen` optional; `--duration-secs`; private paths;
node is an explicit non-Nix pinned L2 build), document index with the three
new links.

## 2. Verified accurate (operator guide `docs/LAB-RUNTIME.md`)

Checked against `bins/spc-lab/src/main.rs` and the `spc-lab` library:

- Subcommands and flags used in the guide exist with those exact names:
  `runtime --scenario --run-dir --node --pool --miner --scanner
  --postgres-bin-dir --redis-server --redis-cli --base-revision
  --dirty-tree-hash --listen --duration-secs --interactive`; `snapshot
  --run-dir --cursor`; `events --run-dir --after`; `manifest --run-dir`;
  `command --run-dir --json`; `reap --run-dir`. (`--pow-bits`, default
  `0x200fffff`, is not mentioned; harmless.)
- `command --json '{"kind":{"type":"stop_run","reason":"manual"}}'` is a
  valid payload: `CommandKind` is `#[serde(tag = "type", rename_all =
  "snake_case", deny_unknown_fields)]` with `StopRun { reason: String }`; the
  CLI fills `command_id`, `run_id` and actor.
- HTTP routes `/api/lab/v1/runs/<id>/{snapshot,events,commands,catalog}`
  exist (`http.rs:299-305`); the catalog exposes declared public SP
  identities with `used` (`runtime.rs:1977-2012`); typed commands
  `add_miner`, `switch_pool`, `set_worker_budget`, `pause_workers`,
  `resume_workers`, `stop_run` exist.
- `private/` 0700 and 0600 files (`artifacts.rs:59`, `secrets.rs:96,108`);
  `completed_with_errors` state exists; `reap` "never kill by PID alone" is
  the CLI's own doc string.
- Node exception: SHA-256 `eb257af7...320f7c`, source `d2f37b91...`, "no Nix
  node output", `sv2_tp_bootstrap` 6/6 only, L2
  `CLEARED_WITH_RECORDED_BASELINE_LIMITATIONS`, `no_pow_retargeting = true` -
  all taken verbatim from the approved runtime report.
- `flake.nix` builds `spc-pool` with `rpc` (`mkBinary "spc-pool" "rpc"`), so
  "the flake package does that" is correct; `spc-lab` is built from
  `bins/spc-lab`.
- Scenario `crates/spc-lab/scenarios/lab-live-v2-two-pools.json`,
  `testing/regtest-lab-v2-e2e.py`, `testing/lab-live/` exist in the approved
  tree.
- Evidence scope: `accounted_target_consistency` joins by
  `(front, job_id, generation)`, reports `consistent | inconsistent |
  unknown` with `checked` / `not_checked` lists; tip sampler, scanner cursor
  and `runtime_metrics` descriptions match `runtime.rs`. The guide states no
  causal hash-budget claim, oracle `inconclusive`, no canonical-coverage
  claim, no namespace CLI, and quotes the topology review as
  REQUEST_CHANGES with the RPC `generate` + `nsenter` caveat.
- Links: every relative link in the eight owned files resolves inside the
  worktree except exactly three in `docs/REGTEST-LAB-IMPLEMENTATION.md`
  (`REGTEST-LAB.md`, `REGTEST-LAB-STATUS.md`,
  `reviews/2026-11-05-regtest-lab-review-summary.md`), all present in the
  main root and declared as expected-missing. No U+2010/2011/2012 or NBSP in
  any owned file; new prose uses ASCII punctuation, retained section signs, multiplication signs, arrows and em dashes are
  pre-existing SPEC style.

## 3. Findings (bounded)

### D1 - `type32_blobs` "MUST be empty in v2" contradicts the approved code

`docs/WIRE-FORMATS.md:171` adds: "In v2 `type32_blobs` MUST be empty: type-32
is not advertised or accepted and no v2 slice may populate the store."
`docs/share-accounting-ext-deltas.md:27` says "the v2 slice path never writes
`type32_blobs`", and the report (`...lab-docs-implementation.md:98`) says
"Bootstrap `type32_blobs` is v1-only and empty in v2."

Approved code: `Store::bootstrap` (`crates/spc-store/src/postgres.rs:1151-1154`)
exports `type32_blobs` as `(slice_id, shares)` for **every** `window_tail`
row regardless of `work_unit_version`, and the v2 front does persist the
identity-free `shares_blob` on every sealed slice (`front.rs` `roll_slice`,
`SliceEvent.shares_blob`), so a v2 pool's bootstrap payload carries blobs for
v2 slices. What is true is the *service* statement: the v2 front never adds
them to `Type32Store` and refuses type-32 requests. The blobs are
identity-free, so there is no privacy impact, but a lower-tier doc now states
a normative MUST that the approved pool does not satisfy, and a verifier
asserting an empty array would fail against the approved binary.

Fix (docs only): replace the WIRE 3.5 sentence with "In v2 the front MUST
ignore `type32_blobs` and MUST NOT serve them; the payload MAY still carry
identity-free blobs for v2 slices because `payout` exports every tail row's
blob." Reword `share-accounting-ext-deltas.md:27` to "the v2 front never
populates the type-32 store from them", and correct report item 8. (A
payout-side filter would be a code change and is not asked for here.)

### D2 - `LAB-RUNTIME.md` section 7 must disclose the approved host's malformed-hash skip (F1)

`docs/LAB-RUNTIME.md:153-160` says `accounted_target_consistency` "checks the
recorded hash against the announced target" and reports `consistent |
inconsistent | unknown`. In the approved host (`crates/spc-lab/src/runtime.rs:506-520`)
a `share_hash` that is not valid hex or not exactly 32 bytes is silently
skipped: the record can still be counted `consistent`. The fix (count
`malformed_share_hashes`, mark the record `inconsistent`) is accepted in the
topology review but lives only in the unapproved topology candidate; it is
**not** in `e891f34c`/`d75ab391`. The guide and the report (section 6 "F1
malformed share-hash handling (topology owner, separate code fix)") mention F1
only as a pending item without saying what the approved host does today.

Fix (docs only): add to section 7 (and the report's section 6) one sentence:
"In the approved host build a malformed or non-32-byte `share_hash` is
skipped, not flagged, and the record can still count as `consistent`; the
accepted F1 fix (`malformed_share_hashes`, record `inconsistent`) is pending
merge from the topology packet. Do not read a `consistent` result from this
build as a hash check over every record."

### D3 - the binding L3 merge edit to `docs/decisions/2026-11-02-share-target-policy.md` is missing while completion is claimed

Both approved L3 decisions list under "Edits required at merge":
"`docs/decisions/2026-11-02-share-target-policy.md`: add a superseded-in-part
note pointing here for v2; do not rewrite its v1 history"
(`lab-share-work-units.md:529`, `window-network-block-work.md:313`). The
file is unchanged in this packet (no v2/superseded note), yet
`docs/REGTEST-LAB-IMPLEMENTATION.md:1324` now states "the approved changelog
fragments and the section edits are now applied by the documentation
integration packet." Decision records are binding (`AGENTS.md`); a listed
merge edit either happens or is named as still owned by the merge session.

Fix (docs only, either): add a two-line "Superseded in part (2026-11-05)"
note at the top of the 2026-11-02 record pointing to the two L3 decisions for
v2 and stating v1 text is unchanged (this does not rewrite history; it is
the prescribed banner), **or** change `REGTEST-LAB-IMPLEMENTATION.md`
section 13 to list that note as a remaining merge-session edit instead of
claiming all section edits are applied.

### Nits (no change required for approval)

- `docs/SPEC.md:8` `updated: 2026-11-03` is stale against the 2026-11-05
  fragments and body edits (pre-existing from the runtime copy).
- `docs/OPERATIONS.md:221` "secret file mode `0o600`" - the secret is an
  inline TOML string (`RedactedSecret`), there is no separate secret file, and
  the binary does not enforce the mode; the following paragraph says so, but
  the table cell should read "pool TOML carrying it mode 0600 (operator
  convention, not enforced)".
- `docs/SPEC.md:451` `pow_limit_bits` default "consensus-derived" - the code
  pins the constant `0x207fffff` and rejects any other value for v2; "pinned
  regtest consensus value" is the accurate phrase.
- Pre-existing, not introduced here: the pool has no `RequestExtensions`
  (`0x0001`) handler; type-32 refusal is at the extension-frame router. The
  SPEC 8.3 line is a test requirement and does not conflate the `0x0020`
  extension id with the type-32 message set; `share-accounting-ext-deltas.md`
  still describes negotiation via `0x0001` from before this packet.
- The report's "no tool-only SPEC entry" choice is acceptable: both 2026-11-06
  records remain as decision/changelog files, their changelog fragments are
  not spliced, and the body text they touch (3.3 scheduling detail,
  OPERATIONS reader connection) carries no normative delta hidden from the
  frontmatter. The merge session should confirm the final splice policy.

## 4. Scope statements confirmed

- No production payout policy change: SPEC 4.4, OPERATIONS 1.1/8.6 and
  LAB-RUNTIME 9 all state the lab's exact-`owed` pre-ladder scope does not
  amend `DESIGN.md`; the 2026-11-03 ladder text is untouched.
- Privacy: the v2 4096-height retention is described as SPc-side only with no
  new Ark field (SPEC 5.1, 11.3; OPERATIONS 1.1); `a_send`/identity material
  stays SPc-side (LAB-RUNTIME 9).
- Namespace handoff: LAB-RUNTIME 9/10 and OPERATIONS 8.5 state the topology
  helpers are an unwired precursor under REQUEST_CHANGES, the two-node
  artifact used RPC `generate` + `nsenter`, the storage boundary is undecided,
  host loopback is not cross-namespace, and no namespace CLI exists. The
  Nix network-tool block is described as not a core package failure.
- Evidence boundary: no node-canonical v2 acceptance, full-window payout
  oracle or causal hash-budget claim is made anywhere in the owned docs.

## 5. Re-review scope

Apply D1-D3 in `lab-docs`, regenerate the manifest with the same recipe, and
re-run the link/lookalike check over the owned files. I will confine the next
pass to those lines plus any file whose hash moves. The namespace guide
handoff remains pending and is correctly not claimed by this packet.

---

# Second pass (revision r2) - 2026-11-06

Reviewer: Fable (subagent). Same rules: no candidate edits, no commits, no
subagents, no Rust build. This appended section is the only artifact written.

## Verdict (r2)

**APPROVE - documentation integration packet.** D1, D2 and D3 are resolved as
specified, the nits are fixed, no approved normative semantics changed, and
the new baseline record is self-consistent. The namespace guide remains a
pending handoff and is still correctly unclaimed.

## r2.1 Snapshot

Same worktree, HEAD `b0ab4ca3...678c`. Manifest
`/tmp/opencode/lab-docs-review-manifest.json` (file SHA-256
`d29c0510ca4cea704e91c70c9e34a8dd6a80f25974cfcdc6485d384a734e263e`, copied to
`/tmp/opencode/Fable-labdocs-r2-manifest-copy.json`), 32 files. Aggregate
recomputed over `base` plus `path`/`sha256` only:

```
1d40b2b705dc59afd2f3496011acc6c4c948c33aa6f2e6d4dab8be334d40d9c0   (matches)
```

All 32 files re-hashed from disk: 0 mismatches; nothing outside the manifest
in `git status --porcelain -uall`; no code, Cargo, flake, script or `.sql`
change. Changed since the r1 snapshot: `docs/SPEC.md`, `docs/WIRE-FORMATS.md`,
`docs/OPERATIONS.md`, `docs/share-accounting-ext-deltas.md`,
`docs/LAB-RUNTIME.md`, the implementation report; new:
`docs/decisions/2026-11-02-share-target-policy.md` (banner only, `+9 -0`)
and `docs/reviews/2026-11-06-lab-docs-baseline.md`. `README.md` and
`docs/REGTEST-LAB-IMPLEMENTATION.md` are unchanged from r1. The 22 copied
approved inputs are still byte-equal to the runtime manifest.

## r2.2 Disposition

- **D1 resolved.** `WIRE-FORMATS.md:166,171`: `type32_blobs` is "identity-free
  share blobs exported for every `window_tail` row"; "In v2 the front MUST
  ignore `type32_blobs` and MUST NOT serve them; the payload MAY still carry
  identity-free blobs for v2 slices because `payout` exports every tail row's
  blob." `SPEC.md:400` and `share-accounting-ext-deltas.md:33` say the same
  (v2 front ignores; never populates the store from them). "Ignore" is the
  accurate consumer verb: the single-site front gets `BootstrapSnapshot`
  straight from `Store::bootstrap` and `apply_bootstrap` never reads the
  field (no decode, validate or explicit discard); `Type32Store` is populated
  only by `roll_slice`, and only for enabled v1. The JSON `BlobJson` path is
  producer-side only (`BootstrapResponse::from_snapshot`); there is no JSON
  consumer in the pool.
- **D2 resolved.** `LAB-RUNTIME.md:160-166` version-binds the caveat to
  `e891f34c`/`d75ab391`: malformed or non-32-byte `share_hash` skipped, record
  can still count `consistent`; missing hash stays `unknown` (report section
  6 says so explicitly; code `runtime.rs:506-523` matches both); F1 fix
  accepted in the topology packet but pending merge and a new combined
  approval; "do not read `consistent` as a hash check over every record".
- **D3 resolved.** The 2026-11-02 record carries a nine-line "Superseded in
  part (2026-11-05)" blockquote pointing to the two L3 decisions and the LP-1
  integration record and stating the v1 policy remains frozen; the historical
  body is unchanged (numstat `+9 -0`). The REGTEST-LAB-IMPLEMENTATION section
  13 "section edits are now applied" statement is now true.
- **Nits resolved.** SPEC frontmatter `updated: 2026-11-06` (26 entries parse;
  the only frontmatter deltas vs base are the four approved 2026-11-05
  fragments and the date); `OPERATIONS.md:221` says the secret is inline in
  the pool TOML and the 0600 mode is an operator convention not enforced by
  the binary; `SPEC.md:451,859` say `pow_limit_bits` is the pinned
  `0x207fffff` with no dynamic consensus query; `share-accounting-ext-deltas.md:22-26`
  scopes the absent generic `RequestExtensions` (`0x0001`) handler as a
  pre-existing contract-vs-wiring gap and places the v2 refusal at the
  extension-frame router.

## r2.3 New baseline record

`docs/reviews/2026-11-06-lab-docs-baseline.md`: every owned-delta row's base
hash equals `git show HEAD:<path>` (or "new file" for paths absent at base)
and every final hash equals the disk file; all 25 copied-input rows equal the
approved runtime manifest hashes; the record states that its own hash lives in
the generated manifest rather than claiming it inline, and the merge order
lists the banner edit and itself. One clarity note, not a defect: the copied
table lists `docs/SPEC.md`, `docs/WIRE-FORMATS.md` and
`docs/REGTEST-LAB-IMPLEMENTATION.md` as byte-for-byte copies (true for the
copy step) while the owned table records their later edits; a reader must read
both tables together.

## r2.4 Checks repeated

Relative links in all ten owned files resolve except exactly the three
declared main-only inputs in `docs/REGTEST-LAB-IMPLEMENTATION.md`; no
U+2010/2011/2012/NBSP and no trailing whitespace in any owned file; no edit to
any approved decision/changelog text; no fictitious `spc-lab` flag added (the
guide's command set is unchanged from r1 and was verified against
`bins/spc-lab/src/main.rs`). Approved normative semantics (v1 frozen `N * d`
and saturation, regtest-only v2 family/caps/pinned pow limit/checked math,
frozen Q and finding-slice exclusion, per-row scrub `height < tip - 4096` with
RAM `tip + 1` one block stricter and the 4096-slice cap, type-32 v1-only,
Ark boundary) are stated identically to r1 where they were already correct.

## r2.5 Still pending (unchanged, not claimed by this packet)

Namespace placement/address publication/controller transport (the network
runtime author is fixing T1-T9 in parallel; no operator flags exist yet), the
F1 host merge fix and its combined approval, node-canonical v2 acceptance,
full-window/payout/header oracle, fee/witness/Ark/ladder/SV1 coverage, and
the merge session's final SPEC/OPERATIONS splice and `STATUS`/`MERGE-QUEUE`
updates.
