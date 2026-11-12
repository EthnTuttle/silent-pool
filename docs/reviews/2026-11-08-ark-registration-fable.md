# Fable concrete-code review: AP-2 pool-side Ark registration, acknowledgment and destination wiring

Date: 2026-11-08
Reviewer: Fable (concrete-code review subagent)
Candidate: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-ark-registration`
Candidate report: `docs/reviews/2026-11-07-ark-registration-implementation.md`
(in the candidate worktree)
Disposition: **REQUEST_CHANGES** (ranked counterexamples in Section 4; each
has a minimal fix and the regression test that must accompany it)

Nothing in the candidate, the primary tree or any other worktree was edited.
No commits, no shared services, no node, no mining. One fresh, owned
PostgreSQL 17.11 cluster and one owned Redis 8.10.3 process were started
under `/tmp/opencode/AP2-fable/storage.*` for this review only and were
stopped afterwards (data directory truncated). This file is the only artifact
written into the primary tree. Scratch evidence is listed in Section 7.

## 1. Snapshot and delta verification

`/tmp/opencode/AP2-review-manifest.json` recomputed independently from the
worktree bytes (declared recipe: `SHA256(json.dumps(files, ensure_ascii=True,
separators=(',',':')))`, files path-sorted, entries ordered
`path, sha256, mode`, no base wrapper):

| Quantity | Declared | Recomputed |
|---|---|---|
| files | 379 | 379 |
| full hash | `f8bf8c80e61d3156ab6a20efd5c87f4cc65b2a79e61819fc4972f889ba644f03` | identical |
| code hash (excludes only the candidate report) | `5e21d48b93d7c7c4529a58e54b2c31d532ca1436e9325bc21852af052e5fcbd2` | identical |

All 379 content hashes match. Nit: the nine *added* files are on disk as
`0664` while the manifest records `644`; every other file is `0644`. Content
is unaffected; the integrator should `chmod 644` on copy.

Delta: the 21 "modified" rows of `/tmp/opencode/AP2-delta.json` were diffed
against `lab-final-integration`, whose copies of all 21 files hash exactly to
the recorded `before_sha256` values (so that tree is a faithful stand-in for
the frozen 370-file baseline `30fedaf...`). The candidate's git `HEAD` is
`b0ab4ca` with 105 dirty paths; only the 30 AP-2 rows were reviewed, as
instructed. No removed files. `Cargo.lock` adds only the `sqlx`/`uuid`
dev-dependency edges of `bins/spc-pool`; no pin changed.

## 2. What was actually run (not counts copied from the report)

All under `nix develop` (the toolchain the candidate's `target/` was built
with; the host `cargo 1.93.0 (083ac513)` could not reuse it), `--jobs 2`,
`--test-threads=1` for DB/stream suites, against the owned fresh PG/Redis.

| Check | Result | Log (`/tmp/opencode/AP2-fable/`) |
|---|---|---|
| `cargo check -p spc-pool --features rpc` (no dev-deps, no tests) | **passes** - production feature set compiles with `spc-crypto/ark` now unconditional; no dev-dependency unification blind spot | `check-rpc-only-nix.log` |
| `cargo check -p spc-pool` (default features) | passes | `check-default-nix.log` |
| `cargo test -p spc-ext` | 8 + 5 passed | `t-ext.log` |
| `cargo test -p spc-store --lib` | 10 passed | `t-store-lib.log` |
| `cargo test -p spc-store --test ark_registration --test postgres` | `ark_registration` 2 passed; **`postgres` 19 passed, 1 FAILED** (`ark_delivery_ids_reports_registered_miners`, Section 4 #2) | `t-store-int.log` |
| `cargo test -p spc-pool --lib` | 227 passed, 1 failed: `payout::tests::republish_is_bounded_to_unpublished_blocks` - re-run alone against a fresh database it passes; it is a pre-existing shared-database isolation defect, not an AP-2 regression | `t-pool-lib.log` |
| `cargo test -p spc-pool --test ark_registration` | 5 passed (real PG + Redis) | `t-pool-reg.log` |
| `cargo test -p spc-pool --test ark_publication` | 3 passed (real Noise/SV2 + PG + Redis) | `t-pool-pub.log` |
| `cargo test -p spc-accounting --lib` | 65 passed | `t-acct.log` |
| `cargo clippy -p spc-ext -p spc-store -p spc-pool --all-targets -- -D warnings` | passes | `clippy.log` |
| `cargo fmt --all -- --check` | passes | `fmt.log` |
| `python3 scripts/check-lab-docs.py` | passes | `doccheck.log` |
| Counterexample program (Section 4 #1 and #3), real PG + Redis | both reproduced | `counterexample-run.log`, source `ce/src/main.rs` |

Report evidence cross-check: the author's logs are consistent with the frozen
source mtimes (`src/*` 06:30, pool lib tests 06:39, `tests/ark_registration.rs`
06:49 -> `AP2-stream-integration-frozen3.log` 06:50, clippy 06:51). One
citation is wrong: the report names `AP2-publication-real2.log` (05:56,
pre-freeze) where the actual post-freeze run is `AP2-publication-frozen.log`
(06:40, 3 passed). The author never ran `spc-store --test postgres`, which the
packet modified; that is how finding #2 was missed.

## 3. What is sound (reviewed against DESIGN.md, SPEC 5.1/11.3/11.4/11.9, WIRE 3.2/3.3/3.5/3.7, MAILBOX.md, binding decisions)

- **Signed `request_id` amendment** (`crates/spc-ext/src/lib.rs:472-482`,
  `600-609`). Preimage is exactly `sp_id || version || ark_id || b_ark || mode
  || B0_33(M) || B0_33(D) || fallback || request_id` under the BIP340 tagged
  hash `SPc/ArkIdentity`, length-prefixed blobs, `sp_id` bound first. An old
  signature moved to a higher id fails `verify` (`tests/registration_v1.rs::
  request_id_is_correlation_and_an_authorization_field`). Zero id is refused
  at decode; `u32::MAX` is accepted and simply becomes a ceiling (client policy
  owns exhaustion, which AP-3 already does). The amendment is clearly marked
  "Fable review required" in SPEC/decision/WIRE and I **approve the
  amendment itself**; signing the counter is the correct fix to the unsigned
  draft.
- **Codec negatives.** All seven message IDs are versioned, trailing bytes
  rejected, `Reader::take` uses `checked_add`, allocations are capped
  (`min(4096)`), raw mode > 3 maps to `unsupported-mailbox-mode`, bad fallback
  / bad state / bad retryable byte map to `invalid-message`. Legacy unversioned
  `SetArkIdentity` bytes are 4 bytes short of a v1 frame and fail with
  `Eof`/`Version`, never reinterpreted. Old `spc:arkreg` entries without the
  trailing id fail to decode and are acked/dropped (fail closed; the client
  retries).
- **Durable journal** (`crates/spc-store/src/postgres.rs:246-470`).
  `sp_identity` is inserted-if-absent then locked `FOR UPDATE` inside one
  transaction; identity/`ark_delivery` or `ark_epoch_mailbox`, state row,
  applied journal row and the payout-assigned `registration_rev` commit
  atomically. Equal id + equal digest -> `already_applied`; equal id +
  different digest -> `registration-mismatch`; lower id with matching journal
  -> `superseded`, no effect; lower id otherwise -> `stale`/mismatch. Eight
  concurrent counters serialize and leave 8 current (`identity_row_lock_
  serializes_duplicate_and_competing_counters`). A pre-0006 `ark_delivery`
  row with no state row is treated as "no current request" and the next id
  applies - the migration path is sound. A commit that fails mid-way rolls
  back and the retry re-resolves. Revision never comes from the client or
  Redis. Schema CHECKs (`1..4294967295`, 32-byte digest, `registration_rev >
  0`, outcome enum) are present in `0006_ark_registration_state.sql`.
- **Front router** (`bins/spc-pool/src/sv2_server.rs:292-341`). The handler
  receives the *open channel's* `ChannelIdentity` (so the BIP340 signer is the
  channel's own `b_scan`, `B_ark = B_spend` is checked against the channel's
  spend key, and `ark_id` against the configured server). Before channel open
  -> `invalid-channel`; no handler -> `unsupported-extension`;
  `RequestExtensions.Success` lists `0x5350` only when a handler exists and
  `pool_info` advertises it only when `ArkServerMetadata::from_config` is
  constructible for a `front` role (`registration.rs:396-401`); `main.rs`
  installs the handler only when that metadata and a Redis connection both
  exist and bails if Redis is down while Ark is enabled. The three states
  agree. Verified live over Noise/SV2 by `ark_publication.rs`.
- **Destinations** (`registration.rs:406-469`). `U_i = ark_user_pubkey(B_ark,
  t_ark)` from the pool-side ECDH at the build height; static = `M + D +
  t_ark*S_mb`; epoch_derived = `M_i + D + t_ark*S_mb` with `M_i =
  derived_epoch_mailbox_pubkey(M, ecdh)` and **D required**; epoch_registered
  uses the exact-height `M_i` row, else static `M`, else `None` under `hold`;
  `none` emits the policy address with an empty delivery list. The static and
  variant-A vectors equal the frozen `spc_ark`/MAILBOX.md vectors
  (`ark1pw4z...lrm0es`, `M_i = 03f4da68...`, `ark1pw4z...6au2cp`); the
  epoch-derived path reuses `spc-crypto`'s frozen `variant_b_M_i` function.
  `Ok(None)` is only the explicit hold branch; every bad point fails closed.
- **Pending, never forfeit.** `route_tiers` sends committed-but-underived or
  held ids to `pending` (`spc-accounting/src/lib.rs:678-688`); no other
  accounting arithmetic, type-32 rule, window/target policy or Ark order cap
  changed (the accounting diff is the two `ArkCapability` lines plus test
  fixtures; 65 tests unchanged in count).
- **Privacy.** `ArkSendOrder`/`ArkSendReceipt` are byte-identical to the
  baseline; `spc:arkreg`, `spc:arkreg:ack`, `ArkRegApplied` and the bootstrap
  additions all live on SPc-side streams/keys. SPEC 5.1 §11.3's ACL is an
  allow-list for the `ark` role (orders read, receipts write only), so the new
  key is excluded by construction.

## 4. Counterexamples (ranked) with minimum fix and required test

### 4.1 BLOCKING - a live Redis cache field returns `Applied` for a request that is durably `superseded`

`bins/spc-pool/src/registration.rs:95-139` (`wait_for_ack`) accepts any cached
field whose `sp_id`, `request_id`, `signed_fields_digest` match and whose
`expires_at` is in the future. Nothing checks that the request is still the
identity's *current* one. Payout only ever overwrites field
`reg:<sp>:<request_id>` for the id it is processing
(`payout.rs:1808-1832`), so the field for an older id keeps saying
`applied` for up to 3600 s after a newer id replaced it.

Reproduced against real PG + Redis (`counterexample-run.log`):

```
CE1 step1 A id=1 -> Success{state: Applied, rev 1794141154129}
CE1 step2 B id=2 -> Success{state: Applied, rev 1794141155231}
CE1 durable current b_ark == B? true ; last_request_id = Some(2)
CE1 step3 replay A id=1 while cache live -> Success{request_id: 1, state: Applied, rev 1794141154129}
CE1 durable resolution for id=1 right now = Superseded
CE1 VERDICT: STALE APPLIED RETURNED FOR SUPERSEDED REQUEST
```

Consequences: (a) the outcome of an identical request depends on whether a
Redis TTL has elapsed (`Applied` inside the hour, `superseded` after), which
violates the packet's own rule that no Redis TTL determines idempotency;
(b) an AP-3 agent that replays an old counter (restore from backup, second
device, lost local journal) receives `applied`, which AP-3 treats as
`is_registered()` -> it starts mining believing payload A is live while the
pool derives destinations from B's `B_ark`/mailbox; the `superseded` state
exists precisely to prevent this and is bypassed. The candidate's test
`streams_two_users_cache_expiry_restart_and_replay_never_roll_back` only
passes because an intervening `registration-mismatch` request overwrote the
`reg:<sp>:1` field with a different digest before the replay (lines
228-266); without it the assertion at line 263 fails.

The defect is also **codified normatively**: SPEC §11.4 line 1243 ("Cache
hits may return the original `Applied`"), decision §5, WIRE §3.3 and the
report §2.2 all state it. The amendment text must change with the code.

Minimum fix (either; (i) is simplest and makes the cache purely a
notification channel, which is what the docs already claim):

1. Front includes a fresh per-attempt nonce in the `spc:arkreg` entry (one
   `B016 attempt` field, not signed, not part of the digest); payout echoes
   it in the ack JSON; `wait_for_ack` accepts only an ack carrying its nonce.
   Every request is then resolved by Postgres, `Applied`/`already_applied`/
   `superseded` are always the durable verdict, and the "cache hit returns
   original Applied" sentence is deleted everywhere.
2. Or: on every *applied* resolution payout also writes
   `cur:<sp_id_hex> = last_request_id`; the front refuses any success-state
   field whose `request_id < cur` and waits for the durable verdict instead.

Required tests: (a) the exact CE1 sequence above in
`bins/spc-pool/tests/ark_registration.rs` asserting `Superseded` with the
cache deliberately *not* expired and no intervening mismatch; (b) the same
for `already_applied` followed by a newer id. Update SPEC §11.4, WIRE §3.3,
decision §5 and `docs/changelog/2026-11-08-ark-registration-ack.md`.

### 4.2 BLOCKING - `Store::upsert_ark_delivery` is broken (7 placeholders, 6 binds) and the modified test that catches it was never run

`crates/spc-store/src/postgres.rs:169-192`: the INSERT now has
`epoch_fallback` as `$7` but the `.bind` chain still binds six values, and the
`ON CONFLICT ... DO UPDATE` clause does not set `epoch_fallback`. Real PG:

```
08P01 bind message supplies 6 parameters, but prepared statement requires 7
```

`crates/spc-store/tests/postgres.rs::ark_delivery_ids_reports_registered_miners`
(a file this packet edited) fails on the frozen source. Production payout no
longer calls this method (it uses `apply_registration`), but it is a public
Store API used by the settlement-rail tests and the report claims the store
test suite green by running only `--test ark_registration`.

Minimum fix: add `.bind(row.epoch_fallback.as_i16())` and `epoch_fallback =
EXCLUDED.epoch_fallback`; run `cargo test -p spc-store --test postgres` and
record it in the report table.

### 4.3 HIGH - payout-side rejections of front-accepted entries never reach the client, and one class poisons the registration consumer

Two paths in `payout.rs`:

(a) `apply_registration` returns `StoreError::Invalid` for a validly signed
`RegisterEpochMailboxes` whose identity is not in `epoch_registered` mode
(`postgres.rs:354,360`) or has no delivery row/identity (`:285`). In
`process_registrations_once` (`payout.rs:1902`) that is `Err(error) => return
Err(error)`: the entry is **never acked**, every pass re-reads it from the PEL
(`read_pending` = `XREADGROUP ... 0`), errors at it and aborts the rest of
that pass. Reproduced:

```
CE2 bob epoch id=2 (static mode) -> Error{retryable: true, "registration-commit-unknown"}
CE2 payout pass: Err("store: invalid column value: epoch mailboxes require epoch_registered mode")  x N
CE2 XPENDING after 3 more passes: 1 entry still pending (payout-0)
```

The waiting client receives *retryable* `registration-commit-unknown`, so an
AP-3 agent retries the same id forever (30 s cap), each retry appending
another poison entry; the PEL grows without bound; `run()` logs a warning
every loop; entries delivered in the same `XREADGROUP` batch behind the poison
are starved (new entries still get through because `new` is processed before
`pending`, which is why Carol in the reproduction still succeeded). Any
authenticated miner can trigger it with one correctly signed frame.

(b) `on_identity_reg`/`on_epoch_reg` map every validation failure (payout's
`error_code`, invalid `M_i` point, unknown identity) to
`PayoutError::Registration`, which is acked and dropped silently
(`payout.rs:1896-1900`). The client again sees only the 5 s
`registration-commit-unknown` and retries indefinitely; the correct answer is
terminal (`bad-mailbox-fields`, `invalid-point`, ...).

Minimum fix: make payout-side terminal rejections *resolutions*: write the
ack field with a terminal `state`/`error_code` (the JSON already has both
fields; add `bad-mailbox-fields`/`invalid-point`/`invalid-message` to the
allowed `state` list in WIRE §3.3), then ack the stream entry. In
`process_registrations_once` only `PayoutError::Store(Db | Pool | Io)` and
`Redis` may short-circuit; `StoreError::Invalid` must be handled like
`Registration`. Also have the front refuse `RegisterEpochMailboxes` for an
identity it does not know to be `epoch_registered` (it has
`ark_registrations` in `Front`; expose a read-only lookup to the handler).

Required tests: static-mode identity sends a valid epoch registration ->
terminal error within the deadline, entry acked, `XPENDING` empty, a second
identity's registration in the same batch still applies.

### 4.4 REQUIRED FOR INTEGRATION - a registration-driven destination change does not roll the job

`refresh_ark_destinations` runs inside `refresh()` (`front.rs:846`) and the
resulting `Distribution` replaces `self.distribution` on every refresh
(`front.rs:962`). The job id moves only when `distribution.fingerprint`
changes (`front.rs:899`), and `spc_accounting::fingerprint` hashes `height ||
a_send_x || coinbase outputs` only. `Rail::Ark`/`Rail::Pending` entries and
`dest_id` are not coinbase outputs, so a `SetArkIdentity` that changes
`B_ark`/mailbox (new `dest_id`) or flips an id from `pending` to `ark_due`
leaves `job_id` unchanged while the distribution the block event is built
from (`front.rs:1123`) silently changes. A share mined on the "old" job
therefore produces a block event carrying post-registration metadata under
the same `fingerprint`, and `distribution_snapshot` is `ON CONFLICT DO
NOTHING` by fingerprint, so settlement cannot tell the two apart. Consensus is
unaffected (same coinbase bytes) but this contradicts SPEC §11.4's "applies to
distributions built after acceptance" and the frozen-generation claim in the
report; the primary's T4 `Published.job_id` dedup key relies on the front
rolling `job_seq` for every job-changing input.

Minimum fix: keep a `published_entries: Option<[u8; 32]>` beside
`published_fingerprint` (sha256 over `entries` rail + `sp_id` + `dest_id` in
order) and treat a change in either as `bumped`. Add a `front.rs` test: build
job J, apply `ArkRegApplied` with a new `B_ark`, refresh, assert `job_id`
changed and that a share carrying J's id is `StaleJob`. If the integrator
instead decides destinations are frozen at *find* time, say so in SPEC §11.4
and delete the per-generation claim - but pick one.

### 4.5 SHOULD FIX - normative text and config gaps

- `[ark] server_pubkey` and `mailbox_pubkey` are new **REQUIRED** keys for the
  `front` role (`config.rs:1003-1010`, refuses to start) and `ark_id` must now
  equal `ArkId::from_server_pubkey(server_pubkey)`; neither appears in the
  SPEC `[ark]` config block (lines 1336-1352), `config/spc-pool.example.toml`,
  ARCHITECTURE.md or the decision record. Add them.
- SPEC §11.4 / decision §3 describe `stale-request` as "a pruned older
  request". Nothing prunes; in the code `stale` is a *never-applied lower id*
  (e.g. the loser of a concurrent reorder). Describe the real semantics.
- `registration-busy` and `rate-limited` are listed as emitted retryable
  codes but no code path produces them. Mark them reserved.
- ARCHITECTURE.md line 187 (`spc:arkreg` shape) still lacks `request_id`.
- WIRE §3.2 should state that pre-AP-2 `ArkRegApplied` payloads do not decode
  and that a front must bootstrap (not replay) across the upgrade; the code
  does fail closed, the docs do not say so.
- Report §2.2 cites `AP2-publication-real2.log`; the frozen run is
  `AP2-publication-frozen.log`.

### 4.6 LOW - domain separation between the two signed request kinds

`SetArkIdentity` and `RegisterEpochMailboxes` sign under the same tag
`SPc/ArkIdentity` with no kind byte in the preimage. I checked the
cross-interpretation lengths: an honestly formed identity preimage is 78 or
144 bytes, an epoch preimage is `39 + 37n`; they never coincide, and an
attacker cannot pick the honest signer's bytes, so this is not exploitable
today. But the wire contract is new and the journal keys both kinds in one
`(sp_id, request_id)` space; use a distinct tag (`SPc/EpochMailboxes`) or
prepend `msg_type` now rather than after deployment. Update WIRE §3.7.

### 4.7 LOW - miscellany

- `epoch_fallback` is accepted and stored for any mode (`validate_mailbox_
  fields` ignores it); SPEC says "`epoch_registered` only". Either reject
  `hold` outside `epoch_registered` as `bad-mailbox-fields` or relax the spec.
- `apply_front_event` uses `.expect()` on identity bytes/points from the
  `spc:front:events` stream (`front.rs:683-694`). The stream is SPc-internal
  and payout validated them, but a front should log-and-skip, not abort.
- `RequestExtensions` handling is new on the native mining listener and
  answers `0x0003` as supported; SPEC §3.2 does advertise it, but the native
  listener implements no `0x0003` message. Confirm this is the intended claim.
- `registration_ark_id.unwrap_or(e.ark_id)` (`payout.rs:1708`) disables the
  server check when payout has no Ark metadata. `main.rs` always sets it when
  Ark is enabled, so it is latent; prefer rejecting when unset.
- The privacy test builds the order struct by hand rather than capturing
  payout's emitted `spc:ark:orders` bytes; acceptable because the structs are
  byte-identical to the baseline, but the §11.9 "captured stream" wording
  overstates it.
- The 5 s wait runs inline in the connection's read loop; it blocks only that
  connection, acceptable since a miner must not mine before success.

## 5. AP-3 handoff (primary already integrates AP-3)

- AP-3 `RegistrationPayload::local_digest` deliberately excludes
  `request_id` and is only the client's same-attempt comparator; AP-2's wire
  digest includes it. These are different objects and must stay so; the
  adapter passes the reserved id into `SetArkIdentity::sign`. The candidate's
  decision text already says this; approve that boundary.
- AP-3's `epoch_derived(..., mailbox_dh: None)` helper is incompatible with
  the derivation (`D` required at `registration.rs:454`); it would be
  rejected `bad-mailbox-fields`. Candidate correctly refuses to make D
  optional; AP-3 must supply `D = b_ark * S_mb`.
- Interplay to record for the integrator: an agent restored with an old
  counter gets `superseded` (id n), then its next local id may equal the
  server's current id -> `registration-mismatch`; AP-3 treats two consecutive
  terminals for one payload as fatal. AP-2's `Success` carries
  `registration_rev` but not the server's current `last_request_id`; adding it
  (or a `current_request_id` field in `Success`) would let the client jump.
  Not blocking for AP-2; a wire-additive follow-up.
- No real wallet/registration flow is proven; both ends are exercised with
  synthetic keys and the candidate says so.

## 6. Verdict

**REQUEST_CHANGES.** The codec, the signed-counter amendment, the Postgres
journal, the router/advertisement gating and the derivation are sound and
match the frozen vectors; the production feature set compiles without the
dev-dependency mask. The packet cannot be integrated until:

1. 4.1 is fixed in code and in SPEC §11.4 / WIRE §3.3 / decision §5 /
   changelog fragment, with the un-expired-cache replay test.
2. 4.2 is fixed and `spc-store --test postgres` is green and recorded.
3. 4.3 is fixed (terminal acks for payout-side rejections; no poison in the
   consumer loop) with the static-identity epoch test.
4. 4.4 is resolved one way with a `front.rs` test, since the primary's T4
   `Published.job_id` depends on the front's job roll.
5. 4.5 config keys are documented (operator-facing refuse-to-start).

4.6-4.7 are recommended in the same pass but would not alone block. No AP-1
profile, live Ark, T4b, funded AP-4/AP-5 readiness or primary integration is
asserted by this review.

## 7. Scratch evidence (`/tmp/opencode/AP2-fable/`)

`*.diff` (21 baseline-vs-candidate diffs), `check-rpc-only-nix.log`,
`check-default-nix.log`, `t-*.log`, `fmt.log`, `clippy.log`, `doccheck.log`,
`counterexample-run.log`, `ce/` (standalone reproduction crate; path deps into
the candidate, built with the candidate's `CARGO_TARGET_DIR`; its binary
artifacts were removed from the candidate target afterwards),
`start-storage.sh`, `storage-env.sh`, `storage.*/` (truncated). The host
toolchain `cargo check` attempted first (`check-rpc-only.log`) failed only
because of a toolchain mismatch with the cached target and is not evidence
against the candidate.

---

## Round 2 (2026-11-08, final-green rework) - same reviewer, same confinement

Disposition: **REQUEST_CHANGES (narrow)**. Every round-1 finding is closed
in source and in the normative text; one new poison path was introduced by
the rejection journal (R2.1 below, one-query fix plus one test). Nothing
else blocks.

Resource note: the build slot is reserved for another session
(`silent-pool-build-priority.json`), so **no cargo invocation, test binary or
database was started in this round**. All statements below are grounded in
the final source bytes and the author's post-freeze receipts; the one new
counterexample is traced in code, not executed. No file outside this review
was written.

### R2.0 Snapshot, delta and receipts

- `/tmp/opencode/AP2-finalgreen-manifest.json`: 379 files, all content hashes
  recomputed and matching; full `7a8aea50...17f703e` and code
  `e0ad690d...17d962ba` recomputed with the declared ordered-array recipe and
  identical. Delta vs the frozen 370-file baseline: 23 modified, 9 added, 0
  removed (`config/spc-pool.example.toml` and `docs/ARCHITECTURE.md` are the
  two newly modified files).
- Changed since round 1 (byte-exact diff against the preserved round-1 copies
  under `/tmp/opencode/AP2-*`; diffs in `/tmp/opencode/AP2-fable/r2/`):
  `front.rs`, `payout.rs`, `registration.rs`, `tests/ark_registration.rs`,
  `spc-ext/src/lib.rs`, `spc-ext/tests/registration_v1.rs`, migration 0006,
  `models.rs`, `postgres.rs`, `wire.rs`, SPEC, WIRE-FORMATS, ARCHITECTURE,
  example config, decision, changelog fragment, report. `api_backend.rs`,
  `config.rs`, `main.rs`, `sv2_server.rs`, accounting and bootstrap are
  byte-identical to round 1, so the signed-counter codec, derivation vectors,
  router/advertisement gating, privacy checks and the `--features rpc`
  production build result carry over unchanged and were not re-verified.
- Receipts are source-bound: last source mtime `postgres.rs` 10:51:17; logs
  `AP2-finalgreen-store.log` 10:52 (2 + **20** passed, including the
  previously failing `ark_delivery_ids_reports_registered_miners`),
  `-registration.log` 10:53 (7 passed, incl. the two new CE tests),
  `-publication.log` (3), `-poollib.log` (229), `-codec.log` (8 + 6),
  `-clippy.log` (`-D warnings`, 0 diagnostics), `-finalfmt.log` (devshell
  banner only), `-docs.log` (`OK`), `-stop.log` (owned PG stopped). Exit file
  all zero. The original `f8bf8c80`/`5e21d4` manifest and the failing receipts
  are preserved as `preserved_history`.

### R2.1 BLOCKING (narrow) - a `rejected` journal row at id *n* turns a later valid request with id *n* into an unacked PEL poison

New `Store::reject_registration` (`postgres.rs:255-427`) journals a semantic
rejection as `('rejected', code)` at `(sp_id, request_id)` **without**
advancing `ark_registration_state.last_request_id` (correct: a rejection has
no effect). But `apply_registration`'s `is_new` branch (`postgres.rs:
493-600`) still decides newness only from `last_request_id` and then runs a
plain `INSERT INTO ark_registration_request ... 'applied'` (`:587-591`) with
no journal lookup and no `ON CONFLICT`.

Sequence (any authenticated miner, no special knowledge):

1. `SetArkIdentity` id 1, static -> applied, `last_request_id = 1`.
2. Signed `RegisterEpochMailboxes` id 2 -> payout `bad-mailbox-fields` ->
   `reject_registration` journals `(sp, 2, 'rejected')`. Client gets the
   terminal code (round-1 CE2 fix works as intended).
3. Valid `SetArkIdentity` id 2 with any other digest (a buggy or hostile
   client; a conforming AP-3 client allocates id 3 here). Front validates and
   publishes. Payout: `2 > last(1)` -> `is_new` -> upserts identity/delivery
   -> journal INSERT hits the primary key -> `StoreError::Db` (unique
   violation) -> transaction rolls back (delivery unchanged, good) ->
   `on_reg` passes it through as `other` (`payout.rs:1696`) ->
   `process_registrations_once` `Err(error) => return Err(error)`
   (`payout.rs:1989`) -> entry stays in the PEL forever, every pass errors
   at it and aborts the remainder of that pass, the waiter gets retryable
   `registration-commit-unknown` and retries the same id, re-publishing the
   poison on each retry.

This is the round-1 4.3 class again (unbounded PEL, warn-per-loop,
starvation of entries co-batched behind the poison) through a path the
rejection journal created. `reject_registration` already handles the mirror
case (existing row, different digest -> `Mismatch`, `:300-316`); only
`apply_registration` lacks the symmetric check.

Minimum fix (one query, no wire or doc change): in `apply_registration`,
after the `sp_identity` lock and before `is_new`, `SELECT outcome,
signed_fields_digest, registration_rev, error_code FROM
ark_registration_request WHERE sp_id = $1 AND request_id = $2 FOR UPDATE`.
If a row exists: equal digest -> replay that row's outcome (`rejected` with
its stored code, `applied` -> `already_applied`/`superseded` per
`last_request_id`); different digest -> `Mismatch`. Only when no row exists
proceed to the existing `is_new` logic. Required test in
`bins/spc-pool/tests/ark_registration.rs`: the exact 3-step sequence above,
asserting terminal `registration-mismatch` for step 3, `ark_delivery`
unchanged, `read_pending` empty after two more passes, and a second
identity's registration still applying. Re-run `spc-pool --test
ark_registration` and `spc-store --test ark_registration --test postgres`
only; nothing else in the packet is affected.

### R2.2 Closure of round-1 findings (verified in source and receipts)

| Round 1 | Status | Where |
|---|---|---|
| 4.1 stale cached `Applied` | **Closed.** Front draws a random `B016 attempt` per handle (`registration.rs:223`), appends it to the internal `spc:arkreg` entry (`wire.rs`, after `request_id`, not on the client wire, not in any digest), payout echoes it in the ack JSON and the field name `reg:<sp>:<id>:<attempt>` (`payout.rs:2296-2302`), and `wait_for_ack` requires `attempt` equality (`registration.rs:110`). Every handle therefore resolves through Postgres; an old field is unreachable. Test `live_cache_never_overrides_durable_supersession` runs the exact A1 -> B2 -> A1 sequence with no expiry and no intervening mismatch and asserts `Superseded`, plus the `already_applied` variant. SPEC §11.4, WIRE §3.3, decision §5 and the changelog fragment now state "every handle triggers durable resolution"; the "cache hits may return the original Applied" sentence is gone from all three. | receipts: 7 passed |
| 4.2 `upsert_ark_delivery` 6/7 binds | **Closed.** `.bind(row.epoch_fallback.as_i16())` added; `spc-store --test postgres` 20 passed on the final source. Note the `ON CONFLICT ... DO UPDATE` still omits `epoch_fallback = EXCLUDED.epoch_fallback`; the method is test-only now, so this is a nit, not a blocker. | `postgres.rs:189` |
| 4.3 poison / silent drop | **Closed for the reported paths**, reopened narrowly as R2.1. `on_epoch_reg` pre-checks `ark_delivery.mailbox_mode` and returns `RegistrationCode("bad-mailbox-fields")`; `on_reg` maps `RegistrationCode`, `Codec`, `Registration` and `StoreError::Invalid` to `reject_identity`/`reject_epoch`, which journal and cache a terminal `rejected` ack and let the entry be acked; only `Db`/`Redis` failures stay unacked. Test `signed_epoch_mismatch_is_terminal_acked_and_does_not_poison_later_records` asserts the terminal code, the durable `Rejected` row, an empty PEL and a later identity applying. `registration_ark_id` is now required (`ok_or("wrong-ark-server")`), closing the latent `unwrap_or(e.ark_id)`. | `payout.rs:1666-1697, 1783-1808, 1841-1888` |
| 4.4 job does not roll on dest/rail change | **Closed.** `entries_fingerprint` (`SPc/JobEntriesV1` over `sp_id || rail || len || dest_id`) is tracked as `published_entries` beside `published_fingerprint`; either change bumps `job_seq` (`front.rs:905-910`). Test `registration_destinations_and_rails_roll_job_even_when_coinbase_does_not_change` shows pending -> Ark and a `B_ark` change each roll the job while `distribution.fingerprint` is unchanged, and the old share is `StaleJob`. Integration note stands: on the primary, T4 `Published.job_id` picks this up automatically. | `front.rs:1291-1306` |
| 4.5 config / docs | **Closed.** SPEC `[ark]` block, example config, ARCHITECTURE §5.5 document `server_pubkey`/`mailbox_pubkey` as REQUIRED for the `front` role with the `ark_id` cross-check; `stale-request` is now described as a never-applied lower id; `registration-busy`/`rate-limited` marked reserved; WIRE §3.2 states old `ArkRegApplied` events do not decode and fronts bootstrap across the upgrade; ARCHITECTURE stream table carries `request_id, attempt`. | |
| 4.6 shared tag | **Closed.** `RegisterEpochMailboxes` digests under `SPc/EpochMailboxes` (`spc-ext/src/lib.rs:615`); WIRE §3.7, SPEC §11.4 and decision §3 updated; test asserts the two digests differ. Dated record present. | |
| 4.7 misc | `hold` outside `epoch_registered` -> `bad-mailbox-fields` (`lib.rs:538-540`, SPEC updated, test added); `apply_front_event` logs-and-skips malformed identity bytes instead of `expect` (`front.rs:688-700`, tested). The `0x0003`-on-mining-listener note is unchanged and remains an integrator confirmation item, not a blocker. | |

Interoperability: the whole packet is miner <-> pool (`0x5350`) and pool-
internal (Postgres, Redis). No Ark-server RPC, plugin or patch is introduced
or proposed; server inputs are the public `ArkInfo` pubkeys supplied by the
operator, consistent with the AGENTS.md vanilla-Ark requirement.

### R2.3 Non-blocking notes for the same pass

- `spc:arkreg:ack` fields are now unique per attempt and are never deleted;
  `EXPIRE` on the whole hash is refreshed on every write, so under continuous
  registration traffic the hash grows without bound (one ~250-byte field per
  attempt, including every retry). Suggest `HDEL` by the front once it has
  consumed its field, or per-ack keys with `SET ... EX 3600`. Memory, not
  correctness.
- `reject_registration` for a `Delivery` effect inserts the `sp_identity` row
  *before* journaling, even when the rejection code is the `sp_id`/key
  mismatch (`bad-signature`). Only a forged internal stream entry can reach
  it, but computing `identity_exists` before the insert (or skipping the
  insert for that code) keeps `sp_identity` consistent by construction.
- `on_reg` maps every `StoreError::Invalid` (including a corrupt journal row
  such as an unknown `outcome` string) to a terminal `invalid-message` for
  the client. Acceptable today; if the journal ever carries operator-edited
  rows, distinguish "request invalid" from "row invalid".
- `upsert_ark_delivery`'s conflict clause omits `epoch_fallback` (see table).

### R2.4 AP-3 handoff (unchanged from round 1, now also stated in SPEC §11.4)

Client signs the counter through the canonical `spc-ext` signer; AP-3's
`local_digest` may exclude `request_id` as its own attempt comparator; `D` is
required for `epoch_derived` (client derives it); `superseded` means "not
current, re-register with a higher id". Restore-from-backup counter
unwedging and any `current_request_id` hint in `Success` remain AP-3b
integration items; no Ark RPC is involved.

### R2.5 Verdict

Approve everything except R2.1. Once the `apply_registration` journal
pre-check and its test land and the two affected suites are re-run green on
that source, the bounded AP-2 delta is **APPROVE-ready for narrow hunk
integration after T4/AP-3/T0** with no further Fable pass required for the
unchanged files. No AP-1 profile, live Ark, T4b, funded AP-4/AP-5 or primary
integration readiness is asserted.

---

## Round 3 (2026-11-08, R2.1-only closure) - same reviewer, same confinement

Disposition: **APPROVE** the exact bounded AP-2 delta (23 modified + 9
added files against the frozen 370-file baseline) frozen as
`/tmp/opencode/AP2-r21-final-manifest.json`, full
`d94bb15841d888724fd987365dbdeb81e3cf816443cd4ed07b2f821c26f78953`, code
`de0e2158b2ea24846e2edf3b3ecefd401f9b98880b933fed5378a092b099956e`.

Resource note: the build slot is held by the serial-integration ticket, so
no cargo, test binary or database was started in this round. The R2.1 fix
is reviewed from source bytes and from the replacement worker's receipts,
which are hash-bound to the frozen code (below). No file outside this
review was written.

### R3.0 Snapshot, delta, receipts

- All 379 content hashes recomputed and matching; full and code aggregates
  recomputed with the declared ordered-array recipe and identical.
- Changed since the round-2 snapshot (byte-exact diffs against the preserved
  round-2 copies): `crates/spc-store/src/postgres.rs`,
  `bins/spc-pool/tests/ark_registration.rs`, SPEC §11.4, WIRE §3.3, the
  decision record, the changelog fragment and the candidate report. Every
  other file is byte-identical to the round-2 snapshot, so the round-2
  closures (nonce, rejection journal, job roll, tag, config, docs) and the
  round-1 approvals stand without re-derivation.
- Receipts (`/tmp/opencode/AP2-r21-final-receipts.json`): all 9 log hashes
  recomputed and matching; `source-before` == `source-after` (the gate ran on
  one unchanged source set); `tested_code_hash == final_code_hash`; the only
  file that differs between `source-after` and the final manifest is the
  candidate report (updated after the gate, excluded from the code hash).
  Logs 16:25-16:26 post-date the last code mtime (`tests/ark_registration.rs`
  16:24:50; `postgres.rs` 11:13:15). Results: `spc-store --test postgres
  --test ark_registration` 20 + 2 passed; `spc-pool --test ark_registration`
  **8 passed** including the new
  `rejected_id_collision_is_terminal_mismatch_without_poison_or_mutation`;
  `cargo fmt --check` clean; scoped `clippy --all-targets -D warnings` clean
  (only the cargo cache-permission and sqlx future-incompat notices);
  `check-lab-docs.py` OK; owned PG stopped (`-stop.log`). Exit file all zero.

### R3.1 R2.1 closure (source)

`postgres.rs:493-553`, inside `apply_registration`, after the `sp_identity
... FOR UPDATE` lock and the `ark_registration_state` read, **before**
`is_new`: `SELECT outcome, signed_fields_digest, registration_rev,
error_code FROM ark_registration_request WHERE sp_id = $1 AND request_id =
$2 FOR UPDATE`. If a row exists the transaction commits with no insert and
no effect:

- different digest -> `Mismatch` (`registration-mismatch`), which is exactly
  the R2.1 counterexample step 3 (valid `SetArkIdentity` at a rejected id);
- equal digest, `rejected` -> `Rejected` with the stored, whitelisted code
  (original terminal error replays deterministically);
- equal digest, `applied` -> `AlreadyApplied` when `last_request_id ==
  request_id`, else `Superseded` (never a historical `Applied`);
- the dead `superseded`/`mismatch`/`stale` outcome strings (never written)
  map harmlessly.

Only an absent row proceeds to the unchanged counter policy, so the PK
collision that produced the `StoreError::Db` -> unacked PEL entry is
unreachable; both the apply and reject paths now resolve every existing
`(sp_id, request_id)` row under the same identity lock, and either path's
failure still rolls the transaction back. `reject_registration` is unchanged
from round 2. The test drives the exact sequence - id 1 applied, signed
epoch id 2 -> terminal `bad-mailbox-fields`, valid identity id 2 (different
digest) -> terminal `registration-mismatch`, `last_request_id` still 1,
`ark_delivery` row and `registration_rev` unchanged, exact rejected replay
-> original terminal error, `read_pending` empty, then id 3 applies with the
new `B_ark` - against real PG + Redis and passed in the bound receipt.

Normative text is consistent: SPEC §11.4 (journal replay before `is_new`,
equal/different digest outcomes, absent-row counter policy), WIRE §3.3
journal/outcome policy, decision §4 ("rejected-ID insert collisions cannot
become pending-stream poison") and a dated changelog bullet. No client wire,
signature preimage, Ark-server interface or pin changed; the packet remains
miner <-> pool `0x5350` plus pool-internal Postgres/Redis, consistent with
the unmodified-`captaind` requirement.

### R3.2 Carried non-blocking notes (unchanged source, not re-raised as blockers)

Per-attempt `spc:arkreg:ack` fields are never deleted while the hash TTL is
refreshed on every write (memory growth under steady traffic);
`reject_registration` inserts `sp_identity` before journaling for a
`Delivery` effect even on the `sp_id`/key-mismatch code (reachable only from
a forged internal entry); `upsert_ark_delivery`'s conflict clause omits
`epoch_fallback` (test-only method). Suitable for a follow-up packet; none
affects correctness of delivery, counters or the consumer loop.

### R3.3 Verdict

**APPROVE** the bounded AP-2 delta at the hashes above for narrow hunk
integration after T4/AP-3/T0, preserving those packets' owned hunks in
`main.rs`/`front.rs`/`payout.rs`/SPEC and re-checking the `0006` migration
slot at merge. The signed-counter amendment, `SPc/EpochMailboxes` tag,
per-handle attempt nonce, rejection journal and entries-digest job roll are
approved as recorded. No AP-1 profile, live Ark, T4b, funded AP-4/AP-5 or
primary-integration readiness is asserted; vanilla Second.tech/Bark
interoperability is a property of the design reviewed here, not an
end-to-end claim.
