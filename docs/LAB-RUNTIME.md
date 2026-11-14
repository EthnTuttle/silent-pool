# Lab runtime: setup, live controls and recovery

This is a local regtest operator tool. The recipes below describe published
application source `98f0e6caa2482646e3c7073949f6388cd39f200b` and its current
CLI/API. They do not assert a full funded campaign verdict. `DESIGN.md` and
`docs/SPEC.md` remain authoritative for payout, privacy and Ark behavior.

## 1. What it runs

`spc-lab runtime` owns the node/pool/miner processes, per-pool Redis/PostgreSQL,
rendered configuration, process leases, durable commands and observation API.
An Ark scenario also supplies a self-hosted stock `captaind`, distinct pool
Ark wallets and SP-aware user wallet agents. A traffic scenario adds the native
bootstrap/transaction driver. No browser is a miner or an independent signer.

The scenario selects the actual roster:

| Scenario/profile | Actual scope |
|---|---|
| `lab-live-v2-two-pools.json` | Older host-miner profile: one node, two pools, two CPU miners; no Ark or traffic population. |
| `lab-ark-wallets.json` | Wrapper default: one node, two pools, four wallet agents/miners at 2 H/s each; no `traffic`, phases or acceptance assertions. |
| `portable prepare-shared` output | Full shared profile: two nodes, two pools, four SP wallets/CPU miners, eight background/four payment wallets, 40 H/s and 0.2 ordinary tx/s, with native funding and audited source/lifecycle configuration. |

Use the prepared profile for the full campaign. Adding `--txd` alone does not
add traffic to a scenario. The wrapper hardcodes `lab-ark-wallets.json`; a
second `--scenario` argument is not a supported override. The full campaign
owner launches the admitted lab executable with its generated scenario instead.

Runtime private material is generated for a fresh run under `<run>/private`
(0700 directory, 0600 files); logs and recorded/shareable artifacts belong to
that run. Pool Ark float, SP/Ark earnings and T1 test-payment funds are separate
accounts. No identity-bearing data is added to the Ark role's input streams
(SPEC §11.3).

## 2. Fresh-machine source and Nix setup

Use x86_64 Linux, Nix with flakes enabled, Git and Python 3, and an ordinary
non-root user (PostgreSQL does not run as root). Before building, follow
`AGENTS.md`'s disk budget: at least 50 GiB available and projected peak growth
leaving at least 20 GiB; one heavy build at a time. Reuse realized Nix outputs.

These Bash variables select immutable application source and a new HOME-local
workspace. Commands may be run from any checkout; the flake URL pins the source.

```sh
APP_REV=98f0e6caa2482646e3c7073949f6388cd39f200b
APP_REF="github:EthnTuttle/silent-pool/$APP_REV"
umask 077
WORK="$(cd "$HOME" && pwd -P)/.spc-lab/$(date -u +%Y%m%dT%H%M%SZ)-$$"
mkdir -p "$(dirname "$WORK")"
mkdir "$WORK"
export WORK
```

Keep this canonical workspace path short: the prepared shared profile checks
that its collector Unix-socket paths are shorter than 108 bytes. If your HOME
path is unusually long, select a shorter canonical, user-owned workspace before
preparing inputs. Use a new workspace for each experiment.

For a local source checkout, clone `https://github.com/EthnTuttle/silent-pool.git`
and check out `$APP_REV` detached. No developer worktree, private capture or
historical wallet directory is a build input. `nix run .` selects the default
`spc-pool` package; explicitly select the lab or AP5 package below.

For the full campaign, continue with the complete build/assembly/preparation
recipe in [the AP5 guide](../testing/lab-ark-funded/README.md). It selects
`ap5-inputs` from the application revision and the Python tools/QA descriptor
from a separately published reviewed consumer revision. A new QA consumer does
not require changing the application pin. The currently published reviewed
consumer is `ce7b703ac0c68e6738e5f4510f056cb1209fa982`.

## 3. Node and application provenance

The current flake provides `rbitcoin-node-ark-lab`, stock `captaind`/`bark`,
`spc-lab-ark-tools` and the self-hosted launcher. Its node source is the public
Git revision `c1505776117b1d4d42489f309af33978bbe124cc`; the Ark source is pinned
Bark `c419159fa54085572f8ca72d2ba1716556a9d3eb`. Use the flake recipes and
`UPSTREAM_DEPS.md`, including the node's Git-fetch/export-subst distinction.
The old external L2 binary described in historical reports is not a fresh-user
prerequisite.

For a qualified campaign, application paths and source claims come from the
actual `ap5-inputs/outputs.json` and admitted bundle. Do not substitute a debug
binary, developer store path, or a free-text approval/hash. The committed
source contract and an installed package are not themselves a funded verdict.

## 4. Start the intended run

Choose one profile below; the small wrapper is not a prerequisite for the full
campaign, and starting both would create two independent runtimes.

### Full two-node, traffic and native-funded campaign

Follow [AP5 setup and shared execution](../testing/lab-ark-funded/README.md).
`portable prepare-shared` produces fresh `scenario.json`, `qa-inputs.json` and
an execution plan. It provisions source-attestation inputs and validates the
scenario but does not start actors. `shared` owns one runtime, audit/drain and
stop/reap sequence. Its configured roster and native funding checks, rather
than the small wrapper defaults, define the full simulation.

Keep `APP_REF` and `CONSUMER_REF` distinct. The published reviewed consumer
`ce7b703ac0c68e6738e5f4510f056cb1209fa982` expects exactly the
`t3-controller-r23` QA descriptor (`ap5-qa-source-r23`). A later consumer and
matching QA descriptor must again be a published, reviewed compatible pair.
Use that consumer's release instructions if its interface changes. No consumer
is selected by a moving branch or invented commit.

### Small self-hosted infrastructure profile

This source-pinned command selects the wrapper's one-node profile from section
1. It is useful for infrastructure/UI inspection, not the full native-funded
acceptance campaign. Its initialization/readiness checks may fail closed;
an initialized or registered wallet is not a funded wallet.

```sh
nix build --max-jobs 1 \
  "$APP_REF#spc-lab-self-hosted-ark" "$APP_REF#spc-lab-ark-tools" --no-link
LAUNCHER="$(nix eval --raw "$APP_REF#packages.x86_64-linux.spc-lab-self-hosted-ark.outPath")/bin/spc-lab-self-hosted-ark"
SPC_LAB="$(nix eval --raw "$APP_REF#packages.x86_64-linux.spc-lab-ark-tools.outPath")/bin/spc-lab"
SOURCE_PATH=$(nix flake metadata --json "$APP_REF" | python3 -c 'import json,sys; print(json.load(sys.stdin)["path"])')
SOURCE_HASH=$(nix hash path --type sha256 --base16 "$SOURCE_PATH")
export RUN_DIR="$WORK/runtime"
"$LAUNCHER" --run-dir "$RUN_DIR" \
  --base-revision "$APP_REV" --dirty-tree-hash "$SOURCE_HASH" \
  --listen 127.0.0.1:0 --operator-ui --interactive
```

Here `SOURCE_HASH` records the fetched application source's NAR hash as launch
provenance; it is not an AP5 qualification grant. Full campaigns use their
bundle's source binding instead. The wrapper supplies all binary paths, not
the run directory, provenance, listener or lifetime. The latter are explicit
CLI arguments. Type `stop` in this same process to stop this manual runtime.

For finite headless `runtime`, omit `--listen` and `--interactive`, and pass
`--duration-secs 120`. There is no `runtime --headless` or `--run-root` flag.
Without a duration, interactive stop or API stop, runtime remains alive.
The duration begins after startup/phases; it is not a startup timeout.

The UI origin is printed as `spc-lab-runtime: http=http://127.0.0.1:<port>`
only after startup succeeds. Pool operator URLs are printed after their own
route checks. `--operator-ui` is host-only and conflicts with `--topology`.
For namespace/fault work, use a separately selected topology scenario and
reviewed broker/tools; see sections 10-11 for the historical topology evidence.

Always choose a fresh run directory. Preserve a failed run's private state,
logs and receipts for diagnosis, then use a new directory for a new run; do not
delete old wallet/feedback material to make a retry appear fresh.

## 5. Operational UIs and access tokens

Open the actual controller origin, not a guessed port:

| Route | Use |
|---|---|
| `/` | Worker controls, scoped SP/Ark and T1 wallet views, durable pending commands. |
| `/lab` | Operator role/process directory and links to actually served pool pages. A user claim opens its own root view instead. |
| `/api/lab/v1/runs/<id>/me` | Authenticated run/principal/actor/capabilities. |
| `/api/lab/v1/runs/<id>/snapshot`, `/api/lab/v1/runs/<id>/projection`, `/api/lab/v1/runs/<id>/wallets` | Authenticated status; own-user filtering applies. |
| `/api/lab/v1/runs/<id>/events` | Authenticated SSE using the same scope. |
| `/api/lab/v1/runs/<id>/commands` | Typed durable command POST. |
| `/api/lab/v1/runs/<id>/inventory`, `/api/lab/v1/runs/<id>/catalog` | Operator-only in expanded runs. |

For `expanded_access: true`, the runtime generates an operator claim at
`<run>/private/access/operator.json` and one per declared user in that same
directory. Plain preset names such as `alice` use `alice.json`; unsafe filename
components are hashed, so inspect a claim's `principal` rather than guessing a
path from an arbitrary ID. Each file contains its token, run ID, actor and
capabilities. Open the appropriate local 0600 file in a text editor and paste
only its `token` value into the UI's **Operator or user run token** field.
Neither a loopback address nor a public bootstrap page grants operator access.

The browser sends Authorization headers and stores credentials only in scoped
sessionStorage. Do not put tokens in URLs, command arguments, logs, screenshots
or shared artifacts. Signing out clears the tab's token/view; deleting or
rotating a claim revokes it server-side. Legacy scenarios with expanded access
off retain their tokenless loopback behavior.

SP/Ark earnings, watch-only coinbase outputs and T1 test-payment funds are
displayed separately. Missing/stale providers remain unavailable; unknown is
not zero. A command being queued/running does not prove payment or confirmation.
Retained unknown requests reuse their exact ID/payload when checked. Worker
controls remain independent of an outstanding wallet operation.

Pool operator pages use their separate read-only loopback listener and are
linked when actually served; an HTTP 200 proves the route, not fund health.
The design-review/mockup feedback utility (often port 18481) is separate, is not
installed by this lab launcher, and never proves that mining or Ark is running.
Keep its feedback and process out of lab cleanup.

## 6. Live stop, recorded review and recovery

For a manual runtime started with `--interactive`, type `stop` on that process's
stdin. Otherwise send `stop_run` to the **live controller API**. A second
`spc-lab command --run-dir ...` is not an HTTP client: it attempts attachment
and cannot take the owner lock while the live controller holds it.

The following local Python 3 example reads the operator claim without printing
its token, checks the live run identity, and posts the real command envelope.
Set `LAB_URL` to the printed live origin and `RUN_DIR` to the actual run root.
Keep `STOP_ID` if the request has an ambiguous transport result; rerun only the
Python request with the same ID. Do not send this to a recorded `serve` listener.

```sh
export LAB_URL='http://127.0.0.1:PORT'  # replace with the printed live origin
export RUN_DIR='/absolute/path/to/this/run'
export STOP_ID="$(python3 -c 'import uuid; print(uuid.uuid4())')"
python3 - <<'PY'
import json, os, pathlib, re, time, urllib.parse, urllib.request
def require(ok, message):
    if not ok:
        raise SystemExit(message)
base = os.environ['LAB_URL'].rstrip('/')
url = urllib.parse.urlsplit(base)
require(url.scheme == 'http' and url.hostname in ('127.0.0.1', '::1') and url.port, 'use the printed loopback origin')
require(not (url.username or url.password or url.query or url.fragment or url.path), 'origin must not contain credentials, path or query')
claim = json.loads((pathlib.Path(os.environ['RUN_DIR']) / 'private/access/operator.json').read_text())
require(claim['principal'] == 'operator' and 'operator' in claim['may'], 'operator claim required')
require(re.fullmatch(r'[0-9a-fA-F]{64}', claim['token']), 'malformed claim token')
class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None
client = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
headers = {'Authorization': 'Bearer ' + claim['token'], 'Origin': base,
           'Content-Type': 'application/json'}
with client.open(urllib.request.Request(base + '/api/lab/v1/health', headers=headers), timeout=10) as response:
    require(json.load(response)['run_id'] == claim['run_id'], 'claim and live run differ')
command = {'command_id': os.environ['STOP_ID'], 'run_id': claim['run_id'],
           'issued_at_wall': int(time.time() * 1000), 'actor': claim['actor'],
           'kind': {'type': 'stop_run', 'reason': 'manual stop'}}
path = '/api/lab/v1/runs/' + urllib.parse.quote(claim['run_id'], safe='') + '/commands'
request = urllib.request.Request(base + path, data=json.dumps(command).encode(),
                                 headers=headers, method='POST')
with client.open(request, timeout=60) as response:
    outcome = json.load(response)
print(json.dumps({key: outcome.get(key) for key in ('command_id', 'state', 'detail')}))
PY
```

This example targets expanded runs. For a legacy access-off run there is no
claim file: use interactive/duration stop or the same command API with the
legacy envelope. A timeout/connection close is not a successful stop; inspect
the owner/log/result before recovery. A shared AP5 campaign owns its own
audit/drain/stop/reap sequence. Interrupting it or sending a manual stop is
cancellation, not a passing full-campaign verdict.

Once the live owner has exited, use that run's application executable. For a
qualified bundle, obtain its exact path locally (the path is not a secret):

```sh
SPC_LAB=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["binaries"]["lab"]["path"])' "$WORK/qualified/bundle.json")
"$SPC_LAB" snapshot --run-dir "$RUN_DIR" --cursor 0
"$SPC_LAB" events --run-dir "$RUN_DIR" --after 0
"$SPC_LAB" manifest --run-dir "$RUN_DIR"
"$SPC_LAB" serve --run-dir "$RUN_DIR" --listen 127.0.0.1:0
```

`serve` is recorded/read-only and does not restart actors; end this standalone
viewer when finished. Manifest output is private. For a dead owner's remaining
resources, run `"$SPC_LAB" reap --run-dir "$RUN_DIR"` after inspecting the actual
recorded run. Reap uses persisted PID/start/resource ownership, never PID alone.
It is not a second concurrent controller or a global process cleanup command.

The cleanup-only attach path handles never-granted traffic capability and
native-audit pre-miner startup failures without inventing live capability;
persisted contradictory/foreign bindings still fail closed. See decisions
`2026-11-12-lab-reap-cleanup-never-granted` and
`2026-11-13-lab-reap-cleanup-native-audit-attach`. Topology actors use the
controller/broker lifetime helpers described in section 10. Preserve all
unrelated review/feedback services and their state.
## 7. Evidence: what is and is not checked

Runtime status, a served UI and requested hashrate are observations, not a full
funded acceptance result. Read the selected run's actual receipts and source
bindings. The full shared campaign separately checks native funding/maturity,
independent accounting/history, SP/Ark delivery, refresh/renewal/offboard and
cleanup. A missing or ambiguous provider/proof stays unknown/inconclusive.

The older host collector's `accounted_target_consistency` joins announced and
booked evidence; it is not the full independent header/window/payout oracle.
Requested and measured rates, submitted/block-qualified/canonical results,
source gaps and stale/unavailable wallet observations remain distinct. Historical
host/namespace receipts keep their original scopes and are not upgraded by newer
code or documentation.

## 8. Provenance and qualification

The current full-profile reproducibility and qualification path is
[the AP5 guide](../testing/lab-ark-funded/README.md). A successful source/package
qualification is a prerequisite, not a full funded campaign verdict. Preserve
the bundle, selected immutable consumer/QA descriptor, generated run inputs,
actual output paths, source hashes, execution receipts and stop/cleanup result.
The documentation and `docs/lab-build-report.html` do not replace that verdict.

Historical host proofs remain in
[`2026-11-06-lab-runtime-implementation`](reviews/2026-11-06-lab-runtime-implementation.md)
and [`2026-11-05-lab-live-implementation`](reviews/2026-11-05-lab-live-implementation.md).
Their external-L2 and narrower workload limits apply to those recorded runs.

## 9. Limits and profile selection

- A wrapper's one-node, no-traffic preset is not the two-node native-funded
  profile, even if all its declared processes start and pages load.
- Periodic-retarget response, production TDP/JD/SV1/general-SV2 interoperability,
  mainnet and a green full upstream node suite are not established by this guide.
- Wallet balances/capabilities require their actual providers and source ages.
  Empty inputs are not a zero-balance assertion; queue submission is not delivery.
- The full campaign's independent checks and final verdict must actually run.
  A source-bound contract, successful build, local UI or scoped reaper result
  does not imply funded or global-cleanup acceptance.
- The project's payout derivation/tier policy is `DESIGN.md` and SPEC; a lab
  profile does not authorize a different ladder or donation fallback. SPc
  identity data stays out of Ark-role inputs (SPEC §11.3).

Sections 10-11 preserve the specific namespace architecture and older external
node validation context. They are not additional prerequisites for the default
host/qualified full-profile recipes above.
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

Historical namespace-packet scope: the following recorded external L2 node
identity is not the current fresh-machine Nix prerequisite (see section 3).
That recorded binary has SHA-256
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
