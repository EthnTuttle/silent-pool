# L1 native SV2 conformance implementation

Date: 2026-11-05
Packet: L1 (`lab-sv2-conformance`, base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`)
State: R1-R3 rework and tests complete; ready for Fable re-review. No commit made.

## Scope and result

Implemented the reviewed native SV2 prerequisites rather than lab-only shims:

- `(prev_hash, nbits)` activation sends a future job followed by
  `SetNewPrevHash`; other same-key changes send a nonfuture job without
  another prevhash. A same-prevhash nbits change therefore cannot leave miners
  hashing stale header bits. Ordinary curtime-only refreshes preserve the
  complete immutable issued job. The explicit v1 stale policy is
  latest-job-only: every other job id gets `stale-share`, and SV1 republishes
  with `clean_jobs = true`.
- Targets are LE256. Effective target changes emit `SetTarget` before the job,
  including the first job after an early open. Jobs capture targets at
  activation/receipt; active jobs are not retroactively retargeted.
- Coinbase Merkle paths are deepest-first in the producer and every pool/agent
  consumer, including SV1. Independent multi-level checks use `channels_sv2`
  8.0.0 and separately calculated roots.
- One connection has one extended channel and one identity. Channel ids and
  front-namespaced prefixes are deterministic per front process. Negotiated
  minimum sizes and maximum targets are checked; submissions must carry exactly
  four miner-controlled extranonce bytes (the pool prefix is added only by the
  pool).
- Bits and version participate in job identity/publication; ordinary curtime
  does not. Ignored time refreshes cannot silently change the issued
  `min_ntime`. The front also reconstructs coinbase, Merkle root and header
  hash before credit; ingress-supplied hashes and wrong extranonce lengths are
  insufficient.
- Fronts require a configured matching Noise authority pair. Agents pin the
  configured public key; restart and wrong-pin tests pass.
- Native SV2 success replies wait for the correlated front outcome and carry
  the actual credited difficulty for that job generation. Queue insertion alone
  cannot produce success. Replays remain explicit errors and do not enqueue
  duplicate work.

No share-work-unit, `N x d` window, payout, denomination or Ark boundary change
was made. L3 remains untouched.

## Test-first and independent evidence

Before fixes, these newly added tests failed for the intended reasons (logs in
`/tmp/opencode`):

- `coinbase_branch_is_deepest_first_upstream_vector` rejected the previous
  topmost-first path.
- `announced_target_is_the_pool_share_target` rejected big-endian wire bytes.
- `header_only_template_changes_bump_the_job` showed a bits-only update did not
  change the job id.

R1-R3 also started red against snapshot `c3bad8d0`:

- `/tmp/opencode/l1-r1-red-curtime.log`: curtime + 1 replaced the issued job.
- `/tmp/opencode/l1-r2-red-nbits.log`: same-prevhash nbits produced `Active`
  instead of the required `Future` activation.
- `/tmp/opencode/l1-r3-red-clean.log`: an ordinary same-tip job was not clean
  under the latest-only SV1 policy.

`bins/spc-pool/tests/sv2_conformance.rs` feeds wire messages into upstream
`channels_sv2::client::extended::ExtendedChannel`, not the in-repo agent's job
state. It verifies initial future-job activation, ordinary same-key updates,
future+prevhash activation on a same-prevhash nbits change, upstream
`get_chain_tip().nbits()`, target updates before future/nonfuture jobs, old
active-target preservation until replacement, and independent compact-target
decoding. The agent adds the complementary case `SetTarget` after a future job
was received but before activation versus after activation. A second wire test
uses two non-coinbase transactions: it checks both branch levels and accepts a
real share through upstream `ExtendedChannel::validate_share`. SV1 has a
separate two-level PoW vector.

Final commands/results:

| Command | Result |
|---|---|
| `nix develop -c cargo fmt --all -- --check` | Pass |
| `CARGO_BUILD_JOBS=2 nix develop -c cargo test --workspace` | Pass after R1-R3: 444 recorded passed, 0 failed (`/tmp/opencode/l1-r123-workspace-tests.log`) |
| `CARGO_BUILD_JOBS=2 nix develop -c cargo test -p spc-pool` | Pass: 208 pool library + all pool integration tests (`/tmp/opencode/l1-r123-pool-tests.log`) |
| `CARGO_BUILD_JOBS=2 nix develop -c cargo test -p spc-pool --test sv2_conformance` | Pass: 4/4 independent upstream-state/negotiation/extranonce/real-merkle-share tests (`/tmp/opencode/l1-r123-conformance-green.log`) |
| `CARGO_BUILD_JOBS=2 nix develop -c cargo test -p spc-agent` | Pass: all agent tests, including 39 library and 2 client e2e (`/tmp/opencode/l1-r123-agent-tests.log`) |
| `CARGO_BUILD_JOBS=2 nix develop -c cargo clippy -p spc-pool -p spc-agent --all-targets -- -D warnings` | Pass (`/tmp/opencode/l1-r123-clippy.log`) |
| `CARGO_BUILD_JOBS=2 nix develop -c ./testing/regtest-e2e.sh` | Pass after the SV1 clean-jobs change: `OK: regtest e2e passed` (`/tmp/opencode/l1-r123-sv1-regtest.log`) |
| R1 curtime, R2 nbits activation, R3 clean-jobs and N1 redaction focused tests | Pass (`/tmp/opencode/l1-r{1,2,3}-green-*`, `/tmp/opencode/l1-n1-green-redaction.log`) |

The final workspace run includes 208 pool library tests, 39 agent library tests,
SV1/SV2 integration tests and the independent conformance tests. Nix emitted
pre-existing global cargo-cache permission/auto-clean warnings in this
environment; the commands themselves exited successfully.

## Fable R1-R3 dispositions

- **R1 - done.** Removed curtime from both `job_changed` and `Published`.
  Curtime-only refresh regression asserts full equality with the issued job,
  including its original `min_ntime`; bits/version still republish. Existing
  +/-7200-second ntime validation is unchanged, so reasonable rolling remains
  available without rewriting validation inputs every second.
- **R2 - done.** Channel activation state now keys on `(prev_hash, nbits)`.
  A valid same-prevhash nbits change sends `SetTarget`, future job and
  `SetNewPrevHash`. The independent test asserts the upstream chain tip's
  activated header bits, not just the target. This also applies to default
  minimum-difficulty/RPC behavior on regtest; it is not restricted to testnet.
- **R3 - done.** Restored `clean_jobs = true` for every SV1 republication,
  matching latest-only submission rejection. No historical-job acceptance was
  added to SV1 or SV2.
- **N1 - done.** `RedactedSecret` prevents accidental `Pool`/`Config` debug
  disclosure of the inline authority secret; a test asserts no fixture secret
  appears in formatted debug output.
- **N10 - done.** Added the two-transaction ExtendedChannel wire test. It
  independently checks both branch levels and accepts a real share via
  upstream `validate_share`.

## Changed files

### Protocol and role code

- `bins/spc-pool/src/sv2_server.rs`: job/future activation, LE256 `SetTarget`,
  channel allocation/negotiation, exact extranonce checking, deepest-first
  hashing, stable authority injection, correlated front outcomes, replay/lag
  handling and unit tests.
- `bins/spc-pool/src/front.rs`: bits/version job identity, immutable issued
  min-ntime, `clean_jobs = true`, front coinbase/header revalidation and
  regression tests.
- `bins/spc-pool/src/main.rs`: authority/front-id wiring, publication key with
  bits/version but no ordinary curtime, correlated outcome resolution before
  block-submission handling.
- `bins/spc-pool/src/block.rs`, `sv1.rs`: deepest-first producer/consumers;
  independent multi-level SV1/SV2 vectors.
- `bins/spc-agent/src/sv2_client.rs`: authenticated connect, future/active job
  state, per-job target capture, LE256 decoding, exact miner extranonce,
  deepest-first hashing and correlated `ShareResult` routing.
- `bins/spc-agent/src/translator.rs`: authority pin and share-result handling.
- `bins/spc-agent/src/config.rs`, `jdc.rs`: authority parsing/validation and
  valid fixture key.

### Tests and support

- `bins/spc-pool/tests/sv2_conformance.rs` (new independent state-machine,
  target-lifecycle and negotiation coverage).
- `bins/spc-pool/tests/e2e_sv2.rs`, `bins/spc-agent/tests/e2e_sv2_client.rs`,
  `e2e_translator.rs` (protocol order, replay, real share outcomes and Noise
  pinning).
- `bins/spc-pool/src/config.rs`, `api_backend.rs`: authority validation,
  redacted secret `Debug` output and test fixtures.
- `bins/spc-pool/Cargo.toml`, `Cargo.lock`: `channels_sv2 = 8.0.0` is a pool
  **dev-dependency only**. There are no production dependency/version bumps;
  the lockfile change only adds it to the existing pool package's dependency
  list.
- `config/spc-pool.example.toml`, `config/spc-agent.example.toml`: matching
  throwaway authority pair and pin.
- `testing/regtest-e2e.sh`, `regtest-rbitcoin-e2e.sh`,
  `regtest-rbitcoin-reorg-e2e.sh`, `regtest-services-e2e.sh`: the same valid
  throwaway pair so the real front passes fail-closed startup.
- `docs/decisions/2026-11-05-sv2-native-conformance.md` and
  `docs/changelog/2026-11-05-sv2-native-conformance.md`: normative policy and
  merge-ready changelog fragment.

## Explicit boundaries / remaining work

1. **Latest-only stale policy.** Older same-tip jobs are rejected by design;
   accepting a bounded same-tip history would require coordinated front,
   distribution and slice-generation work and is not claimed. Upstream
   `channels_sv2` may retain such jobs as a client, but the pool returns its
   documented `stale-share` error.
2. **SV1 acknowledgement.** The SV1 boolean remains an ingress
   acknowledgement, not correlated accounting credit. Only native SV2 results
   are upgraded here. Do not use the SV1 boolean as proof of credit, block
   acceptance or settlement without separate observations.
3. **Other Noise interfaces.** Miner-facing MP authority is provisioned and
   pinned. JDS and node-TDP authority provisioning are separate interfaces;
   this packet does not add a JDS pin or alter the owned TDP packet.
   Key-generation/rotation tooling and a secret-file storage policy are
   operational follow-up; examples deliberately use a published throwaway
   pair and must be replaced for real deployments.
4. **Persistent allocation across restart.** Prefixes are unique for a front
   process lifetime. Restart issues new process-start jobs; no durable
   cross-restart prefix allocator is claimed. This does not alter front RAM-only
   state or replay-window policy.
5. **Node-side multi-transaction acceptance.** The new ExtendedChannel wire
   test covers a real share with two branch levels, and the real SV1 regtest
   smoke passed. A new rbitcoin run with a non-empty multi-transaction template
   through the complete node path was not added; the TDP merge note below is
   required to preserve the corrected order. L2's node profile and the
   persistent CPU-miner/lab packets remain separate.
6. **No work-unit semantics.** CPU-scale non-block share scheduling and the
   work/window review remain L3/follow-up. `new_shares_sum` uses only the
   existing credited-`d` policy for the accepted job.

## Deferred nonblocking integration gates (LP1/LP2)

These remain deliberate boundaries, not silently fixed scope creep:

1. A native SV2 miner against a real node through `main.rs`, including
   non-block shares and the actual correlated `SubmitSharesSuccess` /
   `new_shares_sum`, is still unrun. The regtest smoke here is SV1-only, and
   smaller integration tests resolve outcomes directly. N7 therefore remains
   an LP1/LP2 acceptance gate.
2. Inline response waiting allows one in-flight share per connection. At the
   10-second timeout the miner receives `invalid-share`, while the front may
   later accept the same share; that result is **unknown/ambiguous**, not a
   rejection or proof of credit. LP1/LP2 needs out-of-line response handling
   and reconciliation (N5), together with a test of the production wiring.
3. `main.rs` resolves every ingress share against the SV2 outcome hub. A random
   SV1 channel id can theoretically collide with a live SV2
   `(channel_id, sequence)` entry. Add an ingress-source tag or filter only SV2
   allocated ids before mixed-ingress production use (N3). `credited_work` is
   also recomputed from the same job policy rather than returned as the exact
   booked `d`; return the booked value in the production-outcome follow-up
   (N6), although both derivations match today.
4. A pre-template open announces `0xff..ff`, so only an all-ones `max_target`
   is accepted before a job exists (N4). Either document this explicitly or
   floor the placeholder with the configured share policy. A job published
   between subscription and open can also arrive twice with the same id (N9);
   the agent currently supports one future job and drops a mismatched future
   id on prevhash (N11). These are compatibility follow-ups, not new acceptance
   claims.
5. Valid SV1 layouts other than 4+4 bytes make the fixed 4+4 SV2 layout fail
   at open rather than at startup (N8). Add a combined listener-layout startup
   validation or negotiated sizing before advertising those combinations.
6. Inline secret storage is now debug-redacted, but file-backed secret storage,
   mode checks, key generation/rotation, JDS/node-TDP authority provisioning,
   and the stale `docs/OPERATIONS.md` RAM-only row remain follow-ups (N2).
7. Non-empty multi-transaction acceptance at an actual node, the `sv2-tp`
   reversal merge fix, SPEC/OPERATIONS integration, latest-only-history
   reconsideration, work units/windows and the persistent miner/lab scenarios
   remain as listed below. SV1 submit success remains ingress-only.

## Shared-file and integration merge notes

- `bins/spc-pool/src/config.rs`: retain both this packet's
  `authority_secret_key` parsing/match validation and all `sv2-tp` fields. Add
  the valid test pair to any TP config fixture. Missing/mismatched authority
  must remain fatal for the front role.
- `bins/spc-pool/src/main.rs`: preserve TDP template-source construction and
  its separate TP authority parsing, while adding this packet's
  `.with_authority_secret_key(...)`, `.with_front_id(...)`, correlated outcome
  resolution, and `version`/`bits` publication-key fields (no curtime). Resolve
  an SV2 share immediately after `Front::on_share`, before block submission and
  before the next refresh select iteration.
- `sv2-tp` integration (`template_tp.rs` on that branch): TDP's path is already
  deepest-first. Around its template conversion (reviewed at 1ec1dd0^, about
  line 495), use `cached.merkle_path.clone()` rather than `.iter().rev()`.
  Update its corresponding expected-path tests. Do not retain the old reversal
  added for the previous pool order. Its `testing/regtest-rbitcoin-tp-e2e.sh`
  config heredoc also needs the valid throwaway pair.
- `Cargo.toml` / `Cargo.lock`: retain the pool dev-only `channels_sv2` entry;
  do not bump any upstream pin. Run the merge session's Nix vendor/flake gate
  against the combined manifest rather than assuming dependency equivalence.
- `config/*.example.toml`: combine rather than replace TP fields with the
  authority additions. `docs/SPEC.md` was not edited locally: splice the
  supplied changelog fragment at merge and update its config/protocol prose
  from the decision record. `docs/OPERATIONS.md` also needs its Noise-key row
  reconciled with configured persistent secret material (it currently says the
  authority exists only in front RAM).
- The four regtest scripts in this diff contain only public test credentials.
  Their other startup, storage and economics logic is unchanged.

All changes remain uncommitted in this topic worktree. Other worktrees and the
main checkout were not modified.
