# AP5 full-run API review and observation-race fixes (2026-11-11)

Reference evidence: `/tmp/opencode/AP5-lifecycle-api-review-r1`.
The wallet archive SHA256 05f6903d433c68dc7f9e655ea8e180d1822cb2e5ba759387046df3d8a39c67af
is the approved source-code packet 2a022552. Thirty extracted wallet source
rows and six captured runtime-helper rows were verified against their source
manifests. Three QA r17 reference rows were extracted from its hashed archive;
the mutable QA working tree was not used as the tested final-drain contract.

## Checked interfaces

| Harness operation | Tested owner interface and current binding |
| --- | --- |
| Scenario | Two nodes/pools, four SP wallet actors, four 10 H/s CPU workers, eight network plus four payment wallets, 0.2 ordinary tx/s; original runtime bootstrap operation and 40 H/s budget. No extra signer/hasher is introduced by lifecycle operations. |
| Pool board | R4 pool-wallet-executor request with board_float, net_target_sats and max_fee_sats. Poll exact original request/ID after ambiguity. The current explicit 115M target and independent spendable-float proof remain separate from historical board success. |
| Original board proof | R4 board_proof_v1 takes a different read operation ID and the unchanged original_board_operation_id. Retained canonical request, terminal context, full Pubkey policy, server key, Full VTXO bytes and anchor are checked. No remote reconstruction of a pruned original. |
| Wallet command submission | Approved wallet maintenance.rs WalletCommand: operation_id, 1..64 explicit 36-byte hex vtxo_ids, action kind refresh or offboard/address. Existing agent --config/--wallet-command submits to the same private queue; no additional wallet role or key. |
| SP refresh | round_confirmed and funding txid from round.rs; RoundCheckpoint version 2 retains locked/released Full bytes and its digest binding. The original owner imports the outputs into its stock child wallet. |
| Automatic second renewal | queue_stock_renewal hashes SPc/StockAutoRefresh/v1, raw VTXO ID and big-endian u32 trigger height. Stock result is stock_submitted/round_submitted:txid. Harness checks this recipe, exact consumed set, exclusion of manual IDs, actual output anchors and later expiry. |
| Ordinary offboard | Stock maintenance returns stock_submitted/offboard_broadcast:txid; raw transaction, actual destination/value/fee and both-node confirmations are required. |
| Derived offboard | The same wallet routes explicit SP inputs to direct_sp_offboard and retains DirectOffboardCheckpoint signed bytes; result is offboard_confirmed and txid. The harness compares signed bytes with actual onchain raw bytes. |
| Public decoding | Pinned Bark dev vtxo decode returns VtxoInfo with amount_sat, id, user/server keys, expiry and chain_anchor. Decoding is not crypto validation; the already-admitted compiled verifier remains mandatory. |
| Return address | Native driver FundingObserved contains owner_role, script_hex and the original allocation context. P2PKH conversion is public-only, uses no signing key and now ignores unfinished live journal tails. |

No controller pause/stop is used to wait for pool float. Insufficient or unknown
float remains a bounded observation while mining continues. Ambiguous wallet
or pool replies do not authorize replacement-ID mutations or fake credits.

## Concrete owned fixes

1. The first-refresh path previously selected every newly seen Spendable stock
   row. A concurrent wallet operation could add an unrelated output, or auto
   renewal could spend a real first-round output before the observation. This
   could reject an otherwise valid first refresh and relied on remote GetVtxo
   still retaining its historical output.

   It now enumerates exactly RoundCheckpoint.released, decodes those retained
   Full bytes, matches each to the same wallet's public-key registry and stock
   row/value/expiry/original funding anchor, and invokes the compiled verifier.
   Historical children need not remain currently spendable. Duplicate, absent,
   malformed, foreign-key and wrong-anchor outputs reject. Existing independent
   component replay still checks the original checkpoint digest and exact
   released/output census.

2. The renewal candidate selector previously excluded every stock ID already
   present at the first-refresh observation. A fast automatic second renewal
   already in that snapshot was therefore unobservable to the harness. It now
   finds candidates from the actual stock operation/input history, requires
   every original child row to be Spent, and leaves automatic-ID/anchor/crypto
   verification to the existing lifecycle and component validators. Candidate
   selection alone confers no renewal acceptance.

3. Return-address lookup used splitlines/json.loads over live driver journals.
   An unfinished tail in an earlier driver's file could abort lookup before
   reaching the requested user's allocated script in the next driver. It now
   reads only complete newline-terminated records, rejects duplicate JSON keys
   and paths outside the owned run, and seals the chosen allocation row and
   hash of the bytes read. An unterminated allocation is not usable evidence.

Twenty-five affected tests passed using the admitted Python 3.14 interpreter:
nine observation-race tests, three live-adapter tests and thirteen retained-board
tests. The synthetic decoded objects in race tests test orchestration only;
they are not cryptographic or funded-run evidence. An initial invocation with
system Python 3.10 failed imports because tomllib is unavailable; that failure
is preserved separately. No Rust or verifier build/test was repeated.

## Full shared-run bindings still requiring integration

The existing scenario creates the correct roster but contains no
wallet_agents[].native_audit profiles. The tested NativeAuditProfile requires
collector_socket, pool_pubkey and collector_pubkey from the actual independent
collector; absence renders no profile. The tested runtime bind_native_audit
requires the real wallet config and controller-published peer lifecycle for an
audited CPU. Supply those real bindings in the full shared scenario, rather
than borrowing component fixture authorities or silently using unaudited mode.

Current AP5 main owns its runtime and always invokes Controller.stop in finally.
QA r17's actual source-feed runtime owns final_drain() and close(), including
PID-bound pool writer termination. Those independent ownership paths must not
run concurrently on shared actors. Bind one shared campaign owner before the
full run; AP5 must not call stop_run before QA captures exact checkpoints,
economic/source convergence, writer EOF and final raw SQL. Keep AP5 cleanup
observations/verdict separate. See SHARED-CAMPAIGN-FINAL-DRAIN.md. This review
does not claim that shared-controller integration has already been executed.

These are full-campaign integration inputs, not a new prerequisite for the base
IBD diagnostic. The next actual typed readiness version and tested source union
remain with the primary. The existing v2 projection packet, failed runs, old
r5 missing-H23, successful reap and compiled verifier remain preserved.
