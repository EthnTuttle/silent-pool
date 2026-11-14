# Lab roles, processes and actual interfaces

This guide describes the application at published revision
`98f0e6caa2482646e3c7073949f6388cd39f200b`. A declared process, an implemented
interface and a successful funded workflow are different facts. Use the current
run's source/qualification/execution receipts. `DESIGN.md` and SPEC remain
authoritative for payout and privacy; this role map changes no protocol.

## 1. Operational browser entrypoints

Start the intended scenario as documented in [LAB-RUNTIME](LAB-RUNTIME.md).
`--listen 127.0.0.1:0` prints the actual controller origin after startup:

| Surface | Actual role and authority |
|---|---|
| Controller `/` | Worker controls and SP/Ark/T1 wallet views through the durable controller API. User reads/commands are scoped to the current claim. |
| Controller `/lab` | Operator process/role directory, observations and actual endpoint links; a user claim opens its own root view. |
| Controller `/api/lab/v1/runs/<id>/inventory` and `/catalog` | Operator-only in expanded runs, not public raw manifests. |
| Controller `/snapshot`, `/projection`, `/wallets`, `/events` under that run prefix | Authenticated, server-filtered observations; SSE uses Authorization headers. |
| Controller `/commands` under that run prefix | Typed durable command POST, with current claim scope checked before effects/replay. |
| Pool operator page | Separate read-only loopback listener, enabled by host-only `--operator-ui`; linked after an actual route check. It is not an auth retrofit on the public pool API. |

The controller's bootstrap HTML/CSS/JS may load without credentials; private
data does not. For expanded runs, read the appropriate local claim under
`<run>/private/access/` and enter its `token` in the page. Operator is
`operator.json`; the preset user names have corresponding user files. Arbitrary
IDs may have hashed filenames: inspect the claim's `principal`. Tokens are
session-only in the browser and must not be put into URLs or shared logs.
See LAB-RUNTIME section 5 for the full workflow.

`spc-lab serve --run-dir ...` opens a recorded/read-only view, not a restarted
simulation. Process `running` in a recorded manifest is historical. Source ages,
gaps, unknown and unavailable fields remain explicit; HTTP 200 is route
liveness, not proof of wallet funds, payout, mining or cleanup.

## 2. Scenario roster

| Profile | Declared roster |
|---|---|
| `lab-live-v2-two-pools.json` | Historical host-miner profile: one node, two pools and two CPU miners; no Ark/traffic population. |
| `lab-ark-wallets.json` / self-hosted wrapper | One node, two pools, four SP wallet agents and four CPU miners at 8 H/s aggregate; stock local Ark infrastructure, but no traffic section or full acceptance assertions. |
| Prepared shared full profile | Two nodes/two pools/four SP wallet agents/four CPU miners, eight background/four user-payment wallets, 40 H/s aggregate and 0.2 ordinary tx/s, with native funding and source-audited lifecycle checks. |

The full profile comes from the admitted bundle and selected compatible
QA/AP5 consumer's `portable prepare-shared`, not by renaming the wrapper's
one-node scenario. Application and consumer releases are independently pinned;
[the AP5 guide](../testing/lab-ark-funded/README.md) supplies the sequence.

## 3. Role and account boundaries

| Component | Responsibility and actual access |
|---|---|
| `spc-lab` | One controller owns process leases, runtime config, evidence, durable commands, UI and stop/recovery. |
| rbitcoin node(s) | Consensus/canonical chain, templates and authenticated RPC. No browser RPC credential bridge is provided. The lab node is built from the flake's pinned published source. |
| Pool front process | May co-locate `front`, `payout`, `api` logical roles; directory counts do not invent extra daemons. Native SV1/SV2/JDP are protocol interfaces, not HTML pages. |
| Pool payout/API | Ledger settlement, public pool JSON/metrics and authenticated owner views. `/api/pool` is JSON; it is not the dedicated operator page. |
| Pool Ark role | Independent pool wallet/float and authorized opaque delivery inputs; no SPc identity-bearing stream or pool Postgres access (SPEC §11.3). |
| Stock `captaind` | Self-hosted local test server/storage, distinct from every pool/user wallet. No custom Ark-server fork/RPC is required by the UI. |
| `spc-agent` wallet | SP-aware registration, scanning and wallet lifecycle for its identity. The controller reads allowlisted status/private input observations and submits typed operations to the owner; it does not become a wallet signer. |
| `spc-cpu-miner` | Native CPU work and the owned control/snapshot pipe. Requested and measured H/s are separate observations; a wallet/browser is not another miner. |
| `spc-lab-txd` | Native bootstrap and transaction-driver-owned wallets. The four user test-payment accounts are separate from the background wallets and from SP/Ark earnings. |
| Redis/PostgreSQL | Separate pool storage resources, plus the Ark server's database where configured. Protocol/SQL endpoints are not browser dashboards. |
| Scanner/collectors | Source observations and proof data; runtime collectors are not invented standalone wallet/miner actors. |

SPc coinbase outputs are watch-only in the live user workbench. SP/Ark funds,
pool float and T1 test funds are not added into one balance. Source/provider
failure renders unknown/unavailable, never an invented zero. An empty private
input list means no eligible proven input, not zero total funds.

Queue submission/running is not confirmation. Unknown requests retain immutable
command IDs; definite terminal results require the actual source/durable
resolution. Worker controls can proceed while a wallet operation is pending.
All effects remain behind the controller's current scope and backend checks.

## 4. Lifecycle and the separate feedback tool

For manual live runs, stop using the original process's interactive `stop`, its
duration, or the authenticated live command API. A second `spc-lab command`
process tries to attach and conflicts with the live owner's lock. Use the exact
API example in LAB-RUNTIME section 6. Shared campaigns own their audit/drain/
stop/reap sequence. Post-owner-exit `reap` uses persisted identities; it is not
global cleanup of other services.

The design-feedback/mockup review utility, commonly on `127.0.0.1:18481`, is a
separate user utility and is not shipped by this checkout's operational lab
launcher. Its synthetic screens, notes and review labels are not mining/Ark
observations or production approvals. Preserve its process, mockups, drafts and
persisted feedback independently of lab runs. Do not recreate missing feedback
or treat an available gallery as a functioning full simulation.

Full funded acceptance remains a real-campaign verdict, not a property of this
role catalog or a generated HTML writeup.
