# 2026-11-02 - Share target and share-credit policy

**Trigger.** Implementing the front and both ingresses exposed that share
difficulty had no pool-level definition. The front credited every accepted
share at the *network* difficulty (`difficulty_from_bits(template.bits)`), the
SV2 listener opened extended channels with the all-ones target `0xff..ff`, and
the SV1 listener validated no target at all. Arbitrary shares were therefore
accepted and paid at full network difficulty, and on regtest-style bits the
window target `N x D` rounded to nothing so the window could not fill.

## Decision

1. **`[accounting] share_difficulty: u64` (default 1, MUST be > 0).** The
   config rejects 0 rather than clamping it; `spc-pool` refuses to start with a
   share difficulty that would accept arbitrary work.

2. **The effective share target is never harder than the network target.**

   ```
   target_for_difficulty(d) = difficulty_1_target / d      # 0x00000000ffff0000...
   network_target(bits)     = target(compact(bits))
   effective_target         = max(target_for_difficulty(d), network_target)
   ```

   `max` is the *easier* target. On regtest-style bits the network target wins,
   so a miner is never asked for more work than a block; on mainnet the pool
   share target wins, so shares below network difficulty are still accepted.

3. **The credited `d` is capped and never zero.**

   ```
   d = min(share_difficulty, max(1, round(network_difficulty(bits))))
   ```

   `network_difficulty` is measured against the difficulty-1 target, so
   regtest-style bits (difficulty far below 1) credit exactly 1. A share is
   never credited above the network difficulty it could have met, and the
   window target `N x D` is therefore `N x d`, not `N x network_difficulty`.

4. **Both ingresses validate every share against `effective_target`.** SV2 puts
   it in `OpenExtendedMiningChannelSuccess.target` and answers a submission
   that does not meet it with `SubmitSharesError{difficulty-too-low}`; SV1 sends
   `mining.set_difficulty` with the pool share difficulty and answers
   `low difficulty share`. The front re-checks the target before accounting
   (defence in depth) and rejects the share outright.

5. **Block detection stays on the network target.** `effective_target` is a pool
   acceptance threshold; a candidate block still has to meet consensus, so the
   block path keys off `network_target(bits)` only. Reporting a share as a
   block because it met the (easier) share target would be a consensus failure.

6. **The same `d` is used everywhere in accounting**: the `ShareRec`, the
   window target (`N x d`) and the slice aggregates. There is one policy
   function set (`crate::sv1`) and no second derivation.

## Why

- The pool pays for *shares*, not blocks. Crediting a share at network
  difficulty overpays by the ratio of network difficulty to pool share
  difficulty and makes the pool insolvent immediately on a real chain.
- The window is defined in units of `N x D` for the same `D` the shares are
  credited at. Mixing network difficulty into the window while crediting pool
  difficulty would make the window unreachable (regtest) or trivially short
  (mainnet).
- Capping the credit at `max(1, round(D_net))` keeps two invariants: a share can
  never be worth more than the block it might have found, and `d` is never 0, so
  a share always contributes to the window.
- Rejecting a share at both the ingress and the front is deliberate
  redundancy: the front is the accounting authority and must not depend on a
  listener having validated correctly.

## Alternatives rejected

- **Credit the network difficulty and accept only network-difficulty shares**
  (close to the old behaviour): on mainnet that is one share per block, so the
  window is always a single share and PPLNS degenerates; on regtest the window
  cannot fill.
- **Use the chain's `powLimit`-relative difficulty for the credit**: it makes
  regtest credit 1 at the maximum target but misprices every chain whose
  difficulty-1 target is not its `powLimit`; the difficulty-1 base is the
  Bitcoin convention and matches `bitcoind` on mainnet.
- **Clamp `share_difficulty = 0` to 1**: a config that says 0 is a mistake, not
  a request for difficulty 1; silently accepting it would hide a bad deploy.
- **Reject shares below `share_difficulty` when the network target is easier**:
  that would ask a regtest miner for mainnet work it can never produce.

## Edits applied

- `docs/SPEC.md` frontmatter changelog (2026-11-02 (d)): §3.4 share validation
  and §4.3 window target amended as above. The §3.4/§4.3 prose bodies still
  need the same sentence inline; the changelog records the amendment.
- `bins/spc-pool/src/sv1.rs`: policy functions and SV1 enforcement.
- `bins/spc-pool/src/sv1_server.rs`, `bins/spc-pool/src/sv2_server.rs`: announced
  difficulty/target and share validation.
- `bins/spc-pool/src/front.rs`: credited `d`, window target, front-side
  rejection, JD `job_fees`, pending carry-forward, bootstrap and
  `spc:front:events` application.
- `bins/spc-pool/src/config.rs`, `config/spc-pool.example.toml`:
  `[accounting] share_difficulty`.

## Follow-up: window, high water, slice ids and templating cadence

**Trigger.** Integration of the front with settlement and the JDS exposed four
gaps: the high-water mark pointed at the still-open slice, slice ids restarted
from 0 on every front start, the bootstrap snapshot's `window_tail` was ignored,
and the JD distribution was derived from coinbase sats instead of the routing
weights. The front also only refreshed the template at startup and after it
found a block, so an external new block left the JDS and miner jobs stale.

### Decision

1. **High water is the newest sealed slice the window consumed.** `refresh`
   stores `window.slices.first().map(|s| s.slice_id)` and `on_share` puts that
   value in `BlockEvent.high_water` (`"0"` when the window holds no sealed
   slice, which selects no stored slice). The finding share's own slice is
   still open and therefore above the mark, so settlement never needs the
   "is the first slice the finding slice" retry heuristic.
2. **Slice ids follow the MULTI-REGION composition.** `slice_id = front_id << 56
   | local_seq`, `local_seq = max(last_local_seq + 1, unix_ms)` masked to 56
   bits - the same shape as the JDS distribution id (§3.5, §5.1). The sequence
   lives in RAM but is seeded from the restored tail, so a restart never reuses
   or collides with an id settlement has already seen.
3. **The bootstrap snapshot restores the window.** Each `SliceRow` with
   per-identity credits becomes a `SliceAgg` (credits parsed, `n` newest first,
   truncated to `keep_slices`); rows without credits, with duplicate credits or
   whose stored difficulty disagrees with the credits are skipped rather than
   zero-filled. `high_water` and the local sequence come from the newest
   restored id. A restored balance whose miner keys are not known (the snapshot
   carries no `B_scan`/`B_spend`) is moved to the `pending` rail - the coinbase
   stays whole and no output is paid to a guessed key - until the miner's keys
   are learned (`learn_identity`, or the next share, which carries them).
4. **JD weights come from the routing.** `refresh` builds
   `build_weights(&routing, raw.pool_sats)`; the JD distribution is
   `jds::distribution_from_weights(&weights, &p_x_by_id, pool_script, &a_send_x,
   dust_limit)` with `p_x_by_id` taken from the coinbase distribution's
   entries. The old "sats as weights" conversion is gone, so coinbase-mode sats
   and JD-mode weights are the same routing scaled (§4.7, §11.2).
5. **The front refreshes the template periodically.** `[template]
   refresh_interval_ms` (default 1000, MUST be > 0) drives a `Skip`-behaviour
   tokio interval in the front loop; each tick refreshes, publishes slice/block
   events, and republishes jobs plus the JDS distribution only when
   `(height, prev_hash, distribution fingerprint)` moved. A found block clears
   the marker so the next publish always happens.

### Why

- The inclusive high-water vector is the settlement contract (§4.3, §4.7); an
  open-slice mark made settlement depend on a retry heuristic.
- Per-front slice ids are required by §5.1 and by the multi-region trim/lease
  rules; sequential ids collide across fronts and after a restart.
- Restoring the window is what keeps a restarted front's PPLNS window paying
  the shares it already accepted (§5.3); refusing to pay undeliverable balances
  keeps the fail-closed rule that no output is ever derived from a guessed key.
- Publishing JD weights from the routing is the spec's "the two modes share
  everything except the final scaling" (§4.7); deriving them from coinbase sats
  mixed the merchant-fee tier back into the weights.
- A seconds-scale refresh bounds staleness after an external block without
  churning jobs when nothing changed.

### Alternatives rejected

- **Publish every tick regardless**: each JDS publish burns a new
  `distribution_id` and invalidates the previous one, breaking in-flight JDE
  work for no reason (§3.5 grace rule).
- **Pay restored balances to a placeholder or the pool key**: a wrong output is
  worse than a delayed payment; `pending` is the designed holding rail (§4.6).
- **Keep sequential slice ids and only persist them**: a restarted front would
  still collide with another front's ids; the front id prefix is the fix.
