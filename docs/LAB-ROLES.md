# Lab roles, processes and actual interfaces

Status: source-owned guide to the current read-only `/lab` hub. It describes
what the code launches now, not the target production architecture. The
standard experiment below is
`crates/spc-lab/scenarios/lab-live-v2-two-pools.json`; another scenario can
change process counts and declared roles. `DESIGN.md` and `docs/SPEC.md`
remain normative for production payout, privacy and Ark behavior. This page
does not amend them.

## 1. Read the hub without confusing role, process and health

Start `spc-lab runtime` with `--listen 127.0.0.1:0` as in
[`LAB-RUNTIME.md`](LAB-RUNTIME.md). The printed loopback origin serves:

| Route | Actual function |
|---|---|
| `/` | Existing advanced run monitor and durable miner controls. |
| `/lab` | Read-only role/process inventory and links to actual interfaces. |
| `/lab.js`, `/lab.css` | Static assets for `/lab`. |
| `/api/lab/v1/runs/<id>/inventory` | Allowlisted JSON projection used by `/lab`. |

The existing snapshot, events, catalog and typed-command APIs remain at their
documented routes. The hub adds no daemon, listener, proxy, role-start button,
funds control or second command plane. It shares the existing `HttpServer`,
run lock, port and lifetime. Recorded review with
`spc-lab serve --run-dir <run-dir> --listen 127.0.0.1:0` exposes the same
pages with an explicit **recorded / read-only** banner; it does not adopt
processes or enable miner commands.

Three columns mean different things:

1. **Declared role** comes from the scenario, including several logical pool
   roles on one process.
2. **Last recorded process state** comes from the supervisor/manifest. In
   particular, `running` is a process record, not proof of current health,
   successful mining, correct payout or API reachability.
3. **Observed availability** comes from evidence already collected by node,
   pool, storage or miner observers, with its actual observation timestamp and
   age. A missing observation is shown as unknown. The hub sends no health request
   and creates no synthetic heartbeat. Its own response time only proves that
   the controller served that response.

Dependencies, role notes and integration detail use progressive disclosure;
the main table stays compact. It has no raw event/manifest dump, financial
dashboard, log viewer, RPC cookie, connection URL, process argument or
filesystem path.

## 2. Current standard two-pool experiment

### 2.1 What is actually running

The standard host scenario declares one rbitcoin node, two independent
`spc-pool` front processes, two `spc-cpu-miner` processes, two Redis
instances and two PostgreSQL databases/clusters. `spc-lab` is the controller,
not another scenario actor. Its scanner and metrics observers are in-process
collector threads, not additional daemons. Two pools do not imply separate
front, payout and API binaries: each front's `roles` array declares `front`,
`payout` and `api`, all co-located in that front's one `spc-pool` process.
The topology scenario can declare two nodes and a namespace broker instead.

| Actual component | Current function | Interface and browser reality | Source owner |
|---|---|---|---|
| `spc-lab` controller | Parses the versioned scenario; renders runtime configuration; owns real process supervision, durable commands, evidence, collection and loopback HTTP/SSE. | `/` is the advanced control/monitor UI; `/lab` is this read-only role hub. Both have the same lifecycle as the controller. Recorded `serve` is read-only. | `bins/spc-lab/src/main.rs`, `crates/spc-lab/src/runtime.rs`, `crates/spc-lab/src/engine.rs`, `crates/spc-lab/src/http.rs`, `crates/spc-lab/src/inventory.rs` |
| rbitcoin node(s) | Regtest consensus/canonical chain and difficulty. Supplies block templates and accepts submission; provides P2P and authenticated JSON-RPC. The lab helper also consumes tip observations. | Authenticated JSON-RPC is not a browser UI. The hub never exposes the cookie or turns RPC into a browser link. | `crates/spc-lab/src/adapters.rs`, `crates/spc-lab/src/runtime.rs`, `bins/spc-pool/src/template_rpc.rs`; the external pinned binary contract is in `docs/LAB-RUNTIME.md` |
| `spc-pool` front process | The declared `front`, `payout` and `api` logical roles run together per front in the standard scenario. | Native mining protocols are for miners, not browsers. The API and metrics links below belong to the `api` role, not to a pool HTML operator UI. With `spc-lab runtime --operator-ui` (host runs only) each front also serves the approved read-only operator page on a separate leased loopback listener, linked in `/lab` as the typed `operator_ui` interface after a start-time HTTP 200 route proof. | `crates/spc-lab/src/scenario.rs`, `crates/spc-lab/src/engine.rs`, `bins/spc-pool/src/main.rs` |
| Logical `front` role | Job construction/publication, SV1/SV2 ingress, share validation, block assembly/submission and the JDS surface. | Native SV1/SV2/JDP listeners; no role-native HTML UI. | `bins/spc-pool/src/front.rs`, `bins/spc-pool/src/sv1_server.rs`, `bins/spc-pool/src/sv2_server.rs`, `bins/spc-pool/src/block.rs`, `bins/spc-pool/src/jds_server.rs` |
| Logical `payout` role | Persists accounting, maintains pending balances and applies the confirmation-gated ledger. It is not a wallet UI or a new-funds control. | Postgres and Redis through explicit storage adapters; no HTML UI. | `bins/spc-pool/src/payout.rs`, `crates/spc-store/src/postgres.rs`, `crates/spc-store/src/redis.rs` |
| Logical `api` role | Read-only pool JSON, authenticated SP audit/challenge endpoints, health and Prometheus metrics. | `GET /api/pool` is JSON and `GET /metrics` is Prometheus text. They are explicitly labelled as such, not as an operator UI. Other actual routes are listed in the source router. | `bins/spc-pool/src/api.rs`, `bins/spc-pool/src/api_backend.rs`, `bins/spc-pool/src/metrics.rs` |
| `spc-cpu-miner` | Real SHA256d work under declared budgets; native SV2 extended-channel sessions and worker drivers. | Owned JSONL control is wired through `spc-lab`; the existing `/` page issues the durable pause/resume/budget/add/switch commands. It is not `spc-agent` and has no separate browser UI. | `bins/spc-cpu-miner/src/main.rs`, `bins/spc-cpu-miner/src/miner.rs`, `bins/spc-cpu-miner/src/worker.rs`, `bins/spc-cpu-miner/src/hash.rs`, `bins/spc-cpu-miner/src/control.rs` |
| Scanner/collector | The standalone `spc-scan` CLI/library exists. In this runtime, `spc-lab` calls `spc_scan::scan_block` from an in-process collector over fetched blocks. | No standalone scanner actor or browser UI is invented. The CLI is available separately. | `crates/spc-lab/src/runtime.rs` (`spawn_scanner`), `crates/spc-scan/src/lib.rs`, `crates/spc-scan/src/main.rs` |
| Redis (one per pool) | Pool event and front streams/event bus; the collector performs authenticated reads and records their observations. | Redis protocol only; no browser UI and no direct authenticated URL in the hub. | `crates/spc-lab/src/resource.rs`, `crates/spc-lab/src/runtime.rs` (`spawn_redis_observer`), `crates/spc-store/src/redis.rs` |
| PostgreSQL (one database plane per pool) | Ledger and accounting persistence; the collector performs authenticated aggregate reads. | SQL storage only; no browser UI and no credential-bearing database link in the hub. | `crates/spc-lab/src/resource.rs`, `crates/spc-lab/src/runtime.rs` (`PostgresHook`, `spawn_postgres_observer`), `crates/spc-store/src/postgres.rs` |

`spc-pool` role selection and co-location are visible in
`bins/spc-pool/src/main.rs`; the scenario-to-process mapping is in
`crates/spc-lab/src/engine.rs`. Therefore the hub deliberately shows one
front process row with several logical-role labels instead of inventing three
healthy role daemons.

### 2.2 Host API links versus internal namespace transport

On a host run, an allocated loopback pool API listener can produce only the
known `/api/pool` and `/metrics` links. These are unauthenticated public
endpoints already served by `spc-pool`; the hub does not proxy them. Node RPC,
Redis and PostgreSQL never receive browser credential handoffs.

With `--topology`, namespaced actor addresses are internal transport and are
not assumed reachable from the host browser. The inventory consequently omits
those clickable URLs. Where the runtime already collected a front/node/storage
observation, the hub shows its reachability result, timestamp and collector
source. Otherwise it says availability unknown / no browser UI; it does not
substitute a broken localhost address.

## 3. Optional, absent and required-next capabilities

Absence is not reported as a crashed or down process.

| Capability | Current experiment fact | UI and next-profile status | Source owner / evidence |
|---|---|---|---|
| Namespace broker | Not used by the standard host scenario. `--topology` explicitly enables broker-owned namespaces and network faults. | No standalone user UI. When enabled, only existing broker observations are shown; no browser fault-control API is added. | `bins/spc-lab/src/main.rs`, `crates/spc-lab/src/namespace.rs`, `crates/spc-lab/src/links/netns.rs`, `scripts/lab/lib/lab_backend.py` |
| `spc-agent` translator/JDC | The binary exists but the standard lab launches `spc-cpu-miner`, not `spc-agent`. It is not labelled down or running. | No role-native browser UI. Translator and JDC are wired. The standard scenario does not launch an agent wallet or wire live registration. See the primary-integration note below before carrying an older global "wallet unimplemented" claim into the merged tree. | `bins/spc-agent/src/main.rs`, `bins/spc-agent/src/translator.rs`, `bins/spc-agent/src/jdc.rs` |
| `spc-pool` Ark delivery/float role for each pool | No front in the standard scenario declares `ark`; the default package builds the pool with `rpc`, not the separate `ark` feature. The hub says **not running in this experiment / planned required integration**, not that Ark is permanently optional. | The expanded live lab requires a separate Ark delivery and float role for each pool. The pool operator's Ark wallet/float is distinct from each SPc user's wallet. No operator-wallet UI or new-funds control is claimed. | `crates/spc-lab/scenarios/lab-live-v2-two-pools.json`, `bins/spc-pool/src/main.rs` (`Role::Ark` gates), `bins/spc-pool/src/ark/`, `flake.nix` |
| Independent Ark server | `ark_backends` is empty in the standard scenario; no Ark server or settlement is marked running. | **Required in the expanded live lab** as an independent service; current status is **not running in this experiment / planned required integration**. This guide does not implement that service or relabel a fixture as Ark settlement. | `crates/spc-lab/scenarios/lab-live-v2-two-pools.json`, `crates/spc-lab/src/scenario.rs`, `crates/spc-lab/src/engine.rs`; production third-party-server policy remains in `DESIGN.md` / `docs/SPEC.md` |
| SPc user wallet: registration, receive and refresh | No per-user wallet actor is launched. Existing scanner collection and miner SV2 identity use do not prove wallet registration, receipt or refresh. | **Required for each SPc user in the expanded live lab; not running in this experiment / planned required integration.** Miner wallets remain distinct from the operator Ark float wallet. Desktop/mobile wallet UIs are not implemented. | Standard-scenario absence plus the candidate `bins/spc-agent/src/main.rs`; see the primary-integration note below rather than treating the candidate entry point as current primary capability. `crates/spc-scan/src/lib.rs`, `spc_ark_client/` (reference prototype only, not a running lab role) |
| Lab payout wallet | Pool configuration uses a throwaway regtest payout fixture. It is not an operator Ark wallet, an SPc user wallet or a real-funds account. | No wallet desktop/mobile UI, real wallet funds or funding controls. The scenario guide identifies the descriptor as throwaway lab material; the hub never publishes it. | `crates/spc-lab/src/runtime.rs` (`POOL_PAYOUT_DESCRIPTOR` and rendering), `docs/LAB-RUNTIME.md` |

Primary-integration note (checked read-only on 2026-11-08): T4a witness
primitives, AP-1a Ark RPC probe tooling, AP-3a agent wallet foundation and the
T0 lab traffic contract are integrated in the primary tree. They are **not**
included in this candidate. AP-3a is explicitly a foundation: its own scope
leaves live registration transport, full stock-bark wallet opening, chain scan
and delegated refresh/offboard unwired. Neither the candidate nor the primary
standard two-pool scenario launches an agent wallet or an Ark backend. At merge,
preserve that distinction between backend foundation and what this run actually
starts; do not carry forward the candidate's older blanket "wallet
unimplemented" wording.

The Ark server, each pool's Ark delivery/float and every SPc user's wallet
capability are separate required integrations. Combining them into one green
"Ark" badge would misrepresent all three. Production SPc/Ark privacy rules,
including the prohibition on SP identity material crossing into the Ark role,
are unchanged. The hub publishes no scan/spend key, SP identity material,
payout descriptor, order, receipt or wallet secret.

## 4. Capability matrix for the running experiment

| Component / interface | Serve role JSON | Native protocol/API available | Browser UI | `/lab` link policy |
|---|---|---|---|---|
| `spc-lab` | Run/evidence APIs, yes | HTTP/SSE loopback | Yes: `/` advanced monitor; `/lab` read-only hub | Link both actual routes |
| rbitcoin | No lab role JSON endpoint claimed | P2P + authenticated JSON-RPC | No | Never expose RPC auth |
| `spc-pool` front | Through pool-owned APIs/evidence as implemented | SV1/SV2/JDP | No role-native UI | No protocol-to-browser rewrite |
| `spc-pool` payout | Through pool-owned storage/evidence | Postgres/Redis adapters | No role-native UI | No storage credentials |
| `spc-pool` api | `/api/pool` and other router JSON endpoints | HTTP JSON + `/metrics` | No HTML operator UI | Host loopback only; labelled API JSON/metrics |
| `spc-cpu-miner` | Owned control result stream | SV2 + lab-owned JSONL | No standalone UI; use `/` durable controls | Link only `/` |
| Scanner | Collector evidence | Library/CLI; collector reads node data | No standalone UI | No invented daemon/link |
| Redis / PostgreSQL | Existing already-collected observations only | Authenticated native storage protocols | No | Never expose authenticated URLs |
| Namespace broker | Existing bounded broker evidence only | Controller-owned backend | No | No namespace address as a host link |
| `spc-agent` | Existing binary interfaces; not launched by standard lab | Translator/JDC in the standard profile | No standard-run agent wallet or live registration | Not labelled running/down |
| Ark services and wallets | Not running in standard experiment | Planned required integration | No role-native wallet UI implemented | Explicit capability gap, never a running claim |

For production interfaces rather than this experiment's actual surface, use
`docs/SPEC.md` and `docs/WIRE-FORMATS.md`. The older architecture map is a
historical target design, not the source of this runtime role table.
