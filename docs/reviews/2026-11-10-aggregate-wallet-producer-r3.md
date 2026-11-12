# Frozen-order producer R3 compatibility intake

Date: 2026-11-10
Status: source contract verified; Rust compilation/tests and live HTTP pending

## Exact producer binding

- Source: `0515a6a88686f75ccf4ed0ff768a0a0324c7a0b3ae4b95b5acd4a2c126a48a7d`
  (717 rows; parent verified the full snapshot recipe/modes).
- Archive: `6d65f6256078de1a0951d30a887b34c071d9cf5ee66fa17ebe601074920e6a6c`.
- API schema: `ae1eaed457e2d3373b3e4ffd032def20bdf10f806539769cbd5b64ccd901c466`.
- Handoff: `/tmp/opencode/ladder-followup-source-r3/handoff.json`.
- Selected intake: `/tmp/opencode/aggregate-wallet-producer-r3-binding.json`.

Independently verified the archive SHA256, all six declared API source rows
against the manifest and archive modes, the compact sorted-JSON API schema
hash, and the 114-byte stored fixture's SHA256 and full signed-order digest.
Additional selected source rows establish scanner/codec and SourceFeed merge
compatibility. No current producer worktree, private runtime database or server
was accessed for this intake.

## Client changes and compatibility

The exact stored signed ArkSendOrder has no envelope. Selectively imported
`MIN_ENCODED_LEN = 93` and `MAX_ENCODED_LEN = 348` into the existing codec and
use that maximum in the transport. Encoder, decoder, order signature preimage,
grant codec and frozen-order digest implementation otherwise match the producer.
The client requires HTTP 200, `application/octet-stream` and `no-store` before
decoding, checking ID/canonical roundtrip/signature, and storing immutable bytes.
The receipt join still checks the grant digest against those exact signed bytes,
the frozen anchor/full policy/full server, complete genesis/funding, unique IDs
and exact aggregate total before its atomic credit/member allocation.

The producer authenticates before checking the 32-hex order ID. The client
always generates that ID from 16 bytes. HTTP 400/401/404/503 remain errors with
no watch closure. Unknown and foreign orders share 404 semantics. Frozen,
published, authorized, blocked_unknown, sent and cancelled orders are readable;
the client wakes receipt work on Sent, while retaining other lifecycle watches.
Owner status field shapes and 2048-row limits match the current consumer.

`crates/spc-ext/tests/frozen_order_owner_api.rs` is a raw-codec compatibility
test against the actual stored unit fixture. It checks the exact bytes/hash,
order ID, 10,000 sats, height 207, roundtrip, full digest including signature,
93/348-byte boundaries and strict trailing/truncated-byte refusal. The fixture
destination is `tark1unit-destination`, not a valid payable Ark address. No
verification key is provided with it, so the test does not claim signature
verification, a positive HTTP execution, ownership or funding. Existing genuine
grant/full-stock-proof policy/server negatives remain separate authored tests.

The imported recovery scanner surfaces (`Found`, `scan_coinbase`, `OutputView`,
`scan_outputs`, `scan_block`, `scan_light`) are unchanged. New SourceFeed/audit
and Rule B economic surfaces do not require a wallet derivation or recovery
algorithm replacement. The exact authorization and ladder module imports are
unchanged. No legacy codec fallback was added.

## Selective SourceFeed union

The producer's agent config snapshot predates the fixed wallet's Ark endpoint,
key and chain fields. Replacing the wallet config with it would remove those
fields and `ark.aggregate_api` validation. The optional external join
`/tmp/opencode/aggregate-wallet-sourcefeed-r3-join.patch` is instead bound to the
producer's exact config/Cargo before rows. Its composed after rows retain:

- Producer `AgentConfig.source_audit`, `pub mod source_feed`, and its runtime
  `rand` / `async-trait` dependencies.
- All client wallet endpoint/chain fields, first-class `ark.aggregate_api`,
  public binding validation and Ark/reqwest/url dependencies/features.

The producer's `lib.rs` already matches the composed module declarations; its
hash is recorded with no replacement hunk. This join assumes the rest of the
actual SourceFeed packet is supplied by its owner. It does not import a second
pool baseline into the wallet tree. When applying over a SourceFeed-composed
agent, use this join for config/Cargo instead of conflicting core hunks and
union Cargo.lock dependency edges (`async-trait`, `reqwest`, `url`) without
changing package versions or pins. The runtime join remains the R2 three-file
patch against its captured primary before rows.

## Remaining boundary

There is no further frozen-order wire/source blocker. The other owner's build
ticket still takes priority; no Cargo build/test or lock attempt was made.
Default/ark compilation, codec and receipt regressions, runtime join tests,
clippy, serial SourceFeed/UI/T1 union and live authenticated HTTP/funded
acceptance still require their assigned execution/integration steps. Known
contract limits remain conservative: no explicit per-member no-delivery
reconciliation, and status exceeding 2048 rows returns unavailable rather than
paginating. Neither permits dropping watches or accepting opaque Sent.
