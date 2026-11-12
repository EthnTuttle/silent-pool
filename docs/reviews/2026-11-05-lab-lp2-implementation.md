# LP-2 native SV2 CPU miner implementation

Date: 2026-11-05
State: R3-B5 event-ordering fix ready for Fable fourth review. No approval is claimed yet.
No commit, merge or pin change was made.
Base: `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.

## Scope and result

Implemented a new native extended-channel SV2 CPU miner with its own protocol
state machine, SHA256d workers, paced/total budgets, JSONL control/events,
versioned TOML config and tests. It does not link `spc-pool`/`spc-agent` or copy
the pool's old job helper. Wire codecs, framing, messages and authenticated Noise
transport use the already-pinned `stratum-core` 0.6.0 and `stratum-apps` 0.8.0.
Target/job state is implemented independently here and is exercised by both
upstream-material vectors and the actual L1 pool/main + Bitcoin Core path.

Verified behavior:

- future jobs remain idle until matching `SetNewPrevHash`; activation captures
  that prevhash/nbits and the channel target effective at activation;
- immediate jobs on the same tip use the latest prevhash/nbits; active targets
  are not rewritten by a later `SetTarget`;
- deepest-first Merkle folds, actual 80-byte headers, real SHA256d, inclusive
  little-endian SV2 targets and strict compact-target decoding;
- one connection/identity/channel, multiple disjoint nonce/extranonce worker
  partitions, bounded batches, independent hash threads, real attempt counters,
  and no sleeps that fabricate attempts;
- multiple outstanding submissions, exact `(session, sequence)` reply matching,
  reconnect resets without old-header reuse, and explicit stale cancellation;
- mandatory pinned Noise public keys, including x-only/SEC hex and the public
  key-utils base58check encoding used by L1's public pair;
- add/remove/budget/pause/resume/switch/snapshot/stop commands with idempotency,
  conflict rejection, graceful cancellation and a strict file config for N
  identities/pools/workers;
- explicit `submitted`, ingress ACK/rejected, `transport_unknown`, `accounted:
  unknown`, local `block_qualified`, and `candidate_submission: unknown` states.
  No ingress or local-PoW observation is promoted to accounting or node proof.

`bins/spc-cpu-miner/PROTOCOL.md` is the stable version-1 contract for LP-3.
`fixtures/control.jsonl` and `fixtures/events.jsonl` are reusable schema fixtures.
The process does not duplicate the runner's durable `LabCommand` journal.

## Files

New binary/library:

- `bins/spc-cpu-miner/Cargo.toml`, `PROTOCOL.md`
- `src/ids.rs`, `config.rs`, `control.rs`, `miner.rs`, `main.rs`: nominal command,
  miner, connection, identity, worker and job IDs, strict config, typed
  operations, idempotent execution and CLI lifecycle
- `src/connection.rs`, `session.rs`: pinned Noise/setup/channel, job/prevhash,
  target activation, replies, reconnect and stale cancellation
- `src/hash.rs`, `target.rs`, `worker.rs`, `budget.rs`: real header/hash work,
  LE/compact targets, disjoint search cursors and measured budgets
- `src/report.rs`: typed JSONL events, snapshots, bounded publication and gap
  accounting. Rate credits cap each batch before hashing, so shares never wait
  behind a post-burst pacing sleep.
- `src/lib.rs`

Tests and fixtures:

- `tests/sv2_session.rs`: future/activation/same-tip target lifecycle,
  ntime hold/roll and upper bounds, non-empty multi-level Merkle asymmetry,
  consensus upstream tree roots, and the rust-bitcoin mainnet block-100000
  coinbase/header vector
- `tests/target_budget.rs`: independent SHA256d vector, asymmetric LE boundary
  hashes, compact-target boundaries, exact pacing formulas, saturation, reduced
  nonce domains and counter exhaustion (not a full-domain timing claim)
- `tests/live_session.rs`: real local Noise sessions with future job activation,
  real share hashing, several outstanding response correlations, wrong-authority
  rejection, reconnect/new-header uniqueness, switch fake-endpoint lifecycle,
  and command idempotency. These tests make no interoperability claim by
  themselves.
- `tests/control_schema.rs`: typed command/events fixtures, replay/conflict and
  key-utils authority parsing
- `fixtures/{miner.toml,two-pools.toml,control.jsonl,events.jsonl}`

Shared local build files:

- `Cargo.toml`: only the new workspace member was added.
- `Cargo.lock`: only the new package stanza and its dependency list were added;
  no crate version or upstream pin changed.
- `flake.nix`: new package/overlay entry and the required local vendor hash.
  The changed base32 vendor hash is only for this worktree's manifest. The merge
  session must recompute it for the combined L1/LP1/LP3 manifest rather than
  blindly copying it.

## Verification actually run

All Rust commands below ran under `nix develop` with `CARGO_BUILD_JOBS=2`.

| Check | Result / log |
|---|---|
| `cargo fmt -p spc-cpu-miner -- --check` | Pass, included in `/tmp/opencode/LP2-final-verification-02.log` |
| `cargo test -p spc-cpu-miner` | Pass: 24 integration tests total: 8 protocol-state, 8 target/budget, 3 live-session, 5 control-schema. Same log. |
| `cargo test -p spc-cpu-miner --test sv2_session` | Pass: 8/8. Same log. |
| `cargo test -p spc-cpu-miner --test target_budget` | Pass: 8/8. Same log. |
| `cargo clippy -p spc-cpu-miner --all-targets --locked -- -D warnings` | Pass, `/tmp/opencode/LP2-final-verification-02.log` |
| `cargo test --workspace --locked` | Pass: 453 recorded passed, 0 failed, `/tmp/opencode/LP2-final-verification-02.log` |
| `cargo run ... --check-config` on empty and two-pool fixtures | Pass, `/tmp/opencode/LP2-cli-check-empty.log`, `/tmp/opencode/LP2-cli-check-two-pools.log` |
| stdin `snapshot` then `stop` JSONL | Two typed results, process exit 0, `/tmp/opencode/LP2-cli-jsonl.log` |
| `git diff --check` | Pass on tracked shared files; new binary files were separately written in plain text. |
| `nix build .#spc-cpu-miner --offline` | Vendor/Cargo.lock consistency passed, then the build stopped because Nix did not copy the still-untracked new package into `src`. This is not reported as a successful package build. See `/tmp/opencode/LP2-nix-build-02.log`; rerun required after merge/staging. No index mutation was made to hide it. |

The repeated global cargo-cache permission/auto-clean warnings are environmental.
An attempted `--offline` clippy run also hit the host's incomplete local index;
the successful recorded run used `--locked` without changing any dependency.

## Real L1 + node interoperability smoke

The approved L1 tree was used read-only as source; no L1 file was edited. Its
binary was rebuilt with `--locked` and the `rpc` feature at
`/home/ethan/.local/share/opencode/worktree/ad8886/lab-sv2-conformance/target/debug/spc-pool`.
This is the approved L1 snapshot `fa532607fd20ce8f9dabcae1f552973e3c45c12b99ab7be1eb2f148e36056cd2`
integration condition, not a self-pool protocol-only test.

Command: `nix develop -c python3 /tmp/opencode/LP2-main-smoke-driver.py`
(exit 0), log `/tmp/opencode/LP2-main-integration-driver-10.log`.
Artifacts: `/tmp/opencode/LP2-main-integration-xptd5vo2/` with `summary.json`,
`miner.jsonl`, `pool.log`, `miner.stderr`, `bitcoind.log` and the exact configs.
The executed pool binary SHA256 was
`4597dec51a647c34447102f5b8295c8268389dff6bcce170bf336bcf77d68550`;
the miner binary SHA256 was
`37a0dc4ab63bce6fcc28f24b0ff67ebf8bee84fae4dbb1b3e0fadeed2c2bf46c`.

The driver started Bitcoin Core regtest, generated 101 setup blocks, launched the
L1 front with its matching public authority pair, and ran this miner's config
with four negotiated local extranonce bytes. It then:

- accepted the actual L1 future job and activated with its supplied target;
- submitted 62 real extended shares before one correlated ingress ACK was read;
- verified every submitted event's 80-byte header independently by Python
  SHA256d and checked it against its recorded LE target;
- advanced the node from height 101 to 102;
- matched the node's best block
  `202871e012a186cd0fb742a2e768fa49fe4ffafaa248af8a0379c82570ae6e24`
  to a local block-qualified submission;
- stopped gracefully with miner return code 0.

This demonstrates actual native-main SV2 legacy-policy interoperability and a
node-accepted block. It does **not** demonstrate a non-block share: legacy
regtest makes every accepted share block-qualified here. Accounting remains
`unknown` and a candidate is not inferred from ingress. `summary.json` records
`lp1_nonblock_share_claim: false`.

The main network tip had an empty Merkle path. Multi-level coverage is the
independent upstream tree vector plus L1 real-job path; a non-empty real-node
multi-transaction template remains a separate integration gate.

## Remaining integration gates, without implied claims

1. LP-1 is required for non-block share acceptance, multiplier/target-credit
   changes and mixed-target share-policy evidence. No such credit is computed by
   this miner.
2. LP-3 still needs to wrap the JSONL process in its durable command/evidence
   log, retain cursors/gaps and implement the full headless scenario runner.
3. A live negative matrix (stale, duplicate, below-target and exact protocol
   codes) belongs in the integrated conformance/scenario run. The independent
   fixture exercises duplicate rejection, but is not substituted for that run.
4. Non-empty multi-transaction real-node coverage, sustained partition/rejoin
   scenarios and retarget economics remain integration work.
5. The merged Nix package/check needs a fresh source/eval/vendor pass after all
   packet manifests are present.

## Merge notes

- Keep the new crate isolated. Do not reintroduce dependencies on the pool or
  agent binaries.
- Retain L1's pool-side channels dev dependency and protocol behavior; this
  packet changes no L1 source and only consumes its approved executable.
- No script, runner, accounting, payout, store or `docs/SPEC.md` changes are
  part of this packet. No new operational decision record is required by this
  implementation-only packet.
- `flake.nix` conflicts should resolve by keeping every package, then rebuilding
  the combined vendor hash. `Cargo.lock` needs only the ordinary workspace
  package-stanza merge; no upstream bump is implied.
- `PROTOCOL.md` operation/result/event version 1 is published for LP-3. Changes
  to field names or semantics must use a new protocol version rather than
  silently changing version 1.

## R1-B4 rework after Fable REQUEST_CHANGES (af531370)

The review in `docs/reviews/2026-11-05-lab-lp2-fable-review.md` identified four
runtime defects. All were addressed without changing the crate's dependency
pins or the pool/node code.

### Dispositions

- **B1 - live pacing debt and uninterruptible wait.** The pacing state now keeps
  an independent lifetime `attempted` for total-attempt limits, while every rate
  budget change starts a fresh `(attempted, elapsed)` rate epoch. Raising or
  lowering HPS cannot create debt from the old rate. `required_delay` is charged
  only against the new epoch. Raw `thread::sleep` was replaced by the existing
  condvar with budget, pause, job and stop wakeups. Repeated
  remove/switch/stop joins no longer wait for a pending 1 H/s pacing interval.
  `live_rate_reduction_rebases_credit_without_taking_attempt_history` simulates
  the previous 5940-second backlog and requires a bounded delay with lifetime
  attempts preserved. `interruptible_pacing_stop_and_budget_rebase_are_bounded`
  runs a real worker and joins it in less than 100 ms under a 1 H/s budget.
- **B2 - authoritative same-tip ntime.** The elapsed-time `NtimeTooHigh` check
  was removed. `NtimeRegression` remains, while same-tip `curtime` increments of
  one and 600 seconds now activate. Hold/roll stays an explicit miner policy
  with the configured 7200-second cap.
- **B3 - batch correlation and bounded history.** `SubmissionTracker` resolves
  the named last-correct sequence plus the oldest no-error preceding prefix,
  exactly `new_submits_accepted_count` results. Submissions arriving after that
  batch remain pending. Empty/exact-repeat ACKs are no-ops, mismatches disconnect
  with an explicit protocol error, and `new_shares_sum` stays batch-only data.
  `ingress_accepted_reported` counts unique resolved submissions, not repeats.
  The 1024-share outstanding bound and 256-share known-history bound report
  explicit evictions; only pending shares can become `transport_unknown`.
- **B4 - actual activation only.** `apply_changes` iterates
  `changes.activated` instead of re-announcing the current job on a future-job
  receipt or `SetTarget`. Worker stale counters change only for actual clear or
  replacement work, not an idempotent target update.

### Rework verification

Commands again used `nix develop` with `CARGO_BUILD_JOBS=2`:

| Check | Result / log |
|---|---|
| Initial failing B1 reproduction | `5940.001s`, `/tmp/opencode/LP2-rework-red-03.log` |
| `cargo test -p spc-cpu-miner` | Pass: 28 tests (6 live, 9 target/budget, 8 protocol-state, 5 control-schema), `/tmp/opencode/LP2-rework-tests-02.log` |
| `cargo clippy -p spc-cpu-miner --all-targets --locked -- -D warnings` | Pass, same log |
| `cargo fmt -p spc-cpu-miner -- --check` | Pass, same log |
| `cargo test --workspace --locked` | Pass: 457 recorded passed, 0 failed, `/tmp/opencode/LP2-rework-workspace-tests.log` |

The direct cap test covers evictions, already-resolved sequences, inconsistent
batch counts, duplicate ACKs and a closed u32 wrap. The live batched test sends
`count=3`, the exact same batch again, then an empty repeat; all three shares
end `ingress_ack`, with `ingress_accepted_reported == 3` and no
`transport_unknown`. The pacing test's share receiver is drained so it tests the
pacing wait, not unrelated queue backpressure.

### Rerun L1 + node smoke

The L1 + bitcoind driver ran again successfully: exit 0, log
`/tmp/opencode/LP2-rework-main-integration-driver-02.log`, artifacts
`/tmp/opencode/LP2-main-integration-j1mitlwz/`. It observed 249 real submissions
and one correlated ingress ACK, advanced height 101 -> 102 and matched best block
`756225dbade89f54f26007f6191285f748f8a69e9e884276f51fbaa6bc2f6b4f`
to a local submission. The driver waits at most two additional seconds only for
the real ACK; it does not synthesize shares. Every submitted header was again
independently SHA256d/LE-target checked. Miner return code was 0 and accounting
and non-block LP-1 claims remain absent. Executed miner binary SHA256 was
`560f9df00f48f94c3dc8f27509f4df23a9d0b46650cf04895479d9635b8cb82d`;
L1 pool SHA256 stayed
`4597dec51a647c34447102f5b8295c8268389dff6bcce170bf336bcf77d68550`.

### Version-1 wire handoff changes

Low-level JSON fields keep their existing names and version 1 is retained for
this pre-release handoff. Additions/semantic clarifications are:

- new additive `submissions_evicted` event for resolved known-history evictions;
- explicit pending-eviction `submissions_unknown` reasons
  `outstanding-cap-evicted` and `known-history-cap-evicted`;
- `ingress_accepted_reported` now reports unique newly resolved submissions;
- `job_activated` no longer duplicates an unchanged generation.

LP-3 should handle the additive event. Envelope field-name and target/hash wire
formats did not change. `PROTOCOL.md` now also makes payload extension tolerance
(non-strict payload objects), switch/new-worker lifetime counters, batch-count
semantics and saturation measurement explicit.

### Other bounded review evidence

The reviewer independently built the unmodified path source successfully with
Nix and recorded `/tmp/opencode/Fable-LP2-nix-build-01.log`. This packet did not
execute or claim that check itself. Final combined git-source packaging still
needs the tracked source and combined lockfile vendor-hash check. Non-blocking
remarks about inactivity timeouts and supervisor stdin adoption are LP-4/LP-3
inputs rather than new protocol features. Unused `rand`, `tracing` and
`tracing-subscriber` dependency declarations remain deliberately unchanged to
avoid unrelated vendor churn; they are not required by current code.

LP-1 remains under review, not approved, and is not treated as evidence here.
Non-block acceptance, v2 target credit and the negative matrix remain integration
gates.

## R2-B3b final batch-correlation fix

The second Fable review approved B1, B2 and B4 and isolated one remaining rule.
No pacing, ntime or activation code was changed in this revision.

- **B3b - no guessed batch membership.** For `count >= 2`, the name of a correct
  last sequence identifies only that sequence. The oldest no-error prefix below
  it must have exact cardinality `count - 1`. Any mismatch is an immediate,
  explicit batch-prefix error and marks nothing ACKed. `count = 1` still resolves
  only its exact named sequence. The regression now proves pending `{0,1,2,3}`
  with `last = 3, count = 2` cannot be turned into the false ACK `[0,3]`;
  consistent batches, duplicate ACKs and cap/late-verdict cases still pass.
- **Presentation trim is not transport unknown.** A pending `Submitted` share
  leaving the 256-item presentation history now emits only
  `submissions_evicted`. It remains pending in the separate bounded tracker and
  can still be resolved later. `transport_unknown` is reserved for actual
  outstanding-cap loss or a transport ending without a verdict. Resolved-history
  trim uses the same non-failure event.
- **Bounded late-verdict handling.** Outstanding-cap eviction additionally
  remembers only a bounded retired-sequence set. A late ACK/reject for a retired,
  previously issued sequence produces the additive `late_verdict` observation
  instead of reconnecting on local history loss. Never-issued sequences still
  fail closed. This does not increment resolution credit or recreate per-share
  history.

Red/green regression: `/tmp/opencode/LP2-b3b-red-01.log` reproduces the old false
acceptance before the guard, and `/tmp/opencode/LP2-b3b-green-03.log` passes after
it. Final checks used `nix develop` with `CARGO_BUILD_JOBS=2`:

- `cargo test -p spc-cpu-miner --locked`: all 28 tests pass,
  `/tmp/opencode/LP2-b3b-live-tests-02.log`;
- fmt/clippy: pass, same log;
- `cargo test --workspace --locked`: 457 recorded passed, 0 failed,
  `/tmp/opencode/LP2-b3b-workspace-tests.log`;
- L1 + bitcoind smoke rerun: exit 0, 262 real submissions, one real ingress ACK,
  height 101 -> 102, node tip
  `3c9616e40a3472b738f83f44ff8704c9618ddd057d1b67e50628392a1e4ce584`
  matched a submission, miner exit 0. Driver
  `/tmp/opencode/LP2-b3b-main-integration-driver-01.log`; artifacts
  `/tmp/opencode/LP2-main-integration-8qrnqxj6/`.

Wire compatibility: version 1 is retained. Only additive events were introduced
in these review reworks: `submissions_evicted` (history-only eviction) and
`late_verdict` (explicit retired-sequence ACK/reject). `transport_unknown` no
longer falsely represents presentation-history loss. No pool/job/target field,
existing status name or credit formula changed. LP-3 should consume these events,
not reprice any shares.

## R3-B5 publication ordering and loss accounting

Fable's third-pass B3b closure is retained without changes to pacing, job state or
batch semantics. The new concurrent event-ordering blocker is addressed in
`report.rs` only, plus protocol/report text.

- One shared producer `PublicationState` mutex now covers cursor allocation,
  cumulative drop-count reads, bounded `try_send` and failed-send increments.
  This makes delivered stream order exactly cursor order; every ID gap is an
  actual bounded-queue publication loss. `atomic fetch_add` wraps cannot occur;
  ID exhaustion saturates into counted loss rather than reuse.
- `dropped_before` remains cumulative and is non-decreasing in delivered order.
  Trailing drops are still available through snapshot `events_dropped_total`.
  Queue-full behavior remains drop-on-full, not consumer backpressure. The
  implementation explicitly does not claim wait-free producers: the only short
  lock is producer serialization, and stdout/consumer I/O never runs under it.
  Poisoned producer locks recover their state rather than leaking unsafe access.
- New `concurrent_publishers_have_one_strict_cursor_order` releases eight OS
  producers through one ample sink and asserts every `(event_id, source_cursor)`
  pair is unique and exactly `1..=2048`, with no inversions, false gaps or drops.
- New `full_queue_counts_loss_without_waiting_for_consumer` uses a two-entry queue
  and a delayed consumer. Eight initial emits complete before the reader wakes,
  six are counted as loss, and a final sentinel carries `dropped_before = 6`.
  It proves nonblocking emit, unique cursors, cumulative trailing drop values and
  `delivered + dropped == emitted`.

The original regression failed before this fix (`/tmp/opencode/LP2-b5-red-02.log`);
the regressions now pass (`/tmp/opencode/LP2-b5-green-01.log`). Final checks used
`nix develop` with `CARGO_BUILD_JOBS=2`:

- `cargo test -p spc-cpu-miner --locked`: 30 passed,
  `/tmp/opencode/LP2-b5-final-verification-02.log`;
- fmt/clippy: pass, same log;
- `cargo test --workspace --locked`: 459 recorded passed, 0 failed, same log.

No envelope field names or shapes changed. PROTOCOL.md now states the precise
strict stdout cursor ordering, exact-gap loss, cumulative monotone drop count and
producer-serialization (not wait-free) guarantees. Event payload structure did
not change, so a new node smoke was not necessary for this queue-ordering-only
revision; actual concurrent regression results are above rather than claims from
an external probe.

## Re-review manifest

The canonical `base` + `files` manifest is
`/tmp/opencode/lab-lp2-review-manifest.json` (27 files, recipe per Fable).
See the manifest's `snapshot_sha256` for the current freeze after this report is
included. Regenerate if any reviewed file changes; no Fable approval is presumed
from this report alone.
