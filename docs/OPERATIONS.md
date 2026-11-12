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

Production template sources: `[template] source` MUST be `"rpc"` (built
behind the `rpc` cargo feature). This build refuses to start on any other
value except `"mock"`, which is accepted only on regtest/signet when
`[accounting] allow_unchecked_chain = true`. The spec'd `"ipc"` and `"sv2-tp"`
TDP sources are not implemented in this build; they land with the
rbitcoin/TP packet. Do not configure them expecting startup.

### 1.1 v2 work-unit profile (opt-in, regtest only)

The production accounting policy remains v1 (`share_difficulty`, `N * d`). The
`network_block_work` profile is an opt-in lab feature, not a silent production
change:

```toml
[accounting]
work_unit_version = 2
work_reference_target = "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"  # R = 2^255 - 1
share_work_multiplier = 2                 # m = 2^j
max_work_multiplier = 4294967296
max_window_target_units = "18446744073709551616"
pow_limit_bits = 0x207fffff               # must equal the node's effective consensus value
window_rule = "network_block_work"
# share_difficulty MUST be absent in v2.
```

v2 is restricted to `pool.network = "regtest"`. It requires exact power-of-two
reference/multiplier values, rejects a reference below the effective pow limit,
rejects legacy `share_difficulty`, disables SV1 ingress, and **does not
advertise or accept type-32**: requests receive `unsupported-extension` and no
v2 slice populates the type-32 store. The issued target family is
`S(2^j) = 2^(k-j) - 1`, issued credit is exactly `2^j`, and a new generation
is allocated on retarget/multiplier change. v2 arithmetic is checked; overflow
or mixed-profile restore fails closed. The production denomination ladder,
`DESIGN.md`, and exact `owed` remaining the settlement authority are unchanged;
the lab feature does not amend payout policy.

The v2 window is `Q(T) = ceil(N * (R + 1) / (T + 1))` network-reference units,
not `N * d`. `Q` and the profile are persisted; settlement uses the snapshot's
frozen version/fingerprint/rule/target. If the bounded history cannot reach Q,
the result is explicit `window_underfilled` (warmup/inconclusive), never an
N-block coverage claim.

**Retention/scrub.** Durable deletion is selected per stored row version, not
by the running payout's configured version. v1 keeps its legacy horizon and is
never extended by a v2 setting. v2 uses the explicit `height < tip - 4096`
cutoff (a row at `tip - 4096` is retained). The live v2 RAM tail is capped at
4096 slices and uses the template height `tip + 1` (`height >= tip + 1 - 4096`),
one block stricter than SQL, so RAM never serves a credit SQL has expired. This
v2-only identity-retention extension never crosses the Ark boundary and adds no
identity field there.

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
| `front`, `api` | Tor onion services (optional) - the Tor daemon forwards to the loopback listener | `[reach] onion` + Tor `HiddenServicePort` | - |
| `spc-agent` | outbound via SOCKS5 (optional) - upstream SV2 and JDP only | `[net] proxy` | SOCKS 9050 |

Both `front` listeners (SV1 and SV2) feed the same share, slice and
distribution path in v1. In v2 the SV1 listener is rejected and only the native
SV2 listener is served. The `payout` and `ark` roles have no inbound ports; they
talk to Redis/Postgres and the Ark server respectively. `spc-agent` connects
upstream to `[pool] listen` (SV2) and serves local SV1 miners.

Startup fails closed. A `front` refuses to start when Redis is unreachable
(slice and block events are the accounting trail), `payout` refuses to run
without a chain view (template source `rpc`), and the `ark` role requires
`[ark]` with `enabled`, `float_target_sats`, `ark_id`, `order_pubkey`,
`retry_blocks`, a non-empty `send_command` (SPEC §11.10), and template
source `rpc` (it refreshes its tip from the chain view). The single dev
escape hatch is `[accounting] allow_unchecked_chain` - regtest/signet only,
enforced at config validation - which re-enables those unchecked paths.

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

### Tor reach (optional)

Tor is a transport option, not a deployment mode (SPEC §3.8), and it covers
**mining connections only**. `spc-pool` accepts `[reach]`; `spc-agent` accepts
`[net]` for its upstream SV2 and JDP connections. Ark egress is direct for both
binaries (decision 2026-11-04-tor-scope-mining-only). The pool does not create
or manage onion keys - the Tor daemon does, and forwards to the listener the
pool already binds.

```nix
# Illustrative; exact option shape follows the pinned NixOS release.
services.tor = {
  enable = true;
  client.enable = true;                      # spc-agent mining egress
  settings.SocksPort = [ "9050 IsolateSOCKSAuth" ];
  relay.onionServices.spc-sv2.map = [
    { port = 34254; target = { addr = "127.0.0.1"; port = 34254; }; }
  ];
};
```

- **Inbound.** Set `[reach] onion` and bind that listener to `127.0.0.1`.
  `spc-pool` refuses to start when an onion-published listener is not loopback,
  so an advertised onion address can never be decorative. List the public
  endpoints in `[reach] advertise`; they are served from `GET /api/pool`.
- **Outbound.** Set `[net] proxy = "socks5h://127.0.0.1:9050"` on `spc-agent`
  only, for the upstream SV2 and JDP connections. `socks5h` (remote
  resolution) is mandatory: local DNS leaks the query the proxy exists to
  hide. Stream isolation gives each connection its own circuit. A `.onion`
  upstream with no proxy is a startup error, never a silent direct connection.
- **Ark is not proxied.** The `ark` role dials the Ark server directly and the
  agent's mailbox reads are direct, so the Ark server and mailbox endpoint see
  the reader's network address. That is the accepted residual of decision
  2026-11-04-tor-scope-mining-only; use an operator-level network control if it
  matters. `spc-pool` has no `[net]` section, and a pool config carrying one
  fails to parse.
- **Multi-region.** One onion service terminates on one `front`. Either publish
  one address per front in `advertise` (and let `spc-agent` rotate on failure)
  or put OnionBalance in front of the set. Back up each front's onion key with
  its other secrets.
- **Abuse controls.** Onion clients arrive from `127.0.0.1`, so IP reputation
  and per-IP limits do nothing. Rely on the per-connection concurrency cap and
  per-`sp_id` share-rate limits; watch `spc_channel_reject_total{reason}`.
- **Costs.** Pool-template job delivery is one-way, so the added RTT is cheap;
  reconnect transients (new extranonce, VarDiff ramp) are the real cost. Keep
  the JDC role direct unless the miner explicitly asks for Tor.

## 3. Postgres and Redis

Redis and Postgres are the only durable state of the SPc side (SPEC §9), and
no spending key lives on that side - it holds only the xpub of
`pool_payout_descriptor`.

**Postgres** (`[storage] postgres`): the `payout` role is the sole writer; the
`api` role reads. Schema lives in `crates/spc-store/migrations/`. The
`spc-stack` NixOS module provisions the `spc` database and user. See §5 for
migrations. v2 rows carry `work_unit_version = 2`, a nonzero reference
fingerprint and `window_rule = 1`; v1 rows carry `1`, NULL and `0`. Mixed
profile rows or a v2 row with a zero fingerprint fail the schema checks.

**Redis** (`[storage] redis`): the inter-role bus. Streams used today:
`spc:slices`, `spc:blocks` (front -> payout), `spc:front:events`,
`spc:arkreg`, and the Ark order/receipt pair `spc:ark:orders` (payout -> ark)
and `spc:ark:receipts` (ark -> payout); byte formats in
`docs/WIRE-FORMATS.md`. Trim discipline (SPEC §5.2): `XTRIM MINID` behind the
slowest live reader (payout group ack, front cursor heartbeats), 600 s floor,
with a 1M-entry backstop alarm and re-bootstrap. The front event reader owns
its own Redis connection; it must not share the publisher's blocking client. The NixOS module runs the
`spc` Redis server with AOF `everysec` and no RDB. When fronts are remote,
use TLS and per-front ACLs (`docs/MULTI-REGION.md` §6).

## 4. Secrets

| Secret | Where it lives | How it is supplied |
|---|---|---|
| `a_send[H]` (per-height ephemeral scalar) | `front` RAM only - **never touches disk** | derived in-process per height |
| Noise `authority` key pair | inline in the pool TOML | `[pool] authority_public_key` is the public pin; `[pool] authority_secret_key` is private and never logged/published; both MUST be a matching pair. The TOML carrying it SHOULD be mode 0600 (operator convention; the binary does not enforce file mode). |
| `ark` seed (bark wallet + pool output keychain, hot) | `ark` host only, own secret store | `[ark] seed_path`, default `/run/secrets/ark/seed` |
| Ark order-signing key (BIP340, CSPRNG) | `payout` host | `[ark] order_key_path`, default `/run/secrets/spc/ark-order-key`, hex 32 B, `0o600` file / `0o700` dir |
| Ark order pubkey | `ark` host | `[ark] order_pubkey` (config) |
| Postgres/Redis URLs | pool hosts | `environmentFile` (NixOS module); never in the Nix store |

The Noise authority pair is inline in the pool TOML and must match exactly:

```toml
[pool]
authority_public_key = "9auq...PH72"   # public pin; safe to publish
authority_secret_key = "mkDL...n2"     # throwaway example only; mode 0600; redact from logs/artifacts
```

The secret is required when the `front` role is selected. A mismatched pair,
an ephemeral key, or a public key without its secret is a startup error. Do not
put the secret in a shell argument or commit it; the pool reads the TOML.

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
  changes are additive until v1.0 (SPEC §9). Migration `0005_lab_work_units.sql`
  adds the v2 version/fingerprint columns and the distribution window rule with
  legacy v1 defaults; it does not convert existing rows.
- Upgrading: deploy the new binary; migrations apply at start. Take a
  pre-migration `pg_dump` first - see the rollback rule.
- Rollback (SPEC §9): stopping `front` stops mining; that is the whole blast
  radius of a stop. Coinbase-tier payouts are already on-chain and not at
  risk. Pending balances and scheduled Ark payouts survive in Postgres and
  are paid when the pool resumes; the Ark send is idempotent per
  `(block_hash, sp_id)`. **Downgrading `spc-pool` across a schema version is
  unsupported** - restore from the pre-migration dump instead.
- Postgres backups inherit the scrub policy: a backup older than the applicable
  horizon is itself attribution data and should be rotated on the same schedule
  (SPEC §9). v2 `slice_credit` uses the explicit 4096-height cutoff; v1 keeps
  its legacy horizon. A v2 setting never extends v1 identity retention.

## 6. Regtest harnesses

End-to-end harnesses, pairs of bitcoind-backed and rbitcoin-backed scenarios.
All run from the devshell (they need the node binary, `python3`, `jq`,
`curl`; the full-stack ones also need `initdb`/`redis-server`). `KEEP=1`
preserves the temp dir for inspection.

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

**rbitcoin-backed** - the same two scenarios against `rbitcoin-node` (the
pinned Rust node, `UPSTREAM_DEPS.md`) instead of `bitcoind`, proving the
pool's `[template] source = "rpc"` consumes its Core-compatible RPC surface
unchanged (cookie Basic auth, `getblocktemplate`, `submitblock`,
`invalidateblock`, `getblock`/`getblockstats`). Unlike `bitcoind`, the node
READS an existing Core-format `username:password` cookie (mode 0600, no
trailing newline) from `--rpc-cookie-file`, so the harnesses write it before
starting the node; blocks are funded with `generatetoaddress` to a fixed
P2TR hex script (no wallet), and `--regtest-pow-bits 0x200fffff` keeps PoW
trivial but real. Both scripts print `SKIP` and exit 0 when the binary is
absent, use their own temp dirs and ports disjoint from the bitcoind
harnesses (so all four can run concurrently), and accept `KEEP=1`:

- `testing/regtest-rbitcoin-e2e.sh` - front-only. Asserts the height
  advances when the synthetic miner submits shares AND that `spc-scan find`
  recovers at least one miner output (a hard assert, unlike the bitcoind
  harness's warning). Ports: RPC 29443, SV1 34333, SV2 34454, API 29080.
- `testing/regtest-rbitcoin-reorg-e2e.sh` - full stack (ephemeral Postgres
  55433 + Redis 56380, `confirmation_depth = 2`, API 29081). Settles a found
  block with 2 confirmations, then `invalidateblock`s a second found block
  and mines a competing branch across its height. Asserts the SPEC §8.4
  step-4 orphan invariant: `block_found` carries exactly one row for the
  height with `status='orphaned'`, `ledger_history` and `ark_payout` row
  counts are unchanged from after the settle leg (the orphaned payout is
  never booked, so it cannot be booked twice), `/api/pool/liability` is
  unchanged, `/api/pool/blocks` shows the orphan, and the pool keeps
  producing on the competing branch.

Environment: `RBITCOIN_NODE` (default
`/home/ethan/code/rbitcoin/target/dev/debug/rbitcoin-node`),
`RBITCOIN_CLI` (optional; derived from the node dir, used only for a clean
stop), `KEEP=1`. Run them from the devshell like the bitcoind harnesses,
e.g. `nix develop -c ./testing/regtest-rbitcoin-reorg-e2e.sh`.

Ark end-to-end (`just ark-e2e`, `testing/ark-e2e.sh`) additionally needs a
`captaind` in `PATH` and `SPC_ARK_E2E=1`; CI otherwise runs the mock
transport.

**Host lab runtime.** The approved host slice builds and runs rbitcoin +
`spc-pool` + `spc-cpu-miner` + Redis + PostgreSQL and exposes a browser control
surface. See [`LAB-RUNTIME.md`](LAB-RUNTIME.md) for the exact Nix store paths,
private artifact layout, commands, node provenance exception, and known
capability limits. It is an operator/lab tool, not a production deployment
mode.

## 7. Metrics and alarms

The `api` role serves `GET /healthz` (liveness) and `GET /metrics`
(Prometheus text 0.0.4). **The exporter is the source of truth:** scrape
`/metrics` to discover what this build actually emits, and only build
dashboards or alert rules on the families in "Exported today". A family whose
store query failed is omitted from the scrape - a missing series means
"unknown", never "empty" - and the failure counts in
`spc_metrics_scrape_errors_total` (alarm on any growth). Page-worthy items
are marked.

### Exported today

Every family in this table is emitted by `bins/spc-pool/src/metrics.rs` in
this build.

| Metric | Type | Alarm |
|---|---|---|
| `spc_blocks_total{status}` | counter | any `orphaned`/`pending` growth |
| `spc_pending_liability_sats` | gauge | reconcile against `/api/pool/liability` |
| `spc_ark_due_sats` | gauge | growth without `spc_ark_payouts_total{status="sent"}` |
| `spc_ark_payouts_total{status}` | counter | `failed_to_pending` growth |
| `spc_forfeited_sats_total` | counter | growth beyond expected unregistered miners |
| `spc_config_window_n` | gauge | capacity review |
| `spc_config_coinbase_threshold_sats` | gauge | config audit |
| `spc_config_budget_floor_wu` | gauge | compare with ceiling |
| `spc_config_budget_ceiling_wu` | gauge | pinned at ceiling |
| `spc_process_uptime_seconds` | gauge | restart detection |
| `spc_metrics_scrape_errors_total` | counter | any growth - a source query failed |

### Planned (not exported in this build)

These are the SPEC §9 contract names sourced by roles that are not yet
instrumented in this build. They are **not** emitted by `/metrics`; do not
assume them in a dashboard. Wire the alarm rules as each lands.

| Metric | Alarm |
|---|---|
| `spc_a_send_age_seconds` | > 1 h - a stuck height |
| `spc_a_send_repeat_total{front_id}` | > 0 - `A_send` reuse breaks unlinkability |
| `spc_open_slice_shares` | trend |
| `spc_slices_sealed_total{reason}` | unexpected `reason` mix |
| `spc_distribution_slots{kept,trimmed,dust}` | sustained all-`trimmed` |
| `spc_budget_wu`, `spc_budget_utilisation` | pinned at ceiling |
| `spc_ark_overdue_total` | **page** - scheduled past `due_height + ark_retry_blocks` |
| `spc_ark_float_sats` | below `float_target_sats` - refill needed at next matured pool output |
| `spc_ark_surplus_sats` | operator tooling |
| `spc_ark_orders_rejected_total{reason}` | any `bad-signature` - **page** |
| `spc_channel_reject_total{reason}` | identity-parse rejects trending up |
| `spc_jd_declaration_reject_total{code}` | any single code trending |
| `spc_ext32_requests_total{msg}` | trend |
| `spc_outbox_depth`, `spc_outbox_age_seconds` | rising - a full outbox fails closed |
| `spc_stream_lag_seconds{stream}` | rising per stream |
| `spc_tip_divergence_seconds` | > `tip_divergence_secs` (default 30) |
| `spc_registration_lag_revs` | rising |
| `spc_orphaned_settlement_total{reason}` | any `missing_block_event` - reconstruct from the on-chain coinbase (SPEC §7.4) |

`GET /healthz` is the liveness probe.
## 8. What is deliberately not complete

From `docs/STATUS.md` "Remaining"; these are the gaps an operator must know
about, not bugs:

1. **The bark wallet adapter.** Order emission (`payout` signs and caps
   `ArkSendOrder`s) and order verification (the `ark` role) are done, and
   the `ark` role executes sends through the external-command transport
   (`[ark] send_command`, SPEC §11.10), fail-closed on every ambiguity.
   What is still missing is the stock bark wallet/server adapter behind
   that command - it needs a pinned bark checkout plus a stock `captaind`.
   Until it lands, the Ark tier pays out only through an operator-supplied
   send command, not against a live server.
2. **JD fallback.** The rules for a JDS decline or 10 s silence, and the
   full-template rebuild path in the JDC, are not implemented. Live JDS/JDC
   declaration and recompute-and-compare work; the fallbacks do not.
3. **Testnet soak.** Everything above passed regtest only. The spec'd gate -
   testnet4 (`tsp`) soak for >= 24 h with at least one found block
   independently recovered by a third party's `spc-scan` (SPEC §9 rollout) -
   has not run. Mainnet is out of scope for v0.1.
4. Also pending, lower operational impact: the `"ipc"`/`"sv2-tp"` template
   sources (§1), the seeded `--features cheat-slice-difficulty` statistical
   test, and the normative/security review over the integrated tree.
5. **Host and namespace lab scope.** The approved `e891f34c` host slice is
   bounded evidence, not production deployment. The current namespace-runtime
   candidate additionally documents `--topology`: a host-resident controller,
   actors entering broker user+network namespaces, per-pool namespaced Redis,
   host PostgreSQL over private Unix sockets, namespace-scoped node/front RPC,
   and durable partition/heal evidence; see [`LAB-RUNTIME.md`](LAB-RUNTIME.md)
   sections 10-12. Convergence is an explicit versioned `phase.convergence`
   action with declared observation/resume bounds, not an implicit runtime
   fallback. Actor isolation is a network boundary on a shared filesystem,
   not a security sandbox. This is a separate candidate requiring Fable approval of its
   exact source/binary manifest, not an inherited host approval or a general
   production topology. Host loopback still does not cross namespaces. Nix
   network-tool outputs are externally blocked and are not required for the
   core package; host-tool namespace proofs are labelled separately.
6. **v2 is a lab profile.** It is regtest-only, v1 remains the frozen default,
   type-32 is v1-only, and the lab's exact-`owed` pre-ladder scope does not
   amend the production denomination policy. Runtime header/payout oracles and
   causal hashrate-response claims remain out of scope.
