# Lab runtime: host and isolated topology

Status: operator guide for the approved host slice and the namespace-runtime
candidate. This is a lab tool, not a production deployment mode. The host
baseline `e891f34c` / tested code `d75ab391` is approved. The namespace runtime
has a separate implementation report and requires its own exact Fable approval;
do not inherit the host approval. The normative pool behavior remains
`docs/SPEC.md`; `DESIGN.md` remains authoritative for payout derivation, tier
policy and Ark delivery.

## 1. What it runs

`spc-lab runtime` reads a versioned scenario and starts real child processes:

- rbitcoin node (externally supplied, pinned L2 build; see section 3);
- one or two independent `spc-pool` fronts, each with its own Redis instance,
  PostgreSQL cluster/database/role/password, listener set and Noise authority;
- native `spc-cpu-miner` processes, controlled over the approved
  `spc-cpu-miner-control` v1 JSONL protocol;
- Redis and PostgreSQL from explicit binaries;
- the scanner/collector side (node tip sampler, pool `spc_evidence` tail,
  authenticated PostgreSQL/Redis reads, `spc_scan::scan_block`).

With `--topology`, the controller stays in the host user/network namespaces and
uses the same engine, run lock, listener and resource plan. Node/front/miner/
Redis actors enter the broker user namespace and their endpoint network
namespace at launch. Each pool has a separate storage network namespace.

The runtime renders every config itself. It does **not** merge an operator
TOML or authority fragment. Private files are created under
`<run_dir>/private` (mode `0700` for the directory, `0600` for files):
identities, secrets, node cookie, Noise authority, generated pool/miner/Redis
configs. Public/shareable output is under `<run_dir>/shareable`; logs are under
`<run_dir>/logs`.

Two pools mean two isolated ledgers and storage resources. They currently share
the same throwaway payout descriptor; that is a lab convenience, not two
production payout destinations.

## 2. Build the four binaries

Run from the approved host-runtime source (the `lab-runtime` worktree of the
approved packet, not this documentation worktree):

```sh
nix build \
  .#packages.x86_64-linux.spc-lab \
  .#packages.x86_64-linux.spc-pool \
  .#packages.x86_64-linux.spc-cpu-miner \
  .#packages.x86_64-linux.spc-scan \
  --print-out-paths
```

The pool must be the package built with the `rpc` feature; the flake package
does that. A locally rebuilt `target/debug/spc-pool` without `--features rpc`
fails the rendered `template.source = "rpc"` startup, so prefer the Nix store
paths recorded in the run manifest.

Do not treat a debug binary hash as a source binding: debug hashes vary between
otherwise identical builds. Use the Nix outputs and record their store paths
and derivations.

## 3. Node binary exception

No Nix node output is required by the core package. The approved lab used the
externally built, pinned L2 rbitcoin binary and recorded it separately:

- binary path: explicit (`--node <absolute-path>`);
- expected SHA-256: `eb257af7c10cdf0ded584adfadf2ae8124466304b13d26c17f855e76f9320f7c`;
- source snapshot: `d2f37b9113e8855245866289ed393043aed6e6153e8101375072cbb23624d499`.

The node's provenance is **not** claimed to be a Nix build. Optional Nix network
tools are a separate, externally blocked capability gate; they are not required
for this host run and their absence is not a core package failure.

## 4. Start a two-pool run

Use explicit paths and the frozen code identity (the actual values are emitted
by the packet's manifest builder; do not leave the old literal defaults):

```sh
SPC_LAB=/nix/store/...-spc-lab-0.1.0/bin/spc-lab
POOL=/nix/store/...-spc-pool-0.1.0/bin/spc-pool
MINER=/nix/store/...-spc-cpu-miner-0.1.0/bin/spc-cpu-miner
SCAN=/nix/store/...-spc-scan-0.1.0/bin/spc-scan

"$SPC_LAB" runtime \
  --scenario crates/spc-lab/scenarios/lab-live-v2-two-pools.json \
  --run-dir /tmp/spc-lab-run-1 \
  --node /path/to/rbitcoin-node \
  --pool "$POOL" \
  --miner "$MINER" \
  --scanner "$SCAN" \
  --postgres-bin-dir /path/to/postgresql/bin \
  --redis-server /path/to/redis-server \
  --redis-cli /path/to/redis-cli \
  --base-revision b0ab4ca34fda07c61a6335d05f598eaacc69678c \
  --dirty-tree-hash <frozen-code-snapshot-sha256> \
  --listen 127.0.0.1:0 \
  --duration-secs 120
```

Add `--operator-ui` to the host command above to render the approved
EXP-OP read-only operator page (see
`docs/decisions/2026-11-08-pool-operator-page.md`) into every pool config
and print its URL after a route-readiness proof:

```sh
"$SPC_LAB" runtime \
  ... same arguments as the host command above ... \
  --operator-ui
```

- Off by default; host runs only. `--operator-ui` with `--topology` is
  rejected at prepare time (no reviewed controller relay to an isolated
  front's loopback listener exists yet), and every front must carry the
  `api` role because the operator listener runs inside the pool API
  process.
- The process count is unchanged: the page is served by an extra loopback
  listener inside each existing pool API process, leased through the same
  port-lock machinery as every other listener (`front.<id>.operator`).
- An HTTP 200 on the page proves the route is served; it is not a funds
  health claim. The float fact reads `not_configured` until the AP-4
  status source exists; the liability and payout facts are the pool
  store's actual values.
- The printed `spc-lab-runtime: operator front-<id>=http://127.0.0.1:<port>/`
  line appears only after the proof. The `/lab` hub links the page as a
  typed `operator_ui` interface under the same rules (recorded serves
  unlink it).

For the isolated two-node/two-pool/two-miner scenario, use the same binary
paths and add the live backend. `ip`, `nft`, `unshare`, `nsenter` and `python3`
must be resolvable on `PATH` (or passed explicitly for the last two):

```sh
"$SPC_LAB" runtime \
  --scenario crates/spc-lab/scenarios/lab-network-runtime-two-pools.json \
  --run-dir /tmp/opencode/LP-network-kimi-reproduction \
  --node /path/to/rbitcoin-node \
  --pool "$POOL" \
  --miner "$MINER" \
  --scanner "$SCAN" \
  --postgres-bin-dir /path/to/postgresql/bin \
  --redis-server /path/to/redis-server \
  --redis-cli /path/to/redis-cli \
  --pow-bits 0x207ffffe \
  --base-revision b0ab4ca34fda07c61a6335d05f598eaacc69678c \
  --dirty-tree-hash <frozen-code-snapshot-sha256> \
  --topology \
  --broker-script scripts/lab/lib/lab_backend.py \
  --listen 127.0.0.1:0 \
  --duration-secs 3
```

The `0x207ffffe` initial target is a regtest convenience, not a change to the
scenario's `pow_limit_bits = 0x207fffff` and not a retarget-response test. The
scenario uses real phase commands: baseline work, pause, P2P partition,
independent work on both fronts, heal, pause, and both front accounting
assertions. `converge-after-heal` is an explicit native action with
`version: 1`, `timeout_secs: 90`, `max_observations: 90`, `allow_resume: true`
and `max_resume_attempts: 1`. Only this declared action may perform one real
SV2 resume to break a tied fork, followed by a durable pause and atomic
revalidation. Any resume invalidates earlier quiescence, even if its outcome is
partial or unknown; a later common-tip certificate requires a new successful
pause and a fresh atomic observation. Existing scenarios without `convergence`
get no post-phase resume/hash fallback. Every resume/pause is journaled; timeout is
`not_converged` and stop/interruption is `unknown` with a reason. The budget is
cooperative between potentially blocking node-observation rounds, not a hard
per-RPC deadline. The action executes after all declared phases regardless of
where it appears, so the provided scenario places it last; position-dependent
semantics are otherwise outside this packet.

The full no-bypass probe is deliberately performed with workers paused because
it takes many IPv6 1-second negatives; finishing a tied fork without mining
after heal can be impossible even when the transport is healthy. No phase calls
`generate*`, `setmocktime` or a fake accounting hook.

`--listen 127.0.0.1:0` prints the bound URL. Use a specific loopback port if a
browser needs a stable address. Without `--listen` the run still starts; use
`--duration-secs` for a finite headless run. `--interactive` accepts a typed
`stop` on stdin.

The run directory must be fresh. A failed preparation can leave private
material behind; inspect and remove that run directory before retrying. Do not
reuse a run directory that already has a `run-manifest.json`.

## 5. Browser and durable controls

Open the printed URL. The root page (`/`) shows the snapshot, entities,
evidence timeline and miner controls. It is an observer with the same durable
API:

- `GET /api/lab/v1/runs/<id>/snapshot?cursor=...`
- `GET /api/lab/v1/runs/<id>/events?after=...`
- `POST /api/lab/v1/runs/<id>/commands`
- `GET /api/lab/v1/runs/<id>/catalog`

Open **`/lab` on the same printed origin** for the small read-only live hub.
It lists the actual run id/lifecycle, recorded actor count, co-located logical
roles, last-recorded process states, collected observation age and actual
interfaces. Its `Open live controls` link returns to `/`; it adds no daemon,
listener, proxy, role-start action or wallet/funds control. The backing route
is `GET /api/lab/v1/runs/<id>/inventory`, an allowlisted projection rather
than the raw private manifest or process records. Pool links are explicitly
JSON (`/api/pool`) or metrics (`/metrics`), not an HTML operator UI. Internal
namespace endpoints do not become host-browser links. See
[`LAB-ROLES.md`](LAB-ROLES.md) for the source-owned role map and absent UIs.

Recorded-run review uses the same pages without adopting processes:

```sh
"$SPC_LAB" serve --run-dir <run_dir> --listen 127.0.0.1:0
```

Both `/lab` and `/` identify this as recorded/read-only; miner controls are
disabled. A recorded `running` state is historical, not a current health
claim. Observations retain their actual evidence timestamps. The hub itself
performs no new probe and creates no synthetic heartbeat.

The catalog exposes declared fronts/pools, public SP addresses with `used`, and
live miner connection/worker specs. It is loopback-only and contains no secret
material. An added miner reuses a declared persistent identity; it does not
generate replacement identity material.

Supported typed commands:

- `add_miner` (including retry with the same `command_id`);
- `switch_pool`;
- `set_worker_budget`;
- `pause_workers` / `resume_workers`;
- `stop_run`.

The command journal is durable. The same `command_id` with the same payload
returns the stored outcome; the same id with a different payload returns a
conflict and does not duplicate the effect. Closing the browser tab does not
stop the run.

## 6. Inspection and cleanup

```sh
"$SPC_LAB" snapshot --run-dir <run_dir> --cursor 0
"$SPC_LAB" events --run-dir <run_dir> --after 0
"$SPC_LAB" manifest --run-dir <run_dir>
"$SPC_LAB" command --run-dir <run_dir> --json '{"kind":{"type":"stop_run","reason":"manual"}}'
"$SPC_LAB" reap --run-dir <run_dir>
```

Normal stop is `stop_run`/duration/interactive; it writes the shareable result
and stops owned processes. `reap` is for a controller that was killed and uses
persisted ownership records; it never kills by PID alone. A run whose final
state is `completed_with_errors` needs the manifest inspected before starting
another run.

For a traffic run, `reap` also works when the run stopped before the traffic
capability was granted (FAILED/INCOMPLETE provisioning): cleanup never grants
or exercises a capability, so the absent projection does not block resource
cleanup, while a recorded-but-invalid projection still rejects
(`docs/decisions/2026-11-12-lab-reap-cleanup-never-granted.md`). Traffic nodes
that were never launched are reported as exited (expected-not-launched) rather
than vanishing from the cleanup report.

For a topology run, normal stop also drops the namespace broker and its owned
holders. The controller is never re-executed, so its original `flock`, HTTP
listener and `Child`/stdio handles stay intact. Each real actor is exec'd by an
`actor-exec` helper which sets `PR_SET_PDEATHSIG=SIGKILL`, verifies the expected
controller start-time token and parent, and only then execs the actor. The
broker has the same parent-death boundary. This design must clean up a genuine
controller `SIGKILL` in bounded time; acceptance uses the packet's explicit
crash proof rather than assuming it from `Drop`. Normal `reap` remains the
fallback for old host records, never a claim that a namespaced actor survived
backend death safely.

## 7. Evidence: what is and is not checked

- Scenario assertions require typed `job_issued`, pool `spc_evidence` plus
  node observations, and at least one accounted record on every declared
  front. An empty or one-pool run therefore cannot pass the two-pool scenario.
- `accounted_target_consistency` joins each accounted record to its announced
  job by `(front, job_id, generation)`, checks target/`m_issue`/booked `d`, and
  checks the recorded hash against the announced target. It reports
  `consistent`, `inconsistent` or `unknown` with an explicit checked/not-checked
  scope. It does **not** recompute the 80-byte header, derive the family target
  independently, verify DB credit sums, payout, settlement or canonical
  inclusion; the full header/payout oracle remains `inconclusive`.
  In this combined namespace candidate the accepted F1 fix is integrated:
  malformed/non-32-byte hashes increment `malformed_share_hashes` and are
  `inconsistent`. The older approved host build (`e891f34c` / `d75ab391`)
  skipped them; that version-bound limitation still applies to the old host
  artifacts, not to this combined run.
- The node collector is a **tip sampler**, not continuous canonical coverage or
  a reorg tracker. A tip is accepted only as an atomic
  `getbestblockhash` -> `getblockheader` -> matching `getblockhash(height)`
  triple. A racy or failed read is `unknown`, never a fabricated displacement.
  Displacements come only from the bounded last-K recheck. Phase observations
  preserve per-node height/hash/peer views and the actual heal outcome.
- The scanner records a contiguous cursor, explicit `skipped_heights` and a
  retry position; it never advances past a failed height.
- `runtime_metrics` reports requested and measured rates separately,
  `front_evidence` joins credits/stale/rejected/transport-unknown per front,
  and counters distinguish submitted/block-qualified/canonical observations.
  A block-qualified share is not a canonical block.

## 8. Provenance and approval gate

The host runtime's source-bound proof and exact binary/store provenance are in
[`reviews/2026-11-06-lab-runtime-implementation.md`](reviews/2026-11-06-lab-runtime-implementation.md).
The approved live-driver provenance and gate instructions are in
[`reviews/2026-11-05-lab-live-implementation.md`](reviews/2026-11-05-lab-live-implementation.md)
and `testing/regtest-lab-v2-e2e.py` / `testing/lab-live/`. The approved driver
binds exact LP-1/LP-2/L2 snapshots and refuses an unrelated or rejected hash;
do not replace that binding with a free-text approval. The combined runtime +
pool binary needs a new exact manifest/approval for acceptance; a diagnostic
run is provisional.

## 9. Known capability limits

Do **not** claim from this lab:

- causal hashrate-budget/retarget response (the baseline node profile has
  `no_pow_retargeting = true`);
- full header/window/payout/settlement oracle verification or approved-oracle
  equivalence;
- fee/witness workload coverage, the denomination ladder, Ark settlement, SV1
  interoperability, TDP production integration, or general SV2 interoperability;
- an Ark server, a pool Ark delivery/float role or an SPc user wallet in the
  standard scenario. These are **not running in this experiment / planned
  required integration** for the expanded lab, not permanently optional ideas.
  The independent Ark server, each pool's operator Ark delivery/float wallet,
  and every SPc user's registration/receive/refresh wallet are separate work.
  Operator and miner wallets are distinct. The standard scenario launches no `spc-agent` wallet role and wires no wallet desktop/mobile UI; backend wallet foundation on primary is described in `LAB-ROLES.md` section 3 and is not launched by this run.
  No wallet mockup is a running role UI, and the throwaway pool payout
  fixture is not real wallet funds;
- a green rbitcoin full suite. Only `sv2_tp_bootstrap` passed 6/6; L2 is
  CLEARED_WITH_RECORDED_BASELINE_LIMITATIONS at `d2f37b91...`;
- approval of the namespace candidate by the older host review. Use the exact
  combined source/binary manifest and new Fable verdict instead;
- Nix-built network tools. `iproute2`, `nftables` and `util-linux` realization
  remains externally blocked on this host. Host-tool namespace evidence is
  labelled explicitly and do not imply that the default Nix check exercised it.
- traffic runs. The `traffic` scenario section, the `traffic_capability`
  manifest field and the AP-1a projection gate are frozen (see
  [`reviews/2026-11-08-traffic-contract-implementation.md`](reviews/2026-11-08-traffic-contract-implementation.md)
  and `docs/decisions/2026-11-08-lab-traffic-contract-t0.md`), and the
  private history journal contract is version 2 - the full record mirror
  of the Fable-approved T1a core header, proven against the real writer
  by the conformance suite (`docs/decisions/2026-11-08-traffic-history-v2.md`)
  - but no traffic
  run can start yet: `RunEngine::prepare` rejects every traffic-bearing
  scenario with `traffic_capability_unrecorded` until a T1 runtime records an
  approved-lineage, recognized AP-1a projection into the run manifest, and
  adoption re-checks the same gate before taking the owner lock. A
  config-declared capability name is never evidence, and no
  `NODE_CAPABILITIES` claim exists. This is the intended safe-unavailable
  state, not a regression.

The lab's payout policy is the exact `owed` pre-ladder test scope; it does not
amend the production `DESIGN.md` payout policy. `a_send` and all SPc identity
material stay on the SPc side; no identity field crosses into the Ark role.

## 10. Namespace ownership, storage and controls

### Controller and actor lifetime

The controller never joins another user/network namespace and never re-execs.
One process owns the engine resource plan/lock/listener/supervisor for its whole
lifetime. Isolated actors are launched exactly once through:

```text
nsenter --user=<broker-userns> --net=<endpoint-netns> \
  --preserve-credentials -- <actor> [args...]
```

The broker and every actor use the safe helper described in section 6. The exec
handshake canonicalizes the expected binary, verifies both namespace-owner
start-time tokens, reports early launcher failure, and kills/waits an owned
failed launch. Miner stdin/stdout remains the ordinary owned JSONL pipe. Node
RPC and front API calls use bounded broker `exec` helpers in the correct
namespace. No controller lock/FD is adopted or transferred and no stdio is
hijacked. Actors do not mutate host networking: `ip`/`nft` changes stay inside
the broker-owned **network** namespaces; there is no host `sudo`, `modprobe` or
`sysctl`. This is a network-isolation boundary, **not a filesystem or security
sandbox**: actors and their descendants remain ordinary same-uid host processes
with the shared host filesystem and run directory trust domain.

### Storage boundary

- PostgreSQL remains non-root in the host user/network namespace and is started
  once by the same engine after resource planning. A front connects over its
  run-private Unix socket using the exact allocated port, role, database and
  password. The SQLx URL uses a nonempty `localhost` authority solely to parse,
  followed by `?host=<private-socket-dir>`; it does not use cross-netns loopback.
- Each pool's Redis runs only in that pool's `storage-<pool>` namespace, bound
  to the broker-published address on the front/storage veth. Its private Redis
  URL is `redis://:<password>@<published-address>:<allocated-port>/0`.
  Other namespace loopbacks are never aliased to `127.0.0.1`. Authenticated
  collector reads use that same address; `PONG` and stream lengths are recorded.
  Wrong passwords, loopback and wrong ports fail in the negative namespace test.
- A P2P consensus fault does not touch either storage plane. This packet does
  not claim an IP partition of the Redis link or of the host Unix socket.

### Placement, faults and evidence

The broker manifest is the only source of network-namespace paths and published
addresses. It is produced before config rendering and recorded in the run
manifest. Declared scenario link IDs are preserved verbatim; no duplicate
created route bypasses the named `link-p2p-nodes` fault. Phase commands and
observations share one durable journal. Each step records atomic per-node
height/hash/peer views, both front API results, host HTTP health and (for
consensus links) raw nftables packet counters. `nft_dropped_packets` counts
packets, not dropped blocks. The full no-bypass check includes IPv4/IPv6
non-link and host-address probes; an empty address enumeration fails closed.

Successful acceptance requires:

- `assert-front-a1-accounted` and `assert-front-b1-accounted` passing;
- distinct fork views while partitioned and a subsequent common, atomic tip
  hash on both nodes after resumed mining, never a fabricated convergence;
- positive packet drops during the fault, `no_bypass.ok = true`, heal to
  `normal`, host controller/front APIs and authenticated storage reachable;
- requested versus measured hashrate, per-front credits/stale/unknown, the
  scoped `accounted_target_consistency` result, and all ten actor records stopped
  or exited with no surviving broker/holder process.

## 11. Node contract and provenance for validation waves

This packet keeps the approved external L2 node; it does not upgrade nodes or
rebase upstream pins. Its recorded binary has SHA-256
`eb257af7c10cdf0ded584adfadf2ae8124466304b13d26c17f855e76f9320f7c` and
source snapshot `d2f37b9113e8855245866289ed393043aed6e6153e8101375072cbb23624d499`.
A stock rbitcoin v0.9.0 binary is not a drop-in: it lacks the lab's
`--regtest-pow-bits` flag and `getblockchaininfo.consensus` profile object. A
separate validator may assess v0.9.0+lab patch only after this packet handoff.

The runtime needs this existing L2 Core-compatible surface:

- `getblockchaininfo` exposes `consensus.pow_limit_bits = "207fffff"`;
  `--regtest-pow-bits 0x207ffffe` is the accepted initial-target override;
- `getblocktemplate`, `submitblock`, `getblockcount`, `getbestblockhash`,
  `getblockheader(hash).height`, `getblockhash(height)`, `getblock(hash, 0)` and
  `getpeerinfo` work over cookie Basic authentication at the published RPC
  address;
- `--listen` and `--connect` accept the separately published address *and peer
  port*. Both peers use `--no-seeds --no-discover --max-outbound 1` and regtest;
- canonical tips are consumed atomically; there is no mock time, `generate*`,
  fake credit or shell-launched mining shortcut.

Project binaries are built from a git-metadata-free frozen source with Nix.
Their outputs, derivations and binary/source hashes are in the current
implementation report. Host `ip`, `nft`, `unshare`, `nsenter`, Python, Redis and
PostgreSQL versions are recorded separately. Reproduce against that manifest,
not a free-form checkout or a debug hash.

## 12. Proof locations

Read `<run_dir>/shareable/result.json` for both assertions and the scoped
accounting result, and `<run_dir>/shareable/topology.json` for phase views,
packet counters, no-bypass and the actual convergence. Durable commands are in
`private/commands.jsonl`; collector evidence is in `private/evidence.jsonl`;
process identities and published placement are in `run-manifest.json`.
Phase/convergence events are versioned typed payloads carrying actual
observations, counters/no-bypass/status/reasons; they are not stringified
`unknown` events. Every node view retains its own height/hash plus
`previousblockhash` from the actual header response. `topology_phase` and
`topology_convergence` also retain the pre-partition common anchor. Divergent
heights alone are never called a fork or convergence; these fields allow an
independent parent-chain/phase validator to distinguish an ancestor-following
node from a genuine partition branch. Legacy `unknown` records remain readable
and generic SSE handling is unchanged.

`run-manifest.json` is **private**, mode 0600. Its resource connection state may
include authenticated Redis URLs needed for restore. It is not a public
projection and must never be copied as one. The `shareable/` manifest, topology
and result projections exclude those credentials. RPC helper calls use the
private mode-0600 cookie through `--auth-file`; helper argv and broker responses
contain that path, never the cookie body. Malformed/unreadable/group-readable
files fail closed, and `--params` requires an actual JSON array.

The implementation report records the frozen run directory, exact commands,
provenance, crash-cleanup proof and all remaining non-claims.
