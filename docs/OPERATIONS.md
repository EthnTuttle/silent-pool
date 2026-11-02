# Operations guide

How to build, deploy and run silent-pool. `docs/SPEC.md` §9 is normative for
deployment; this guide restates it as procedures and records what the current
build actually does. `docs/STATUS.md` tracks completion. Everything here assumes
the repo root (`/path/to/silent-pool`) unless stated.

## 1. Build and checks (Nix only)

Nix is the only supported build path; no non-Nix path is maintained
(`docs/IMPLEMENTATION-PLAN.md` §7).

```sh
nix develop                    # toolchain + services (below)
just check                     # fmt + clippy + cargo test --workspace
nix flake check                # the CI gate
nix build .#spc-pool .#spc-agent .#spc-scan
```

What the flake provides:

- `packages`: `spc-pool` (also `default`), `spc-agent`, `spc-scan`; built with
  a pinned stable Rust toolchain (1.93.0 via `rust-overlay`) and the committed
  `Cargo.lock`.
- `checks`: `fmt`, `clippy` (`-D warnings`), `doc`, `test`
  (`cargo test --workspace`) and `test-ark` (`spc-crypto`/`spc-pool` with
  `--features ark`). `nix flake check` runs all of them.
- `devShells.default`: the pinned Rust toolchain, `just`, `sqlx-cli`,
  `postgresql_17`, `redis`, `bitcoind`, `jq`, `curl`, `cargo-nextest`,
  `cargo-audit`, `cargo-deny`, with `RUST_BACKTRACE=1` and `SPC_TEST_MODE=1`
  set. This is the environment the regtest harnesses expect (§6).
- `nixosModules`: `spc-pool`, `spc-agent`, `spc-stack` (§3).

`just` targets: `check` (fmt + clippy + test), `e2e`
(`testing/regtest-e2e.sh`), `ark-e2e` (needs `captaind` in `PATH` and
`SPC_ARK_E2E=1`).

Production template sources: `[template] source` is `"rpc"` (built behind the
`rpc` cargo feature) or `"mock"` in this build. The spec'd `"ipc"` and
`"sv2-tp"` TDP sources are not implemented yet - the binary refuses to start
with them (§9).

## 2. Roles and ports

One binary, `spc-pool`, four roles, selected with `--roles` (or `[pool] roles`
in the TOML). Dev/testnet runs `--roles front,payout,api` in one process;
`ark` MAY be added on regtest/signet only. Production splits: one `front` per
region (RAM-only state, nothing durable of its own), a single `payout` (the
sole Postgres writer), N `api` replicas, and a single `ark` on its own host
(SPEC §9). Each `front` claims its `front_id` (0..127) with a Redis `SET NX`
lease at start and refuses to start if the lease is held.

| Role | Listener / connection | Config key | Default port |
|---|---|---|---|
| `front` | SV2 (Noise) | `[pool] listen` (required) | 34254 (NixOS module default) |
| `front` | native SV1 (plain TCP) | `[sv1] listen` | 3333 |
| `front` | JDP - Job Declaration listener (the `spc-jds` module, always on) | `[jd] listen` | 34264 |
| `api` | read-only HTTP | `[api] listen` | 8080 |
| `payout` | none - consumes Redis streams, writes Postgres | - | - |
| `ark` | none - dials out to the third-party Ark server over HTTPS (`[ark] server`) | - | - |
| `spc-agent` | SV1 downstream for ASICs | `[downstream] listen` | 34255 |

Both `front` listeners (SV1 and SV2) feed the same share, slice and
distribution path. The `payout` and `ark` roles have no inbound ports; they
talk to Redis/Postgres and the Ark server respectively. `spc-agent` connects
upstream to `[pool] listen` (SV2) and serves local SV1 miners.

Example NixOS configuration (`nixosModules.spc-stack` imports the pool and
agent modules and enables Postgres/Redis):

```nix
services.spc-pool = {
  enable = true;
  configFile = /etc/spc/pool.toml;      # docs/SPEC.md §6.3 shape
  roles = [ "front" "payout" "api" ];
  environmentFile = /run/secrets/spc/env;   # DB/Redis URLs; never in the store
  openFirewall = true;                  # opens SV1, SV2 and API ports only
};
```

The module hardens the service (`NoNewPrivileges`, `ProtectSystem=strict`,
`ReadWritePaths=/var/lib/spc-pool`) - the pool writes state to Postgres/Redis
and nothing else to disk. `openFirewall` does not open the JDP port; open it
separately if remote JDCs declare against this front. Validated starting
configs: `config/spc-pool.example.toml`, `config/spc-agent.example.toml`.

## 3. Postgres and Redis

Redis and Postgres are the only durable state of the SPc side (SPEC §9), and
no spending key lives on that side - it holds only the xpub of
`pool_payout_descriptor`.

**Postgres** (`[storage] postgres`): the `payout` role is the sole writer; the
`api` role reads. Schema lives in `crates/spc-store/migrations/`. The
`spc-stack` NixOS module provisions the `spc` database and user. See §5 for
migrations.

**Redis** (`[storage] redis`): the inter-role bus. Streams used today:
`spc:slices`, `spc:blocks` (front -> payout), `spc:front:events`,
`spc:arkreg`, and the Ark order/receipt pair `spc:ark:orders` (payout -> ark)
and `spc:ark:receipts` (ark -> payout); byte formats in
`docs/WIRE-FORMATS.md`. Trim discipline (SPEC §5.2): `XTRIM MINID` behind the
slowest live reader (payout group ack, front cursor heartbeats), 600 s floor,
with a 1M-entry backstop alarm and re-bootstrap. The NixOS module runs the
`spc` Redis server with AOF `everysec` and no RDB. When fronts are remote,
use TLS and per-front ACLs (`docs/MULTI-REGION.md` §6).

## 4. Secrets

| Secret | Where it lives | How it is supplied |
|---|---|---|
| `a_send[H]` (per-height ephemeral scalar) | `front` RAM only - **never touches disk** | derived in-process per height |
| Noise `authority` key pair | `front` RAM (SV2 sessions) | `[pool] authority_public_key` publishes the public key |
| `ark` seed (bark wallet + pool output keychain, hot) | `ark` host only, own secret store | `[ark] seed_path`, default `/run/secrets/ark/seed` |
| Ark order-signing key (BIP340, CSPRNG) | `payout` host | `[ark] order_key_path`, default `/run/secrets/spc/ark-order-key`, hex 32 B, `0o600` file / `0o700` dir |
| Ark order pubkey | `ark` host | `[ark] order_pubkey` (config) |
| Postgres/Redis URLs | pool hosts | `environmentFile` (NixOS module); never in the Nix store |

Rules that are load-bearing (SPEC §2.2, §7.2, §9, §11.3):

- **`a_send` never touches disk.** It exists only inside `front`'s zeroising
  `SendKey` and is erased once the coinbase is built (`Drop` zeroises). CI
  greps for `a_send` in any `serde` derive or SQL string. The NixOS service
  writes only logs. Logs never contain `user_identity`, `sp_id`, `P_i`,
  `U_i`, mailbox ids or VTXO ids at `INFO`; `DEBUG` redacts to 8-hex-char
  prefixes.
- The `ark` seed is generated from its own CSPRNG draw, is NOT shared with any
  SPc-side key, and is not stored in the same secret store as Postgres/Redis
  credentials. Compromising the SPc side risks attribution data only; it
  holds only the descriptor xpub. Compromising `ark` risks the float, never
  SPc receiver privacy.
- Order-key rotation is operator-driven: generate a new keypair on the
  `payout` host, update `order_pubkey` on the `ark` host, restart `ark`.
  In-flight orders follow the existing retry logic (orders are short-lived).
- Miner side: `spc-agent` holds `b_scan`, `b_ark` and mailbox keys - never
  `b_spend` or the master seed.
- `[ark] server` MUST be a third-party Ark server the pool does not operate
  (SPEC §11.3); a self-hosted `captaind` is for regtest/signet tests only.

## 5. Migrations, upgrades, rollback

- Migrations are sqlx (`crates/spc-store/migrations/`), run at boot under
  `pg_advisory_lock(0x5350434d494752)`; any process may run them. Schema
  changes are additive until v1.0 (SPEC §9).
- Upgrading: deploy the new binary; migrations apply at start. Take a
  pre-migration `pg_dump` first - see the rollback rule.
- Rollback (SPEC §9): stopping `front` stops mining; that is the whole blast
  radius of a stop. Coinbase-tier payouts are already on-chain and not at
  risk. Pending balances and scheduled Ark payouts survive in Postgres and
  are paid when the pool resumes; the Ark send is idempotent per
  `(block_hash, sp_id)`. **Downgrading `spc-pool` across a schema version is
  unsupported** - restore from the pre-migration dump instead.
- Postgres backups inherit the scrub policy: a backup older than
  `scrub_after_blocks` is itself attribution data and should be rotated on the
  same schedule (SPEC §9).

## 6. Regtest harnesses

Two end-to-end harnesses, both run from the devshell (they need `bitcoind`,
`python3`, `jq`, `curl`). `KEEP=1` preserves the temp dir for inspection.

**`testing/regtest-e2e.sh`** - the fast path (also `just e2e`):

- Starts a real `bitcoind` on regtest (RPC 18443), builds `spc-pool` and
  `spc-scan` with `--features rpc`, runs the pool as `--roles front` only
  (SV1 43333, SV2 43454, API 18080; Postgres not required, Redis optional),
  mines 101 blocks, then a synthetic SV1 miner (`testing/regtest-miner.py`)
  submits two shares as a fresh silent-payment identity and the pool submits
  a block.
- Proves: the SPc coinbase builds, the block submits, and `spc-scan find`
  recovers the miner's output from the mined chain.

**`testing/regtest-services-e2e.sh`** - the full stack:

- Adds ephemeral Postgres (`initdb`, trust auth, port 55432) and Redis (port
  56379, no persistence) and runs `spc-pool --roles front,payout,api`
  (SV1 43333, SV2 43454, JDP 43464, API 48080).
- After the miner's block: asserts settlement (the `payout` role writes
  `block_found`), polls `GET /api/pool/liability`, checks
  `GET /api/pool/blocks?limit=5`, then runs the same `spc-scan` recovery.
- Proves: front -> Redis -> payout -> Postgres -> API, i.e. the settlement
  and read paths, not just mining.

Ark end-to-end (`just ark-e2e`, `testing/ark-e2e.sh`) additionally needs a
`captaind` in `PATH` and `SPC_ARK_E2E=1`; CI otherwise runs the mock
transport.

## 7. Metrics and alarms

The `api` role serves `GET /healthz` (liveness) and `GET /metrics`
(Prometheus). The alarm set below is the SPEC §9 contract; the metric names
are normative. Page-worthy items are marked.

| Metric | Alarm |
|---|---|
| `spc_a_send_age_seconds` | > 1 h - a stuck height |
| `spc_a_send_repeat_total{front_id}` | > 0 - `A_send` reuse breaks unlinkability |
| `spc_open_slice_shares` | trend |
| `spc_slices_sealed_total{reason}` | unexpected `reason` mix |
| `spc_distribution_slots{kept,trimmed,dust}` | sustained all-`trimmed` |
| `spc_budget_wu`, `spc_budget_utilisation` | pinned at ceiling |
| `spc_blocks_total{status}` | any `orphaned`/`pending` growth |
| `spc_pending_liability_sats` | reconcile against `/api/pool/liability` |
| `spc_ark_due_sats` | growth without `spc_ark_payouts_total{status="sent"}` |
| `spc_ark_payouts_total{status}` | `failed` growth |
| `spc_ark_overdue_total` | **page** - scheduled past `due_height + ark_retry_blocks` |
| `spc_ark_float_sats` | below `float_target_sats` - refill needed at next matured pool output |
| `spc_ark_surplus_sats` | operator tooling |
| `spc_forfeited_sats_total` | growth beyond expected unregistered miners |
| `spc_ark_orders_rejected_total{reason}` | any `bad-signature` - **page** |
| `spc_channel_reject_total{reason}` | identity-parse rejects trending up |
| `spc_jd_declaration_reject_total{code}` | any single code trending |
| `spc_ext32_requests_total{msg}` | trend |
| `spc_outbox_depth`, `spc_outbox_age_seconds` | rising - a full outbox fails closed |
| `spc_stream_lag_seconds{stream}` | rising per stream |
| `spc_tip_divergence_seconds` | > `tip_divergence_secs` (default 30) |
| `spc_registration_lag_revs` | rising |
| `spc_orphaned_settlement_total{reason}` | any `missing_block_event` - reconstruct from the on-chain coinbase (SPEC §7.4) |

Status caveat: the `/metrics` route currently returns a placeholder - the
Prometheus registry wiring is pending (§9). The names above are what the
build is converging to; wire the alarm rules once the exporter lands.
`GET /healthz` works today and is the probe to use for liveness.

## 8. What is deliberately not complete

From `docs/STATUS.md` "Remaining"; these are the gaps an operator must know
about, not bugs:

1. **The bark wallet adapter.** Order emission (`payout` signs and caps
   `ArkSendOrder`s) and order verification (the `ark` role) are done, but the
   transport to a real Ark server is a mock. The stock bark wallet/server
   call is missing and needs a pinned bark checkout plus a stock `captaind`.
   Until it lands, the Ark tier does not pay out against a live server.
2. **JD fallback.** The rules for a JDS decline or 10 s silence, and the
   full-template rebuild path in the JDC, are not implemented. Live JDS/JDC
   declaration and recompute-and-compare work; the fallbacks do not.
3. **Testnet soak.** Everything above passed regtest only. The spec'd gate -
   testnet4 (`tsp`) soak for >= 24 h with at least one found block
   independently recovered by a third party's `spc-scan` (SPEC §9 rollout) -
   has not run. Mainnet is out of scope for v0.1.
4. Also pending, lower operational impact: the `"ipc"`/`"sv2-tp"` template
   sources (§1), the `/metrics` exporter (§7), the seeded
   `--features cheat-slice-difficulty` statistical test, and the
   normative/security review over the integrated tree.
