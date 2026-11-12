# AP-2 Ark registration implementation and verification (2026-11-07)

Status: **R2.1 focused PostgreSQL/Redis/fmt/clippy gate passed on final source; parent verification and same-Fable review pending.**
No end-to-end Ark delivery, Ark-server compatibility, funded VTXO, refresh or
offboard claim is made. This packet implements the pool-side SPc registration,
durable acknowledgment, destination wiring and related normative fragments.

## 1. Exact baseline and delta

The approved final runtime source was restored from
`lab-final-integration` using `/tmp/opencode/LP-final-runtime-manifest.json`
and re-verified byte-for-byte and mode-for-mode before AP-2 edits:

| Field | Value |
|---|---|
| Git binding baseline | `b0ab4ca34fda07c61a6335d05f598eaacc69678c` |
| Frozen full source hash | `full9ffff6da` (prefix supplied by parent) |
| Frozen code hash | `30fedafdbba57b79fe4c13e2de0658aa3f05f527b778a242cbfab9c4ba253d76` |
| Frozen files | 370 |
| Restored verification errors / extra files / removed files | 0 / 0 / 0 |
| AP-2 modified files | 23 |
| AP-2 added files | 9, including this report |
| AP-2 deleted files | 0 |
| AP-2 rows in `/tmp/opencode/AP2-rework-delta.json` | exact before/after hash+mode records are regenerated after rework |

The initial verification log is `/tmp/opencode/AP2-bootstrap.txt`. The Git
`HEAD` diff is intentionally **not** used as packet delta because it also
contains the separately approved runtime integration. Every AP-2 delta below
is computed against the frozen manifest, not against `HEAD`.

Modified (23, after Fable-requested rework):

- `Cargo.lock`
- `bins/spc-pool/Cargo.toml`, `src/api_backend.rs`, `src/config.rs`,
  `src/front.rs`, `src/lib.rs`, `src/main.rs`, `src/payout.rs`,
  `src/sv2_server.rs`
- `crates/spc-accounting/src/lib.rs`, `src/tests.rs`
- `crates/spc-ext/src/lib.rs`, `src/tests.rs`
- `crates/spc-store/src/bootstrap.rs`, `src/models.rs`, `src/postgres.rs`,
  `src/redis.rs`, `src/wire.rs`, `tests/postgres.rs`
- `docs/ARCHITECTURE.md`, `config/spc-pool.example.toml`, `docs/SPEC.md`,
  `docs/WIRE-FORMATS.md`

Added (9):

- `bins/spc-pool/src/registration.rs`
- `bins/spc-pool/tests/ark_publication.rs`
- `bins/spc-pool/tests/ark_registration.rs`
- `crates/spc-ext/tests/registration_v1.rs`
- `crates/spc-store/migrations/0006_ark_registration_state.sql`
- `crates/spc-store/tests/ark_registration.rs`
- `docs/changelog/2026-11-08-ark-registration-ack.md`
- `docs/decisions/2026-11-08-ark-registration-ack.md`
- `docs/reviews/2026-11-07-ark-registration-implementation.md`

No primary, frozen reference input, design gallery, user notes, manual-QA tree,
T4a or AP-3 tree was edited. The separate migration-slot check found no other
`0006*` in sibling trees at dispatch; integration must rerun that check rather
than overwrite a newly allocated migration.

## 2. Implemented wire/API/stream/schema contract

### 2.1 Extension negotiation and router

Extension identifier `0x5350`, all payloads with `U8 version=1`, trailing bytes
rejected. Message IDs and exact payload shapes are normatively recorded in
`docs/WIRE-FORMATS.md` section 3.7:

| ID | Message | Correlation |
|---:|---|---|
| `01` | SetArkIdentity | trailing U32 request_id after B064 sig |
| `02` | Success | version, request_id, U64 registration_rev, B004 ark_id, U8 state |
| `03` | Error | version, request_id, U8 retryable, B0_255 error_code, B0_255 detail |
| `04` | RegisterEpochMailboxes | version, U16 count, B037 entries, B064 sig, request_id |
| `05` | Epoch Success | same shape as `02` |
| `06` | Epoch Error | same shape as `03` |
| `07` | ArkPayoutNotice | version, U256 block_hash, U64 sats, B0_64 vtxo_id, U32 expiry_height |

`SetArkIdentity` fields, in order, are `B004 ark_id`, `B033 b_ark`, `U8
mailbox_mode`, `B0_33 mailbox_pubkey`, `B0_33 mailbox_dh`, `U8
epoch_fallback`, `B064 sig`, `U32 request_id`, following the version byte.
`state` is 0 `applied`, 1 `already_applied`, 2 `superseded`.

The **explicit AP-2/Fable contract amendment** is canonical:

```
signed_fields_digest = hash_SPc/ArkIdentity(
  sp_id || version || ark_id || b_ark || mailbox_mode ||
  B0_33(mailbox_pubkey) || B0_33(mailbox_dh) || epoch_fallback || request_id)
```

The ID remains after `sig` on the wire but **is signed**. The earlier unsigned
interpretation would permit an old signature transplanted to a new ID to
authorize rollback. `RegisterEpochMailboxes` equivalently signs
`sp_id || version || count || entries || request_id`. Requests start at 1 per
identity; zero/wrap is `request-id-exhausted`. Raw mode >3 is
`unsupported-mailbox-mode`; known bad fields are `bad-mailbox-fields`;
structural failure is `invalid-message`.

The actual Noise/SV2 router uses the open channel's `ChannelIdentity` and
revalidates signer/identity/server/mailbox fields. API and RequestExtensions
advertise `0x5350` only when `[ark]` is enabled and public server metadata
constructs a real front handler. Disabled negotiation omits it and direct
requests get `unsupported-extension` (or `invalid-channel` before channel
open). No disabled path synthesizes success. `ark_id` must match
`ArkId::from_server_pubkey(server_pubkey)`.

### 2.2 Durable resolver and acknowledgment (rework)

Migration `0006_ark_registration_state.sql` retains the two durable tables and
adds `epoch_fallback` on `ark_delivery`. The public
`Store::upsert_ark_delivery` helper now binds and conflict-updates the same
field (the review's seven-placeholder/six-bind defect is fixed). Applied
identity/delivery or epoch effects, journal state and revision commit
atomically under the identity-row `FOR UPDATE` lock.

A validly parsed/signed semantic rejection has a separate `rejected` journal
outcome and exact terminal `error_code` with a payout revision, without changing
`ark_delivery` or current request state. If no identity exists for the FK, the
deterministic invalid-message outcome has no journal row but is still terminal.
Only actual database or Redis failures remain unacknowledged/unknown; no
terminal rejection is intentionally left in PEL as commit-unknown.

The internal `spc:arkreg` entry and epoch entry append a fresh `B016 attempt`.
That field never appears on client wire and is not signed or a durable
registration operation ID. The matched notification is keyed:

```
reg:<sp_id_hex>:<request_id>:<attempt_hex>
```

and JSON echoes the same nonce. Each front handle generates a different nonce,
triggers the durable resolver, and requires identity/request/digest/attempt and
expiry to match. A cached field from an earlier attempt can therefore never
satisfy the current handler or race another identical concurrent request.
A1 applied or already_applied followed by B2 current must resolve A1 as
`superseded` even with no TTL expiry and no intervening mismatch. Cache TTL
only controls retention, never current outcome. Fallback `hold` is valid only
in `epoch_registered`, and epoch requests now sign under
`SPc/EpochMailboxes`, separately from `SPc/ArkIdentity`.

The `SPc/JobEntriesV1` digest is tracked beside the existing coinbase
fingerprint, over ordered `(sp_id, rail, dest_id length, dest_id)` entries.
Rail-only or destination-only changes roll front's job ID and stale old work,
and populate the new distribution's frozen `dest_id`. This is the AP-2 handoff
concept for the primary's independent T4 `Published.job_id` hunk, not an import
or overwrite of T4 source.

Redis continues to return retryable `registration-commit-unknown` only when
no exact attempt acknowledgment resolves within five seconds; it never proves
unregistered status. Success still follows durable apply. Full event/bootstrap
registration recovery, per-generation derivation and committed-but-underived
`pending` behavior are unchanged.

### 2.3 Destination and pending semantics

`bins/spc-pool/src/registration.rs` derives `U_i` from current `a_send` and
registered keys, then actual server/mailbox public metadata. It constructs:

- `static`: `M + D + t_ark*S_mb`;
- `epoch_derived`: `M_i + D + t_ark*S_mb`, with frozen `SPc/Mailbox` tweak;
- `epoch_registered`: exact-height `M_i`, or static fallback, or `hold`/no
  destination when missing;
- `none`: policy address with explicitly empty delivery.

`Front` installs full live registration events and restores full bootstrap
state. Per frozen distribution generation it recomputes `dest_id`, which
travels in the production block event/snapshot. Committed-but-underived or
`epoch_registered + hold` routes `pending`, not `forfeit`; settlement continues
to re-decide against `ark_delivery`. Old v1 accounting behavior, v2 checked
arithmetic and per-job credit, type-32 restrictions, unknown share outcomes,
pool latency behavior and Ark order cap/retry semantics were not changed.

The Ark-role order/receipt structs were untouched. Tests explicitly capture
payloads without `sp_id`, `B_scan`, `B_spend`, `P_i` or block hash. Registration
streams/events are SPc-side only and are not exposed to the `ark` role.

## 3. AP-3 semantic adapter (independent fixture/handoff)

There is only one 0x5350 codec, `spc-ext`; no wallet fork was imported. The
adapter from AP-3 semantic state should use these definitions exactly:

| Semantic value | Canonical AP-2 mapping |
|---|---|
| `RegistrationPayload` | Populate `SetArkIdentity` fields from SP address/scan key, B_ark, M, D, fallback; `version=1` implicit/constant |
| reserved request counter | Populate trailing `request_id`, then call canonical `sign`; counter participates in signature and cache/journal matching |
| `applied` / `already_applied` | Success `state=0/1`, current fields and returned durable `registration_rev` |
| `superseded` | Success `state=2`: old request was applied historically and is **not current**; do not honor its old fields as live delivery |
| terminal mismatch / stale | Error `registration-mismatch` / `stale-request`, never roll back; unwind with a fresh same-payload counter under AP-3 policy |
| commit unknown | Retryable error `registration-commit-unknown`; same ID retry and no mining |
| durable revision | Payout commit revision. It is **not** fresh front-event `rev` and not a client input |

Independent exact fixture generation is available from
`crates/spc-ext/tests/registration_v1.rs::request` (BIP340 fixture) and
`bins/spc-pool/src/registration.rs::tests` (approved static, epoch-derived and
variant-A address vectors plus `none`). Exact payload schema is section 2.1
above and WIRE 3.7; adapters do not need to restate SV2 framing internals.
`RegistrationService::handle` is the production server semantic seam;
`RegistrationTransport` remains client-side.

Two AP-3 integration points require attention rather than pool-policy changes:

1. AP-3 comments/helpers describe unsigned request counters. The canonical AP-2
   signer includes the counter. The adapter must not provide a second digest.
2. AP-3 `epoch_derived` helper shows `mailbox_dh=None` while WIRE/derivation
   require D for `M_i + D + t_ark*S_mb`. The adapter must supply D or reject
   locally; pool `bad-mailbox-fields` does not imply a new D-optional policy.

Its stock persister rebuild/recovery and receiving workflow remain outside this
proof.

## 4. Normative fragments

- `docs/decisions/2026-11-08-ark-registration-ack.md`: decision and explicit
  signed-counter/Fable amendment plus AP-3 boundary.
- `docs/changelog/2026-11-08-ark-registration-ack.md`: exact bullet for the
  merge session to splice into SPEC frontmatter (newest-first), per
  `docs/MERGE-QUEUE.md`.
- `docs/SPEC.md`: migration tables in section 5.1 and registration contract/
  behavior in 11.4 and tests in 11.9.
- `docs/WIRE-FORMATS.md`: full `ArkRegApplied` in 3.2, `spc:arkreg` + scoped
  cache in 3.3, bootstrap registration additions in 3.5, exact v1 payloads and
  signed preimages in new 3.7.
- `scripts/check-lab-docs.py` passes.

## 5. Actual verification

All jobs used `--jobs 2`; DB/stream integration used `--test-threads=1`. Tests
used the worktree/Nix development tools, fresh PostgreSQL 17 and Redis 8.10.3,
real Noise/SV2 and public synthetic server keys. There is no node, mining or
Ark-service dependency in these proofs.

| Check | Actual result | Log under `/tmp/opencode` |
|---|---|---|
| `cargo test -p spc-ext --jobs 2` | 13 passed (8 unit + 5 v1 negative/correlation/validation) | `AP2-codec-frozen.log` |
| `cargo test -p spc-accounting --lib --jobs 2` | 65 passed | `AP2-accounting-frozen.log` |
| `cargo test -p spc-store --lib --jobs 2` | 10 passed | `AP2-store-lib-frozen.log` |
| `cargo test -p spc-pool --lib --jobs 2` | 228 passed, including all destination variants/API advertisement and prior pool behavior | `AP2-pool-lib-frozen.log` |
| real `spc-store --test ark_registration` | 2 passed: two users ID 1, duplicate/mismatch/newer/older, counter zero, restart + journal recovery | `AP2-store-integration-frozen.log` |
| real `spc-pool --test ark_registration` | 5 passed: cache expiry/restart, replay states, commit-before-success/late reply, row-lock concurrency, real epoch mailbox journal, actual 5 s retryable unknown | `AP2-stream-integration-frozen3.log` |
| real `spc-pool --test ark_publication` | 3 passed: real SV2 + emitted events + bootstrap + frozen block `dest_id` + hold pending, disabled router, payload privacy | `AP2-publication-frozen.log` |
| `cargo clippy -p spc-ext -p spc-store -p spc-pool --all-targets --jobs 2 -- -D warnings` | passed | `AP2-clippy-frozen.log` |
| `cargo fmt --all -- --check` | passed | `AP2-fmt-frozen.log` |
| `python3 scripts/check-lab-docs.py` | passed | `AP2-doc-check-frozen.log` |

The five-second deadline test measured elapsed time in the 4.8-8 s accepted
assertion interval and returned retryable commit-unknown with no payout running.
A separate shortened-deadline run committed only after the caller timed out;
the identical retry then returned the durable registration. The row-lock test
runs eight concurrent counter registrations and leaves counter 8 current with a
single applied journal effect per request. Cache expiry really expires the
Redis ack key before replay; the cache-hit-vs-expired state distinction is
asserted rather than assumed.

Earlier failing evidence is retained under `AP2-test-first/` and intermediate
`AP2-*real*.log` files; only the exact commands above are claimed for the final
source. A whole-workspace test/flake check was **not** run in order to avoid the
requested full build storm. Targeted changed-library/all-target checks and real
service-backed AP-2 integration are the checks claimed here. The development
shell emitted benign warnings about its read-only Cargo cache, not test errors.

## 6. Precise merge instructions against T4a/AP-3 integrated primary

Apply narrowly by hunk, **after T4a then AP-3**, preserving their approved
main/front/payout, `hd` module/crypto exports/Cargo edges and SPEC sections.
Do not copy whole files or overwrite integration handoffs:

- `Cargo.lock`: merge/regenerate only after `bins/spc-pool/Cargo.toml`; this
  packet adds direct test deps `sqlx`/`uuid` and `spc-crypto/ark` to pool.
  Preserve AP-3 dependency edges and do not replace the integrated lockfile.
- `bins/spc-pool/src/lib.rs`: add only `pub mod registration;`.
- `bins/spc-pool/src/config.rs`: add public `server_pubkey`/`mailbox_pubkey`,
  defaults and front-required validation plus fixtures. Keep AP-3 config units.
- `bins/spc-pool/src/main.rs`: bind registration server ID in payout; require
  Redis for enabled delivery; install actual RegistrationService on SV2; pass
  `ArkServerMetadata` into `front`. **Exclude all T4a `Published.job_id` and
  witness work.**
- `bins/spc-pool/src/front.rs`: add registration maps/full live+bootstrap apply,
  per-generation destination refresh and `ArkCapability` commit state only.
  **Exclude T4a witness wiring.**
- `bins/spc-pool/src/payout.rs`: add durable registration handlers/cache/events,
  journal accessors, clone/config stream names, epoch fallback and explicit
  `BYTEA[]` settlement delivery cast. **Exclude T4a `detect_mode`, 224-WU and
  witness-carrier changes.**
- `docs/SPEC.md`: apply only migration table/11.4/11.9 fragments. Preserve
  AP-3 11.5/11.10 and T4 2.4.2/3.3/4.8. Splice the separate changelog bullet;
  do not edit its frontmatter by whole-file replacement.
- Codec, accounting and store registration files are independent component
  edits, but must still be diffed against final integrated primary in case a
  later shared Cargo edge has moved. Migration numbering must be rechecked.
- T4a serialization/main/publication tests and AP-3 crypto/HD/SPEC modules
  remain in their respective packets and are not part of this delta.

## 7. Remaining live gates and explicit non-claims

- AP-1 named `bark-v31` node/Bark RPC compatibility and strict fresh probe are
  not exercised or claimed here.
- T4b canonical mixed-witness mining is a later prerequisite. T4a was not
  imported by this packet, whether earlier candidate or later approval.
- AP-4 actual stock `captaind`, independent service DB/datadir/mnemonic, funded
  pool floats and stock Bark wallet services are not running.
- AP-5 actual VTXO send/receive, expiry/ancestry handling, refresh, offboard,
  float conservation and continuous-mining conservation checks remain live
  acceptance gates. No `H_f+6` funded VTXO or refresh is claimed from these
  synthetic-key tests.
- AP-3 stock persistent wallet rebuild, real VTXO recovery and real Ark receive
  paths are outside this packet's proof. Codec and pool adapter fixtures are
  delivered for separate integration.
- R3/R4 float/expiry scenario work does not change any pool policy implemented
  here. No float capacity, expiry or live-server success has been invented.

## 8. Frozen candidate manifest and stopping owned resources

`/tmp/opencode/AP2-review-manifest.json` is generated after all checks. It
contains sorted file paths, SHA256 and modes in compact JSON order with
`files=[{path,sha256,mode}]`. `full_hash` hashes that full source list including
this report. `code_hash` hashes the same list with only this report excluded
(as requested), never Git metadata, target/result/cache/temp directories, or
`spc_ark_client/test`. It also carries the frozen baseline, exact 30-row delta
list, manifest verification outcome and migration slot. A final read-only
verification recomputes both hashes and verifies bytes/modes again.

One fresh owned PostgreSQL cluster and one fresh owned Redis process supply
integration proof. They are stopped privately after the last evidence run;
no shared system service is reused or modified. Their startup/stop logs and
private temporary directory are recorded in `AP2-storage-*` artifacts only.


## 9. Fable rework findings and dispositions (2026-11-08)

The original `f8bf8c80`/`5e21d48b` manifest and its original receipts are
preserved unchanged under `/tmp/opencode/AP2-review-manifest*` and the original
`AP2-*` logs. This section dispositions the concrete Fable
`REQUEST_CHANGES` review against the reworked candidate, not against an
unchanged prior receipt.

| Fable finding | Implemented disposition and current evidence boundary |
|---|---|
| 4.1 stale cache `Applied` after newer durable current | Internal fresh attempt nonce is appended to both internal request kinds and required in both cache-key and JSON. Attempt-scoped keys stop overwrites; every handle necessarily resolves via Postgres. Tests `live_cache_never_overrides_durable_supersession` covers exact A1->B2->A1 and already_applied->B2->A1 without expiry/mismatch. SPEC 11.4, WIRE 3.3, decision 5 and changelog now say this nonce is internal only, not an operation ID or extra effect. |
| 4.2 `upsert_ark_delivery` 7/6 bind | Added the seventh bind and `epoch_fallback = EXCLUDED.epoch_fallback`, preserving the public API. Real modified-file command is staged as `spc-store --test postgres --test ark_registration`, not just the author's previous selected integration test. |
| 4.3 poison pending epoch request | Added semantic rejection resolution with exact code and optional durable `rejected` journal. Payout distinguishes semantic rejection from DB/Redis/unknown failure. Wrong-mode epoch ack is immediate `bad-mailbox-fields`, PEL is resolved, stream processing continues. Unknown missing identity cannot create an FK-unsafe journal. `signed_epoch_mismatch_is_terminal_acked_and_does_not_poison_later_records` tests terminal ack, pending empty and a later applied identity. |
| 4.4 rail/destination change did not roll job | `published_entries` tracks a deterministic digest beside published coinbase fingerprint. `registration_destinations_and_rails_roll_job_even_when_coinbase_does_not_change` covers hold/pending -> derived Ark plus changed Ark destination with same coinbase fingerprint and StaleJob for the old job. No T4 `Published.job_id` source was imported. |
| 4.5 operator/config and stale text | SPEC/example/ARCHITECTURE document public server/mailbox keys with front-only startup validation and server-id match. stale-request now means unapplied lower counter, not promised pruning; busy/rate-limited are reserved rather than claimed emitted. Pre-AP-2 short registration events require bootstrap, not replay. Frozen report log citation corrected to `AP2-publication-frozen.log`. |
| 4.6 cross-kind domain | Epoch requests now sign `SPc/EpochMailboxes`, covered by an independent digest test. Client payload-reuse digest can intentionally exclude request ID; canonical signature digest still includes it. |
| 4.7 miscellanies | `hold` outside epoch_registered becomes bad-mailbox-fields; malformed internal identity events log-and-skip without `expect`; registration server ID is required, not `unwrap_or(e.ark_id)`; semantic identity placeholder uses configured network. Actual 0x5350 handler and disabled-advertisement gating remain. |

### 9.1 Preliminary pre-validation receipts (superseded by section 11)

- `cargo check -p spc-pool --features rpc --jobs 2`: **passed** in the heavy
  lock wrapper, proving production rpc-only compilation without test
  dependencies (`/tmp/opencode/AP2-rework-check-rpc4.log`).
- `cargo check -p spc-pool --all-targets --jobs 2`: **passed**
  (`/tmp/opencode/AP2-rework-check-all3.log`).
- `python3 scripts/check-lab-docs.py`: **passed**
  (`/tmp/opencode/AP2-rework-doccheck-preliminary.log`).

The consolidated real PostgreSQL/Redis command submitted the exact modified
store and regression tests but returned **status 43,
`BLOCKED: /tmp/opencode/silent-pool-heavy-build.lock is busy`**:

- `spc-store --test postgres --test ark_registration`;
- `spc-pool --test ark_registration`, including the exact stale-cache and
  static-mode poison regressions described above;
- `spc-pool --lib`, including the new rail/destination job-roll test.

It did not run them. There is **no new green receipt for these cases**, and the
final clippy/rustfmt/test set has not been claimed. Original earlier passing
receipts remain separate. The fresh pair was under
`/tmp/opencode/AP2-rework-storage.*`; only this packet owned it and it was stopped
before manifest generation (`AP2-rework-storage-stop.log`). The heavy-lock
and disk-wrapper is `/tmp/opencode/AP2-heavy-exec.sh` (free >50 GiB before the
work was attempted; estimated growth leaves >20 GiB, one nonblocking lock).
No polling loop, alternate toolchain, no-lock path, new target, shared GC,
node/mining/live Ark path or primary write was used.

### 9.2 Source-bound rework receipt and integration hold

The exact source list, hashes and modes after the rework are regenerated into
`/tmp/opencode/AP2-rework-manifest.json` with the same compact JSON recipe used
by Fable (path-sorted `files=[{path, sha256, mode}]`; `full_hash` includes the
report, `code_hash` excludes only it). `/tmp/opencode/AP2-rework-delta.json`
retains explicit frozen-before hashes/modes and comparison against the approved
370-file baseline rather than the noisy Git HEAD. A separate verification file
recomputes all entries and both canonical hashes. The original manifest is not
rewritten by these artifacts.

Merge is **held**. The narrow merge order remains T4, AP-3, T0/operator storage
and config handoffs, then this AP-2 diff. In particular, keep T4's
`Published.job_id` and witness changes, AP-3's HD module/exports/Cargo edges,
and split the operator config/store/server-metadata and internal event changes
 narrowly; do not impose AP-2's registration handler/config on an ark-only role.
Integration requires the same Fable's re-review and the blocked actual PG/Redis
receipts, not an assertion of approval from this report. No primary/frozen/other
worktree was edited; the old review-report self-contained handoff is local to
`lab-ark-registration` only.


## 10. Guarded validation attempt (2026-11-08, exact outcomes)

A single lock-holding parent ran the corrected storage/test sequence with
`flock -o` replaced by an explicit child-FD close helper. PG/Redis started only
after lock acquisition, and daemon children did not retain the parent's lock.
The first full run found three errors; subsequent source fixes did not weaken
any test assertion:

| Real check | First-run result | Exact follow-up |
|---|---|---|
| `spc-pool --features rpc` check | 0 | Pass, but not invoked again after bounded fixes. |
| `spc-ext` | 0 | 14 tests pass, including the epoch-domain/fallback case. |
| `spc-store --test postgres --test ark_registration` | `ark_registration` 2 pass; `postgres` 19 pass, **1 fail** | Fable's bind counterexample reproduced as `08P01`: six binds against seven placeholders in `upsert_ark_delivery`. Added the seventh bind and matching conflict assignment. |
| `spc-pool --test ark_registration` | 6 pass, **1 fail** | Exact stale-cache A1->B2->A1 and already_applied replay cases pass. Poison client got correct terminal `bad-mailbox-fields`, but reader then failed `Db(ColumnNotFound("error_code"))`. Added `error_code` to `ark_registration_resolution`'s SELECT. |
| `spc-pool --test ark_publication` | 3 pass | Actual SV2/publication/privacy checks pass. |
| `spc-pool --lib` | 228 pass, **1 fail** | New job-roll test called `share_for` before initial refresh (`front.job().unwrap()` at front.rs:1817). Setup now derives scan/spend/sp_id before first job without changing the assertions. |
| Targeted clippy (`spc-ext`, `spc-store`, `spc-pool`, warnings denied) | 0 | Pass on first-run source. Not rerun after three bounded fixes; no final green claim. |
| `cargo fmt --all -- --check` | 1 | Formatting mismatches. Host rustfmt was staged as a patch. Its consolidated rerun was blocked, so this fix is also unverified. |

After those three precise fixes (`crates/spc-store/src/postgres.rs`, test setup
in `bins/spc-pool/src/front.rs`), the required consolidated rerun returned
**status 43, lock busy, before storage allocation**, logged at
`/tmp/opencode/AP2-postvalidation-final-gate.txt`. Therefore:

- stale-cache regression is **really proven** on the current attempt;
- the full PG/Redis regression suite, poison journal/PEL follow-up, and new
  job-roll regression are **not green yet**;
- the 7th bind, added SELECT column, and test setup are source-fixed but have
  no fresh full suite receipt;
- final fmt/clippy remain pending under the required locked gate.

No assertions were removed, no fixture was relaxed and no receipt was invented.
Both the initial status-43 manifest and the previous full/code hashes remain
unchanged on disk. The next valid locked run must use the postvalidation
manifest, not interpret these fixed source sites as runtime results.


## 11. Historical pre-R2.1 source-bound validation receipt (2026-11-08)

The final guarded run used the exclusive AP-2 priority reservation. Owned
formatting ran first on AP-2-owned files, then one parent process held the
heavy lock through the remaining checks. `flock -o 9` closed the daemon's
inherited descriptor while the parent retained it. The fresh PostgreSQL and
Redis instances were allocated only after that acquisition and formatting
gate, and were stopped by the EXIT/INT/TERM trap on completion.

The consolidated process returned **status 0**, with every required command
returning zero in `/tmp/opencode/AP2-finalgreen-exits.txt`:

| Final check on this source | Exit / actual result | Log under `/tmp/opencode` |
|---|---|---|
| `spc-store --test postgres --test ark_registration` | 0: 2 registration journal + **20 postgres tests passed**, including the seven-bind upsert and fallback conflict assignment | `AP2-finalgreen-store.log` |
| `spc-pool --test ark_registration` | 0: **7 passed** - two users ID1/nonce, restart/cache expiry, replay/mismatch/stale states, commit-before-success and late unknown, poison rejection plus PEL empty and later progress, epoch journal and row-lock concurrency | `AP2-finalgreen-registration.log` |
| `spc-pool --test ark_publication` | 0: **3 passed** - actual SV2 + emitted front event + frozen block destination + bootstrap hold pending, disabled advertisement and identity-free Ark payloads | `AP2-finalgreen-publication.log` |
| `spc-pool --lib` | 0: **229 passed**, including job-roll initialization, unchanged coinbase fingerprint with rolled rail/destination and StaleJob on old work | `AP2-finalgreen-poollib.log` |
| `spc-ext` | 0: **14 passed**, including `SPc/EpochMailboxes` domain separation and invalid hold outside epoch_registered | `AP2-finalgreen-codec.log` |
| `cargo fmt --all -- --check` | 0 | `AP2-finalgreen-finalfmt.log` |
| Targeted clippy with warnings denied | 0 | `AP2-finalgreen-clippy.log` |
| `python3 scripts/check-lab-docs.py` | 0 | `AP2-finalgreen-docs.log` |

`AP2-finalgreen-stop.log` records the owned PostgreSQL server shutdown; the
Redis instance was shut down through the same cleanup path. No full-workspace
or Nix flake rebuild was run, no live Ark/node/mining services were used, and
no earlier 101/43 logs were removed to obtain this result. The final manifest
is therefore bound to actual passing tests, not compile-only checks.

The postvalidation artifact is `/tmp/opencode/AP2-finalgreen-manifest.json`,
with independent `/tmp/opencode/AP2-finalgreen-manifest-verification.json` and
`/tmp/opencode/AP2-finalgreen-delta.json`. Earlier frozen manifests and the
8621ec/9182e3 postvalidation hashes remain untouched in history. The source
binding and hash recipe are unchanged: path-sorted compact JSON entries
`{path, sha256, mode}`; `full_hash` includes this report and `code_hash`
excludes only it. The real rerun required by Fable exists now; no primary or
integration approval is inferred.

For integration, keep the narrow T4/AP-3/T0 handoff boundaries from section 6.
Fable must re-review these exact files and the final green logs. Live Ark,
server profile, funded delivery and broader end-to-end gates remain separate
AP-1/AP-3/AP-4/AP-5/T4b work.


## 12. Historical R2.1 source checkpoint (before the section 13 verification)

Round-2 finding R2.1 is a true SQL resolution bug: `reject_registration` can
journal `(sp_id, request_id, 'rejected')` without advancing the current counter,
then a valid request at that ID used to take the new-effect path and collide on
the journal primary key. The bounded fix in
`crates/spc-store/src/postgres.rs` performs
`SELECT ... FOR UPDATE` of the existing `(sp_id, request_id)` outcome after the
identity lock and **before** `is_new`. Different digests resolve mismatch; equal
digest replays rejected terminal code/current already_applied/older superseded.
No insert means no PK violation and no side effect. Missing/pruned lower rows
keep prior Stale semantics. No counter is advanced merely to hide the race.

Authored regression in `bins/spc-pool/tests/ark_registration.rs`:
`rejected_id_collision_is_terminal_mismatch_without_poison_or_mutation` covers
static ID1 Applied -> valid signed epoch ID2 Rejected -> different-digest valid
identity ID2 terminal Mismatch. It also asserts unchanged last request/rev,
same-rejected-digest returns `bad-mailbox-fields` (not commit unknown), empty
PEL after two passes, and ordinary valid ID3 Applied. The pre-existing
`live_cache_never_overrides_durable_supersession` regression remains unchanged.

Validation has **not been started in this source-only checkpoint**. Current
priority belongs EXPACCESS; no lock, Cargo/Nix, storage, protocol work or other
SOURCE edits were attempted. `/tmp/opencode/AP2-finalgreen-manifest.json` and
its receipts are untouched. The exact final commands for the reserved window
are:

```
nix develop -c cargo test -p spc-store --test postgres --test ark_registration --jobs 2 -- --test-threads=1
nix develop -c cargo test -p spc-pool --test ark_registration --jobs 2 -- --test-threads=1
```

Run both against one newly created owned PostgreSQL/Redis pair, after
`flock -n /tmp/opencode/silent-pool-heavy-build.lock` acquires with the parent
window, >=50 GiB free and >=20 GiB retained after estimated growth, and close
inherited descriptors in daemon children. The later receipt must include final
fmt/clippy because this source changed. No Ark/server/mining/wider workspace
is needed. Nonblocking `0001` style cache growth, upsert-on-conflict fallback,
and internal-sp_id mismatch are follow-up notes and are **not** represented as
fixed in this checkpoint. Only the SPc-side SQL resolution changed; the vanilla
Second.tech captaind path is untouched.

## 13. R2.1 final focused gate and freeze (2026-11-08)

Replacement verifier session `ses_e42f6aeb7ffeFD76hA9245WKg5` received the
explicit parent window and verified the already-authored SQL fix. The failed
replacement launch `ses_e4683467dffeSvKkshwNsMjADp` produced no code/test result.
The priority file was checked inside each acquired nonblocking
`flock -n -o -E 43 /tmp/opencode/silent-pool-heavy-build.lock` invocation;
`-o` kept its descriptor out of the Nix shell and all daemon descendants.
The existing worktree `target/` was explicitly reused. The pinned worktree
`nix develop` shell supplied Cargo, PostgreSQL 17.11 and Redis 8.10.3.
Actual target, Nix store and scratch filesystems each had at least
81,191,440,384 bytes available throughout the final run; the 14 GiB peak-growth
estimate left more than 20 GiB. Storage started only inside that guard.

### Exact attempts and bounded edits

- `AP2-r21-attempt1-*`: store tests passed (2 + 20); pool regression compilation
  returned **101**. The new test passed an unawaited `drive` to `success` twice,
  awaited the synchronous result once, and shadowed the `error` helper. The
  compiler log and actual failing test source bytes are retained at
  `/tmp/opencode/AP2-r21-attempt1-registration.log` and
  `/tmp/opencode/AP2-r21-attempt1-test-source.rs`.
- Only that new test was corrected: await both drives, rename the shadowing
  local, assert initial Applied, and compare the complete stored delivery row
  after the mismatch. The production SQL was not edited by this verifier.
- `AP2-r21-final-*` gate/logs without `green` are the **intermediate failed
  formatting attempt**, not the final success: store 2 + 20 and pool 8 passed,
  then fmt returned **1**. Its three formatting diffs were applied only within
  the owned regression/helper. The failed log remains intact.
- `AP2-r21-final-green-*` is the final complete successful invocation below.

The regression proves static ID1 Applied, signed epoch ID2 terminal
`bad-mailbox-fields`, different-digest valid identity ID2 terminal
`registration-mismatch`, unchanged delivery/current counter/current revision,
same rejected epoch replay retaining the original terminal code, drained PEL,
then valid ID3 Applied. Existing two-user/progress and live-cache supersession
regressions also pass. Journal lookup remains before `is_new` under the identity
row lock, so an existing rejected ID never reaches the new-effect INSERT.

### Actual final commands and results

All Cargo commands below ran inside the same guarded `nix develop` shell,
using real owned storage and `--jobs 2` for compilation.

| Command | Exit / result | Log under `/tmp/opencode/` |
|---|---|---|
| `cargo test -p spc-store --test postgres --test ark_registration --jobs 2 -- --test-threads=1` | 0: **2 + 20 passed**, no ignored/filtered tests | `AP2-r21-final-green-store.log` |
| `cargo test -p spc-pool --test ark_registration --jobs 2 -- --test-threads=1` | 0: **8 passed**, including R2.1, no ignored/filtered tests | `AP2-r21-final-green-registration.log` |
| `cargo fmt --all -- --check` | 0 | `AP2-r21-final-green-finalfmt.log` |
| `cargo clippy -p spc-ext -p spc-store -p spc-pool --all-targets --jobs 2 -- -D warnings` | 0 | `AP2-r21-final-green-clippy.log` |
| `python3 scripts/check-lab-docs.py` | 0 | `AP2-r21-final-green-docs.log` |

The consolidated gate exited **0**. Exact exit records, timestamps and disk
checks are in `AP2-r21-final-green-exits.txt` and
`AP2-r21-final-green-gate.log`. The 379-file, raw-mode (644/755) inventories
captured at 21:24:57 UTC before commands and 21:26:04 UTC after commands are
identical: tested full hash
`8feb7c05d22ee24e2c978d846a9661a9754cafa8f34e2420bd3f89adbd24cadd`,
code hash
`de0e2158b2ea24846e2edf3b3ecefd401f9b98880b933fed5378a092b099956e`.
Only this report changes after that gate; the final code hash remains identical.

Final owned storage was `/tmp/opencode/AP2-r21-storage.8B0zYh`, PostgreSQL
port 58099 and Redis port 41739. Each attempt used its own fresh pair.
Cleanup compared saved PID/start time/executable/command line before shutdown;
`AP2-r21-final-green-stop.log` records successful cleanup. The wrapper released
the lock on exit. The parent-owned reservation remains for the parent to
transfer. Datadirs and evidence are retained; no shared storage was used.

### Frozen handoff and review disposition

New artifacts are `/tmp/opencode/AP2-r21-final-manifest.json`,
`AP2-r21-final-delta.json`, `AP2-r21-final-manifest-verification.json`,
`AP2-r21-final-receipts.json` and `AP2-r21-final-source.tar.gz`. The source archive
contains exactly the manifest's 379 files (no targets, caches, datadirs or Git
metadata). The hash recipe is unchanged: compact ASCII JSON of the path-sorted
ordered array of `{path,sha256,mode}`, no canonical/base wrapper; full includes
this report and code excludes only this report. Raw filesystem modes are
verified, not normalized silently. Baseline delta remains 23 modified, 9 added,
0 removed against the approved 370-file runtime. The R2.1 delta against the
pre-R2.1 finalgreen manifest is seven modified files, zero added or removed.

The old full `7a8aea5069747c6e6099962f09f7ee5f7ef3f1fd65c07f476f9047dff17f703e`
and code `e0ad690da836be525f43930f6f937ce585f178c0c717a8b0b2be491517d962ba`
remain historical tested identities. Old manifests/receipts are retained;
their hashes alone do not assert preservation of every historical source byte.
The new source archive supplies byte-level preservation for this final freeze.

Round-2 Fable closed all earlier findings and approved the unchanged scope;
R2.1 was its sole blocking finding. This run supplies R2.1's missing actual
regression evidence, not a reviewer approval. The parent verifies the new freeze
before the same reviewer `ses_e44ea4744ffeHG3mTTzBN5l93T` follows up. Narrow
T4/AP-3/T0 integration boundaries in section 6 still apply. Optional ack-cache
growth, upsert conflict fallback and forged-internal-identity notes remain
follow-ups. Unchanged core/publication/229-library proofs remain their earlier
receipts. No funded Ark, mining, server modification or broader readiness is
inferred from this pool-side registration gate.
