# LP-2 native SV2 CPU miner: Fable review

Date: 2026-11-05
Reviewer: Claude Fable 5.1 (`amazon-bedrock/us.anthropic.claude-fable-5-1`).
Packet: `lab-cpu-miner` (TFF), uncommitted in
`/home/ethan/.local/share/opencode/worktree/ad8886/lab-cpu-miner`, base
`b0ab4ca34fda07c61a6335d05f598eaacc69678c`.
Contract: `lab-contracts/docs/REGTEST-LAB-IMPLEMENTATION.md` §5 and the LP-2
packet in §10, read together with the contract's §2 invariants and §8.1.

## Result

**REQUEST_CHANGES**, tied to snapshot
`af5313704b81ee20868d634dc19127c97687a8d89b541e00c9d8d105a120e635`
(27 files, all per-file SHA256 values verified, canonical recipe reproduced).

The miner is a real, independent SV2 state machine over real SHA256d work. The
target lifecycle, byte orders, Merkle fold, compact decode, pinned Noise
authority, worker partitioning, reply correlation and the `accounted: unknown`
discipline are implemented as the contract describes, and the L1 + Bitcoin Core
smoke is genuine legacy-policy interoperability evidence (verified
independently below). Four defects block approval. Each is a bounded code
change inside `bins/spc-cpu-miner/` with a bounded test; none is a redesign.
Items under "Non-blocking" may be folded into the same revision or deferred
with a note. No candidate file was edited by this review; the worktree's
`git status` and all 27 hashes are unchanged after the review ran.

## Snapshot and environment verification

```sh
cd /home/ethan/.local/share/opencode/worktree/ad8886/lab-cpu-miner
git rev-parse HEAD     # b0ab4ca34fda07c61a6335d05f598eaacc69678c
git status --short     # M Cargo.lock, M Cargo.toml, M flake.nix, ?? bins/spc-cpu-miner/, ?? docs/reviews/2026-11-05-lab-lp2-implementation.md
python3 - <<'EOF'
import json,hashlib
m=json.load(open('/tmp/opencode/lab-lp2-review-manifest.json'))
for f in m['files']:
    assert hashlib.sha256(open(f['path'],'rb').read()).hexdigest()==f['sha256'], f['path']
h=hashlib.sha256(json.dumps({'base':m['base'],'files':m['files']},sort_keys=True,separators=(',',':')).encode()).hexdigest()
assert h=='af5313704b81ee20868d634dc19127c97687a8d89b541e00c9d8d105a120e635'
print('manifest ok', len(m['files']))
EOF
```

Result: `manifest ok 27`. Re-run after all review commands: zero mismatches,
same snapshot hash.

Shared-file diffs are exactly as the implementation report states:
`Cargo.toml` adds one workspace member; `Cargo.lock` adds only the
`spc-cpu-miner` stanza (22 lines, `stratum-core 0.6.0`, `stratum-apps 0.8.0`,
`secp256k1 0.28.2` all already locked by `spc-pool`, no version or pin
change); `flake.nix` adds the package/overlay entries and a new vendor hash.

Implementer checks re-run by this review (all under `nix develop`,
`CARGO_BUILD_JOBS=2`):

| Command | Result | Log |
|---|---|---|
| `cargo test -p spc-cpu-miner --locked` | 24 passed, 0 failed (5 control_schema, 3 live_session, 8 sv2_session, 8 target_budget) | `/tmp/opencode/Fable-LP2-tests-01.log` |
| `cargo fmt -p spc-cpu-miner -- --check` | pass | `/tmp/opencode/Fable-LP2-fmt-clippy-01.log` |
| `cargo clippy -p spc-cpu-miner --all-targets --locked -- -D warnings` | pass | same |

## Nix packaging

The implementer's `nix build .#spc-cpu-miner --offline` failed only because the
flake's git source omits untracked files (`/tmp/opencode/LP2-nix-build-02.log`:
vendor/lockfile consistency passed, then
`failed to read /build/source/bins/spc-cpu-miner/Cargo.toml`). The report does
not claim a package build; that is correct.

To verify the `flake.nix` change without touching the worktree index, this
review built from an approved filtered path source (worktree copied with
`rsync -a --exclude target --exclude .git`, 4.1 MB):

```sh
rsync -a --exclude target --exclude .git --exclude 'spc_ark_client/test' \
  /home/ethan/.local/share/opencode/worktree/ad8886/lab-cpu-miner/ /tmp/opencode/Fable-LP2-src/
nix build "path:/tmp/opencode/Fable-LP2-src#spc-cpu-miner" --offline --max-jobs 1 --cores 8 \
  --out-link /tmp/opencode/Fable-LP2-nix-result --print-build-logs
# exit 0; /nix/store/h8lxs6c7w8vchyvragjvmdvbvyf74yjl-spc-cpu-miner-0.1.0
# bin/spc-cpu-miner sha256 77f7b6417aeddb2a116d64b68315677b53b7fb6f56561f39e0d06873995ca489
/tmp/opencode/Fable-LP2-nix-result/bin/spc-cpu-miner --config /tmp/opencode/Fable-LP2-empty.toml --check-config  # exit 0
```

Log: `/tmp/opencode/Fable-LP2-nix-build-01.log`. The vendor hash
`sha256-lagUfDAlqNmjgXo18xsRYT/ybDdtm+jz/ZjA4BjPmDs=` is correct for this
manifest's `Cargo.lock`; the `cargo-deps-vendor` consistency check passed and
the release build completed. Packaging is therefore **not** a defect of this
packet. Two conditions remain for the merge session, not for the implementer:

1. `nix build .#spc-cpu-miner` from the git source only succeeds once the new
   crate is tracked (`git add -N bins/spc-cpu-miner` in the topic worktree, or
   the merge commit). Record that build, not this path-source one, as the merge
   evidence.
2. The vendor hash must be recomputed for the combined L1/LP-1/LP-2/LP-3
   `Cargo.lock`; the implementation report already says so.

## Independent verification of the L1 + node smoke

Artifacts: `/tmp/opencode/LP2-main-integration-xptd5vo2/` (`miner.jsonl`
sha256 `8b3e19fb...2cb46`, `summary.json` sha256 `68df6ddc...ebcc`), driver
`/tmp/opencode/LP2-main-smoke-driver.py` (sha256 `eff4af69...b148`), log
`/tmp/opencode/LP2-main-integration-driver-10.log`.

What the artifacts show, recomputed by this review from `miner.jsonl` and
`bitcoind.log` without using the driver's assertions:

- 152 events, `event_id` 1..152 contiguous, `dropped_before` always 0; no
  event loss.
- Session 0 only; one `target_updated` (generation 1,
  `share_target_le = 00..00ffff7f`) before the first `job_activated`, i.e. the
  future job captured the target in effect at `SetNewPrevHash` activation.
- 62 `submitted` events, 62 distinct 80-byte headers, one job
  (`2925188636`), one `ntime`, one 4-byte local extranonce `00000000`
  (session 0, worker 0, counter 0), nonces disjoint. All 62 are
  `block_qualified` because the legacy regtest share target equals the network
  target; `summary.json` records `lp1_nonblock_share_claim: false`.
- Sequence 0: `sha256d(header)` display form
  `202871e012a186cd0fb742a2e768fa49fe4ffafaa248af8a0379c82570ae6e24`;
  `bitcoind.log` line 341 `UpdateTip: new best=202871e0... height=102`;
  header `prev_hash` bytes reverse to the height-101 tip `37e149fc...cb20`;
  header Merkle root `c6afd3e6...5018` equals the L1 `pool.log` line
  `share found a block; submitting merkle=c6afd3e6...`; re-assembling
  `coinbase_tx_prefix || extranonce_prefix || extranonce || coinbase_tx_suffix`
  from the `job_activated` payload and hashing reproduces that root (empty
  Merkle path).
- One `ingress_ack` (sequence 0, count 1), fifteen `ingress_rejected`
  (`stale-share`, sequences 1..15, after the pool replaced the job), and one
  `submissions_unknown` for 16..61 at local shutdown. Every `submitted` share
  carries `accounted: unknown`; the ack was never promoted. `jobs_invalidated`
  with reason `new-prev-hash` followed by activation of `2925188637` on the
  new tip, then a clean `stop` with return code 0.

Classification: this is **native SV2 legacy-policy interoperability with the
approved L1 executable plus a node-accepted block**. It is not evidence of a
non-block accepted share, of v2 work-unit credit, of multi-level Merkle paths
from a real template (the path was empty), of the protocol-code negative
matrix, or of accounting. Those remain LP-1/LP-4 integration gates and are
correctly listed as such in the implementation report.

Caveat for the artifact chain: the miner binary SHA256 recorded in the report
(`37a0dc4a...`) is no longer reproducible from `target/debug/` because the test
runs (the implementer's and this review's) rebuilt it; the artifacts and
driver remain intact. The L1 tree at `lab-sv2-conformance` was read-only for
this review as well.

## Code review

Read in full: `session.rs`, `connection.rs`, `worker.rs`, `hash.rs`,
`target.rs`, `budget.rs`, `miner.rs`, `control.rs`, `config.rs`, `report.rs`,
`main.rs`, `ids.rs`, all four test files, `PROTOCOL.md`, the fixtures and the
implementation report. Where a claim was not obvious from reading, a probe was
run against the candidate crate from an external crate in
`/tmp/opencode/Fable-LP2-probe/` (`tests/probe.rs`, log
`/tmp/opencode/Fable-LP2-probe-02.log`, 3 probes passed meaning the defect
reproduced). Probes depend on `spc-cpu-miner` by path and did not change it.

### Confirmed correct

- Independence: no `spc-pool`/`spc-agent` dependency; `ProtocolState` is its
  own state machine; only `stratum-core 0.6.0` / `stratum-apps 0.8.0` codecs,
  framing and Noise are reused (existing pins).
- Future jobs idle until a matching `SetNewPrevHash`; activation captures
  prevhash, `nbits`, the channel target and the generation at that moment.
  `SetTarget` mutates the channel target and generation only; an active job's
  `share_target` is never rewritten; an immediate same-tip job takes the
  current target. Unknown activation job, active-before-prevhash, duplicate job
  id, and wrong channel id fail closed (session teardown).
- Targets are inclusive LE256 via `Target::from_le_bytes` / `is_met_by`;
  `decode_compact_target` mirrors Core's `SetCompact` negative/overflow rules
  (sizes 33/34 allowed when the mantissa fits) and rejects zero.
- Merkle fold is `sha256d(root || sibling)` over the path in wire order
  (deepest first), verified against `bitcoin::merkle_tree::calculate_root` and
  the upstream block 100000 vector; header is version/prev/root/ntime/nbits/
  nonce little-endian, 80 bytes, hashed with real SHA256d per nonce.
- Authority: `connect_with_noise(stream, Some(pinned))` always; x-only hex,
  compressed SEC hex and key-utils base58check accepted; anything else is a
  config error. No unauthenticated path exists.
- Workers: `nonce = index + k * count` partitions are disjoint; extranonce
  byte 2 is the worker index and bytes 0..2 are the process-unique session
  slot, so reconnects never reuse a header; the counter rolls on exhaustion and
  closes rather than wrapping; `hold` keeps `min_ntime`, `roll` is capped;
  pause keeps the cursor; found shares go out before any pacing sleep; a full
  share queue blocks the hash thread (network backpressure), never the event
  path.
- Events are published with `try_send` on a bounded channel; drops are counted,
  the ID gap and `dropped_before` expose them; command results use
  backpressure on the control loop only. Snapshot keeps the newest 256
  submissions.
- Idempotency: same `command_id` + same fingerprint replays the stored result;
  different payload is `command_id_conflict`; multi-worker pause/resume checks
  all IDs before changing any; `switch_pool` validates the new config, then
  fully shuts the old connection down (task joined, workers joined) before
  the new handshake; `stop`/EOF/Ctrl-C close everything. Verified live with
  the built binary (`/tmp/opencode/Fable-LP2-schema-probe.err` empty, exit 0):
  unknown envelope field, empty `command_id`, empty id list, bad authority,
  duplicate worker ids, unknown worker, wrong version are all rejected.
- Reply correlation is keyed by `(session_slot, sequence_number)`;
  `SubmitSharesError` and per-share `SubmitSharesSuccess` are matched exactly;
  `transport_unknown` is emitted for everything unresolved at session end.
  `accounted` is hard-coded `unknown` and `candidate_submission` is `unknown`
  for block-qualified shares; nothing infers accounting or canonicality from an
  ACK.

### Blocking defects

**B1. Lowering a live rate budget creates an unbounded, uninterruptible
pacing sleep** (`budget.rs` `available_attempts`/`required_delay`,
`worker.rs` line 331-340).

Credit is `elapsed_active * hps_new - attempted_total`. After 60 s at
100 kH/s (6,000,000 attempts), `set_worker_budget` to 1,000 H/s yields a debt
of 5,940,000 attempts and `required_delay` of 5940 s, executed as a single
`std::thread::sleep` that checks neither `cancelled` nor a new budget. Probe 2
reproduced `required_delay = 5940.001s`. Consequences: the contract's
`set_worker_budget` live update stalls the worker for the whole history;
`stop`/`remove_connection`/`switch_pool` block in `WorkerHandle::join` for the
same time on a tokio worker thread. Fix (bounded): rebase the credit baseline
(`attempted`, `elapsed`) when the budget changes, and replace the raw sleep
with slices bounded to a few tens of milliseconds (or the existing condvar
`wait_timeout`) so `stop()` and budget changes wake the thread. Test:
`set_budget` 100k -> 1k after a simulated backlog yields a bounded delay;
`stop()` returns within 100 ms while a 1 H/s worker is pacing.

**B2. The local `NtimeTooHigh` bound rejects legitimate L1 same-tip jobs**
(`session.rs` lines 215-226).

An immediate job is rejected when
`min_ntime > prev.min_ntime + floor(local_seconds_since_prevhash)`. L1
(`lab-sv2-conformance/bins/spc-pool/src/front.rs` 699-708,
`sv2_server.rs` 578-600) issues an immediate job with
`min_ntime = template.curtime` whenever the tx set changes on the same
prevhash, and reuses the originally issued `ntime` in `SetNewPrevHash`
otherwise. Probe 1 shows `min_ntime = prev + 1` within the same local second
is rejected (`NtimeTooHigh { actual: 1700000001, maximum: 1700000000 }`),
which happens whenever the pool's second boundary falls between the two
templates; after a reconnect the prevhash carries the stale issued `ntime`,
so any later bump exceeds the bound by the whole gap and tears the session
down again. This is the LP-4 transaction-injection path. sv2-spec `d8bf5b8`
imposes no such upper bound. Fix (bounded): remove the elapsed-based ceiling
(a generous consensus-shaped sanity bound such as `prev.min_ntime + 7200 +
elapsed` is acceptable); keep the `NtimeRegression` check if desired. Test:
immediate jobs with `min_ntime = prev + 1` and `prev + 600` right after
activation activate successfully.

**B3. Batched `SubmitShares.Success` leaves falsely-unknown history and an
unbounded pending map** (`connection.rs` lines 472-494, 532-547).

Only `last_sequence_number` is removed from `outstanding`. Probe 3: a
spec-conformant batch (`last = 2`, `new_submits_accepted_count = 3`) marked
sequence 2 `ingress_ack` and left 0 and 1 as `submitted`; at session end they
were reported `transport_unknown` although the pool acknowledged them, and
`ingress_accepted_reported` (3) disagrees with the per-share history (1).
`outstanding` has no cap, so a batching pool grows it without bound, and a
Success whose `last_sequence_number` is already resolved is a session-ending
error (`share success has unknown sequence`). L1 acknowledges per share, so
the smoke did not hit this; the contract still requires correlated replies
and PROTOCOL.md §3 documents batch handling. Fix (bounded): resolve the
`new_submits_accepted_count` oldest outstanding sequences `<= last` that have
no error as `ingress_ack` (and, if the count does not match, mark them with an
explicit ambiguous state rather than guessing); cap `outstanding` with an
explicit eviction event. Test: the probe scenario yields three `ingress_ack`
(or one explicit ambiguous verdict) and no `transport_unknown`, and a
repeated/stale ack is handled without tearing the session down or is
documented as a deliberate fail-closed teardown.

**B4. Spurious `job_activated` re-emission and worker re-publish**
(`connection.rs` `apply_changes` lines 603-612).

Every `StateChanges` with an unchanged active job (future-job receipt,
`SetTarget`) re-emits `JobActivated` and republishes the same job with a new
assignment sequence. The smoke artifact shows it: event 81 duplicates event 5
for job `2925188636` (same generation) at the moment the future job arrived,
and the worker's `stale_cancelled` is 2 while the connection's is 1. No
duplicate header results (the cursor is not reset), but LP-3 will over-count
activations and the per-worker counter is not "actual". Fix (bounded): emit
and publish only when `changes.activated` is non-empty (a `SetTarget` needs no
republish because the active job keeps its captured target). Test: a
future-job receipt and a `SetTarget` after activation produce no
`job_activated` and leave the worker's `stale_cancelled` unchanged.

### Non-blocking

1. Payload fields are not strict. `snapshot` with
   `payload: {"unknown_payload_field": 1}` succeeded (live probe). Only the
   envelope is `deny_unknown_fields`; variant payloads cannot carry that
   attribute. Either move payloads into dedicated `deny_unknown_fields`
   structs or state in PROTOCOL.md §2 that payload strictness is not provided.
2. `saturated` documentation says "two consecutive 250 ms samples below 95%";
   `finish_sample` sets it from `sample_count >= 2 && current sample < 95%`,
   i.e. the previous sample need not have been low. Align doc or code.
3. `switch_pool` constructs new `WorkerHandle`s, so `attempted`, `elapsed` and
   the total-attempt budget restart from zero, while §5 says a total budget
   never resets on reconnect. Document that a switch is a new worker lifetime
   or carry the counters over.
4. Unused dependencies in `bins/spc-cpu-miner/Cargo.toml`: `rand`, `tracing`,
   `tracing-subscriber` (no references in `src/`).
5. No read-inactivity timeout on an open session. Under a silent partition
   (LP-4) the miner keeps hashing the last job until TCP gives up; shares
   block on the write. Not required by §5, but LP-4 should know.
6. Fail-closed teardown on any protocol anomaly (duplicate job id, unknown
   reply sequence, unknown activation) drives a reconnect loop with the
   configured backoff. Acceptable for the lab; make sure LP-3 surfaces
   `connection_state` `disconnected` details.
7. `block_qualified_submissions` counts shares, not distinct blocks; the smoke
   shows 62 for one block. Name or document accordingly.
8. The default future-job handling clears all other future jobs at
   activation. Conservative and spec-compatible; noted only so nobody reads it
   as a bug later.

## Integration conditions (outside LP-2, carried forward)

- LP-3 adoption (§8.1 item 6): the miner stops on stdin EOF by design
  (PROTOCOL.md §1), so a supervisor crash that closes the pipe stops every
  miner and nothing is left to adopt. This is compatible with the contract's
  "stdin/stdout protocol" wording but incompatible with adoption unless LP-3
  holds the pipe in a process that survives the supervisor, or a later LP-2
  revision adds a socket transport. Record it as an LP-3 design input; the
  miner does not claim a durable command journal.
- Non-block accepted shares, multiplier/target-credit changes and the
  protocol-code negative matrix (stale/duplicate/below-target) require LP-1
  and the integrated scenario run; the fake-peer tests here exercise the
  miner's own code only and are not interoperability evidence.
- Non-empty Merkle paths from a real multi-transaction template, multi-pool
  faults and retarget economics remain LP-4 gates.
- Merge: recompute the flake vendor hash for the combined lockfile and run
  `nix build .#spc-cpu-miner` from the tracked tree; keep every package entry
  on `flake.nix` conflict.

## Bounded requests to the implementer

1. Fix B1-B4 in `bins/spc-cpu-miner/src/{budget,worker,session,connection}.rs`
   with the tests named above (new cases in `tests/target_budget.rs`,
   `tests/sv2_session.rs`, `tests/live_session.rs`).
2. Decide item 1 (payload strictness) and item 3 (switch counters) and update
   PROTOCOL.md either way; bump nothing to version 2 unless a field meaning
   changes.
3. Re-run `cargo test -p spc-cpu-miner --locked`, `cargo clippy -p spc-cpu-miner
   --all-targets --locked -- -D warnings`, `cargo fmt -p spc-cpu-miner -- --check`
   and the L1 smoke driver; publish a new manifest. I will re-review against
   that manifest; the smoke need not be re-run if no `connection.rs`/`hash.rs`
   change alters the submitted header bytes, but B3/B4 touch `connection.rs`,
   so re-run it.

## Review artifacts

- `/tmp/opencode/Fable-LP2-tests-01.log` - candidate tests, 24/24.
- `/tmp/opencode/Fable-LP2-fmt-clippy-01.log` - fmt and clippy.
- `/tmp/opencode/Fable-LP2-nix-build-01.log`, `/tmp/opencode/Fable-LP2-nix-result`
  - path-source Nix package build.
- `/tmp/opencode/Fable-LP2-probe/tests/probe.rs`,
  `/tmp/opencode/Fable-LP2-probe-02.log` - B1/B2/B3 reproductions.
- `/tmp/opencode/Fable-LP2-empty.toml`, `/tmp/opencode/Fable-LP2-schema-probe.err`
  - control-schema live probe.

---

# Second pass: R1-B4 rework

Date: 2026-11-05
Reviewer: Claude Fable 5.1 (`amazon-bedrock/us.anthropic.claude-fable-5-1`).
Same worktree, same base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.

## Result

**REQUEST_CHANGES**, tied to snapshot
`bcb9b34280fb8d21b0f5d10d07af1a18a4e4de8cc8e148bc6c519d21d24849bb`
(27 files, all per-file SHA256 values verified, canonical recipe reproduced,
re-verified unchanged after every command below; `git status` unchanged:
`M Cargo.lock`, `M Cargo.toml`, `M flake.nix`, `?? bins/spc-cpu-miner/`,
`?? docs/reviews/2026-11-05-lab-lp2-implementation.md`).

B1, B2 and B4 are resolved and verified. B3 is resolved for consistent
batches, duplicate ACKs, bounded pending state and honest counters, but the
batch-resolution rule still selects shares by guess in one reachable case
(B3b below). That is one bounded change in `SubmissionTracker::complete_success`
plus a small event-contract clarification; everything else in this packet is
approved as it stands and should not be reworked. No candidate file was
edited by this review.

## What changed since `af531370`

12 files changed, none added or removed; the shared build files
(`Cargo.toml`, `Cargo.lock`, `flake.nix`) are byte-identical to the first
pass. Diffs read in full: `budget.rs`, `worker.rs`, `session.rs`,
`connection.rs`, `report.rs`, `PROTOCOL.md`, `fixtures/events.jsonl`, the four
test files and the implementation report.

## Verification re-run

```sh
cd /home/ethan/.local/share/opencode/worktree/ad8886/lab-cpu-miner
CARGO_BUILD_JOBS=2 nix develop -c sh -c 'cargo test -p spc-cpu-miner --locked && cargo fmt -p spc-cpu-miner -- --check && cargo clippy -p spc-cpu-miner --all-targets --locked -- -D warnings'
# 28 passed (5 control_schema, 6 live_session, 8 sv2_session, 9 target_budget), fmt ok, clippy ok
# log: /tmp/opencode/Fable-LP2-r2-tests-01.log
```

Nix path-source build re-run on the revised source (same vendor hash, same
filtered rsync copy as the first pass):

```sh
nix build "path:/tmp/opencode/Fable-LP2-src#spc-cpu-miner" --offline --max-jobs 1 --cores 8 \
  --out-link /tmp/opencode/Fable-LP2-r2-nix-result --print-build-logs
# exit 0; /nix/store/275cqllsai3p0dcqxk54hckw3xvpp049-spc-cpu-miner-0.1.0
# bin/spc-cpu-miner sha256 03d04a65b8ca5cbac8e8060e7a55de5f3968fe6e482f0c7e9d6e0b023c5519a8
# --check-config on an empty-connections config: exit 0
# log: /tmp/opencode/Fable-LP2-r2-nix-build-01.log
```

The git-source `nix build .#spc-cpu-miner` and the combined vendor hash remain
merge-session conditions exactly as stated in the first pass; neither is a
defect of this packet.

Probes against the revised crate (external crate
`/tmp/opencode/Fable-LP2-probe/tests/probe2.rs`, log
`/tmp/opencode/Fable-LP2-r2-probe-01.log`):

```text
B1 required_delay after 100k->1k rebase = 1ms, lifetime attempted = 6000000
B2 same-tip +1s: Ok(1)
B3 ambiguous (pending 0..3, success last=3 count=2): result_ok=true acked=[0, 3] still_pending=[1, 2]
B3 n=1 last=2 with 0,1 pending: acked=[2] pending=[0, 1]
B3 late verdict for evicted seq 0: success=Some("share success has unknown sequence 0") error=Some("share error has unknown sequence 0")
```

## Rerun L1 + node smoke, verified independently

Artifacts `/tmp/opencode/LP2-main-integration-j1mitlwz/` (`miner.jsonl` sha256
`ee337eb7...c247`, `summary.json` sha256 `7e7f0721...e89b`), driver log
`/tmp/opencode/LP2-rework-main-integration-driver-02.log` (exit 0). Recomputed
from the artifacts, not from the driver's assertions:

- 597 events, `event_id` contiguous 1..597, `dropped_before` always 0.
- Exactly two `job_activated` events (jobs `2930557372`, then `2930557373`
  after `jobs_invalidated` reason `new-prev-hash`); no duplicate activation
  although the pool sent a future job and a `SetTarget`. Worker
  `stale_cancelled` 1 equals the connection counter. B4 holds on the real path.
- 249 `submitted` shares, 249 distinct headers, one job, one session; every
  header's SHA256d equals `hash_le` and is `<=` `share_target_le` as LE256.
- Sequence 0 (nonce 1) hashes to `756225dbade89f54f26007f6191285f748f8a69e9e884276f51fbaa6bc2f6b4f`;
  `bitcoind.log` line 341 `UpdateTip ... height=102` names the same hash; L1
  `pool.log` reports `share found a block; submitting merkle=4c7e0088...`.
- One `ingress_ack` (sequence 0, count 1), 90 `ingress_rejected`
  (`stale-share`), one `submissions_unknown` for 91..248 at local stop; all
  shares `accounted: unknown`; return code 0.

Classification is unchanged: native SV2 legacy-policy interoperability with
the approved L1 executable plus a node-accepted block. Not a non-block share,
not v2 credit, not the negative-code matrix, not a non-empty Merkle path from
a real template, not multi-pool fault behaviour. Those remain LP-1/LP-4 gates.

## Disposition of the first-pass items

**B1 - resolved.** `Pacer` keeps lifetime `attempted` for total caps and
rebases a rate epoch `(attempted, elapsed)` whenever `hashes_per_second`
changes (`budget.rs` `rebase_budget`); `required_delay` is charged only within
the epoch. Probe: 1 ms after a 60 s / 6,000,000-attempt history lowered to
1 kH/s, lifetime attempts preserved. `worker.rs` replaces `thread::sleep` with
`wait_duration`, a `wait_timeout` on the existing wake condvar that `stop`,
`set_budget`, `set_paused`, `publish` and `clear_work` all notify; the live
test joins a 1 H/s worker in under 100 ms. The saturation sampler now counts
consecutive low samples and resets otherwise, matching the documented
"two consecutive" rule.

Stop under submission backpressure (asked for explicitly): if a hash thread
is blocked in `found.blocking_send` because the 1024-share queue is full and
the connection task is itself blocked in the submit `write_message` (socket
backpressure), `shutdown.changed()` cannot fire because the write is awaited
inside a select arm, so `ConnectionHandle::shutdown` waits its 5 s timeout and
aborts the task; dropping the task drops `found_rx`, `blocking_send` returns
`Err`, the worker calls `stop()` on itself, and `worker.join()` returns. No
deadlock; worst case about 5 s. Source-only finding, not re-tested under a
fault; see non-blocking item 2 for a cheaper path.

**B2 - resolved.** The `NtimeTooHigh` variant and the local elapsed-time
ceiling are gone from `session.rs`; `NtimeRegression` remains. `sv2_session`
activates same-tip jobs at `prev + 1` and `prev + 600`; probe confirms `+1`
within the same local second.

**B4 - resolved.** `apply_changes` iterates `changes.activated` only; the
live test asserts exactly one `JobActivated` for job 77 after a future-job
receipt and a `SetTarget`, and zero worker `stale_cancelled`; the real smoke
shows the same.

**B3 - partially resolved; one blocking remainder (B3b).**

Resolved: `SubmissionTracker` bounds pending shares at 1024 with explicit
`submissions_unknown` reason `outstanding-cap-evicted`; the 256-entry known
history reports `submissions_evicted` for resolved shares; a consistent batch
(`last = 2`, `count = 3`) marks 0, 1, 2 `ingress_ack`; an exact repeat and an
empty repeat are no-ops; `ingress_accepted_reported` counts newly resolved
shares only; `new_shares_sum` is carried on the event and never apportioned to
jobs; a count larger than the available prefix fails closed; already-resolved
shares never become `transport_unknown`; a late verdict for an evicted
sequence is an explicit protocol error rather than a silent miss.

**B3b (blocking). The prefix rule still guesses when more un-errored pending
sequences precede `last_sequence_number` than `count - 1`.**
`complete_success` takes `pending.range(..last).take(count - 1)`, i.e. the
oldest `count - 1`, and only fails when fewer exist. Probe: pending
`{0,1,2,3}`, `Success(last = 3, count = 2)` returns `acked = [0, 3]` and leaves
1 and 2 pending. Under sv2-spec `d8bf5b8` 5.3.17 (`last_sequence_number`:
"most recent sequence number with a correct result";
`new_submits_accepted_count`: "count of new submits acknowledged within this
batch") a pool that processes in order and reports errors as they occur must
have every un-errored pending sequence below `last` inside this batch, so
`count - 1 == pending_before_last` is the only consistent shape; anything
smaller means the pool processed out of order, dropped submissions, or has
error replies still in flight, and in all three the identity of the accepted
shares is unknown. Selecting the oldest in that situation is a guess, and it is
the less defensible guess (in-order processing makes the newest preceding
shares the more likely members). The result is exactly the category §5.4 and
the first pass forbid: a share marked `ingress_ack` that the pool never
acknowledged. L1 replies per share (`count = 1`, prefix length 0), so the
smoke cannot reach this branch; the rule is nonetheless the published
PROTOCOL.md §3 contract and the first-pass requirement was "not a guessed
assignment".

Required change (bounded, `connection.rs` `complete_success` and one
PROTOCOL.md sentence): when `count >= 2` and the number of pending sequences
below `last` differs from `count - 1`, do not select. Either (a) fail closed
exactly as the existing "inconsistent batch count" branch does, or (b) resolve
only `last` (it is explicitly named correct) and leave the preceding shares
pending, emitting the `ingress_ack` with an explicit `ambiguous_prefix: true`
or reason so LP-3 does not read the count as per-share evidence. `count = 1`
stays as implemented (no selection is made). Test to add to
`tracker_caps_pending_and_rejects_inconsistent_batches`: pending `{0,1,2,3}`,
`complete_success(3, 2)` yields an error (a) or `[3]` with the ambiguity flag
(b), never `[0, 3]`.

Fold into the same change (same code area, not a separate round): the known
history cap (256) is below the pending cap (1024), so a pending share evicted
from the snapshot window is reported as `submissions_unknown` reason
`known-history-cap-evicted` while it remains pending in the tracker and can
still be acknowledged. For a per-share pool the later `ingress_ack` event
names the sequence and corrects the record; for a batched ACK the event does
not enumerate the sequences it resolved, so the consumer's last word on that
share stays "unknown" although the pool accepted it, and only the aggregate
counter moves. The rerun smoke reached 249 pending shares in a few
milliseconds at legacy regtest difficulty, so a 256+ backlog is realistic.
Pick one: raise the history cap to the pending cap, or emit a distinct event
(not `submissions_unknown`) for pending shares leaving the snapshot window, or
add `resolved_sequence_numbers` to `ingress_ack` (additive, version 1 stays).
Any of the three is acceptable; the current combination mislabels a tracked
share as unknown.

## Non-blocking

1. Lost-wakeup window in `wait_duration`: notifiers call `notify_all` without
   holding the wake mutex and the waiter does not re-check `cancelled`/budget
   after taking the lock, so a `stop()` that lands between the loop-head check
   and `wait_timeout` is observed only when the wait expires. Bounded by the
   pacing interval (at most `1 / hashes_per_second` seconds, 20 ms in
   `wait_work`), so the 100 ms stop test could flake at 1 H/s. Standard fix:
   take the wake mutex in the notifiers or re-check the flags under the lock
   before waiting.
2. Submit writes are awaited inside the `found_rx` select arm, so a stop under
   socket backpressure costs the 5 s abort timeout. Wrapping the write in a
   select with `shutdown.changed()` or a short timeout would make stop prompt
   in that fault too. Not required by §5.
3. A late verdict for a sequence already evicted by the 1024 bound tears the
   session down (`share success has unknown sequence`). Fail-closed and
   documented, but in a sustained legacy-regtest flood it becomes a reconnect
   loop; remembering evicted sequences and reporting `late_verdict_for_evicted`
   would be gentler. LP-4 input.
4. PROTOCOL.md now states payload objects are not strict; the fixture and the
   envelope rule are consistent with the code. Accepted as documented.
5. Unused `rand`, `tracing`, `tracing-subscriber` declarations retained to
   avoid vendor churn; accepted, note for the merge session.
6. First-pass integration conditions stand unchanged: stdin-EOF stop versus
   LP-3 §8.1 adoption; LP-1 non-block/credit and the protocol-code matrix;
   non-empty real Merkle paths, multi-pool faults, retarget; tracked-tree
   `nix build` and combined vendor hash at merge.

## Bounded request

1. Fix B3b in `SubmissionTracker::complete_success` (option (a) or (b)) and
   choose one of the three history/pending alignments; update the PROTOCOL.md
   §3 `ingress_ack`/`submissions_unknown` sentences accordingly; add the
   tracker test case above.
2. Re-run `cargo test -p spc-cpu-miner --locked`, fmt, clippy; publish a new
   manifest. The L1 smoke need not be re-run for this change unless the
   `ingress_ack` event shape changes (if `resolved_sequence_numbers` is added,
   re-run it so the artifact matches the published shape).

With B3b closed this packet is approvable as bounded LP-2; nothing else is
outstanding on the miner side.

## Second-pass artifacts

- `/tmp/opencode/Fable-LP2-r2-tests-01.log` - 28/28, fmt, clippy.
- `/tmp/opencode/Fable-LP2-r2-nix-build-01.log`, `/tmp/opencode/Fable-LP2-r2-nix-result`.
- `/tmp/opencode/Fable-LP2-probe/tests/probe2.rs`, `/tmp/opencode/Fable-LP2-r2-probe-01.log`.

---

# Third pass: R2-B3b rework and the event-cursor question

Date: 2026-11-05
Reviewer: Claude Fable 5.1 (`amazon-bedrock/us.anthropic.claude-fable-5-1`).
Same worktree, same base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.

## Result

**REQUEST_CHANGES**, tied to snapshot
`6e2eb2aa6537187bc78fb9ba822f0a564f895c4a3e8be26d85fc12ed3b146676`
(27 files, all per-file SHA256 values verified, canonical recipe reproduced,
re-verified unchanged after every command below; `git status` unchanged).

B3b is resolved as specified and verified by probe. B1, B2, B4 are stable.
The one remaining item is not from the earlier passes: the parent's concern
about `EventSink::emit` is confirmed by a concurrent probe, and it contradicts
the published PROTOCOL.md §3 cursor contract that LP-3 consumes. It is a
bounded change in `report.rs` (one lock around ID assignment and enqueue, or
an equivalent) plus one sentence in PROTOCOL.md; nothing else in the packet
needs to move. No candidate file was edited by this review.

## What changed since `bcb9b342`

7 files changed, none added or removed; shared build files byte-identical
to both earlier passes. Diffs read in full: `connection.rs`, `report.rs`,
`PROTOCOL.md`, `fixtures/events.jsonl`, `tests/live_session.rs`,
`tests/control_schema.rs`, the implementation report.

## Verification re-run

```sh
cd /home/ethan/.local/share/opencode/worktree/ad8886/lab-cpu-miner
CARGO_BUILD_JOBS=2 nix develop -c sh -c 'cargo test -p spc-cpu-miner --locked && cargo fmt -p spc-cpu-miner -- --check && cargo clippy -p spc-cpu-miner --all-targets --locked -- -D warnings'
# 28 passed (5 control_schema, 6 live_session, 8 sv2_session, 9 target_budget), fmt ok, clippy ok
# log: /tmp/opencode/Fable-LP2-r3-tests-01.log
nix build "path:/tmp/opencode/Fable-LP2-src#spc-cpu-miner" --offline --max-jobs 1 --cores 8 --out-link /tmp/opencode/Fable-LP2-r3-nix-result --print-build-logs
# exit 0; /nix/store/miyi6pxs6d7mmn76f048f9b53wxm3s9g-spc-cpu-miner-0.1.0
# bin/spc-cpu-miner sha256 19b10e0a81f2c34d600b3c9acffa43a9ff76b25ea73f6269a4e4cc314b715326; --check-config exit 0
# log: /tmp/opencode/Fable-LP2-r3-nix-build-01.log
```

Tracked-tree `nix build` and the combined vendor hash remain merge-session
conditions, unchanged.

## B3b - resolved

`SubmissionTracker::complete_success` now requires, for
`new_submits_accepted_count >= 2`, that the number of no-error pending
sequences below `last_sequence_number` equals `count - 1`; otherwise it
returns `ambiguous batch prefix` and changes nothing. `count == 1` resolves
exactly the named sequence. Probe (`/tmp/opencode/Fable-LP2-probe/tests/probe3.rs`,
log `/tmp/opencode/Fable-LP2-r3-probe-01.log`):

```text
pending 0..3, success(3,2): Err("ambiguous batch prefix: 3 pending before 3, count 2"), pending unchanged
success(2,1): acked=[2], pending=[0,1,3]
error(1) then success(3,3): acked=[0,2,3], pending=[4]
retired 0: success(0,1) -> Ok(0 resolved) + late_verdict [0]; error(0) -> Ok(None) + late_verdict [0]
never issued: success(50,1) -> "names unsent sequence"; error(50) -> "unknown sequence"
retired set aged out (cap 2, 7 inserts): success(0,1) -> "unknown sequence" (fail closed); success(3,1) -> late verdict
```

History-only trims now emit `submissions_evicted`, never `submissions_unknown`,
and the real smoke exercised that path (below). Late verdicts for sequences
evicted from the 1024 pending bound are explicit `late_verdict` events; they
do not add unique-resolution credit and do not tear the session down; a
never-issued sequence still fails closed. The additive event is in the
fixture and PROTOCOL.md §3. The tracker test covers the ambiguous case
(`complete_success(3, 2)` on `{0,1,2,3}` is an error), the consistent case
with an explicit error, the late-verdict path and the history-only trim
labelling.

Minor, non-blocking: a late `SubmitSharesError` still increments the
`ingress_rejected` counter and emits `ingress_rejected` before the
`late_verdict`; PROTOCOL.md says late verdicts do not "recreate per-share
history or increment unique-resolution credit", which is true for ACKs. Say
that `ingress_rejected` is a reply counter, or skip the increment on a late
verdict.

## Rerun L1 + node smoke, verified independently

Artifacts `/tmp/opencode/LP2-main-integration-8qrnqxj6/` (`miner.jsonl`
sha256 `1a74d605...a635`, `summary.json` sha256 `1730fbcd...4e84`). Recomputed
from the artifacts: 632 events, `event_id` contiguous 1..632, zero
inversions, `dropped_before` always 0; exactly two `job_activated`; 262
`submitted` shares with 262 distinct headers, every SHA256d equal to `hash_le`
and `<=` the LE share target, all `accounted: unknown`; sequence 0 hashes to
`3c9616e40a3472b738f83f44ff8704c9618ddd057d1b67e50628392a1e4ce584`, which is
`bitcoind.log` `UpdateTip ... height=102`; L1 `pool.log` reports the block
submission; one `ingress_ack` (sequence 0), 87 `stale-share` rejections, six
`submissions_evicted` reason `known-history-cap-evicted` for sequences 0..5
(262 > 256, so the new labelling ran for real), one `submissions_unknown` at
local stop, exit 0. The run also shows a benign startup reconnect (first
TCP attempt refused before the pool listened, retry after 100 ms, session
slot 1), which is the reconnect path working against the real L1.

Classification unchanged: native SV2 legacy-policy interoperability with the
approved L1 executable plus a node-accepted block. Not non-block shares, not
v2 credit, not the protocol-code matrix, not a real multi-level Merkle path,
not multi-pool faults. Those remain LP-1/LP-4 gates.

## B5 (blocking, new): event IDs are not stream-monotonic under concurrent publishers

`report.rs` `EventSink::emit` does `next_id.fetch_add` (atomic), builds the
envelope with `dropped_before = dropped.load()`, then calls `try_send` with no
synchronisation between the ID assignment and the enqueue. Two publishers can
therefore take IDs `n` and `n+1` and enqueue them in the opposite order; the
single stdout writer preserves queue order, so stdout carries the inversion.

Probe (`eventsink_concurrent_ordering` in `probe3.rs`, log
`/tmp/opencode/Fable-LP2-r3-probe-02.log`), 8 OS threads emitting through
clones of one `EventSink`, one consumer:

```text
cap=1000000 emitted=80000 delivered=80000 dropped_counter=0 inversions=22209 max_backjump=109 dupes=0 id_gaps_without_drop=54747
cap=100000  emitted=160000 delivered=114866 dropped_counter=45134 inversions=29720 max_backjump=1080 dropped_before_monotone=false
cap=64      emitted=160000 delivered=8273 dropped_counter=151727 inversions=3 max_backjump=73 dropped_before_monotone=false
```

With zero drops the delivered stream has 22,209 adjacent inversions and
54,747 adjacent ID gaps. Invariants that do hold: every ID is either
delivered or counted in `dropped` (delivered + dropped == emitted in every
run), and no ID is duplicated. What fails is the published contract.
PROTOCOL.md §3 says `event_id`/`source_cursor` are "the same
process-monotonic integer assigned before publication" and that "the next
successful ID gap and its `dropped_before` expose the loss". In stream order
the cursor is not monotonic, a gap does not imply loss, and `dropped_before`
(a cumulative load taken at emit time) is not monotone in stream order
either. A consumer that treats gap as loss will record false losses; a
consumer that resumes from the highest seen cursor (contract §7.5) will skip
later-arriving lower IDs.

Reachability in the miner: publishers are the connection tasks (one per
connection, scheduled on the two runtime threads), every hash thread
(`worker_search_exhausted`) and the control task (`set_state`,
`connection_state`). A single-connection run cannot show it, and the three
smokes did not (all contiguous). Any N-identity configuration
(`fixtures/two-pools.toml` is one) can.

Required change (bounded, `report.rs` plus one PROTOCOL.md sentence), either:

(a) make ID assignment and enqueue one critical section: hold a `Mutex` across
`fetch_add` (or a plain counter under the same mutex), `dropped.load`, and
`try_send`; `try_send` never blocks, so the mining path stays non-blocking and
the lock is held for the enqueue only. Then stream order equals ID order,
a gap equals exactly the number of drops in between, and `dropped_before`
is monotone. Keep `dropped_before` cumulative (document it as cumulative so
LP-3 records deltas, which the parent has already told LP-3 to do); or

(b) move ID assignment into `write_output` (collector order), making
`event_id` a strictly sequential stdout line counter and `dropped_before`
the cumulative drop count observed at dequeue; the same guarantees follow.

Either way amend PROTOCOL.md §3 to state the guarantee precisely:
"`event_id` strictly increases in stdout order; a gap of k between adjacent
events means exactly k events were dropped between them; `dropped_before` is
the cumulative drop count and is non-decreasing". If the implementer instead
prefers to keep the unsynchronised path, the contract must say the opposite
plainly ("arrival may be out of ID order; a gap is not loss; use
`dropped_before` deltas only"), which weakens §7.5 cursor semantics for LP-3;
(a) is strongly preferred and is about ten lines.

Test to add: N threads emit through one sink into an ample channel; assert
delivered IDs are strictly increasing and, with a small channel, that each
adjacent gap equals the `dropped_before` delta.

## Non-blocking (carried, unchanged)

Lost-wakeup window in `wait_duration`; submit write not selectable against
shutdown (5 s abort path); unused dependency declarations; stdin-EOF versus
LP-3 adoption; LP-1 non-block/credit and the protocol-code matrix; real
multi-level Merkle paths, multi-pool faults, retarget; tracked-tree
`nix build` and combined vendor hash at merge; `resolved_sequence_numbers` on
`ingress_ack` would let a batch consumer resolve shares trimmed from the
256-entry history (per-share pools are unaffected).

## Bounded request

1. Fix B5 in `report.rs` (option (a) preferred), update the PROTOCOL.md §3
   sentence, add the concurrent ordering test.
2. Re-run `cargo test -p spc-cpu-miner --locked`, fmt, clippy; publish a new
   manifest. The L1 smoke does not need to be re-run for this change (it has
   a single publisher and the envelope shape does not change).

With B5 closed, this packet is approvable as bounded LP-2.

## Third-pass artifacts

- `/tmp/opencode/Fable-LP2-r3-tests-01.log` - 28/28, fmt, clippy.
- `/tmp/opencode/Fable-LP2-r3-nix-build-01.log`, `/tmp/opencode/Fable-LP2-r3-nix-result`.
- `/tmp/opencode/Fable-LP2-probe/tests/probe3.rs`,
  `/tmp/opencode/Fable-LP2-r3-probe-01.log` (B3b, late verdicts, first ordering run),
  `/tmp/opencode/Fable-LP2-r3-probe-02.log` (ordering with a no-drop channel).

---

# Fourth pass: R3-B5 publication ordering

Date: 2026-11-05
Reviewer: Claude Fable 5.1 (`amazon-bedrock/us.anthropic.claude-fable-5-1`).
Same worktree, same base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.

## Result

**APPROVE** (bounded LP-2), tied to snapshot
`5e27e9180e765e8049cbcccbcadc38b24c85d402505108f8f20c320f73961250`
(27 files, all per-file SHA256 values verified, canonical recipe reproduced,
re-verified unchanged after every command below; `git status` unchanged:
`M Cargo.lock`, `M Cargo.toml`, `M flake.nix`, `?? bins/spc-cpu-miner/`,
`?? docs/reviews/2026-11-05-lab-lp2-implementation.md`).

B5 is resolved; B1-B4 and B3b are unchanged since their verified fixes. No
candidate file was edited by this review. The approval covers the miner
packet as frozen by this manifest and nothing beyond it; the integration
conditions listed at the end are retained verbatim and are not satisfied by
this approval.

## What changed since `6e2eb2aa`

4 files changed, none added or removed: `src/report.rs`, `PROTOCOL.md`,
`tests/control_schema.rs`, the implementation report. Shared build files and
every other source file are byte-identical to the third pass, so the hashing,
protocol-state, worker, budget, connection and tracker code approved earlier
is exactly what was reviewed then. The L1 + Core smoke evidence from the
third pass (`/tmp/opencode/LP2-main-integration-8qrnqxj6/`) therefore still
describes this binary's hashing and submission path.

## B5 - resolved

`EventSink` now holds one `Mutex<PublicationState { next_id, dropped,
ids_exhausted }>` across ID assignment, the cumulative drop-count read, the
envelope construction, `try_send`, and the failed-send increment. Timestamps
and the `miner_id` clone are taken before the lock; no consumer or stdout I/O
runs under it; `try_send` is non-blocking so a full queue still drops rather
than waiting on the writer. A poisoned lock is recovered with `into_inner`
(publication stays coherent after a producer panic). At `u64::MAX` the sink
marks itself exhausted and counts every later publication as dropped; no ID
is reused. PROTOCOL.md §3 now states the guarantee precisely: IDs strictly
increase in stdout order, a gap of `k` means exactly `k` lost publication
attempts, `dropped_before` is cumulative and non-decreasing, the producer
lock is short and not wait-free, and queue-full still drops. No field shape
changed.

Verification:

```sh
cd /home/ethan/.local/share/opencode/worktree/ad8886/lab-cpu-miner
CARGO_BUILD_JOBS=2 nix develop -c sh -c 'cargo test -p spc-cpu-miner --locked && cargo fmt -p spc-cpu-miner -- --check && cargo clippy -p spc-cpu-miner --all-targets --locked -- -D warnings'
# 30 passed (7 control_schema incl. the two new ordering/loss tests, 6 live_session, 8 sv2_session, 9 target_budget), fmt ok, clippy ok
# log: /tmp/opencode/Fable-LP2-r4-tests-01.log
```

Independent concurrent probe re-run against the revised crate (same
`eventsink_concurrent_ordering` that demonstrated the defect in pass 3,
extended to check gap/delta exactness; `/tmp/opencode/Fable-LP2-probe/tests/probe3.rs`,
logs `/tmp/opencode/Fable-LP2-r4-probe-01.log`, `/tmp/opencode/Fable-LP2-r4-probe-02.log`):

```text
cap=64      emitted=160000 delivered=159697 dropped=303 inversions=0 dupes=0 gap_sum_delivered=303 trailing_drops=0 delta_mismatches=0 dropped_before_monotone=true
cap=100000  emitted=160000 delivered=160000 dropped=0   inversions=0 dupes=0 id_gaps_without_drop=0
cap=1000000 emitted=80000  delivered=80000  dropped=0   inversions=0 dupes=0 id_gaps_without_drop=0
```

Compared with pass 3 (22,209 inversions and 54,747 gaps with zero drops),
the stream is now strictly ordered under 8 concurrent OS-thread publishers,
every adjacent gap equals the `dropped_before` delta, the gaps sum to the
drop counter, and `delivered + dropped == emitted` in every run. The
candidate's own tests cover the same two properties (8 x 256 ordered
no-drop cursors; a 2-slot queue with 6 counted drops and `[1, 2, 9]`
delivered with `dropped_before` `[0, 0, 6]`).

Both the Nix path-source build (third pass,
`/nix/store/miyi6pxs6d7mmn76f048f9b53wxm3s9g-spc-cpu-miner-0.1.0`) and the
vendor hash are unaffected: `flake.nix` and `Cargo.lock` did not change and
`report.rs` adds no dependency. Not rebuilt this pass.

## Approval scope

Approved as bounded LP-2 per contract §5 and the LP-2 packet in §10:

- real SHA256d work over a conformant SV2 extended-channel session, with an
  independent protocol state machine on the existing `stratum-core 0.6.0` /
  `stratum-apps 0.8.0` pins and no `spc-pool`/`spc-agent` dependency;
- correct target lifecycle (future-job activation captures the channel
  target; `SetTarget` never rewrites an active job; same-tip jobs take the
  pool's `min_ntime` as authoritative), LE256 inclusive targets, consensus
  compact decode, deepest-first Merkle fold, pinned Noise authority with no
  fallback;
- disjoint multi-worker nonce/extranonce partitions, reconnect-unique
  extranonce, stable cursors across pause/job/reconnect, bounded batches,
  rate-epoch pacing with interruptible waits, lifetime attempt caps, honest
  `achieved_hps`/`saturated`;
- exact reply correlation keyed by `(session_slot, sequence_number)`;
  batched `Success` resolved only when the eligible prefix cardinality is
  exact, `count == 1` resolving exactly its named sequence, duplicate ACKs
  harmless, bounded pending state with explicit eviction, late verdicts
  explicit, never-issued sequences fail closed; `accounted` always
  `unknown`, `candidate_submission` never inferred from an ACK;
- idempotent, conflict-rejecting, atomically preconditioned control surface;
  strict envelope; stop/EOF/Ctrl-C shutdown with task and thread joins;
- bounded, non-blocking event publication with strictly ordered cursors and
  exact loss accounting (B5);
- verified native legacy-policy interoperability with the approved L1
  executable and a Bitcoin Core-accepted block (three independent reruns,
  each recomputed from artifacts in the passes above).

## Integration conditions retained (not satisfied by this approval)

1. LP-1: non-block accepted shares, v2 work-unit credit, multiplier/target
   credit changes, and the stale/duplicate/below-target protocol-code matrix
   against a real pool. The miner computes no credit and this review used
   no LP-1 evidence.
2. Node/template: a non-empty multi-transaction Merkle path from a real
   template; retarget economics (L2 profile).
3. LP-3: wrapping the JSONL process in the durable `LabCommand` journal and
   cursor store; the stdin-EOF stop semantic versus §8.1 adoption (hold the
   pipe in a surviving process or add a socket transport later); consume
   `submissions_evicted`, `late_verdict`, and `dropped_before` as cumulative
   (record deltas).
4. LP-4: multi-pool faults, sustained partitions (no read-inactivity timeout
   in the miner; a sustained legacy-regtest flood past the 1024 pending bound
   is fail-closed and may reconnect-loop), link latency/loss.
5. Merge: `nix build .#spc-cpu-miner` from the tracked tree (path-source
   build proven in passes 1-3 only); recompute the flake vendor hash for the
   combined L1/LP-1/LP-2/LP-3 `Cargo.lock`; keep every package entry on
   `flake.nix` conflict; the three unused dependency declarations may be
   dropped at merge.

Non-blocking notes carried from earlier passes stay as recorded (lost-wakeup
window bounded by `1/hps`, submit write not selectable against shutdown,
late `SubmitSharesError` bumps the `ingress_rejected` reply counter,
`resolved_sequence_numbers` on `ingress_ack` for batch consumers).

## Fourth-pass artifacts

- `/tmp/opencode/Fable-LP2-r4-tests-01.log` - 30/30, fmt, clippy.
- `/tmp/opencode/Fable-LP2-r4-probe-01.log`, `/tmp/opencode/Fable-LP2-r4-probe-02.log`
  - concurrent ordering and gap/delta exactness.
