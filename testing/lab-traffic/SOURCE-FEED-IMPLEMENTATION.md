# Feed-v2 collector, audited warmup and native route

Contract source: `edebd11e723af329117432c96a16e6a74906b7853fee38fa22bbbe1e848f47b0`,
`/tmp/opencode/ladder-audited-warmup-r2/handoff.json`. Interface SHA256:
`981e4dbf96d49e2c6544832bdbb1f860d962eae2a87c723e2c069e0f39465971`.
The bound `crates/spc-scan/src/source_feed.rs` SHA256 is
`612ec34eae7bb811cae966fe5e2fd8421502a0ff567d398a0dc8c5907ef4bfd7`.
That original freeze was authored and uncompiled. The later tested 419 combined
handoff preserves these helper bytes and is now admitted by the trace/SQL
readers; see [ECONOMIC-RELOAD-419.md](ECONOMIC-RELOAD-419.md). Consumer tests do
not approve a funded run. The source-r3 predecessor supplies migration 0011.

## Implemented collector

The candidate path now durably retains the exact admitted provider checkpoint,
and source-triggered FIFO capture continues through confirmation and pool EOF.
The funded campaign requires final raw/source/SQL convergence and exact job
coverage; see [CHECKPOINT-CAPTURE-DRAIN.md](CHECKPOINT-CAPTURE-DRAIN.md).

`source_feed_codec.py` implements the actual `spc.source-feed/2` U32LE JSON
frames and all five tagged binary preimages. Bytes remain integer arrays;
u32/u64 parsing is lossless. Frame/bundle limits are 9/4 MiB, exchange deadline
10 seconds and checkpoint lifetime at most 30 seconds. BIP340 attestation
signing uses only the explicitly configured collector key; the public BIP340
vector is checked. No wallet signer, funding operation or header search exists.

V1 activation frames reject without fallback. Consumed carries mandatory
`window` facts: `window_target`, `actual_sum`, `shortfall` are canonical u128
decimal strings, and `underfilled` is a boolean. `SPc/SourceConsumedV2` signs the
old binary fields followed by those three U128LE values and one U8 boolean.
ReceivedJob, TraceSeal, Checkpoint and Committed retain their V1 preimages;
Committed binds the new ACK digest. JSON is transport, not the signing preimage.

`source_feed_server.py` binds private Unix sockets and checks peer UID plus
the declared wallet PID/start token. It pins pool and scan public keys outside
incoming frames. The durable hash-chained journal binds its authority config,
agent, attempt and ReceivedJob digest. Proof bytes are retained before delivery.
Consumed ACK signature, independently computed window facts and all checkpoint
fields are checked, then fsynced
before a committed signature is created. Exact retries return the same bytes
and original expiry; changed replay, missing prior preparation, wrong keys,
expiry, supersession, truncation and durable-write failure cannot activate work.
Journal reopening never repairs a tail or silently discards an old attempt.

`source_feed_provider.py` is the production server's only accepted provider.
It joins the actual received job to the newest captured generation, signed
complete source prefix, current two-node raw parent, and its exact historical
SQL snapshot. It recursively checks immutable prior block effects against raw
mined outputs and independently calculated P/E/delta. Historical snapshots are
cached from actual collection, never replaced by current database values.
Unsupported non-coinbase/error-derivation or missing registration provenance
remains unavailable. No ready-bundle file or pool `complete` flag is accepted.
Protocol unit fixtures isolate the replay boundary. `test_audited_window.py`
also runs the real ReplayProvider and Unix exchange over signed synthetic
source, raw Bitcoin and SQL inputs. These are offline consumer tests, not
compiled Rust, real wallet/native activation or funded execution receipts.

`source_feed_runtime.py` wires these components into the actual controller.
With an explicitly supplied `source_feed` input it reads existing rendered
`[ark.native_audit]` collector-socket/public-key settings, requires four wallet and four miner bindings, derives
the expected scan public key/sp_id from the actual provisioned public SP
address, and binds the declared wallet/miner/front route. It creates only QA
collector threads/sockets. It neither edits another role's config nor loads
the wallet scan-secret file. Source-r3 SQL goes through the existing owned
placement-bound local/broker collector using the closed query contract.

The input's `source_feed` object supplies `contract`, `pool_pubkeys` by front,
`collector_key_file`, `collector_pubkey`, `migration_checksums` and four `agents`
with `wallet_entity`, `miner_id`, `front` (optional independent `scan_pubkey` and
`sp_id` pins). The rendered socket must be inside this case's `runtime/private`
tree, matching the approved wallet renderer. Its parent directories must
already exist with the actual user's ownership, mode 0700 and canonical paths;
the endpoint must be absent. The QA-private sibling is not a renderer target.
The production provisioner must supply those matching configs; this harness
does not silently patch or invent them. Collector cleanup closes listeners,
waits for handlers and closes journals, with an explicit supplemental verdict.

The post-mining `payout_ladder` audit now invokes `payout_history.py` and
`aggregate_history.py` over the exact retained checkpoints, signed source and
raw chain. This performs full-rail effect/consumption and committed bookkeeping
checks and joins available immutable aggregate/order/grant facts. It does not
turn unknown historical delivery/derivation or wallet receipt into approval.
The pre-mining provider remains fail-closed on its existing narrower supported
rail evidence until the missing historical records have a source-owned API.

## Migration 0011

`economic_checkpoint.py` parses the actual twelve columns and exact captured
PostgreSQL JSONB byte strings. It verifies snapshot SHA256, fresh classification,
the exact uninitialized singleton, empty monetary snapshots, independently
pinned prior SQLx migration checksums, original history_start, preceding
watermark, transaction/snapshot visibility and the real checkpoint INSERT.
Only that validated captured seed initializes row replay; no historical INSERT
or zero monetary row is fabricated. Missing, changed or nonempty/unverified
checkpoints stay unknown. The old SQL query remains the default; source-r3 is
an explicit closed query selection, including namespace helper source seals.

Loss-held -> booked restoration is recognized only under source-r3. It needs
transaction-bound raw confirmation/depth evidence, unchanged frozen effects,
complete ledger restoration and preserved irreversible Sent rows. Missing
reserved-membership or historical restoration observations stay unknown.
The 419 handoff now supplies an actual PG initial-checkpoint execution and
captured row-history replay. Historical restoration observations remain a
separate requirement.

## Historical bypass and the supplied native replacement

The r3 native codec/hook is now source-bound and consumed; see
[NATIVE-AUDIT-CONSUMER.md](NATIVE-AUDIT-CONSUMER.md) for exact hashes, process
bindings, event replay and the remaining execution/outer-receipt retention
requirements. The notes below record the older bypass that motivated the hook,
not the status of the new uncompiled source.

Read-only primary-source references and hashes:
`/tmp/opencode/T3-source-feed-checkpoint-AP5-r6/native-route-inspection.json`.

- `crates/spc-lab/src/engine.rs:1250-1263`, `connection_payload`, uses the
  configured front's **SV2** endpoint.
- `engine/traffic_runtime.rs:1432-1447` sends those AddConnection payloads
  after native bootstrap. Wallet registration readiness is not per-job proof
  consumption.
- `bins/spc-cpu-miner/src/connection.rs:458-471` sends new job/prevhash/target
  changes straight to `apply_changes`.
- `connection.rs:662-670` emits JobActivated and calls `worker.publish` without
  a source-feed acknowledgement. `worker.rs:356` then hashes assigned work.
- The supplied `UnixSourceAuditor` is wired to the translator, whose output is
  SV1. That path is not on these native miner connections.

Requirements implemented in the supplied candidate-gate source:

1. At candidate activation in the native connection actor, clear old work and
   hold the candidate before JobActivated/Worker.publish. Cover job, prevhash,
   target and extranonce changes with a connection/candidate epoch.
2. Send the actual ReceivedJob public context to the **existing wallet-owned
   verifier**. Reuse its scan authority and source-feed exchange. Return the
   consumed/committed evidence and exact candidate correlation to the native
   actor; give the CPU miner only public verification/correlation material.
3. Continue the dedicated Noise reader while verification runs. A newer
   candidate cancels older work. Only matching, unexpired committed evidence for
   the current epoch permits JobActivated and Worker.publish.
4. Emit the commit/ACK digest, job/generation, epoch and first hash/activation
   ordering so QA can prove the path. Keep four wallets, four miners and the
   same 40 H/s budgets and pool SV2 connections. Do not insert an undisclosed
   translator, switch to SV1, or copy scan/master/spend keys into CPU miners.

The controller's pre-mining gate remains INCONCLUSIVE even after a valid Unix
exchange until this actual native activation evidence exists.

## Independently audited warmup

`audited_window.py` implements the supplied r2 warmup contract. It requires the
signed complete source prefix, one fresh bootstrap with no restored slices,
the complete SQL/checkpoint replay and captured slice/account inventories.
It checks candidate job heights/profiles, contextual Q, open/finding exclusion,
selection and exact high water before issuing an in-memory eligibility
certificate. Missing, pruned, cross-session, restored or unmapped multi-front
history remains typed unavailable. Current support is one front per pool.

Empty work uses raw=0/gross K=T only after that proof; actual prior pending remains
owed. The net pool output is T-sumPaid, not an independent K floor; positive
prior can therefore be repaid under the same row/output checks. Partial work selects all
eligible completed slices and uses their actual work denominator. Full work
stops at the Q crossing. The provider constructs jobs/slices directly from
verified source and sets `SourceBundle.window={facts,eligible_history_complete:true}`
only after verification. Legacy full-Q `spc.payout-proof/1` may omit this
annotation; v2 consumption ACKs always include the facts.

`SourceBundle.witness_reserved` retains immutable byte arrays keyed by decimal
combined job ID for current and referenced witness jobs. Full coinbase witness,
commitment/Merkle, immutable nonce span, actual revenue and ladder checks remain
mandatory. Wallet-owned script/derivation verification remains additional.

The source contract supplies the consuming ten-second loaded-key auditor and
public `VerifiedSourceEvidence`. The native r3 outer codec/hook is also now
source-bound. The consumer uses its exact schema/preimages and observer fields.
Actual existing-wallet lifecycle/outer-receipt retention and compiled joined
execution evidence remain required; a valid Unix exchange alone is insufficient.

The campaign has an independent eventual full-Q requirement using the
existing full-window contract. It must observe and verify Q coverage for each
front before a full T3 pass. Warmup does not change the production 40 H/s budget
or grant any extra hashing authority. Readiness work and the exact remaining
settlement provenance requirements are recorded in
[COLLECTOR-READINESS.md](COLLECTOR-READINESS.md).
