# LP-3 HTTP/browser fixes (B8, B10 and HTTP edges): implementation report

Date: 2026-11-05
Author: LP-3 HTTP/browser subagent (TFF implementation; Fable review pending).
Worktree: `lab-runner`, base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.
Scope: disjoint HTTP/browser paths only, shared with the core owner
(engine/supervisor/evidence/config/manifest/command/private artifacts are NOT
touched by this packet).

## Owned files changed

| File | Change |
| --- | --- |
| `crates/spc-lab/src/http.rs` | B10 bounded header reading; Last-Event-ID; cursor validation; generic SSE event type; framing/413 handling |
| `bins/spc-lab/static/app.js` | B8 browser: generic `message` listener, dedupe, manual reconnect with current cursor, miner/worker id mapping, read-only disable |
| `bins/spc-lab/static/index.html` | `#control-error` visible alert line for control refusals |
| `bins/spc-lab/tests/app_helpers.test.cjs` | New: Node builtin test runner tests for the exported pure helpers (10 tests) |
| `crates/spc-lab/tests/http_sse.rs` | Updated to the generic event type; sleep replaced by an observable condition |
| `crates/spc-lab/tests/http_protocol.rs` | New: protocol-bound and cursor tests (7 tests) |
| `docs/reviews/2026-11-05-lab-lp3-http-implementation.md` | This report |

`style.css` needed no change. No new static module was added, so the API asset
routing (`/app.js`, `/style.css`, `/`) is unchanged and complete; the Node test
file lives in `bins/spc-lab/tests/`, deliberately outside `static/`, so it is
not served and needs no route.

## B10 disposition: header bound enforced before buffering

`crates/spc-lab/src/http.rs` replaces the old
`read_line`-then-compare-length reader (Fable probe 4: a 64 MiB request line
with no CRLF was fully buffered before the 400) with:

- `read_bounded_line(reader, budget)`: reads one CRLF-terminated line through
  `BufRead::read_until` behind `io::Take` capped by a shared byte budget. The
  budget covers the request line and the whole header block together
  (`MAX_HEADER_BYTES`, 16 KiB), is decremented by exactly the bytes consumed,
  and a line that does not terminate within the remaining budget is rejected
  immediately without waiting for more data. No line can allocate past what
  was actually read from the socket; the pre-parse socket buffer is 2 KiB.
- A request line of any length with no CRLF is therefore a 400 after at most
  16 KiB read (test `request_line_without_crlf_is_rejected_within_the_16kib_bound`
  sends 32 KiB with no terminator and requires the 400 within a deadline; an
  unbounded implementation blocks and fails the deadline).
- Total-header overflow (one 17 KiB line or many small lines summing past the
  budget, with the final line unterminated so the server consumes exactly the
  budget) is a 400
  (`header_block_at_the_total_16kib_budget_is_rejected_without_a_terminator`).
- Body: the 256 KiB bound is kept. `Content-Length` that does not parse is a
  400 (never a silent zero-length body); a body over the bound is a 413
  answered before the body is read. `Transfer-Encoding` (chunked framing) is
  rejected clearly with a 400 naming the problem. All body/header reads sit
  under the 5 s socket deadline.
- Unchanged: loopback-only bind refusal, `MAX_CONNECTIONS` with 503,
  `shareable/`-only artifact routing with parent-component and `private/`
  denial, 503/404/403 response shapes.

## B8 disposition: browser reconnect, dedupe and controls

### Wire change (one, deliberate): generic SSE event type

Ordinary evidence events are now emitted as `id: <n>` + `event: message` +
`data: <json>`; the specific kind stays in the JSON payload
(`payload.kind`). `event: resync` remains a named event. There is no
standards-compliant `EventSource` listener for "any event name", so the
generic `message` type is the only way a single `EventSource` client can
handle every ordinary kind, including `miner_event`, `spc_evidence` and kinds
added later, without enumerating names (the enumerated list in the old
`app.js` is exactly what Fable B8 flagged). The SSE `id:` line is kept so the
standards reconnect path carries `Last-Event-ID`.

Consumers checked: `common::SseReader::next_event` (core-owned) returns the
event name generically and is only used by my test files; `spc-lab events`
subscribes through the engine, not through HTTP. Nothing outside LP-3
HTTP/browser paths parsed the old named kinds.

### Server side (`http.rs`)

- `Last-Event-ID` (first occurrence, case-insensitive) takes precedence over
  the `after` query parameter; a reconnect that sends the header never
  replays events at or below the id the client last saw.
- Cursor validation: `after`, `Last-Event-ID` and `snapshot?cursor` must be
  well-formed decimal event ids, else 400 with the offending value named
  (previously a bad `after` silently meant 0 and replayed duplicates).
- Future/stale cursors are decided by the evidence store, not the HTTP layer:
  `serve_events` consumes `SubscribeResult::ResyncRequired` and answers with
  an `event: resync` snapshot. The stale case works today; the future case
  (`after > last`) is the core owner's B7 fix in `evidence.rs::subscribe` and
  is asserted by `future_cursor_is_a_resync_never_a_silent_skip` (see
  "pending core items").

### Browser side (`app.js`)

- Reconnect: on `error` the browser closes the `EventSource` (whose
  auto-reconnect would re-request the ORIGINAL `?after=` and replay
  duplicates) and re-attaches from `state.cursor` with exponential backoff
  (1 s to 5 s), reset on `open`.
- Dedupe: `state.lastEventId` tracks the highest delivered id; an event with
  id at or below it is dropped, so the snapshot/stream overlap and a
  reconnect racing an in-flight event cannot duplicate timeline rows or move
  the cursor backwards. Snapshot rendering raises both `cursor` and
  `lastEventId` to `snapshot_cursor`.
- Generic handling: one `message` listener delivers every ordinary evidence
  kind; `resync` re-renders the snapshot, advances the dedupe cursor and
  reconnects from the fresh cursor (the resync stream closes after the
  snapshot event).
- Miner/worker ids: `controlCommand(entity, action, budgetValue)` maps rows to
  the typed command payload. A miner row is the miner itself
  (`pause_workers`/`resume_workers` with `workers: []`). A worker row requires
  `entity.miner_id`: pause/resume sends `workers: [<worker id>]` and budget
  sends `set_worker_budget` with both `miner_id` and `worker_id` (the old
  budget button swapped the two and could never succeed). Worker rows without
  `miner_id` (core owner is adding the field to entity rows) refuse visibly:
  buttons render disabled with an explanatory tooltip, the refusal is shown
  in `#control-error`, and no command is sent with a wrong id. Budget input
  must be a non-negative decimal integer (it is sent as a string, the
  `WideU128` wire form); budget is not offered on miner rows.
- Read-only recorded-run views: preserved. Commands still go through the
  durable command envelope (`actor: "browser"`); when the engine answers
  `rejected_precondition` with the read-only reason, all miner controls are
  disabled and a visible note explains why. No new engine API was needed.
- Run lifetime: untouched - the browser never issues `start_run`/`stop_run`
  and closing the tab has no effect on the run.

## API assumptions and coordination notes for the core owner

1. **`miner_id` on worker entity rows (core-owned): LANDED.** The engine now
   serializes `miner_id: Option<MinerId>` on worker rows
   (`skip_serializing_if = "Option::is_none"`), exactly the contract this
   browser was written against: worker rows carry it and use it; rows
   without it (or `null`) render disabled controls with a visible reason and
   never send a wrong id.
2. **Future cursor -> `ResyncRequired` (core-owned, B7): LANDED.**
   `evidence.rs::subscribe` now returns `ResyncRequired` for a cursor beyond
   the last event;
   `future_cursor_is_a_resync_never_a_silent_skip` passes. `http.rs` never
   decided staleness itself - it consumes the store's `SubscribeResult` for
   both directions.
3. **B14 spelling change (core-owned):** the browser renders whatever status
   string arrives, no coupling.
4. **No Cargo/flake changes.** The Node tests use the builtin `node:test`
   runner only; Node v24.11.1 is used locally (nvm), and the tests do not
   need to run under Nix for this packet - if the core owner wants them in
   the Nix gate, a `nodejs` input plus a `node --test bins/spc-lab/tests/*.test.cjs`
   check is the whole dependency note. No npm framework, no lockfile change.

## Tests and commands

New/updated Rust tests (all in owned files):

```sh
cargo test -p spc-lab --test http_protocol --test http_sse
node --test bins/spc-lab/tests/*.test.cjs
```

`http_protocol.rs` covers: no-CRLF request line bounded at 16 KiB, exact-budget
header block without terminator, 300 KiB body -> 413, chunked framing -> 400
named in the response, malformed `Content-Length` -> 400, malformed
`after`/`cursor`/`Last-Event-ID` -> 400, `Last-Event-ID` precedence with no
duplicate replay, future cursor -> resync, and the generic
`message` event type with kinds carried in the payload. All waiting is
deadline-based (`read_response` polls with a 100 ms read timeout against a
5 s deadline); there are no sleeps.

Exact commands run and logs (all under `/tmp/opencode/LP3-r2-http-logs/`):

- `cargo test -p spc-lab --test http_protocol --test http_sse`
  - `test-http-protocol-sse-1.log`: first run, 4 passed; 3 failed - RST on
    unread bytes in the oversized-header probe (fixed by exact-budget
    construction), reason asserted in the head instead of the body (fixed),
    future-cursor resync pending core B7 (expected at the time).
  - `test-http-protocol-sse-2.log`, `test-http-sse-2.log`,
    `test-http-sse-3.log`: transient compile errors inside the core owner's
    in-flight `evidence.rs`/`engine.rs` edits (not my files; distinguished,
    not "fixed").
  - `test-http-protocol-4.log` .. `-7.log`: iterative fixes of my own probes
    (exact-budget writes so the server consumes every byte and the 400
    arrives as a clean FIN; reason read from the response body; no-CRLF probe
    trimmed to exactly the budget).
  - `test-http-final1.log`: after the core owner landed B7 -
    `http_protocol` 7/7 passed.
  - `test-http-final2.log`: FINAL - `http_protocol` 7 passed, 0 failed;
    `http_sse` 3 passed, 0 failed (including Last-Event-ID replay and the
    real-asset serving check).
- `node --test bins/spc-lab/tests/*.test.cjs` -> 10 passed, 0 failed (log:
  `node-app-helpers.log`).
- `rustfmt --edition 2021` on `http.rs`, `http_sse.rs`, `http_protocol.rs`
  only (core files not formatted - parallel-edit rule).
- `cargo clippy -p spc-lab --all-targets`: zero warnings from my files; the
  only warnings were in the core owner's in-flight `evidence.rs` edits.

Final combined gates (fmt/clippy/package/workspace tests, run-once) are the
final combined owner's job after all files freeze; this packet does not
declare LP-3 approved or complete.

## Non-claims

- No SV2 interoperability, real mining, payout, retarget or canonical-block
  claim is made. The browser is lab infrastructure, not a production UI.
- The HTTP surface remains loopback-only; the bounds are hardening, not a
  security boundary against a hostile network.
- The future-cursor resync behavior is asserted and now passes against the
  core owner's landed B7 fix; it was written before that fix and targets the
  contract, not the implementation.
- A finished run: the evidence subscriber channels close, so the browser
  replays the retained tail and then reconnects with bounded backoff
  (>= 1 s) while showing the last snapshot. The snapshot payload carries no
  run-state field today, so the browser cannot distinguish "finished, stop
  polling" without a new engine-side field; polling is bounded and harmless,
  and adding such a field is a core-side API decision left to the parent.
