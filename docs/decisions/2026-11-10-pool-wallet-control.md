# Live pool wallet client control

Date: 2026-11-10
Status: implementation decision under the AP5 same-wallet execution directive

## Decision

SPEC §11.3 and §11.10 permit a private local client queue for the existing
Ark-role wallet. The two commands are read-only `snapshot` and `board_float`
from ordinary, stock-wallet-owned onchain outputs. The role owns the seed,
stock persister, signer and wallet for its entire lifetime. A separate client
only enqueues a closed request and reads its receipt. It never opens a wallet.

Amounts on this private JSON interface are integer satoshis. A net target is
an upper bound on the desired spendable float, capped by configured
`float_target_sats`. The board output includes the discovered stock board fee;
the explicit fee cap covers both that fee and the signed onchain transaction
fee. The controller does not credit balances. Successful completion requires
the actual stock VTXO, confirmed funding, matching policy/key/amount/server,
valid exit proof and server-confirmed spendability.

Snapshot commands do not sync, derive addresses, move scan cursors or write
wallet state. Periodic live-owner sync is independent of command arrival;
observations fail closed while its checkpoint differs from the active chain.
An observation rechecks wallet-owned outpoints and stock VTXOs against the
node and server. Unknown amounts are JSON null, never configuration targets.

Every operation id binds one immutable payload. Before any potentially
broadcasting call, fsync the actual signed PSBT/raw transaction, input list,
stock key index/public key, policy and fee facts together with that payload.
Stock `board_psbt` owns its separate pre-broadcast checkpoint and recovery.
Ambiguous outcomes retain the proposal and block additional boarding; the
client must replay the same id. A timeout cannot cancel the wallet task.

Automatic boarding likewise retains its outpoint claim on a generic error;
an error is not definite pre-effect evidence permitting another attempt.

One kernel owner lock excludes duplicate role owners; one shared wallet mutex
serializes automatic board ticks, manual commands, maintenance and arkoor
sends, including key allocation. Pending stock boards reserve float capacity.
The queue has no mining-pause capability. Insufficient funds or server policy
refusal does not stop native/user mining.

## Boundaries

### Source-audit rework (S1, S3, S4, S5)

The actual owned datadir is established before the exclusive owner lock and
stock wallet open, without requiring private ancestors. Existing loose owned
datadirs may be tightened; unsafe existing private files are refused.

Restart compatibility classifies the pinned stock PID stamp `LOCK` as public
metadata within that private root. Its same-UID regular single-link inode may
keep `0666 & !umask` permissions, provided owner read/write is present and no
execute/special bits are set. All other root files remain exact-0600 by
default; control/receipt/proof/secret/database guards are not relaxed.
Metadata-only `O_PATH|O_NOFOLLOW` inspection avoids ordinary file opens/closes
that could release process-owned POSIX locks. No stock lock is replaced,
removed, truncated, or used to bypass existing ownership; the controller
lease still precedes stock Wallet::open. The metadata class is tied to
`bark/src/lock_manager/pid_fcntl.rs` (and its matching PID-flock writer) at
the existing c419159 pin.

Only new board admission is gated by pending/unknown operations. Bounded
confirmed-chain scans, stock board/round recovery and standard delegated
maintenance refresh have independent cadences. Read-only RPC budgets may
expire; effectful stock operations are not cancelled. Unobserved manual board
VTXOs are reserved in the stock wallet until a validated acceptance proof is
fsynced, before a child refresh/spend transition can erase its observable
spendability. Persisted proof is completion history, not a balance ledger.

Per-entry faults preserve evidence and conservatively block new funding when
durable operation history is unreadable. A fair persisted cursor lets other
snapshots/known recovery proceed. A generic board_psbt error remains unknown;
no absence-based no-effect or reset/regrant policy is introduced.

These bounded additions supplement SPEC §11.3 Refilling and §11.10 using the
existing datadir, float cap, server identity and lifetime settings. The exact
client contract and work limits are in `docs/POOL-WALLET-CONTROL.md`.

**Deliberate client-adapter divergence:** the live controller replaces stock
`sync_onchain()` with bounded `bdk_bitcoind_rpc::Emitter` confirmed-block
batches applied to the SAME onchain wallet and committed through the SAME
stock `store_bdk_wallet_changeset` persister. It does not ingest the mempool
or rebroadcast as part of that read sync. Selection and the final pre-start
check re-prove inputs with `gettxout(include_mempool=true)`; omitted mempool
ingestion is never interpreted as permission to reuse an input or release an
unknown operation. Stock pending-board checkpoints own recovery broadcasts.
The live maintenance pass uses stock incremental `sync_pending_rounds`,
`progress_pending_rounds` and `maybe_schedule_maintenance_refresh_delegated`
instead of stock `maintenance()`'s interactive round wait. This is client
scheduling over standard primitives, not a second wallet/persister, a server
change, or a new generic no-effect/recovery rule.

The read budget is a real capacity limit: roughly 150 owned UTXOs can fit
when other proof work is small, with fewer fitting under slow RPCs or more
Ark proofs. Over-budget observations fail unavailable/null, never by
truncating or estimating the balance.

### Historical public proof read

AP5 requires `board_proof_v1`, a separate read correlation referring to the
immutable original board id. Before confirming/releasing the original board,
the controller persists a content-addressed, immutable public proof object and
references its digest from the first confirmed receipt. It includes the actual
validated original signed VTXO, raw anchor, full public owner policy, server,
fee/policy facts and original receipt/context binding. Later reads never
rewrite that receipt or query the stock wallet/server for a pruned original.
Missing legacy references or damaged objects yield typed missing/unavailable.

Independent stock-maintenance observations may retain exact public refresh
movement input/output ids and round-funding facts. These partial links never
establish current-float equality from amounts; complete lineage and current
spendability remain separately proven. Proof reads do not derive keys, sync,
touch wallet state or submit financial operations.

No SP identity, block hash, miner mapping, SPc database credential or order
signing key enters a command. Funding transaction ids and wallet input
outpoints are wallet facts, not mining attribution. There is no new Ark-server
RPC, server modification, SQL credit, independent balance ledger or second
wallet seed/database. Manual funding does not authorize aggregate send orders.

## References / edits

- SPEC §11.3 and §11.10: bounded supplement in
  `docs/changelog/2026-11-10-pool-wallet-control.md` for serial integration.
- `docs/POOL-WALLET-CONTROL.md`: exact client/status/recovery contract.
- `docs/reviews/2026-11-10-pool-wallet-control.md`: source investigation,
  validation evidence and remaining joined-runtime proof.
