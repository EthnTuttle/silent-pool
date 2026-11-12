# Pool wallet client executor

`pool-wallet-executor` is packaged alongside `spc-pool` in
`spc-lab-ark-tools`, built with `spc-pool/ark` from the same combined source.
The executor is a local queue client, and the already-running Ark role is its
only wallet worker. Use that role's actual config:

```text
pool-wallet-executor --config <pool-ark/pool.toml> --request <private JSON>
```

The request file and containing directory must be owned by the calling uid,
0600/0700, regular and not symlinked. Every ancestor is traversed without
following symlinks. Requests reject extra fields, duplicate fields, floats,
string amounts, out-of-range integers and invalid schemas. Logs/errors go to
stderr without private parser details; stdout is one result JSON object.

```json
{"schema":"spc.lab.ap5.pool-wallet-request/v1","operation_id":"AP5-pool-a-float-1","action":"board_float","net_target_sats":115000000,"max_fee_sats":5000000}
```

```json
{"schema":"spc.lab.ap5.pool-wallet-request/v1","operation_id":"AP5-pool-a-snapshot-1","action":"snapshot"}
```

## Results and replay

All results have `schema=spc.lab.ap5.pool-wallet-result/v1`, matching
`operation_id`, `state`, and these fields:

- `onchain_trusted_sats`, `float_spendable_sats`: exact JSON u64 or null.
- `observed_at_unix_ms`, `observed_tip_height`: integer or null.
- `error_code`: closed diagnostic string or null.
- `funding_txid`: actual signed proposal's txid, or null.
- `board`: actual fee/policy facts or null. Its fields are
  `gross_board_sats`, `net_board_sats`, `board_fee_sats`, `onchain_fee_sats`,
  `actual_fee_sats` (the sum of both fees), `expiry_height`,
  `required_confirmations`, `server_pubkey`, `vtxo_exit_delta`,
  `min_board_sats`, `max_vtxo_sats` (nullable) and `vtxo_id` (nullable).
- `proof`: present only for a successful `board_proof_v1` read, described below.
- `retained_proof_sha256`: present on new confirmed board receipts, referencing
  the immutable public proof object. Legacy receipts are not rewritten.

| State | Meaning / next step |
|---|---|
| `queued` | Published durably, no wallet receipt within 10 seconds. Replay exactly the same request; timeout does not cancel. |
| `observed` | Fresh read-only snapshot. Use a fresh operation id for a later observation. |
| `rejected` | No board submitted by this operation. Reason and nullable observations are explicit. Same-id replay preserves the rejection. |
| `already_at_target` | Fresh float already meets the requested target. No board, txid or invented fee. |
| `unknown` | Signed proposal retained; actual submission/completion is unresolved. Replay this id only. New boards are held. |
| `submitted` | The bound transaction is actually visible to the node and stock boarding is still pending. Not spendable confirmation. |
| `confirmed` | Stock completed the board and its actual confirmed VTXO passed policy, key, proof and server-spendability checks. |
| `proof_available` | Retained original public board proof; no current balance claim. |
| `proof_missing` | No original operation or no retained proof reference; inconclusive. |
| `proof_unavailable` | Original unresolved, mismatched, damaged, or retained proof unreadable; inconclusive. |

The worker revisits unresolved receipts every two seconds, on its independent
wallet cadence. A replay reads the current durable receipt. Final snapshots
and final operation outcomes are historical correlated observations; a fresh
snapshot id requests current balances. Confirmation records a real operation
completion, not a promise that the board VTXO remains unspent forever. AP5
must additionally require a fresh snapshot with the desired spendable float.

Changing any payload field under an existing id fails before effects (nonzero
client exit). Ids are opaque, at most 128 printable ASCII bytes, hashed into
filenames. Slash characters never become paths. The queue admits at most
4096 permanent operation records and batches at most eight new commands per
pass; reaching capacity refuses new operations instead of pruning replay
history. There is no queue reset or automatic operation-id recycling.

## Funds and source authority

The ordinary address was published by the same stock wallet at provisioning.
Native AP5 funding lands there; periodic stock onchain sync discovers the
transfer. Snapshot itself neither syncs nor derives an address. It refuses
while the stock checkpoint is stale, then rechecks each owned UTXO with
`gettxout`, current tip binding and canonical maturity. For this interface,
trusted means at least three confirmations (and coinbase maturity when
applicable). It excludes unconfirmed change.

The worker discovers live ArkInfo and validates network and both server keys
against the stock properties and configured Ark id. It verifies float VTXOs
through the stock full-VTXO proof path, confirmed anchors, local spendable
state, remaining lifetime, exit depth and standard authenticated
`GetVtxoStatus`. Unavailable/stale/foreign facts are not zero balances.

Boarding solves the pinned stock board-fee function for the exact net
shortfall. It builds/signs with the already-open stock BDK wallet, selecting
only proven-owned canonical inputs. The signed PSBT supplies the actual
onchain fee, and both fees must fit the cap. No output face-value is called
net spendable float. Live min-board/max-VTXO/lifetime policy is enforced.
At this pin the server's separate `max_board_amount` is **not advertised in
ArkInfo**. Standard `RequestBoardCosign` is authoritative for that hidden
limit; a refusal cannot be worked around by assuming an unlimited server.

The controller fsyncs its signed proposal before `Wallet::board_psbt`.
The latter stores its own `Board::Broadcasting` checkpoint with finalized
PSBT and signed VTXO before broadcasting. `sync_pending_boards` drives the
same stock action after restart. A lost reply is resolved using that pending
action, the bound raw transaction/node status and the actual wallet VTXO.
Absence is not success. Both a crash after the controller fence and an ordinary
`board_psbt` error (server down, policy/cosign refusal, movement or checkpoint
persistence error) retain `unknown` and the reservation. Current mempool
absence, current unspent inputs, checkpoint absence and a synchronous generic
error do not erase earlier possible publication/history. This version never
automatically re-enters `board_psbt` or issues a fresh-id attempt for that gap.
The last closed diagnostic is retained while resolution remains unknown.

Automatic refill ticks and manual commands hold the same wallet mutex as
float sends. Each automatic board checks stock pending capacity. A generic
automatic board error retains its outpoint claim; stock pending checkpoint
recovery continues an accepted board. Only NEW boarding is held by unresolved
operations, pending stock boards or pending refresh rounds. Pending refresh
output capacity is not treated as missing float to be boarded again. Chain
scanning, board/round recovery and
refresh of unrelated VTXOs continue independently under the same wallet mutex.
Unobserved manual-board VTXOs have stock action locks until their actual
acceptance proof is durably recorded. The proof binds the full signed VTXO,
funding transaction, server, key, amount, expiry, confirmation count and check
time. It is saved before releasing that lock, so later stock refresh/spend does
not erase completion. A spent original without such proof stays unknown.
The command interface cannot pause mining or authorize/send aggregate orders.

## Bounded work and damaged entries

Confirmed-chain read-sync uses the standard BDK emitter: at most 32 blocks,
160 read RPCs and a five-second budget per batch, with one-second socket
timeouts. Blocking pagination runs outside wallet send capacity; applying and
persisting its BDK changes uses the same live wallet and stock persister under
the mutex. Batches run no faster than ten seconds apart. A partial/failed batch
invalidates freshness; only a completed canonical-tip observation renews it
(maximum age 30 seconds). There is no whole-mempool scan or rebroadcast hidden
in this read-only sync. Actual stock pending-board recovery owns broadcasts.

Queue polling is two seconds with missed ticks skipped. At most eight entries
are handled per pass, with a persisted round-robin cursor and the wallet lock
released between entries. Read-only snapshot/resolution calls have a
five-second deadline; a final in-flight blocking RPC can add at most its
one-second socket timeout. Unknown/partial observations remain null. Stock
effectful recovery/refresh futures are awaited to completion, not cancelled by
these deadlines. Incremental stock round progress and standard delegated
maintenance refresh run on a separate 60-second cadence; this avoids waiting
under the wallet mutex for a future interactive round event. Stock effect RPCs
retain their upstream timeouts and durable outcome semantics.

The 160-call/five-second budget is also a closed observation-capacity limit:
one `gettxout` is needed per owned onchain UTXO, and framing plus Ark VTXO
proof queries share that budget. Roughly 150 owned UTXOs fit only when little
other proof work is needed; latency or additional VTXOs can exhaust it sooner.
A larger wallet returns stale/unavailable/null observations (normally
`rejected/fresh_observation_unavailable` for a new snapshot), not a truncated
UTXO sum, a guessed balance, or an unbounded slower response.

Invalid never-admitted requests receive bounded, durable fault evidence.
Malformed or unreadable admitted/recorded operations block new funding; they
are not quarantined into a fictional no-effect state. Original damaged files
are retained, with metadata, a content fingerprint and a bounded safe prefix
in `faults/`. Other valid snapshots and recorded operations continue. A valid
record remains recovery authority if its request file is later damaged.
Admission is persisted before preparation, preventing crash/error-induced
repeated child-key allocation. An admitted operation whose receipt is missing
remains unknown. Only definite pre-board preparation failures are rejected.

The owned wallet datadir is created 0700 or tightened to 0700 without imposing
that mode/ownership on existing ancestors (including `/var/lib/spc`). Symlinks,
foreign-owned actual wallet directories and unsafe existing private files are
refused. The exclusive controller owner lock precedes stock wallet open.
The pinned stock PID stamp `LOCK` is the sole classified root metadata
exception: the private 0700 root protects this same-UID regular single-link
file, which may retain the stock `0666 & !umask` mode (including 0644/0664).
Owner read/write is required; executable and special permission bits are
refused. The helper never chmods, truncates, replaces or removes this file.
Root entry checks use Linux metadata-only `O_PATH|O_NOFOLLOW` descriptors so
inspection cannot release an active POSIX lock by closing an ordinary I/O fd.
Database, secret, provision, unknown-root and control/receipt/proof files keep
their strict private-file checks; directories remain 0700. This classification
comes from the pinned stock PID-lock writer, not a blanket relaxation for
unrecognized stock or secret files.
Concurrent client enqueue contention returns a nonzero unavailable result;
retry the same request/id, never mint an id to bypass contention.

## Read-only historical original-board proof

Use the same executable/config with a separate fresh READ correlation id:

```json
{"schema":"spc.lab.ap5.pool-wallet-request/v1","operation_id":"AP5-pool-a-proof-read-1","action":"board_proof_v1","original_board_operation_id":"AP5-pool-a-board-1"}
```

The original board id and payload never change. This action cannot carry
`net_target_sats` or `max_fee_sats`; using the original mutation id as its read
id is refused. Replaying a read id returns that historical read outcome. Use
a fresh READ id for a later attempt after a missing/unavailable response.

Successful replies retain the usual result schema, use `proof_available`,
leave current balances/tip/time null, and include `proof` with:

- `schema=spc.lab.ap5.pool-wallet-board-proof-envelope/v1`;
- `proof_sha256` and `canonical_json_hex`: SHA256 and exact compact UTF-8 JSON
  bytes of `original`, serialized in its declared field order;
- `original.schema=spc.lab.ap5.pool-wallet-board-proof/v1`;
- original operation id, original request and request digest;
- `original_vtxo_id`, `original_full_vtxo_hex`, its SHA256 and the pinned
  `ark-lib/ProtocolEncoding/c419159` encoding marker;
- `raw_anchor_tx_hex`, `anchor_txid`, `anchor_vout`, `owner_pubkey`, complete
  `owner_policy_hex`, `owner_policy_kind`, `server_pubkey` and original
  `board` fee/policy facts;
- original acceptance time, tip height, confirmation count and the stock
  checkpoint reference `board.<funding_txid>.<vout>`;
- `terminal_receipt_context_canonical_json_hex`, its SHA256 and
  `terminal_receipt_context_schema=spc.lab.ap5.pool-wallet-terminal-receipt-context/v1`:
  the historical confirmed receipt with ONLY its self-referential
  `retained_proof_sha256` field omitted. Operation, amounts and observation
  context are unchanged and bound; the original receipt's retained digest
  must equal this envelope's `proof_sha256`;
- `lineage_state=recorded_partial|unavailable`, `lineage_complete=false`,
  bounded `round_imports`, and a nullable lineage error code.

The immutable proof object is published with NOREPLACE and fsynced under
`wallet-control/proofs/<digest>.proof` before the original confirmed record
references it and before the board VTXO's protective stock lock is released.
The object contains only public proof data, never PSBT derivation metadata,
private keys, seed or key indices. Later stock refresh/spend/pruning cannot
change these bytes. Retrieval reads the retained object and original receipt
only: no wallet lookup, key derivation, sync, board resubmission or provider
RPC. Existing confirmed receipts without a proof reference remain unchanged
and return `proof_missing/proof_not_retained`; missing or damaged referenced
objects return `proof_unavailable` rather than reconstructing from a server.

On the independent maintenance cadence, successful stock `bark.round` /
`refresh` movements with actual completion/funding metadata retain immutable
`spc.lab.ap5.pool-wallet-round-import/v1` facts: movement id, exact input/output
VTXO ids, round funding txid, offchain fee and completion time. These are
explicit stock import links, not amount comparisons. Reads return only links
reachable from the original VTXO id. They never claim complete descendant
history or current spendability. If facts were not retained, were pruned
before capture, conflict, or exceed the 256-record read budget, lineage stays
unavailable. AP5's separately validated current-float proofs must not assert
descendant equality merely from equal amounts.
