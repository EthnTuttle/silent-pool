# 2026-11-08 - Lab expanded-access foundation (claims, auth, replay, filter)

## Scope

EXP-ACCESS foundation for the spc-lab controller per plan
`/tmp/opencode/role-ui-plan-review-v5/2026-11-07-live-lab-integration-contracts.md`
section 5: per-principal claim files, Bearer authentication for the expanded
profile, principal-bound command scope, an expanded command-log payload hash
scheme, and the server-side user read projection (snapshot, SSE and
miner-poll). New module `crates/spc-lab/src/access.rs` plus the
`scenario.rs` `expanded_access` field, the `command.rs` hash-scheme
vocabulary, the `engine.rs` scope/projection wiring and the `http.rs`
expanded routes. Packet-owned tests `tests/access_scope.rs` and
`tests/access_projection.rs`; a test-only `--snapshot-worker` hook in
`bin/fixture.rs`. No live agent wallet UI, no AP-3b, no Ark codec or claim,
no hub/static gallery edit, no second controller. This is a lab-local
authority boundary, not a production auth platform.

## Decisions

1. **Claims model**: for `expanded_access: true` scenarios only, `prepare`
   generates one claim file per principal under
   `<run>/private/access/<principal>.json` (directory 0700, file 0600,
   CSPRNG 32-byte hex token, one secret per claim, no rotation). Principals
   are `"operator"` (generated with the run, `may: ["operator"]`), user ids
   (equal to the scenario identity id, claim actor `user/<user_id>`), and
   the in-process `"controller"` principal used when the engine itself
   submits expanded commands. `derive_user_bindings` validates the
   identity-to-miner/worker/pool shape (exactly one miner per identity, at
   least one worker per miner, exactly one pool per miner) BEFORE any
   filesystem, lock or claim effect - a malformed scenario rejects at
   prepare with no directory created. It also enforces the principal
   namespace: an identity id equal to the reserved `operator` or
   `controller` principals, starting with the hashed-filename prefix
   `sha256-`, or colliding with another binding's claim filename (the
   operator's included) is a closed prepare error. Unsafe principal ids
   never form path components: only a closed safe-charset pass through
   verbatim, everything else is written as `sha256-<hex>.json`. Writes are
   atomic (temp file plus rename); legacy scenarios produce no access
   directory at all. At load time the store additionally rejects duplicate
   authority: two files claiming one principal or one token, a file
   claiming the reserved `controller` principal, or a user claim carrying
   operator authority (or the operator claim not carrying it) are closed
   errors - never last-wins.
2. **Revocation and reload semantics**: every authentication performs an
   unconditional, fully validated reload of the claim directory - no cached
   authority and no directory-change signal to wait for. Deleting a claim
   file is immediate per-principal revocation, and an in-place rewrite
   (token rotation or scope reduction) is observed on the very next
   request. A directory that fails any load check - unreadable, badly
   permissioned or owned, unparseable, inconsistent, duplicated - clears
   ALL claims: fail closed for everyone, the operator included, until the
   directory is repaired, after which the next authentication recovers
   normally. All-or-nothing, never forgiven per file. The load validates
   the secret files' hygiene via `symlink_metadata` (never following a
   symlink): the directory must be a real 0700 directory owned by the
   process, and every `*.json` entry a regular 0600 file owned by the
   process - group/world-readable, wrong-owner, symlinked and non-regular
   entries are closed errors, and a bad file is never chmod'ed silently
   back into approval. Creation-time 0700/0600 process-owned modes are
   unchanged. At lab scale (a handful of small secret files) the reload is
   the whole cost of a request; no cache heuristic is introduced.
3. **Expanded replay hash scheme**: `LogLine::Begin` gains optional
   `hash_scheme` and `principal` fields (`serde(default)` +
   `skip_serializing_if`, so legacy lines stay byte-identical). The legacy
   format is pinned two ways: a synthetic deterministic command asserted
   byte-for-byte against a frozen format literal (today's writer still
   emits it), and the REAL preserved pre-access journal of a finished
   live hub run (`/tmp/opencode/LP-hub-run-1/private/commands.jsonl`,
   sha256 `ac67e403...`, exactly one StopRun begin/finish pair, only
   public lab facts) copied verbatim into the owned fixture test - its
   stored kind-only payload hash matches today's hash computation, a
   fresh open replays it to `Existing`, and today's writer reproduces
   the journal bytes exactly from scratch. The scheme
   discriminator constants are `kind_only_v1` (legacy) and `expanded_v1`.
   The expanded payload hash is sha256 over the scheme tag, the
   authenticated principal, the run id and the FULL `kind` payload -
   idempotence is keyed by principal, run and complete command semantics,
   never `command_id` + kind tag alone. `begin_scoped(cmd, principal)`
   records it; the legacy `begin()` rejects scheme-bearing entries (pinned
   by a direct test: a fresh open over a scheme-bearing journal denies a
   legacy begin with no journal write), so an old runner never adopts an
   expanded entry as its own outcome, and an unknown or missing
   discriminator on a scheme-bearing line fails closed.
   `BeginOutcome::Denied(CommandOutcome)` returns a generic closed
   rejection ("command id is not available for this principal") without a
   journal write and without revealing whether another principal holds an
   outcome for the id.
4. **Command scope**: `check_command_scope` runs server-side before
   `submit`; a user claim may issue only `PauseWorkers`, `ResumeWorkers`,
   `SetWorkerBudget` (own `miner_id`/`worker_ids`) and `send_user_payment`
   (own user); every other kind requires the operator claim. A user's
   `PauseWorkers`/`ResumeWorkers` MUST name a non-empty subset of the
   claim's own worker ids: an empty list means "the whole miner" and stays
   operator-only, and the rule deliberately does not lean on the
   prepare-time one-identity-per-miner invariant (which operator topology
   commands could invalidate at runtime; today control v1 has no
   `add_worker` operation, and the read whitelist below stays load-bearing
   regardless). The claim's worker list is also the read whitelist: a
   worker entity merely matching the claim's miner id never enters the
   filtered snapshot. Payload actor or id disagreement with the claim is a
   closed rejection with no journal effect; the unavailable candidate
   kinds (`send_user_payment`, `bootstrap_fund`) answer through the normal
   outcome surface as unavailable facts, never as auth bypasses.
5. **HTTP boundary**: when expanded access is on, `handle_expanded` gates
   every route on exactly one `Authorization: Bearer` header
   (constant-time compare over the decoded 32 bytes). The only unauthenticated
   GET surface is the closed static bootstrap list (workbench HTML/JS/CSS,
   favicon, relay static assets) - no wildcard, no status. Whole-run
   shareable artifacts (`/artifacts/**`) and the catalog are operator-only:
   a user token is a closed 403 before any file I/O (the private-path
   sanitizer itself is unchanged and still backs the operator route).
   Same-origin is enforced on browser-effect POSTs
   (`Origin`/`Sec-Fetch-Site`); a known limitation is that browsers send a
   literal-IP `Origin` (e.g. `http://127.0.0.1:<port>`) while the page may
   be reached by that same literal address only - the check accepts the
   serving listener's literal authority, and hostname-based origins are
   not supported. The CSP
   `default-src 'self'; connect-src 'self'; form-action 'self'; base-uri
   'none'` applies to expanded HTML, and no CORS header is added anywhere.
   The role relay is GET-only over declared-topology listeners with a
   closed asset-to-`{status, content_type}` map, bounded body and timeout;
   unknown role/asset is 404, scope failure 403, a relay POST 405. The
   relay's `<role>.api` target is currently a TEST-ONLY mapping: the
   runtime's declared addresses populate `*.api_addr` for fixture roles,
   and no production namespace relay reach is claimed (an AP-4 dependency).
   The expanded root `/` serves the legacy SPA assets without a Bearer
   entry point - it is a reserved static shell, not a functional
   workbench; no new frontend scope is claimed. A tokenless runtime health
   probe would falsely report the run unreachable under the expanded
   profile (every private route is 401); no public health-auth bypass is
   introduced. The existing `namespace::http_request` helper supports
   Basic auth only, so wiring an operator credential into it would expand
   this packet's shared-file delta; this is a known auth-blocked probe
   limitation, handed off rather than worked around.
6. **Filtered reads**: the user snapshot, SSE replay, live events and
   `resync` snapshot are filtered server-side before serialization -
   allowlisted run aggregates plus the principal's own
   miner/worker/wallet/payment fields; another principal's ids, block
   hashes, txids or amounts never enter a serialized response, and the
   browser never performs the hiding. Gaps and watermarks stay explicit
   facts (unknown stays unknown); a stale cursor yields a filtered resync
   event carrying the filtered DTO. Every SSE stream re-authenticates
   before every event and at every idle heartbeat - the operator stream
   included, which is otherwise unfiltered: a deleted, rotated or
   re-scoped operator claim closes the open stream at the next defined
   boundary (event or heartbeat) with nothing further delivered, exactly
   like a user stream. The legacy (expanded-access-off) SSE path passes no
   credential and is unchanged. The operator claim keeps the unfiltered
   whole-run view on every non-stream route.
7. **User projection DTO**: `spc-user-projection/v1`
   (`{schema, user_id, observed_at, workers: [...]}`) is served from the
   engine's miner v1 snapshot poll on the authenticated user route only.
   Each worker row carries its own observation age; a failed or missing
   poll yields `null` with its reason, never zeros. The operator route
   serves the unfiltered array; the whole-run snapshot routes stay
   operator/legacy-only.
8. **Test-only fixture hook**: `spc-lab-fixture` gains the additive
   repeatable `--snapshot-worker <id>` flag so its miner v1 snapshot
   answers real worker rows; no production behavior keys off it. The
   fixture's default event shape (including `occurred_at_mono_ms`) is
   unchanged from the baseline.

## Consequences

Legacy scenarios (`expanded_access` absent) keep byte-identical journal
lines and unauthenticated loopback behavior - pinned by
`legacy_journal_stays_byte_identical_and_replays_unchanged` and
`legacy_loopback_behavior_is_unchanged`. An expanded run never accepts a
legacy unbound command as an authenticated outcome, and the expanded replay
checks survive a journal restart. Tokens are run-local secrets and never
appear in a URL, page source, log or shareable artifact; `b_scan`, the
master seed and SPc spend keys are never claim fields. This packet adds no
dependency. The claims/scoping code has no Ark dependency, and no live Ark
or wallet fact is claimed: the user projection surfaces wallet/payment
facts only as not-available until AP-4/T1 interfaces exist.
