# Normative/Security Review Findings — silent-pool (2026-11-02)

**Status: REQUEST CHANGES**

All tests pass, clippy is clean. The implementation is advanced and largely correct. The findings below are blocking because they violate normative spec clauses, privacy invariants, or fail-closed requirements. Each must be fixed before approval.

---

## 1. Identity — Fail-Closed on Every Ingress

### 1.1 SV2 listener does not reject extended channels with a different identity on the same connection
- **Severity:** blocker
- **File:** `bins/spc-pool/src/sv2_server.rs:131-171`
- **Spec:** §3.1 "Extended channels on the same connection MUST present the same address (differing addresses on one connection → invalid-user-identity)"
- **Finding:** The SV2 `OpenExtendedMiningChannel` handler accepts a new identity per channel without checking whether it matches any existing channel on that connection. A single SV2 connection could open multiple extended channels with different `user_identity` values, violating "one connection = one identity".
- **Fix:** Track the connection's authorized `sp_id` (like SV1 does in `sv1.rs:213-218`) and reject any subsequent `OpenExtendedMiningChannel` whose parsed `sp_id` differs. The `Channel` struct should be stored per-connection, and the first successful open sets the connection identity.

### 1.2 SV1 listener allows a second `mining.authorize` with a different identity after the first succeeds
- **Severity:** major (already correctly implemented; confirmed as working)
- **File:** `bins/spc-pool/src/sv1.rs:213-218`
- **Status:** The code correctly rejects a second authorize with a different `sp_id`. No action needed.

### 1.3 SV2 `RequestExtensions` is not validated before channel open
- **Severity:** minor
- **File:** `bins/spc-pool/src/sv2_server.rs:124-129`
- **Spec:** §3.2 — extensions are negotiated after `SetupConnection.Success`, before channel open.
- **Finding:** The server accepts `SetupConnection` and replies `Success` without checking `requested_extensions`. A client requesting an unknown extension should get an error list. Not blocking for v0.1 but should be fixed.
- **Fix:** Parse `SetupConnection.requested_extensions` and reply with `RequestExtensions.Error` for unsupported IDs per the table in §3.2.

---

## 2. Coinbase / Extranonce / Carriers — Byte Correctness

### 2.1 Pool-template `build_pool_template` uses `extranonce_len` from config, not the actual SV1/SV2 connection values
- **Severity:** blocker
- **File:** `bins/spc-pool/src/front.rs:236-243`, `bins/spc-pool/src/coinbase.rs:44-50`
- **Spec:** §2.4.1 — "Budget: 100 − 4 (height) − 33 (push32) = 63 B for the full extranonce; default allocation 16 B leaves ~4× headroom."
- **Finding:** `FrontConfig.extranonce_len` is set from `c.sv1.extranonce1_bytes + c.sv1.extranonce2_bytes` (config.rs:105). But SV1 connections have per-connection `extranonce1` (4 bytes) and miner-chosen `extranonce2` (4 bytes). SV2 extended channels negotiate `extranonce_size` per channel. The coinbase template uses a single static `extranonce_len` for all channels. This means:
  - The `scriptSig` length prefix in `coinbase_tx_prefix` is wrong for any channel whose actual extranonce length ≠ config value.
  - The 100-byte budget check at SV1 subscribe (config.rs:558) uses the config sum, but a miner could send a longer `extranonce2`.
- **Fix:** The coinbase prefix/suffix split must be built per-connection (or per-SV2-channel) using that connection's actual extranonce length. The SV2 `OpenExtendedMiningChannelSuccess` already sends the correct `extranonce_prefix` and `extranonce_size`. The front should build a `CoinbaseTemplate` per connection/channel, not one global template. Alternatively, enforce that all connections use the same fixed extranonce layout (16 bytes total) and reject any that don't match.

### 2.2 `script_sig_budget` check at SV1 subscribe uses config height 0 (1-byte push) not the actual mining height
- **Severity:** major
- **File:** `bins/spc-pool/src/config.rs:558`
- **Spec:** §2.4.1 — "The budget MUST be checked at `mining.subscribe` and a connection whose `extranonce1 + extranonce2` does not fit is rejected before any job."
- **Finding:** `height_push_len` for height 0 is 1 byte (`OP_0`). At height 800,000 it's 4 bytes. The check uses the worst case (5 bytes) which is correct for the upper bound, but the comment says "height 0 uses 1, but the check is intentionally conservative". This is fine for rejection (conservative = more rejections), but the actual `scriptSig` length written into the coinbase at height H will be larger than what was checked if H > 16,777,215 (5-byte push). The budget check should use the maximum possible height push (5 bytes), which it does. **No fix needed** — the logic is correct and conservative.

### 2.3 `emit_script_sig` does not enforce the 100-byte budget; `script_sig_budget` is only called at config validation
- **Severity:** minor
- **File:** `bins/spc-pool/src/carriers.rs:243-259`
- **Finding:** The budget is a consensus-level constraint (the Template Provider reserves 100 B unconditionally). If the front ever builds a coinbase that exceeds 100 B, the block will be rejected by the TP or bitcoind. The check should be at coinbase build time as a guard.
- **Fix:** Add an assertion or `Result` check in `build_pool_template` that `script_sig_budget(height, extranonce_len)` passes.

### 2.4 `parse_script_sig` accepts `OP_PUSHBYTES_5` for the height push but Bitcoin only uses up to 4 bytes for heights < 2^31
- **Severity:** minor
- **File:** `bins/spc-pool/src/carriers.rs:152`
- **Spec:** BIP 34 — heights are encoded as minimal CScript numbers; block height fits in 4 bytes (u32 < 2^31 for all practical purposes). The code allows `0x01..=0x05` (1-5 bytes).
- **Finding:** A 5-byte push with a zero sign byte is technically valid CScript for a positive number >= 2^31, but Bitcoin block heights will never reach that. Accepting it is permissive but not incorrect for parsing. No action needed.

### 2.5 `parse_op_return` does not check that the output value is 0
- **Severity:** minor
- **File:** `bins/spc-pool/src/carriers.rs:184-218`
- **Spec:** §2.4.2 — "Output value MUST be 0."
- **Finding:** `parse_op_return` takes `TxOut { value, script_pubkey }` but never checks `value == 0`. A non-zero `OP_RETURN` output is invalid per Bitcoin consensus (standardness), but the parser would accept it as a carrier.
- **Fix:** Add `if out.value != 0 { continue; }` in `parse_op_return`.

### 2.6 JD mode `build_pool_template` is used for JDC coinbase construction but the carrier is `OP_RETURN`, not scriptSig
- **Severity:** blocker
- **File:** `bins/spc-agent/src/jdc.rs:161-169`
- **Spec:** §2.4.2 — JD mode uses `OP_RETURN` carrier; pool-template uses scriptSig. The JDC builds a coinbase using `spc_pool::coinbase::build_pool_template` which emits scriptSig with `push32(A_send)`, then appends an `OP_RETURN` output.
- **Finding:** This produces a coinbase with **both** a scriptSig carrier (incorrect for JD) and an OP_RETURN carrier (correct). The scriptSig carrier wastes 33+ bytes and creates a non-standard coinbase for JD mode. The JDC should use a JD-specific coinbase builder that omits the scriptSig `push32` and only includes the `OP_RETURN` output.
- **Fix:** Add `build_jd_template` in `coinbase.rs` that constructs the coinbase without the scriptSig `push32` (empty scriptSig or just height push + extranonce), and includes the `OP_RETURN` output as `additional_outputs[0]`. The JDC must use this.

---

## 3. Accounting — Conservation, Tier Routing, Pending/Trim, JD Claim

### 3.1 `route_tiers` does not implement §4.6(f) "forfeited only if no ark_delivery row in Postgres at settlement"
- **Severity:** blocker
- **File:** `crates/spc-accounting/src/lib.rs:365-380`
- **Spec:** §4.6(f) — "`forfeited(id) > 0` only if `id` has no `ark_delivery` row in Postgres at settlement and `owed(id) < C`."
- **Finding:** The `route_tiers` function takes `ark: &BTreeMap<SpId, ArkCapability>` which only has `registered` and `dest_known` booleans. It has no concept of "committed in Postgres but dest not yet propagated to this front" (MULTI-REGION §6). The code at line 370-373 routes to `pending` if `cap.registered && !cap.dest_known`, but this is an in-memory front-state check, not a Postgres check. The spec says the forfeiture decision is made at settlement by `payout` reading Postgres, not by `front` at distribution build time.
- **Fix:** Remove the `cap.registered && !cap.dest_known` → `pending` branch from `route_tiers`. The front should always route below-C registered miners to `ark_due` (with `dest_known` determining whether `dest_id` is populated in the entry). The `payout` role at settlement will re-check Postgres for the `ark_delivery` row and apply the forfeiture rule (§11.1 branch 4). The front's distribution entries are a snapshot; the authoritative tier decision is at settlement.

### 3.2 `route_tiers` `pending_deltas` for forfeited entries sets `-old` (releases prior pending), but spec says forfeited balances are not carried
- **Severity:** major
- **File:** `crates/spc-accounting/src/lib.rs:374-378`
- **Spec:** §4.6 — "A forfeited balance is not owed; its prior pending, if any, is released so the ledger stays consistent."
- **Finding:** The code correctly releases prior pending (`pending_deltas.insert(*id, -old)`). However, the `forfeited` amount is added to `pay_P` via `pool_sats` in `raw_payouts`. This is correct per SPEC §11.1 "its sats in `pay_P`, nothing pending". No action needed; confirmed correct.

### 3.3 JD claim formula `claim_jd` uses `fixed` parameter but the spec says `fixed = F` (the pool's fixed fee weight value)
- **Severity:** minor (naming only)
- **File:** `crates/spc-accounting/src/lib.rs:399-418`
- **Spec:** §4.6 — `claim(id) = floor(owed(id) · (T_actual − F) / W)` where `F` is the pool fee weight, `W` is total weight.
- **Finding:** The function signature is `claim_jd(owed, t_actual, fixed, weight_total)`. The `fixed` parameter corresponds to `F` (fee weight). The JDS validation in `jds.rs:142-145` calls `distribution.fixed_value()` which sums `additional_outputs` values (the `OP_RETURN` is 0, so `fixed = 0`). But `F` in the formula is the pool's fee weight (`pool_payout.value`), not the additional outputs. The PR #203 formula is `pay_i = floor(weight_i · (T − F) / W)` where `F = pool_payout.value` (the fee weight). The `fixed_value()` is for the `additional_outputs` (carrier), which is a different concept.
- **Fix:** Rename `fixed` to `fee_weight` or `pool_weight` in `claim_jd` and `settle_jd`. Ensure the caller passes `distribution.pool_payout.value` as the fee weight, not `fixed_value()`. The current JDS validation at `jds.rs:152-157` passes `published.distribution.pool_payout.value` as `pool_weight` to `compute_payout_amounts`, which is correct. The `claim_jd` function is not used by the JDS validation path — it's for the payout settlement. Verify the payout settlement uses the correct value.

### 3.4 `trim_to_budget` (in `route_tiers`) sorts by `owed` descending, but the spec says "keeps outputs while `172 × n + overhead ≤ budget` and `n ≤ max_payout_slots`, trims the rest to pending"
- **Severity:** major (confirmed correct)
- **File:** `crates/spc-accounting/src/lib.rs:345-361`
- **Finding:** The code correctly sorts by `owed` descending, then checks `kept.len() < max_payout_slots` and `used_wu + 172 <= budget_wu`. Trimmed miners above `C` go to `pending`. This matches the spec. No action needed.

### 3.5 `raw_payouts` subsidy/fee split uses `fee_bps` on both subsidy and fees, but the spec says "net subsidy `r_net = r − ⌊r·fee_bps/10⁴⌋` and net fees `F_net = F − ⌊F·fee_bps/10⁴⌋`"
- **Severity:** major (confirmed correct)
- **File:** `crates/spc-accounting/src/lib.rs:246-248`
- **Finding:** The code does exactly this. The pool fee is taken from both subsidy and fees proportionally. Correct.

### 3.6 `route_tiers` does not enforce that an id appears on exactly one rail per block (spec §4.6(e))
- **Severity:** minor (enforced by construction)
- **Finding:** The code uses `if/else if/else` chains so each id goes to exactly one of `kept`, `pending`, `ark_due`, `forfeited`. The `pending_deltas` is inserted once per id. Correct by construction.

---

## 4. Settlement — Idempotency, Orphan Reversal, Privacy Scrubbing

### 4.1 `settle` does not re-verify the block is still on the active chain at `confirmation_depth` before booking
- **Severity:** blocker
- **File:** `crates/spc-store/src/postgres.rs:189-295`
- **Spec:** §7.4 — "Orphan check (`confirmation_depth`) runs before the transaction; an orphaned block writes `status='orphaned'` and **no** ledger rows."
- **Finding:** The `settle` function is called by `payout` after receiving a `spc:blocks` event. It checks if the block is already settled/orphaned in Postgres, but it does **not** verify against the Bitcoin node that the block is still on the active chain at `confirmation_depth`. The `confirmation_depth` parameter is only used to compute `due_height` for Ark payouts. The actual chain check is missing.
- **Fix:** Before inserting ledger rows, `settle` must query the template source (bitcoin-core via RPC or Cap'n Proto) to verify the block hash exists at height `H` and has at least `confirmation_depth` confirmations. If not, mark `status='orphaned'` and return without writing ledger rows.

### 4.2 `orphan` reversal deletes `ledger_history` and `ark_payout` rows, but the spec says "orphaned block writes status='orphaned' and no ledger rows" — the reversal should only happen if rows were already written
- **Severity:** major
- **File:** `crates/spc-store/src/postgres.rs:298-336`
- **Finding:** The `orphan` function is called separately (not from `settle`). It correctly reverses `pending_balance` deltas and deletes `ark_payout` and `ledger_history` rows. However, the `settle` function should never have written rows for an orphaned block in the first place (see 4.1). The `orphan` function is a recovery path for blocks that were settled then reorged out. This is correct as a safety net.

### 4.3 `scrub` does not check that all `ark_payout` rows for a block are out of `scheduled` before clearing `distribution_snapshot.entries`
- **Severity:** major
- **File:** `crates/spc-store/src/postgres.rs:356-369`
- **Spec:** §5.1 — "Ark-tier entries live until their send completes (normally `H_f + 6`, longer on retries)... The scrub job deletes `slice_credit` rows for slices older than the oldest unsettled window + `retain_windows`, and empties `entries` for blocks fully settled — booked, and every `ark_payout` row out of `scheduled` — more than `scrub_after_blocks` ago."
- **Finding:** The `scrub` query correctly checks `NOT EXISTS (SELECT 1 FROM block_found bf JOIN ark_payout ap ON ap.block_hash = bf.block_hash WHERE bf.fingerprint = ds.fingerprint AND ap.status = 'scheduled')`. However, it only checks for `status = 'scheduled'`. It should also exclude blocks with `status = 'failed_to_pending'` (which are still pending retry). The `ark_payout` status enum has `Scheduled`, `Sent`, `FailedToPending`. Only `Sent` is truly complete. `FailedToPending` will be retried and could become `Sent`. The scrub should wait until all Ark payouts for the block are `Sent` or the block is past `scrub_after_blocks` AND all are `Sent`.
- **Fix:** Change the `NOT EXISTS` subquery to also exclude `status = 'failed_to_pending'`.

### 4.4 `scrub` nulls `ark_payout.vtxo_id` for `status = 'sent'` and `due_height < scrub_below`, but the spec says "nulls `vtxo_id` for blocks fully settled — booked, and every `ark_payout` row out of `scheduled` — more than `scrub_after_blocks` ago"
- **Severity:** minor
- **File:** `crates/spc-store/src/postgres.rs:371-377`
- **Finding:** The query nulls `vtxo_id` for `status = 'sent'` and `due_height < scrub_below`. This is correct — once the VTXO is delivered and the block is old enough, the VTXO ID is scrubbed. The `due_height` check is a proxy for block age. Acceptable.

### 4.5 `settle` inserts `block_found` with `fingerprint` nullable, but the spec requires `fingerprint` for confirmed blocks (CHECK constraint)
- **Severity:** minor (DB schema enforces)
- **File:** `crates/spc-store/src/postgres.rs:226-237`
- **Finding:** When a block event is lost (`fingerprint` is NULL), `settle` inserts with `fingerprint = NULL` and `status = 'confirmed'`. The DB schema has `CHECK (status <> 'confirmed' OR fingerprint IS NOT NULL)`. This will cause a constraint violation.
- **Fix:** The `settle` function should not set `status = 'confirmed'` if `fingerprint` is None. It should set `status = 'pending'` (per spec: "books it `status='pending'` with `fingerprint` NULL") and alarm. The `payout` code at `payout.rs:182` always sets `status = BlockStatus::Confirmed`. This must be conditional on `fingerprint.is_some()`.

---

## 5. Type-32 — Identity-Free Data and Verification

### 5.1 `ShareLeaf` includes `job_id` and `share_index` — correct, identity-free
- **Severity:** info (confirmed correct)
- **File:** `crates/spc-share-ext/src/lib.rs:37-61`
- **Finding:** The leaf hash is `sha256d(nonce ‖ ntime ‖ version ‖ extranonce ‖ job_id ‖ share_index)`. No `sp_id`. Correct.

### 5.2 `SliceCredit` in `spc:slices` stream carries `sp_id` with `diff_sum` and `diff_fee_sum` — this is the attribution surface that gets scrubbed
- **Severity:** info (confirmed correct)
- **File:** `crates/spc-store/src/wire.rs:377-382`
- **Finding:** The per-slice aggregates carry `sp_id` so `payout` can reconstruct `owed`. The scrub job deletes `slice_credit` rows. Correct.

### 5.3 `Sampler.pick` uses `sha256d(block_hash || "/slice" || i)` for deterministic sampling — correct
- **Severity:** info (confirmed correct)
- **File:** `crates/spc-share-ext/src/lib.rs:199-228`

### 5.4 `verify_slice` checks `share.job_fees > summary.fees + fee_delta` but the spec says "share.job_fees ≤ slice.fees + δ"
- **Severity:** major (confirmed correct)
- **File:** `crates/spc-share-ext/src/lib.rs:178`
- **Finding:** The code returns `Err(Error::Fees)` if `share.job_fees > summary.fees + fee_delta`. This is the correct check (reject if exceeds). Correct.

---

## 6. JDS / JDC — Distribution ID, Same-Height Grace, Declared Coinbase Validation

### 6.1 `Jds::next_id` uses `local_seq = max(last_local_seq + 1, unix_time_ms)` — correct per spec
- **Severity:** info (confirmed correct)
- **File:** `bins/spc-pool/src/jds.rs:52-56`
- **Spec:** §3.5 — `local_seq = max(last_local_seq + 1, unix_time_ms)`.

### 6.2 `Jds::accepts_id` implements same-height grace: "previous id from the same front on the same connection" — but the code checks `prev.height == current.height`
- **Severity:** major
- **File:** `bins/spc-pool/src/jds.rs:108-116`
- **Spec:** §3.5 — "grace window: previous id FROM THE SAME FRONT on that connection ONLY IF it was published at the same height H by the same front on the same connection."
- **Finding:** The code checks `pid == id && ph == ch && cid != id`. This means: previous id == requested id, previous height == current height, and current id != previous id. This is correct for the grace window. However, the spec says "ids are not comparable across fronts" — the `front_id` is encoded in the high 8 bits of `distribution_id`. The check implicitly enforces same front because `pid` and `cid` have the same `front_id` (they're from the same `Jds` instance). Correct.

### 6.3 `Jds::validate` rejects height-straddling declarations: "previous id after a height change: stale" — correct
- **Severity:** info (confirmed correct)
- **File:** `bins/spc-pool/src/jds.rs:248-249`
- **Test:** `validates_current_and_previous_same_height` confirms this.

### 6.4 JDC `build_coinbase` uses `build_pool_template` which emits scriptSig carrier — **see 2.6 (blocker)**

### 6.5 JDC does not verify the declared coinbase matches the distribution before sending `DeclareMiningJob`
- **Severity:** major
- **File:** `bins/spc-agent/src/jdc.rs:172-200`
- **Spec:** §3.6 — "Before mining a job they MUST derive their own `P` and check it appears among the coinbase outputs (extended channel) or `payouts` (JD)."
- **Finding:** The JDC builds the coinbase from the distribution and sends it, but does not verify that its own `P` is in the outputs. The `spc-agent` verify module exists but is not called from the JDC.
- **Fix:** After `build_coinbase`, the JDC should call `verify::derive_own` and `verify::find_own` on the constructed outputs, applying the configured `require_own_payout` policy.

### 6.6 JDS validation recomputes amounts using `template_revenue` from the JDC's declaration, but the pool's `front` publishes distributions with `revenue = template.coinbase_value` at publication time
- **Severity:** major (design clarification)
- **File:** `bins/spc-pool/src/jds.rs:79-100`, `bins/spc-pool/src/front.rs:222-229`
- **Spec:** §3.5 — "the pool and JDS share state (PR #203 §10)". The JDS validates using the revenue recorded when the distribution was published.
- **Finding:** The `Jds::validate_declared` uses the revenue stored with the distribution (`p.revenue` or `prev.revenue`). The JDC declares with its own template revenue (which may differ). The validation correctly uses the pool's recorded revenue, not the JDC's. This is correct per PR #203 §4: "the JDC paid `floor(owed(id) · (T_actual − F) / W)` at its template revenue... so the mined sats differ from `entries[].sats` on every JD block whose `T_actual ≠ T`". The JDS validates against the pool's expected revenue, and the payout settlement reconciles using the actual mined revenue (O11 formula).

---

## 7. Ark — Order Caps, Signature, Identity Separation

### 7.1 `ArkSendOrder` signing payload includes `order_id`, `ark_address`, `sats`, `not_before_height` — correct per SPEC §11.10
- **Severity:** info (confirmed correct)
- **File:** `crates/spc-ext/src/lib.rs:651-657`

### 7.2 `OrderVerifier.check` enforces `max_order_sats` and `max_daily_sats` caps — correct
- **Severity:** info (confirmed correct)
- **File:** `bins/spc-pool/src/ark.rs:57-80`

### 7.3 `OrderVerifier.check` validates `ark_address` starts with `ark1` or `tark1` — correct
- **Severity:** info (confirmed correct)
- **File:** `bins/spc-pool/src/ark.rs:61-63`

### 7.4 `ArkSendOrder` verification uses `hash_SPc/ArkSendOrder` tag — correct
- **Severity:** info (confirmed correct)
- **File:** `crates/spc-ext/src/lib.rs:651-673`

### 7.5 `payout` emits Ark orders with `ark_address` from `entry.dest_id` parsed as UTF-8 String — **privacy leak risk**
- **Severity:** blocker
- **File:** `bins/spc-pool/src/payout.rs:203-233`
- **Spec:** §11.3 — "No `sp_id`, `B_scan`, `B_spend`, `P_i`, block hash or snapshot data crosses into `ark`... each order carries only what the Ark server sees anyway."
- **Finding:** The `entry.dest_id` is the Ark destination (policy key `U_i` + delivery). The code does `String::from_utf8(entry.dest_id.clone())` and puts it in `ArkSendOrder.ark_address`. This is correct — the Ark server needs the address. However, the `dest_id` is computed at distribution build time (`front.rs:222-229`) using `ark_dest` which comes from the front's in-memory `ark_dest` map (populated from `ArkRegApplied` events). The `ark_dest` map stores the full Ark address (including mailbox blinding). This is fine — the Ark role needs the full address to send.
- **BUT:** The `payout` role also has access to `entry.sp_id` and the full `BlockEvent` with all entries. The `ArkSendOrder` is built per-entry and sent to `spc:ark:orders`. The `order_id` is random. The `ark` role only sees the order stream. This separation is correct.
- **Issue:** The `ark_address` in the order is the full Ark address (e.g., `ark1...` with delivery). The Ark server sees this. The pool's `ark` role sees this. No identity fields (`sp_id`, etc.) are in the order. **Correct.**

### 7.6 `ark` role's `OrderVerifier.seen` is an in-memory `HashSet` — duplicate detection lost on restart
- **Severity:** major
- **File:** `bins/spc-pool/src/ark.rs:42-53`
- **Spec:** §11.3 — "reject duplicate `order_id` within the retry window"
- **Finding:** The `seen` set is in-memory. On `ark` restart, duplicate orders within the retry window could be re-sent. The `payout` role re-emits orders on restart (it re-reads `ark_payout` rows with `status='scheduled'`). The spec says "orders are short-lived, max `retry_blocks` = 6". The `ark` role should persist the seen set or use Redis with TTL.
- **Fix:** Store seen `order_id`s in Redis with a TTL of `retry_blocks * block_time` (~1 hour) or use a persistent set.

### 7.7 `ark` role does not verify that the `ark_id` in the address matches the configured server
- **Severity:** major
- **File:** `bins/spc-pool/src/ark.rs:57-80`
- **Spec:** §11.3 order verification step (4) — "reject if `ark_id` parsed from `ark_address` != configured `ark_id`"
- **Finding:** The `OrderVerifier.check` validates the address prefix (`ark1`/`tark1`) but does not extract and compare the `ark_id` (first 4 bytes of server pubkey hash). The `ArkSendOrder` carries the full address; the verifier should parse the `ark_id` from it and compare to config.
- **Fix:** Add `ark_id` to `OrderVerifier` config and check it in `check()`.

### 7.8 `payout` emits `ArkSendOrder` for every `ark_due` entry at settlement, but the spec says "one Ark send per id per found block, never per share or per template" — correct
- **Severity:** info (confirmed correct)
- **File:** `bins/spc-pool/src/payout.rs:203-233`

---

## 8. Panic on Attacker Input, Silent Fallback, Spec Contradiction

### 8.1 `parse_sp_address` uses `CheckedHrpstring::new` which panics on invalid HRP? — **no, returns Result**
- **Severity:** info (confirmed safe)
- **File:** `crates/spc-crypto/src/address.rs:64`

### 8.2 `emit_script_sig` and `parse_script_sig` have no panics; all returns `Result` or `Option` — confirmed

### 8.3 `build_pool_template` panics on `write_compact_size` if outputs > 2^64? — impossible, `outputs.len()` is `usize`
- **Severity:** info

### 8.4 `Front::on_share` calls `ShareRec { d: 1, ... }` — **share difficulty is hardcoded to 1**
- **Severity:** blocker
- **File:** `bins/spc-pool/src/front.rs:282-292`
- **Spec:** §3.4 — shares carry `d: share_difficulty`. The SV1/SV2 share message includes the share's actual difficulty (or the pool computes it from the hash).
- **Finding:** The code sets `d: 1` for every share. This means all shares are treated as difficulty-1, regardless of the actual target they met. This breaks SLICE scoring (which weights by `d`), window selection (which sums `d`), and the autoscaler budget (which depends on output weights derived from `raw` scores).
- **Fix:** The `Share` struct (both SV1 and SV2) must include the share's difficulty `d`. The SV2 `SubmitSharesExtended` does not include difficulty; the pool must compute it from `share_hash` and the current target. The SV1 `mining.submit` also doesn't include difficulty; same computation needed. Update `sv1.rs` and `sv2_server.rs` to compute `d` and include it in the `Share` sent to `front`.

### 8.5 `sv1.rs` `submit` function sets `job_fees: 0` — **pool-template mode has `job_fees = ref_fees` per spec §3.4**
- **Severity:** major
- **File:** `bins/spc-pool/src/sv1.rs:308`
- **Spec:** §3.4 — "SV1 shares take this exact path... `job_fees = ref_fees` (pool-template)".
- **Finding:** The SV1 share sets `job_fees: 0`. In pool-template mode (`δ = 0`), all shares in a slice have the same `job_fees = ref_fees`, so the fee term becomes a difficulty-weighted split of `F_net`. Setting it to 0 breaks the fee term for SV1 miners in pool-template mode (though `δ = 0` makes `score_f ≡ d/D_S`, so the fee term still works if `ref_fees` is used). The `job_fees` should be the template's `fees` (or `ref_fees` of the slice).
- **Fix:** Pass the current template's `fees` (or the slice's `ref_fees`) as `job_fees` for SV1 shares.

### 8.6 `sv2_server.rs` `SubmitSharesExtended` handler sets `job_fees: 0` — same issue for SV2
- **Severity:** major
- **File:** `bins/spc-pool/src/sv2_server.rs:214`
- **Finding:** SV2 extended shares also have `job_fees: 0`. In JD mode, `job_fees` comes from the declared job's template value (§3.5 "Carry `job_fees` from the declared job's template value into `ShareRec`"). The SV2 handler must extract the job's fees from the current job/distribution.
- **Fix:** Track the current job's fees in the `Job` struct and use it for `job_fees`.

### 8.7 `Front::refresh` computes `job_fees` for the slice as `0` (line 183-184) — the `Slice::new` call passes `0, 0` for `ref_job_id, ref_fees`
- **Severity:** major
- **File:** `bins/spc-pool/src/front.rs:178-185`
- **Finding:** The open slice is created with `ref_job_id = 0, ref_fees = 0`. The first share's `job_fees` (which is 0 from the share handlers) is compared against `ref_fees + delta`. With `delta = 0` (pool-template), this means `incoming.job_fees >= 0` is always true, so the slice would seal on the first share due to fee band! But `seal_if_needed` has `if delta > 0 && ...` — so for pool-template (`delta = 0`), the fee band check is disabled. This is correct for pool-template. For JD mode, `delta > 0` and `ref_fees` must be set to the first share's `job_fees`. The current code never sets `ref_fees` on the slice.
- **Fix:** When the first share arrives in a new slice, set `slice.ref_fees = share.job_fees` and `slice.ref_job_id = share.job_id`. The `Slice::new` should not hardcode 0.

### 8.8 `JDC` `build_coinbase` uses `spc_pool::coinbase::build_pool_template` with version=2 — but the JDC should use the version from the template
- **Severity:** minor
- **File:** `bins/spc-agent/src/jdc.rs:161-162`
- **Fix:** Pass the template version from the distribution or template source.

### 8.9 `spc-store` `bootstrap` returns `rev: 0` — fronts apply only newer revs, so this is safe
- **Severity:** info (confirmed correct)
- **File:** `crates/spc-store/src/postgres.rs:596-622`

### 8.10 `spc-ext` `SetArkIdentity::error_code` checks `b_ark == b_spend` — correct per spec "ark-key-is-spend-key"
- **Severity:** info (confirmed correct)
- **File:** `crates/spc-ext/src/lib.rs:482-483`

### 8.11 `spc-ext` `SetArkIdentity::verify` uses `b_scan` (x-only) for signature verification — correct per spec "sig is BIP340 by b_scan"
- **Severity:** info (confirmed correct)
- **File:** `crates/spc-ext/src/lib.rs:461-468`

---

## Summary of Blocking Findings (Must Fix)

| # | Finding | File:Line | Spec |
|---|---------|-----------|------|
| 1.1 | SV2 extended channels allow different identities per connection | `sv2_server.rs:131-171` | §3.1 |
| 2.1 | Coinbase template uses static `extranonce_len` not per-connection | `front.rs:236`, `coinbase.rs:44` | §2.4.1 |
| 2.6 | JDC uses pool-template coinbase builder (emits scriptSig carrier) for JD mode | `jdc.rs:161-169` | §2.4.2 |
| 3.1 | `route_tiers` makes forfeiture decision in front, not at settlement via Postgres | `lib.rs:365-380` | §4.6(f), §11.1 |
| 4.1 | `settle` does not verify block is on active chain at confirmation_depth | `postgres.rs:189-295` | §7.4 |
| 4.5 | `settle` sets `status='confirmed'` with `fingerprint=NULL` (DB constraint violation) | `postgres.rs:226-237`, `payout.rs:182` | §5.1, §7.4 |
| 7.6 | `ark` role duplicate detection is in-memory only (lost on restart) | `ark.rs:42-53` | §11.3 |
| 7.7 | `ark` role does not verify `ark_id` in address matches config | `ark.rs:57-80` | §11.3 step 4 |
| 8.4 | Share difficulty `d` hardcoded to 1 (breaks SLICE scoring, window, budget) | `front.rs:284` | §3.4, §4 |
| 8.5 | SV1 `job_fees` hardcoded to 0 | `sv1.rs:308` | §3.4 |
| 8.6 | SV2 `job_fees` hardcoded to 0 | `sv2_server.rs:214` | §3.5 |
| 8.7 | Slice `ref_fees` never set from first share | `front.rs:178-185` | §4.2 |

## Major Findings (Should Fix)

| # | Finding | File:Line | Spec |
|---|---------|-----------|------|
| 3.3 | `claim_jd` parameter naming confusion (`fixed` vs `fee_weight`) | `lib.rs:399` | §4.6 |
| 4.3 | `scrub` doesn't exclude `failed_to_pending` Ark payouts | `postgres.rs:356-369` | §5.1 |
| 5.4 | `verify_slice` fee check correct but needs test | `lib.rs:178` | §3.7 |
| 6.5 | JDC does not verify own payout before declaring | `jdc.rs:172-200` | §3.6 |
| 8.10 | `SetArkIdentity` signature uses `b_scan` correctly | — | — |

## Minor Findings

| # | Finding | File:Line |
|---|---------|-----------|
| 1.3 | SV2 `RequestExtensions` not validated | `sv2_server.rs:124-129` |
| 2.3 | `build_pool_template` should assert budget at build time | `coinbase.rs:44` |
| 2.5 | `parse_op_return` doesn't check output value == 0 | `carriers.rs:184-218` |
| 7.6 | `ark` role `seen` set should be persistent | `ark.rs:42-53` |

---

## Verdict

**REQUEST CHANGES** — 11 blocking findings must be resolved before approval. The most critical are:
1. Share difficulty `d` hardcoded to 1 (breaks all accounting)
2. SV2 extended channel identity check missing
3. JDC using wrong coinbase builder (double carrier)
4. Forfeiture logic in wrong role (front vs payout)
5. Settlement missing chain verification
6. Ark order duplicate detection not persistent
7. Ark order `ark_id` validation missing

All other findings are major/minor and should be addressed in the same wave.