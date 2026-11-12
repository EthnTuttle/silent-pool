# Wallet loaded-key SourceFeed v2 intake

Date: 2026-11-10
Status: exact helper source imported; wallet request adapter authored; uncompiled

## Bound source

Producer: `edebd11e723af329117432c96a16e6a74906b7853fee38fa22bbbe1e848f47b0`.
Interface: `981e4dbf96d49e2c6544832bdbb1f860d962eae2a87c723e2c069e0f39465971`.
Handoff: `/tmp/opencode/ladder-audited-warmup-r2/handoff.json`.

Verified archive and delta hashes, all eight declared helper/document rows and
their archive modes, plus the compact sorted-JSON interface hash. Intake record:
`/tmp/opencode/aggregate-wallet-warmup-r2-binding.json`.
The four helper files in this wallet tree match the declared whole-file hashes:

- `bins/spc-agent/src/source_feed.rs`: `756807b897326a0ad00a78496b2c12176294750261d717ffeea6519edad7c0bb`.
- `bins/spc-agent/src/verify.rs`: `2ce503b0b4b0bbb2bc5b774f800987a1a8d9d9d1e3f2cbd1bed6c873fe58fe0a`.
- `crates/spc-scan/src/source_audit.rs`: `1100d99a9946fbb0d25b769a675047d1b6f30bcc42f520b004e1171b36ddb15a`.
- `crates/spc-scan/src/source_feed.rs`: `612ec34eae7bb811cae966fe5e2fd8421502a0ff567d398a0dc8c5907ef4bfd7`.

Only the required scanner `PayoutFormat`/`audit_outputs`, ladder re-export,
module/dependency wiring and existing `JobAuditor` trait were added from the
bound predecessor. The accounting work-unit implementation is byte-identical
to that dependency source. The four helpers use the existing source arithmetic;
no pool store, economic implementation, SPEC or entire agent baseline was copied.

## Wallet request bridge

`wallet/native_source_audit.rs` provides `SourceBinding` (public socket/pool/
collector configuration) and `RequestAudit` (no Clone or Debug). Its synchronous
`from_wallet(&WalletKeys, address, &SourceBinding)` calls the EXACT shared
`UnixSourceAuditor::from_keys` with a borrowed B_scan, loaded public B_spend,
address and network. It retains only the helper's owned Zeroizing scan copy.
No wallet reference, Ark signing key, child seed, database or operation lock
crosses into the audit future.

`RequestAudit::verify_received(self, &ReceivedJob, attempt, deadline)` consumes
that one request auditor, passes the native outer 128-bit attempt unchanged,
and applies the service's earlier total deadline around the helper's own
10-second exchange. It returns only `VerifiedSourceEvidence`. The helper owns
all SourceBundle/owned-output/Rule B/history/checkpoint verification, Consumed
signing, durable collector Committed validation and key cleanup. There is no
second crypto implementation, framing helper, signer or secret-file read.

The bounded in-process `auditor_channel` lets the audit thread ask the existing
wallet owner for that one consuming verifier. `AuditorFactory::serve` is driven
concurrently with the wallet loop and borrows the original keys; it retains no
long-lived cloned auditor. Queue entries contain only a deadline and reply
channel, with no key or caller-selected path/binding. Construction is synchronous
and performs no collector I/O. Expired/cancelled requests are skipped; a verifier
whose receiver disappears is dropped through the shared cleanup implementation.
The factory must be polled concurrently, not only at the 15-second wallet tick.

The old retained-key cleanup gap is closed in the new helper source. Its
Zeroizing request copy and ErasedSecret/ErasedKeypair guards remain unchanged.
Their supported best-effort erase semantics and the original wallet key's
continued lifetime are preserved. Imported cleanup tests remain unexecuted.

## SourceFeed v2 and audited underfill

All source transport/evidence uses `spc.source-feed/2`. Mandatory
`Consumed.window` carries canonical decimal-string u128 target, actual work
and shortfall plus underfilled. The shared `SPc/SourceConsumedV2` digest and
collector commit bind those facts. No v1 source activation fallback exists.
ReceivedJob, TraceSeal, Checkpoint and Committed signing preimages stay exactly
as declared. Full-Q payout-proof/1 bundle compatibility remains as declared by
the helper.

Audited underfill requires complete eligible history, independent seals,
high-water marks, finding-slice exclusion, actual raw denominator, prior owed
amounts, conservation/witness checks and every owned payout output. Missing
history never becomes a warmup allowance. The wallet bridge introduces no
separate warmup classification or activation decision.

## Tests and serial union

Imported unchanged owner tests cover full loaded-key/mock-collector verification,
invalid revenue refusal, public v2 evidence, empty/partial/full source windows,
history/witness failures and scan cleanup on success/error/cancellation. Added
wallet adapter tests cover loaded identity binding, no retained wallet borrow,
unchanged outer attempt, cancellation closing before any ACK, outer deadlines,
zero attempt refusal, and v1/incomplete-history rejection. These are unit/mock
fixtures, not native or funded acceptance.

The full audited translator implementation belongs to the producer's existing
SourceFeed packet. This tree imports only its identical public trait needed to
compile the shared helper; it does not enable a translator or gateway in the
native path. During serial union keep the producer's current audited translator
body and omit the trait-only hunk if already present. Likewise reuse matching
helper files instead of adding them twice. Keep the existing SourceFeed config
field and all wallet/aggregate fields from the prior selective join.

The native codec is still pending. No native request parsing, activation tag,
response signing, replay/session policy, peer registry or production socket
startup is invented here. The native service scaffold is ready to call this
bridge once the codec and current runtime peer/Unix lease are bound. The original
708-row freeze/archive, aggregate interfaces and protected wallet layers remain
preserved. No Cargo/test/heavy-lock attempt was made; the active primary ticket
retains priority. Default/ark compilation and real native-feed tests await the
assigned slot.
