# 2026-11-09 ladder completion - source packet, execution blocked

## R1 continuation (2026-11-10): actual checks and remaining blockers

### R2/native closeout update

Current exact check results: **260 pool tests**, **3 real PostgreSQL aggregate
tests**, and targeted six-crate all-targets clippy with Ark/integration features
passed. Latest clippy: `/tmp/opencode/ladder-completion-r2-clippy.log`.
Current source/delta/evidence seal is
`/tmp/opencode/ladder-completion-r2-binding.json`; patch is
`/tmp/opencode/ladder-completion-r2-delta.patch`. The original R0 evidence is
preserved separately. The owned Unix-socket PostgreSQL cluster remains available
for parent/QA readonly collection; no shared database was modified.

The RPC allowance correction and immutable-window publication fix now compile
and execute. `/tmp/opencode/ladder-completion-r2-window-tests.log` passed the
stock-Ark pool suite including the same-output/different-window regression;
`/tmp/opencode/ladder-completion-r2-pool-final.log` additionally passed the fresh
owner-proof aggregate API test. The stricter all-member authorization guards
passed the real owned PostgreSQL suite again:
`/tmp/opencode/ladder-completion-r2-pg.log` (**3 passed, 0 skipped**).

**Actual native-SV2 + stock Bitcoin + PostgreSQL proof passed.** In
`/tmp/opencode/ladder-completion-native-r3`, the CPU miner produced 18 real
accepted blocks from genesis without generate RPCs, injected jobs or credits.
Canonical block 12
`1717edd5758407e37771fb8a7f5b583bb2ea79b5318367cd9660c901ac57a717`
recovered **all 990 owned outputs / 4,950,000,000 sats**, with **990 exported
tweaks**, and settled as **one logical ledger row** with **990 key-array
entries**, full owed 4,950,000,000 and frozen v2 Q=9. The operator trace check
verified its full 118-event hash chain, 9 actually visible selected slices at
height 12, raw SPCB/2 publication bytes, and complete-event SQL snapshot IDs.
Receipts: `receipt.json`, `settlement-check.json`, `trace-check.json`, raw block
and private scanner/tweak/source logs in that run directory.

Sealed pool binary:
`/tmp/opencode/ladder-completion-native-seals/eeb63350ecceaad4e8512ba939ddaab829824baebb3347d4728135aa7c37f971/spc-pool`.
All three binary seals are in `manifest-r2.json` beside it. R2's earlier
recovery-only run passed too; R3 waited for actual confirmation-gated settlement
before stopping, so it also verifies the grouped ledger/frozen-window path.

This is **native ladder component evidence**, not full funded Ark acceptance.
The provisional-ancestry formula clarification below remains open, and the
parent's combined QA/AP-3b/feed and funded stock-captaind lifecycle tests remain
separate. No old 990-first-match result was accepted as recovery evidence.

The original 27-file packet is preserved in
`/tmp/opencode/ladder-completion-r0-source.tar.gz` (365,378 bytes,
SHA256 `e4a5883d6c63beed3599e13d152105a4ffc4900010e27c57f78c5cc07a58fb99`).
Parent assigned `/home/ethan/code/silent-pool/target` as cache only; no primary
source or old candidate source/target was modified. Builds use the immutable
11 MB baseline devshell, existing readonly vendor directory and nonblocking
global lock with filesystem checks. No new target or source snapshot tree.

Actual R1 receipts:

- Accounting: **71 passed**; scanner: **8 passed**, including independent k
  vectors, all 990 outputs through block/light/coinbase APIs, all tweaks and
  all-key Schnorr checks, plus raw-source share/window/owed mutation checks.
- Agent default library: **42 passed**; extension: **9 passed**; store unit:
  **11 passed**. Log: `/tmp/opencode/ladder-completion-r1-core4.log`.
- Stock-Ark pool feature gate: **258 passed**, no failures or ignored tests.
  `/tmp/opencode/ladder-completion-r1-ark-feature.log`.
- **Real owned PostgreSQL: 3 passed, no environment skip.** Unix-socket-only
  cluster `/tmp/opencode/ladder-completion-pg-r1`, dedicated database
  `ladder_completion`. Tested atomic whole-set receipt/duplicate/partial reject,
  concurrent orphan versus Sent with irreversible pool loss, lost-grant pinning,
  and no-authorization cancellation/requeue/tombstones. Log:
  `/tmp/opencode/ladder-completion-r1-pg3.log`.
- Targeted six-crate all-targets clippy with pool/agent Ark and PostgreSQL
  integration features passed: `/tmp/opencode/ladder-completion-r1-clippy3.log`.
- Actual native-SV2/stock-Bitcoin run attempted with sealed binaries, no generate
  RPC or injected credit. It mined one real block, then found a production RPC
  allowance bug: `RpcTemplateSource` was initialized from extranonce bytes * 4
  (64 bytes), so the newly enforced full-output reservation correctly rejected
  the 990-output next job. The source now uses the bounded payout-output size;
  actual whole-block weight remains checked. This run is **incomplete**, not a
  990-output recovery success. Receipt/logs: `/tmp/opencode/ladder-completion-native-r1`.
  Sealed binary paths/hashes: `/tmp/opencode/ladder-completion-native-seals/manifest.json`.

Later source changes not covered by those passing receipts: corrected RPC
allowance; exact `source_publication` Redis bytes/stream-id trace; job-window
digest preventing later observed slices from silently changing an issued job
when outputs are identical; stricter all-member grant reservation/amount checks;
delivery-state proof fields. Their rebuild attempts yielded the occupied lock.
Latest formatting and `git diff --check` completed. Native retry and current
targeted regression execution remain required before tested integration.

### Concrete implementation added after R0

Parent authorized oldest-stored-anchor aggregation in the dated decision.
Migration 0007 adds frozen orders, members/reservations and durable nonce grants;
0008 adds append-only, redacted accounting ancestry. Runtime payout discovers
stock ArkInfo limits, freezes bounded complete members, commits publication,
freshly checks all covered blocks and commits a signed request-specific grant
before response. Production Ark transport requests that grant and sends once;
unknown/transport failure pins instead of refunding. Complete SPRP/1 receipts
carry every stock VTXO proof, checked against real Bitcoin anchor transactions,
destination/sum/signatures before atomic all-member application. No custom
Ark-server RPC or fork is used. Definitive no-authorization cancellation is
implemented; potentially sent orders without supported definitive evidence stay
blocked. Actual funded delivery/uncertainty drills have not run.

Fresh B_scan-authenticated status exposes own positive unbatched obligations and
complete anchor/member state. Owner-only receipt retrieval returns full proof
bytes; pool Sent never means wallet received. Runtime actual-SV2 job verification
hooks now run before translator publication when its protected source feed is
provided; refuse mode without that feed stops rather than mines unaudited work.
QA/AP-3b connect their owned collector/wallet work through these hooks; their
source trees were not edited.

Exact handoff: `docs/reviews/2026-11-09-ladder-source-api-handoff.md` and
`/tmp/opencode/ladder-completion-owner-api-r1.json` (named tables/columns,
publication/window/share/history/witness mappings, private boundary, endpoints).
This retains SPCB/2 full key-array/full-owed values and distinguishes the SQL
complete-event snapshot key from the event's coinbase fingerprint.

### Actual unresolved provisional-ancestry boundary

The supplied primary baseline ignores `BlockFound` at the front and has no
runtime `provisional_paids` producer. This is marked UNKNOWN, never omitted=0.
MULTI-REGION §6 says subtract full unbooked paids from owed, while authoritative
§4.6 computes new raw plus error-held pending; full subtraction would suppress
new PPLNS entitlement. Parent clarification requested: reserve only consumed
prior-pending amounts per unconfirmed block, reconcile on booking, restore on
orphan. No new formula was silently invented. Full over-revenue entitlements
continue failing closed, without partial payment or capacity deferral.

The historical R0 details below describe the earlier packet and are superseded
by this R1 progress section where they differ. Full lab/funded readiness remains
unclaimed.

## Binding and ownership

- Owned tree: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-ladder-completion`.
- Git base: `b0ab4ca34fda07c61a6335d05f598eaacc69678c`; no commits made.
- Immutable primary baseline: `/tmp/opencode/ladder-integration-baseline-src`.
  All **682 files / 11,105,307 bytes** passed manifest hash and mode checks
  before bootstrap. Only manifest rows were bootstrapped.
- Snapshot digest: `4561a2df12d9980ee1a16ee285e6667a94d807af7b9c5aa6d30bc81a7ef012b6`.
  Manifest file SHA256 is separately
  `e50db9e7735c630959375bb3ebc3d13aac6652d6b8d5b26fcde71898c33930ab`.
- Delta and per-file bindings: `/tmp/opencode/ladder-completion-delta.patch`
  and `/tmp/opencode/ladder-completion-binding.json`.
- Old candidate was read for primitive/vector reference only. Its source,
  binaries, target and history were not modified. No other worktree edits,
  subagents, upstream-pin changes, cache cleanup or new target were performed.

## Implemented source/dataflow

1. `spc-accounting::ladder`: four fixed rungs, greedy bounded decomposition,
   checked capacity/revenue failures. Pool routing has no trim or partial-pay
   branch. Unknown public keys hold before capacity validation; committed but
   underived Ark destinations remain pending. Kept-row remainders are separate
   accounting components, not duplicate identity rows.
2. Front uses an explicit `route_jd_tiers` path for exact JD weights and derives
   its own JD k0 key map, including identities below the ladder floor. Slice
   fee delta does not select the payout policy. Pool outputs bind amounts to k
   before global script sorting. Defaults use 1024 slots; the example budgets
   200k WU for 990 regtest outputs and documents 2048 headroom.
3. `Entry.p_x` is a key array in accounting, wire and SQL JSON. Mandatory
   `owed_sats` binds the complete entitlement, not merely its rounded band.
   `SPCB`/2 and JSON `format_version:2` explicitly reject old encoding. Point,
   duplicate identity/key and aggregate-count validation is bounded at 2048.
   The lower-WIRE inconsistency is raised and fixed in the dated decision.
4. JobEntriesV2 binds keys, sats, owed, rail and destination; existing
   `Published.job_id` participation remains. Complete-event snapshot IDs stop
   same-coinbase/different-destination/window snapshots from aliasing one row.
5. Settlement checks full owed equality, canonical count, k-associated values
   and sum; it writes one coinbase ledger row with its Ark remainder or pending
   component. Frozen window reconstruction has no alternative-slice retry.
   Store scheduling now includes any positive `ark_due_sats`, including kept
   rows. Pending debits use UPDATE rather than a negative INSERT that fails a
   PostgreSQL CHECK before ON CONFLICT can consume the old balance.
6. Actual serialized coinbase/block weights are checked before job publication,
   including CompactSize lengths and witness injection. Witness commitment
   publication and mode detection remain in place.
7. Scanner returns every owned output with explicit k/vout/tweak association;
   duplicate or gapped owned sets reject. Recovery does not infer ladder policy
   from observed amounts. `audit_outputs` and agent `verify_entitlement` require
   explicit payout format and independently reconstructed owed.
8. Store orphan/receipt operations share a block-row lock; sent Ark evidence is
   retained as irreversible pool loss; failed-to-pending credits are included
   in reversal. These changes still need real concurrent PostgreSQL tests.

## AP-3b API handoff

`scan_block` / `scan_light` retain `ScanOutcome.found: Vec<Found>`.
`Found` adds `k: u32`; `vout` remains the actual transaction output index.
`scan_outputs(height, A_x, views, scan_key, spend_pubkey)` returns the complete
bounded owned set. `audit_outputs(found, owed, PayoutFormat)` returns the exact
remainder; formats are `FixedLadderV1`, `JdExactV1`, `LegacyExactK0`.
`spc_agent::verify::verify_entitlement` combines discovery/audit and returns all
outputs plus remainder. Existing CLI export and runtime counting already
consume the vector, so no first-match truncation is introduced.

The AP-3b scheduler owner must retain Ark epochs after coinbase discovery:
coinbase payment does not settle a remainder. No files in that owner's tree
were edited. Pre-mining audit still needs runtime wiring to an independently
audited frozen-window source; this packet does not claim that wiring exists.

## B progress and exact remaining policy question

`spc-store::aggregation` now has pure covered-set selection, 10,000-sat/144-block
thresholds, deterministic ordering, duplicate/mixed-identity rejection and
all-member active-chain revalidation. It does **not** persist order membership,
select an aggregate destination or apply aggregate receipts. Individual send
emission rechecks its stored block height against the active chain. Sub-10k
kept-row remainders remain visibly scheduled with
`aggregate_destination_unbound`; emitting forbidden micro-VTXOs is not used
as a substitute for implementing aggregation. This is a concrete incomplete
policy-dependent path, including the 144-block force-send requirement.

SPEC §11.3 never selects AGGREGATEDEST across per-block keys/mailboxes. Proposed
minimal binding decision for the parent: oldest covered row's stored destination;
freeze covered set/destination/amount/order ID before first publication; never
regroup a published or uncertain order; revalidate every covered block before
publication; authenticate covered membership only on the SPc API, and keep the
client anchor epoch open until the aggregate receipt. An uncertain order touched
by reorg needs an explicit recovery rule before membership can be released.
This proposal is recorded as unresolved, not applied policy.

Full error-held entitlement exceeding coinbase revenue also fails closed.
The one-input coinbase cannot supply additional treasury funds; no partial cap,
donation, altered rung or invented funding path was introduced.

## Evidence and remaining execution

- Executed: 682-row hash/mode verification and bootstrap; independent Python
  affine secp256k1/hashlib derivation of k=0,1,2,3,989 vectors (sender/receiver
  shared-secret equality checked); `git diff --check`.
- Added Rust tests: 67-output Alice/Bob/Carol vector and exact remainders;
  explicit JD/pool routing; committed-underived/unknown-key holds; slot/weight
  and over-revenue rejection; full-owed grouped v2 settlement with a witness
  commitment; key swaps/missing/duplicate keys and missing frozen-v2 target;
  versioned wire/JSON rejection and snapshot destination/owed binding; 990-output
  full-block/light/coinbase recovery, complete tweak export and all-key Schnorr
  verification; remainder threshold/timeout/all-covered reorg membership.
- **Rust tests, clippy, owned-DB Rust integration and native-SV2/regtest acceptance
  have not executed on this delta.** Historical ladder logs are not current
  approval, and the historical scanner finding only one of 990 outputs is not
  accepted as recovery evidence.
- Source formatting check parsed the then-current files but returned formatting
  differences (`/tmp/opencode/ladder-completion-rustfmt-check.log`). A guarded
  formatter attempt later failed nonblocking flock acquisition and yielded;
  formatting and compiler validation remain outstanding.
- Last space observation was 83.482 GiB on the existing target filesystem.
  Neither the old ladder target nor primary target has been assigned/released
  to this session. Parent must name the exact reusable cache and confirm its
  ownership/active-use clearance before Rust execution. No cold target was made.
- Existing low-satoshi C-gate/trim/partial-payment tests and one-output front
  fixtures still need reconciliation to the authoritative ladder expectations.
  They were not deleted, disabled or reported passing. The two old accounting
  distribution fixtures remain specifically incompatible with denomination
  construction. Current source is therefore an **incomplete, unvalidated packet**,
  not working A/C acceptance or complete lab readiness.
- Still required: persisted aggregate membership/atomic all-covered receipts,
  uncertain/reorg recovery; real owned PostgreSQL execution; runtime audit-source
  wiring; combined JD/witness/native-SV2 publication proofs; a real owned chain
  recovering all 990 outputs; funded stock-captaind receive/refresh/offboard.
  Fixture/header tests establish none of those funded-service claims.
