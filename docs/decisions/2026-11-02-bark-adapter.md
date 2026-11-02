# 2026-11-02 - Bark adapter: stock-wallet pool transport, in-tree agent keystore (O13)

**Trigger.** Phase 7 Ark work is unblocked (implementation-readiness review
2026-10-14 resolved), and `bins/spc-pool/src/ark.rs` now contains the order
verifier plus an `ArkTransport` trait with no implementation. Two things must
be fixed before transport code lands: exactly which bark crates and calls a
stock-bark adapter uses, and the O13 answer (upstream patch vs fork) for the
agent's SP-aware keystore. `UPSTREAM_DEPS.md` carries no bark row yet; the
only bark pin lives in the prototypes' Cargo manifests.

## Decision

1. **Pin: `ark-bitcoin/bark @ c419159fa54085572f8ca72d2ba1716556a9d3eb`**
   (the `c419159` the prototypes pin; full SHA from
   `spc_ark_client/Cargo.lock`). New `UPSTREAM_DEPS.md` row covering:
   `ark-lib` (address/mailbox/policy formats and the blinding formula of
   SPEC §11.8), `bark-server-rpc` (the public gRPC surface), `bark-wallet`
   (send/board/refresh semantics the pool adapter rides on). Excluded from
   the pin's guarantee: server configuration defaults (`vtxo_lifetime`,
   fee schedule, `max_arkoor_amount`), which are discovered per server via
   `ArkInfo` at startup, never assumed. `bark-server` appears only in
   dev-dependencies for regtest `captaind` runs. The workspace adopts the
   prototype's `[patch.crates-io] ark-lib` -> git-rev mapping so
   `spc-crypto`'s `ark-lib = "0.7.1"` and the pinned git crate stay the
   same package (crate identity is load-bearing: `Address`, `Vtxo` and
   `Keypair` types cross between `spc-crypto`, the adapter and the RPC
   bindings).

2. **Pool-side adapter (`spc-pool`, feature `ark`): stock `bark-wallet`, no
   patch.** RESULTS.md: "Pool-wallet changes required: none." A new module
   under `bins/spc-pool/src/ark/` implements the existing trait surface

   ```rust
   async fn send(&self, address: &str, sats: u64) -> Result<(Vec<u8>, u32), String>;
   ```

   by: parse `ark::Address` fail-closed (network prefix `ark1`/`tark1`,
   `ark_id` == configured, policy == `Pubkey`), then
   `Wallet::send_arkoor_payment(&address, amount)`, returning `(vtxo_id,
   expiry_height)`. Policy stays in the verifier, not the transport:
   `OrderVerifier::check` is extended from its current prefix check to the
   full SPEC §11.3 step-(4) parse (ark_id binding), and the transport only
   re-parses defensively. **Chained sends:** stock bark chains payments when
   no single float VTXO covers an order; the receipt (`B0_64 vtxo_id` fits
   exactly one id) records the first delivered VTXO id and the *minimum*
   `expiry_height` across delivered VTXOs, and any chained send increments
   an alarmed `spc_ark_chained_sends_total` metric (a float-hygiene failure,
   not a normal event).

3. **Board / refill leg is a second surface, not part of `ArkTransport`.**
   `send` is the whole order-execution contract. The maturity loop
   (`H_f + 100`, §11.3) is a separate component in the same role: different
   cadence, different inputs, no order stream, and it needs a chain source
   (`bitcoincore-rpc`, the existing `rpc` feature) for tip, coinbase
   maturity and the per-outpoint active-chain re-check. Its bark calls:
   compute `float_balance` from the wallet's VTXO list; on shortfall
   `float_target_sats - float_balance > 0`, get `board_funding_address(..)`,
   build a PSBT spending the matured pool output key-path (account
   `m/86'/coin'/1'`, index `H_f`, per §11.3) paying that address for the
   shortfall with the remainder to an `ark`-controlled on-chain address, and
   register it via `board_psbt`. Idempotent per `(txid, vout)` outpoint;
   when the float is at target nothing is boarded and the surplus stays
   on-chain under `ark` (§11.3). **Pool-side scope note:** mailbox reads
   and `import_vtxos` are agent-side flows (`none` mode, §11.8); the pool
   adapter never reads mailboxes and never imports - delivery is embedded
   in the address the order already carries.

4. **Float hygiene makes `min_remaining_blocks` hold by construction.**
   Stock `send_arkoor_payment` does not filter inputs by remaining lifetime,
   so §11.3's `expiry_height - tip >= min_remaining_blocks` (default 3024)
   cannot be enforced at send time. Instead a maintenance loop in the role
   refreshes float VTXOs on depth and before expiry (bark's
   `vtxo_refresh_expiry_threshold` semantics, wallet refresh calls), keeping
   `arkoor_depth` under the server's `max_vtxo_exit_depth` (§11.3), so every
   send candidate is inside the window. With the stock `vtxo_lifetime` of
   4320 blocks the refresh cadence is comfortably slack. A wallet-internal
   selection filter is a bark-patch candidate only if hygiene proves
   insufficient in operation.

5. **Receipts, retries, `failed_to_pending`.** A `Sent` receipt is emitted
   only after the send RPCs complete; a transport error emits a `Failed`
   receipt (today `process()` returns the error and kills `run()` - to be
   fixed) and `payout` re-orders the next block with the same `order_id`,
   which the ark role dedupes. After `retry_blocks` (default 6) `payout`
   alone marks the row `failed_to_pending`, credits `pending_balance` and
   alarms (§11.3); the ark role never learns of it. Durable idempotence
   fix: the in-memory `seen: HashSet<[u8;16]>` becomes an orders table in
   the role's local store (it runs a sqlite bark wallet regardless). A crash
   between a successful send and the receipt must not double-pay when
   `payout` re-emits the same `order_id` on restart.

6. **O13 resolved: neither an upstream patch nor a bark-wallet fork for
   v0.1.** The agent's wallet is built in-tree over `ark-lib` +
   `bark-server-rpc`, exactly as `spc_ark_client` validated against a stock
   server: `SpArkKeystore` (the `U_i -> u_i` extra-keys table),
   `claim_vtxos` (the `find_signable_clause` replacement receive filter),
   `read_arkoor_mailbox` (mailbox authorization per MAILBOX.md §3),
   `query_vtxo_status` (`VtxoStatusAttestation` signed by `u_i`) and
   `spend_tweaked_vtxo` (the arkoor cosign flow over plain `Keypair`s).
   Signing was never the blocker - `VtxoSigner` is already abstracted over a
   `Keypair`; key lookup was, and in-tree lookup is free. The ~150-300-line
   "clean bark patch" from RESULTS.md (a pluggable extra-keys source
   consulted by `pubkey_keypair`, `find_vtxo_keypairs`, `get_vtxo_key` and
   `find_signable_clause`, plus a tweaked-key type, DB storage, recovery
   re-scan and movement-record knock-ons) stays on the table as the known
   shape of upstreaming, but it is not required to ship: it touches bark's
   DB schema and recovery, and keying v0.1 to an upstream review of that is
   unbounded. **Recorded gap:** in-round refresh and unilateral exit of
   `u_i`-keyed VTXOs ride the same bark key lookups, so v0.1 Ark-tier VTXOs
   cannot be refreshed in a round or exited. Their durability paths are
   arkoor spend (validated by the prototype e2e), cooperative offboard
   (buildable from the same arkoor/cosign primitives; gets a regtest
   acceptance test in Phase 7), and the §11.5 shutdown warning. If
   offboard-from-`u_i` fails validation, Phase 7 does not exit until the
   keystore patch is revisited.

7. **Failure/refusal rules bound at startup and per order.** Refuse to
   start: `server_is_self = true` on any network other than regtest/signet
   (§11.3; §11.9 negative test); REQUIRED values at 0/invalid
   (`float_target_sats`, `order_pubkey`, §11.10); `pool_output_account`'s
   xpub not matching `[pool] pool_payout_descriptor`; the `ark` seed file
   resolvable under the SPc-side secret-store path or looser than
   0o600/0o700 perms (bark `fs_perms` convention). Per order, in
   `OrderVerifier` (§11.3 verification list): parse per WIRE-FORMATS §3.4;
   `sats == 0` or `> max_order_sats` rejected; `not_before_height > tip`
   held against the role's own chain view; wrong `ark_id` rejected;
   duplicate `order_id` rejected; bad BIP340 order signature rejected.
   `max_daily_sats` overflow waits for the next day's allowance and alarms -
   never dropped (§11.10). The `ark` seed is its own CSPRNG draw, shares no
   store with the SPc side, the Noise authority key or the order key, and no
   identity field (`sp_id`, `B_scan`, `B_spend`, `P_i`, block hash) crosses
   into the role or its streams (§11.3 schema test stands). Order caps and
   signature checks never move into the transport or into `payout`.

8. **Test boundary.** Against stock `captaind` at the pin on regtest (the
   `spc_ark_client/run_e2e.sh` pattern): full order -> send -> receipt loop
   in all three mailbox modes; chained-send accounting and its alarm;
   shortfall board on a matured pool output, idempotent per outpoint across
   two same-height pool blocks; float at target -> nothing boarded; killed
   server -> retries -> `failed_to_pending`; every startup refusal above;
   offboard-from-`u_i`. Remains external integration, not regtest: the
   production third-party server's live parameters and sweep behaviour
   (ToS), the mainnet-only `require_board_funding_tx` profile (complied with
   structurally - we PSBT-board a confirmed, matured commitment
   transaction), production server downtime, and the MAILBOX.md network-level
   mitigations (new connection / Tor isolation per epoch read), which no
   regtest can exercise.

## Why

- **Stock server, stock pool wallet is the load-bearing constraint**
  (§11.3, R14). Every pool-side need - arkoor send to a derived address,
  PSBT board of a matured output, float introspection - is a stock
  bark-wallet call validated in RESULTS.md. Patching bark buys nothing on
  the pool side and adds drift.
- **The only real gap was wallet-side key lookup, and the prototype already
  routed around it.** `spc_ark_client` proves receive/status/spend against
  a stock server with no bark-wallet code at all; buying back
  `find_signable_clause` via a DB-schema-touching upstream patch is strictly
  worse for v0.1 and can be pursued later without blocking.
- **Splitting send from board keeps the narrowest interface narrow.**
  `ArkTransport::send` is the only effect the order stream can trigger; the
  board loop cannot be driven by stream content at all.
- **Dedupe durability is payment-safety, not tidiness:** the crash window
  between a successful send and its receipt is exactly when `payout`
  re-emits that `order_id`; RAM-only dedupe would double-pay across a
  restart.

## Alternatives rejected

- **Patch bark-wallet upstream (~150-300 lines, RESULTS.md):** correct long
  term (receive, spend, refresh and exit then work unchanged) but gates v0.1
  on an upstream review timeline for a DB-schema change. Revisit after
  v0.1, or immediately if decision 6's offboard validation fails.
- **Fork bark-wallet in-tree:** carries bark's sqlite schema, config surface
  and maintenance loop for zero receive-path benefit over the in-tree thin
  wallet; the own-suite posture (2026-10-14) is about absorbing drift, not
  multiplying it.
- **Extend `ArkTransport` with board/status/mailbox-read/import methods:**
  the order executor would then hold chain access and wallet-wide
  capabilities it never needs; the send-only trait plus a separate board
  loop matches §11.3's least-knowledge posture. Mailbox read and
  `import_vtxos` belong to the agent (`none` mode) and must not exist
  pool-side.
- **Enforce `min_remaining_blocks` by patching bark's input selection:**
  rejected for v0.1 in favour of float hygiene (decision 4), which the board
  loop needs anyway.
- **Keep `process()` failing the service on transport error:** leaves the
  crash-window double-pay (decision 5) and couples liveness to restart.

## Caveats / open items handed to Phase 7

- Offboard-from-`u_i` is unproven; it joins the §11.9 regtest matrix before
  Phase 7 exits. Until then §11.5's "auto-refresh" language is scoped to:
  spend, cooperative offboard, or warn - there is no round-refresh of
  Ark-tier VTXOs.
- The chained-send rule carries degraded receipt information (one id,
  minimum expiry); the metric must alarm loudly.
- The prototype ran on regtest defaults; `ArkInfo`-discovered parameters
  (`vtxo_lifetime`, fanout caps, fee schedule) must be read at startup on
  the production server and clamp float sizing, not assumed.

## Not implemented yet (state at this commit)

- No `ArkTransport` implementation exists; `ark.rs` has the verifier (a
  prefix-only address check; the §11.3 step-(4) `ark_id` parse is pending),
  the send-only trait, and RAM-only dedupe.
- No board/maturity loop, no float-hygiene loop, no chain-tip source wired
  into `ArkService`, no durable order journal, no `Failed` receipts.
- `payout`-side order emission, receipt consumption, `ArkPayoutNotice` and
  the `0x5350` handlers are being built in other sessions; this record
  constrains only the adapter boundary they call into.
- `spc-agent` has no Ark code at all; decision 6 defines its shape, with
  `spc_ark_client` as the conformance prototype.
- No bark row in `UPSTREAM_DEPS.md`; no bark crates in the main workspace
  lockfile.

## Edits to apply (follow-up change; this commit is the decision record only)

- `UPSTREAM_DEPS.md`: bark pin row per decision 1.
- `docs/SPEC.md`: §10 O13 row -> resolved (decision 6); §11.5 refresh bullet
  scoped per caveats; frontmatter changelog entry.
- `docs/ROADMAP.md` Phase 7: refresh item scoped; offboard-from-`u_i`
  regtest test added.
- `bins/spc-pool/Cargo.toml`, `bins/spc-agent/Cargo.toml` and the workspace
  `[patch.crates-io]` per decision 1 when the code lands.
