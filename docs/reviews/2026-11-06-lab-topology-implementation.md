# LP-topology: namespace placement, address publication and node scoped observation

Date: 2026-11-06
Author: TFF implementation agent
Status: implementation complete, awaiting Fable review (third pass for the
LP-topology topic; the LP-4a precursor is approved at `bce93151`)
Worktree: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-topology`
Base: `b0ab4ca34fda07c61a6335d05f598eaacc69678c`
Bootstrapped from: the runtime candidate snapshot `c9dfa986c2f6ad0daca44912906a0a2ec94129c25d465eee246f19a43ccd543d`
(172 file records), **reconciled onto** the runtime's second frozen candidate
`e891f34c69c70c2a6748be1ca2a77a71aba4a63377eda55a43afeb8613b68043`
(174 file records, `/tmp/opencode/lab-runtime-review-manifest.json`; tested
code `d75ab391b0ca1c8ab5b03af27dd5cf45a8a384ef8ac056a1ceeb2b2f456d8684`)
Baseline record: `/tmp/opencode/lab-topology-baseline.json`
Contract: `docs/REGTEST-LAB-IMPLEMENTATION.md` §8.3 (link-fault backend), §8.1
(supervisor adoption), §3.2/§3.3 (topology entities and links), LP-4 packet
Upstream inputs: the LP-4a namespace broker (approved `bce93151`), the runtime
candidate (`e891f34c`, **not** approved: its Fable re-review is running; the
six file pool/store delta inside it is approved).

This packet is an **independent integration topic**. It does not inherit the
runtime candidate's approval, and it does not touch the runtime candidate's
worktree. Every artifact it produces is labelled `provisional`.

---

## 1. What was built

The LP-topology packet implements the missing half of the LP-4 topology: the
**placement** of the lab actors into network namespaces, the **publication**
of their addresses, the **controller RPC** that reaches into those
namespaces, and the **node scoped canonical observation** a multi node
partition needs.

| Path | Kind | Purpose |
|---|---|---|
| `crates/spc-lab/src/namespace.rs` | new | The broker client: starts the approved LP-4a namespace broker, reads its published manifest, speaks its Unix socket JSON RPC (the controller RPC), and derives the placement. |
| `crates/spc-lab/src/topology.rs` | new | The topology plan (scenario to broker topology, per plane), the shareable/private publication, and the node scoped canonical observer. |
| `crates/spc-lab/src/links/mod.rs` | moved from `links.rs` | The link backend boundary (LP-3 content, unchanged). |
| `crates/spc-lab/src/links/netns.rs` | new | The netns link backend over the broker: `apply`, `heal`, `current`, the raw counters and the no-bypass proof. |
| `crates/spc-lab/src/supervisor.rs` | edited | The namespace launch: `nsenter --net=... -- <program>` plus the **exec handshake**, so the recorded executable identity is the actor and never the launcher. |
| `crates/spc-lab/src/lib.rs` | edited | The new modules. |
| `crates/spc-lab/tests/topology_namespace.rs` | new | Ten process level tests (real broker, real namespaces, real nftables, real TCP). |
| `crates/spc-lab/scenarios/lab-topology-two-nodes.json` | new | Two nodes, two pools, two miners, the explicit node to node link and real assertions. |
| `testing/lab-topology/run-two-nodes.sh` | new | The headless two real rbitcoin node run with the partition and the heal. |
| `docs/reviews/2026-11-06-lab-topology-implementation.md` | new | This report. |

The topology helpers are deliberately **isolated**: they do not depend on the
line positions of `runtime.rs` or `engine.rs`, so they can be merged after the
runtime's own freeze. The merge notes in section 8 name the exact hooks.

---

## 1b. Reconciliation with the runtime's second frozen candidate

The runtime owner froze `e891f34c` (174 files) while this packet was running.
The tree was **reconciled onto it**, not copied over: the nine changed files
and the two new scripts were taken from the runtime, and the files this
packet owns were left alone.

| Runtime change (`c9dfa986` -> `e891f34c`) | Effect here |
|---|---|
| `crates/spc-lab/src/runtime.rs`, `engine.rs`, `evidence.rs`, `scenario.rs`, `bins/spc-lab/src/main.rs`, the two `lab-live-v2*.json` scenarios, `docs/SPEC.md`, the runtime report, plus the new `scripts/lab/build-runtime-manifest.py` and `scripts/lab/analyze-runtime-latency.py` | taken from the runtime verbatim |
| `crates/spc-lab/src/supervisor.rs`, `crates/spc-lab/src/lib.rs` | **unchanged by the runtime**, so this packet's namespace launch and module wiring needed no merge |
| the files this packet owns | untouched by the reconciliation script (it refuses to overwrite them and reports a conflict instead) |

Reconciliation result: **9 changed + 2 added files copied, 0 conflicts, 0
skipped**.

The interfaces the runtime's R2 introduced are preserved on this side:

| Runtime R2 interface | How this packet preserves it |
|---|---|
| `accounted_target_consistency` (scoped front/job/generation, target and credit) | the runtime owns it; this packet does not reimplement it and does not shadow the topic. The scenario it derives carries the runtime's own assertions. |
| per front real work assertions (`accounted_work(front=...,min=1)`) | `lab-topology-two-nodes.json` is re-derived from the R2 two pool scenario, so it carries `assert-front-a1-accounted` and `assert-front-b1-accounted` unchanged. |
| explicit base / code / store path provenance | `TopologyPublication::with_provenance` carries the run's provenance in the private projection; the two node driver records the node binary, its sha256 and the pow limit in the artifact. |
| `tip_sampler` with a bounded last 8 recheck and `no` on a **confirmed** displacement only | `CanonicalObserver` now matches the runtime's semantics exactly: the recheck window is 8 (`RECHECK_WINDOW`), a *different* hash is `no` and the observation is dropped from the window, an unreadable node is `unknown` and **never** `no`, and every observation publishes `scope: "tip_sampler"`. The regression proves the `unknown` case. |
| fixed capabilities and identity validation | owned by the runtime; this packet does not weaken them and does not declare capabilities on the node adapter. |
| control cloned before blocking metrics | owned by the runtime; the topology backend does not share a client with a blocking reader (the broker uses one connection per request). |
| observer stop quiescence | the broker's control socket answers `ping` until it is stopped; the topology backend reports `Unavailable` afterwards instead of a stale answer. |
| scanner retry cursor and skipped height tracking | owned by the runtime's collector; this packet does not touch the scanner. |
| strict SQL identifiers and raw secrets | owned by the runtime; this packet stores no SQL and no secret: the private projection holds the broker run directory and the endpoint mapping only. |

The runtime's R2 is still **not approved**; this packet inherits nothing from
it and every artifact stays `provisional`.

---

## 1c. Required merge fix F1 (fail closed on a malformed share hash)

**Defect (in the approved host snapshot `e891f34c`, `runtime.rs:506-524`).**
The accounted target consistency check compared the recorded share hash with
the announced target only inside a `if let Ok(hash) = hex::decode(...)` and
`if hash.len() == 32` nest. A hash that was **not hex** or that was **not
exactly 32 raw bytes** silently skipped the comparison and left the record
`consistent`. A missing hash was already `unknown`; a malformed one was
counted as a pass.

**Red evidence** (the approved host's behaviour restored temporarily in this
integration copy, artifact `/tmp/opencode/LP-topology-f1/red-green.json`):

```
test runtime::tests::malformed_share_hash_is_never_consistent ... FAILED
  a nonhex share hash must never be consistent:
  {"status":"consistent","consistent_records":1,"malformed_share_hashes":0,
   "records_checked_against_job":1, ...}
  left: String("consistent")   right: "inconsistent"
```

**The fix (this integration copy only; the approved host source is not
edited).** The hash comparison fails closed:

* the hash must decode as hex **and** be exactly 32 raw bytes;
* a hash that fails either test increments `malformed_share_hashes` and the
  record is counted as **`inconsistent`** - never `consistent`;
* a **missing** hash stays `unknown` (unchanged);
* the job/generation/credit checks (the `(front, job_id, generation)` join,
  the target, `m_issue` and the booked `d`) are unchanged;
* the summary publishes the new counter `malformed_share_hashes` and the
  scope line "the recorded share hash is exactly 32 raw bytes of hex; a
  malformed hash is inconsistent, never consistent".

**Green evidence** (this copy):

| Test | Covers |
|---|---|
| `malformed_share_hash_is_never_consistent` | a nonhex hash, a 31 byte hash, a 33 byte hash and an empty hash: each is `inconsistent`, `malformed_share_hashes == 1`, `consistent_records == 0` |
| `valid_share_hash_is_compared_against_the_announced_target` | a valid 32 byte hash below the announced target stays `consistent`; a valid 32 byte hash above it is `inconsistent` with `hashes_above_announced_target == 1` and `malformed_share_hashes == 0` |
| `missing_share_hash_stays_unknown` | a record without a share hash stays `unknown`, never a pass |
| the pre-existing `accounted_target_summary_joins_front_job_and_generation`, `self_consistent_record_with_wrong_announced_target_is_inconsistent` and `conflicting_duplicate_or_missing_jobs_never_pass` | retained and still green |

The fix **weakens no claim**: it can only turn a pass into a non-pass. The
`full_header_payout_oracle` stays explicitly `inconclusive`.

---

## 2. The namespace boundary and its trust model

### 2.1 What the broker is, and what it owns

The namespace broker is the **approved LP-4a primitive**
(`scripts/lab/lib/lab_backend.py`, byte identical to the approved snapshot
`bce93151`, verified against the runtime candidate's copy). This packet does
not modify it. It starts it, reads its manifest and drives its control
socket.

The broker owns:

* one network namespace per endpoint, created with `unshare(CLONE_NEWNET)` in
  a forked holder, addressed by the holder PID (`/proc/<pid>/ns/net`) - never
  by a name in `/run/netns`, which is root owned on this host;
* one veth pair per link, with static `/30` addresses and no default route;
* the nftables fault rules (interface scoped, family agnostic, installed as
  one atomic transaction) and the `tc netem` qdiscs;
* the raw counters and the no-bypass proof.

The broker runs inside `unshare -U --map-root-user -r -n`: its own user
namespace owns its own network namespace, so it can create veth pairs and
nftables rules **without any host privilege**, and every namespace it creates
is owned by that user namespace.

### 2.2 The join, and why the controller stays reachable

A process outside the broker's user namespace cannot enter the namespaces the
broker owns. The supervisor therefore joins the broker's **user** namespace
only:

```
nsenter -U --preserve-credentials -t <broker-pid> -- <command>
```

This was measured on this host, not assumed:

| Question | Measurement |
|---|---|
| Can an outside process join the broker's user namespace? | yes: `nsenter -U --preserve-credentials` -> `uid=0(root)` inside, which is the outer uid 1000 |
| Does the joined process keep the host network namespace? | yes: `ip -o link show` still lists the five host interfaces |
| Can the joined process create a veth in the host network namespace? | **no**: `RTNETLINK answers: Operation not permitted` - it has no `CAP_NET_ADMIN` in the initial user namespace |
| Can the joined process join an endpoint namespace? | yes: `nsenter --net=/proc/<holder>/ns/net -- ip -o addr show` lists the endpoint's addresses |
| Can a namespace reach the host? | no: the namespaces have no route to the host and the broker's no-bypass check refuses every host address |

The consequence is the trust model of the packet:

* **The host network is never touched.** The only processes that run in the
  host network namespace are the controller and the broker's own outer
  namespace; every actor runs in an endpoint namespace.
* **The supervisor keeps the host network namespace** so the runtime's
  controller API stays reachable, and gains `CAP_SYS_ADMIN` over the broker's
  namespaces only.
* **The actor launch is a launcher, and the identity is proven after the
  exec.** `nsenter --net=<path> -- <program>` execs the program, so the
  process is the program; the supervisor waits for `/proc/<pid>/exe` to
  become the declared program before it records the executable path and
  hash. The approved `is_wrapper_script` refusal still applies to the final
  program, and the approved ownership verification (PID start-time token,
  executable path, executable hash, run token) still applies to the actor.
* **PostgreSQL cannot run as root inside a user namespace.** Inside the
  broker's user namespace the only mapped uid is 0, so a storage process
  launched there would be uid 0 and PostgreSQL would refuse to start. The
  documented boundary for this packet is therefore: **the storage actors stay
  outside the lab namespaces** and are reached by the front over the private
  Unix socket on the shared filesystem (PostgreSQL already uses one) or over
  the loopback of the storage endpoint. The Redis and PostgreSQL instances of
  two pools remain separate by instance, directory, credentials and database
  name; the namespaces add the network isolation on top. A later packet may
  move the storage into the namespaces with a non-root uid mapping, but that
  mapping is not available on this host (the join of a `--map-user=1000`
  user namespace that also owns a network namespace fails with `EINVAL`).
* **After the broker dies the links are lost, and the packet says so.** The
  backend's control socket stops answering, the `NamespaceBroker` client
  reports `Unavailable`, and the netns backend maps that to
  `LinkError::Unavailable`. Nothing claims adoption: `supports_survive_supervisor`
  is `false` and the manifest publishes `adoption_model: "pdeathsig"`.

### 2.3 Visibility and trust boundaries

| Boundary | Who can see what |
|---|---|
| Host network | the controller and the broker's outer namespace only; no lab interface, address or route appears on the host (verified after every run) |
| Endpoint namespace | only the actors placed in it, plus the broker (which owns the fault rules) |
| Broker control socket | `0600` inside a `0700` run directory; it is the controller RPC, on the filesystem, so it crosses the namespace boundary |
| Placement source | the broker's published manifest only. A generated configuration is **never** parsed to learn an address. |
| Publication | the shareable projection carries the addresses, the links and the planes; no identity bearing field and no secret. The private projection keeps the endpoint to actor mapping and the broker run directory. |

---

## 3. The APIs

### 3.1 `spc_lab::namespace`

```rust
pub struct NamespaceBroker { /* the client */ }
impl NamespaceBroker {
    pub fn start(options: BrokerOptions) -> Result<Self, NamespaceError>;
    pub fn manifest(&self) -> &BrokerManifest;
    pub fn request(&self, request: Value) -> Result<Value, NamespaceError>;
    pub fn apply(&self, link: &str, policy: Value) -> Result<Value, NamespaceError>;
    pub fn heal(&self, link: &str) -> Result<Value, NamespaceError>;
    pub fn counters(&self, link: &str) -> Result<Value, NamespaceError>;
    pub fn no_bypass_check(&self) -> Result<Value, NamespaceError>;
    pub fn exec(&self, endpoint: &str, argv: &[String], timeout_s: f64) -> Result<Value, NamespaceError>;
    pub fn status(&self) -> Result<Value, NamespaceError>;
    pub fn stop(&mut self) -> Result<(), NamespaceError>;
}
pub fn host_global_addresses() -> (Vec<String>, Vec<String>);   // v4, v6
pub fn join_command(nsenter: &Path, broker_pid: u32, argv: &[String]) -> Vec<String>;
pub struct Placement { /* namespaces, addresses, links */ }
```

A response that carries an `error` object is a rejected request; a response
without one is a served request whose `ok` field is the outcome (an `exec`
reports the command's own exit status, and the caller must read it). This
distinction matters: an `exec` of a command that exits 3 is a *served* request
with `command_exit_code: 3`, not a broker failure.

### 3.2 `spc_lab::links::netns`

```rust
pub struct NetnsLinkBackend { /* over the broker */ }
impl NetnsLinkBackend {
    pub fn new(broker: Arc<NamespaceBroker>) -> Self;
    pub fn counters(&self, link: &str) -> Result<Value, LinkError>;
    pub fn no_bypass_check(&self) -> Result<Value, LinkError>;
    pub fn heal(&self, link: &LinkSpec) -> Result<(), LinkError>;
    pub fn placement(&self) -> Placement;
}
impl LinkBackend for NetnsLinkBackend {
    fn name(&self) -> &'static str { "netns" }
    fn supports_sustained_fault(&self) -> bool { true }
    fn preflight(&self, scenario: &Scenario) -> Result<(), LinkError>;
    fn apply(&self, link: &LinkSpec, policy: &LinkPolicy) -> Result<(), LinkError>;
    fn current(&self, link: &LinkId) -> Option<LinkPolicy>;
}
```

`preflight` checks that the broker published every link and both of its
endpoints; a link the broker does not know is `Unavailable`, never a silent
fallback.

### 3.3 `spc_lab::topology`

```rust
pub struct TopologyPlan { /* endpoints, links, actor_endpoints, consensus_link */ }
impl TopologyPlan {
    pub fn from_scenario(scenario: &Scenario) -> Result<Self, String>;
    pub fn broker_spec(&self) -> Value;          // the LP-4a spec schema
    pub fn endpoint_of(&self, actor: &str) -> Option<&str>;
    pub fn links_of_plane(&self, plane: LinkPlane) -> Vec<&PlannedLink>;
}
pub enum LinkPlane { Consensus, PoolNode, MinerFront, Storage }
pub trait CanonicalProbe { fn hash_at(...); fn tip(...); }
pub struct CanonicalObserver { /* per node, depth K */ }
impl CanonicalObserver {
    pub fn new(depth: usize) -> Self;
    pub fn with_runtime_window() -> Self;   // the runtime's bounded window of 8
    pub fn tracked(&self, node: &str) -> &[ObservedHeight];
    pub fn observe(&mut self, nodes: &[String], probe: &mut dyn CanonicalProbe, now_ms: u64)
        -> Vec<CanonicalObservation>;
}
pub struct TopologyPublication { pub shareable: Value, pub private: Value }
impl TopologyPublication {
    pub fn with_provenance(placement: &Placement, plan: &TopologyPlan,
                           broker_run_dir: &str, provenance: Value) -> Self;
}
```

The **planes** are what keeps the controller apart from the injected links:
the controller plane (the runtime's own API) is never a broker link, so
"no accidental bypass" is measurable over the data plane only.

The **canonical observer** is the node scoped answer to the runtime review's
R4 finding, and it now follows the runtime's R2 semantics exactly: the last
**8** observed `(height, hash)` tip observations are kept **per node** and
re-checked on every pass. A re-check that returns a *different* hash is a
**confirmed** displacement and is reported `no` (and the observation leaves
the window); a re-check that cannot be performed stays `unknown` and never
becomes a `no`; every observation publishes `scope: "tip_sampler"`, because
the heights are not continuously observed and this is not full canonical
coverage and not reorg tracking. The observer takes a probe closure, so it
does not depend on the runtime's internals.

### 3.4 The supervisor's namespace launch

```rust
impl ProcessSpec {
    pub fn in_namespace(mut self, netns: impl Into<PathBuf>, nsenter: impl Into<PathBuf>) -> Self;
}
```

With a namespace set, the launch becomes `nsenter --net=<path> -- <program>`
and the supervisor waits for the **exec handshake**: `/proc/<pid>/exe` has to
become the declared program within the readiness budget, otherwise the
launch fails with `ExecHandshakeTimeout` and the launch is killed. The
recorded `executable_path` and `executable_sha256` are therefore the actor's,
never the launcher's.

---

## 4. Test evidence

### 4.1 The namespace test suite (process level, nothing mocked)

`cargo test -p spc-lab --test topology_namespace` -> **10 passed, 0 failed**
(`--test-threads=1`; each test starts its own broker and its own namespaces).

`cargo test -p spc-lab` -> **all green** (the runtime's own suites plus the
16 `spc-lab` lib unit tests, which now include the three F1 regressions).

| Test | What it proves | Recorded result |
|---|---|---|
| `namespace_address_ownership_and_readiness` | the broker publishes every endpoint, its namespace handle is a pid handle, its addresses are in the lab range, and the endpoint can **bind the published address inside its own namespace** | the endpoint printed the published address it bound |
| `exec_stdio_lifecycle` | the controller RPC runs a command inside the namespace and returns its output; a failing command reports its own exit code instead of a false success | `command_exit_code: 0` and `command_exit_code: 3` |
| `fault_command_idempotence` | reapplying the same fault leaves exactly one lab table and does not accumulate drop counters; the applied policy is tracked and the heal returns to `normal` | one table, counters not accumulated |
| `no_direct_ip_bypass_including_ipv6` | the no-bypass proof finds no violation, and it covers the **IPv6** non-link addresses and the host addresses | `ok: true`, 0 violations, IPv6 probes present |
| `established_connection_is_cut_and_reconnects_after_heal` | a TCP session established before the fault is **cut** by it (`phase2: FAIL:TimeoutError`), and a new session works after the heal | cut and reconnect |
| `unrelated_link_and_control_plane_stay_reachable` | the fault is link scoped: an unrelated link still carries traffic, and the controller socket still answers while the fault is active | `PONG` on the unrelated link, `ping` answered |
| `canonical_observer_detects_a_displaced_height` | the node scoped observer reports a reorganised height as `no` and the new tip as `yes` | `no` for the displaced height |
| `topology_plan_separates_the_planes` | the miner is linked to its front and to nothing else; the consensus, pool-node, miner-front and storage planes are distinct | one miner link, to the front |
| `host_address_collection_returns_addresses_not_interface_names` | the host addresses the broker has to prove unreachable are addresses, not interface names | v4 and v6 separated |
| `fixture_backend_cannot_sustain_a_partition` | **the negative control**: the same observation that the netns backend satisfies is made against the *fixture* backend, and the fixture backend leaves the link open - which is exactly why it cannot satisfy a partition assertion | the session survived the "fault" |
| `malformed_share_hash_is_never_consistent` (runtime lib) | **F1**: a nonhex, a 31 byte, a 33 byte and an empty share hash are each `inconsistent`, never `consistent` | `malformed_share_hashes: 1`, `consistent_records: 0` |
| `valid_share_hash_is_compared_against_the_announced_target` (runtime lib) | **F1**: a valid 32 byte hash is still compared with the announced target | below -> `consistent`, above -> `hashes_above_announced_target: 1` |
| `missing_share_hash_stays_unknown` (runtime lib) | **F1**: a missing hash stays `unknown` | `status: unknown`, `consistent_records: 0` |

The negative control is the fail-first evidence of the suite: the partition
assertion is not satisfiable by a backend that installs nothing, so a passing
partition proof means a real kernel level fault.

### 4.2 The real two node run (two rbitcoin nodes, a real partition and heal)

`testing/lab-topology/run-two-nodes.sh` -> artifact
`/tmp/opencode/LP-topology-f1-final/two-nodes.json`, verdict `passed`.

* node binary: `/tmp/opencode/rbitcoin-lab-retarget-profile/target/debug/rbitcoin-node`
  sha256 `eb257af7c10cdf0ded584adfadf2ae8124466304b13d26c17f855e76f9320f7c`
* the broker published `node-a 10.250.0.1` and `node-b 10.250.0.2` on the
  `p2p-ab` link, each in its own namespace, with no default route.
* both nodes ran inside their namespace, launched through `nsenter` with the
  exec handshake, and answered their own namespace local RPC.

| Phase | node-a | node-b | link counters |
|---|---|---|---|
| healthy (node-a mined 3 blocks) | height 3, `04ed9b41...c14` | height 3, `04ed9b41...c14` | - |
| partitioned (node-a mined 3 more) | height 6, `09df93d0...856` | **height 3, `04ed9b41...c14`** | `nft_dropped_packets: 8, nft_dropped_bytes: 3731` |
| healed | height 6, `09df93d0...856` | **height 6, `09df93d0...856`** | - |

* The split and the rejoin are **per node canonical views**, not a tip
  sample: node-b kept the old canonical hash while the link was down and
  adopted node-a's hash after the heal.
* The raw link counters show the dropped blocks (`nft_dropped_packets: 8`,
  `nft_dropped_bytes: 3731`).
* The no-bypass proof held **while the fault was active**:
  `ok: true, violations: 0`.
* The verdict is `passed` for `split_observed` and `rejoin_observed`.

The artifact is labelled `status: "provisional"`: the runtime candidate whose
node contract this borrows is **not** approved (its own review is open with
R1-R4), and this run makes **no** pool, miner, payout, hash rate or consensus
claim. It proves the network primitives and the node scoped observation, not
the LP-4 scenario.

### 4.3 Host integrity

After every run in this packet:

```
ip_forward = 1            (unchanged; the namespaces carry 0/0)
/run/netns  = empty       (the broker never writes there)
lab addresses on the host = 0
```

The host's three global interfaces and their addresses are unchanged.

---

## 5. What is **not** claimed

* **No approval is inherited.** The runtime candidate is under review; every
  artifact here is provisional. The LP-4a precursor it borrows is approved
  at `bce93151`, and this packet does not change it.
* **No two pool, two miner, native CPU mining or accounted work claim.** The
  runtime wiring that places the pool, the front, the miner and the storage
  into the namespaces is **not** in this packet (section 8 explains why and
  what the exact hooks are). The scenario
  `crates/spc-lab/scenarios/lab-topology-two-nodes.json` is the input for
  that wiring and it validates against the current vocabulary.
* **No retarget, reorg or convergence claim.** The partition run shows a
  *split and a rejoin* of two regtest nodes. It does not show a reorg, and it
  does not show that the nodes would converge in a longer run; the heights
  and hashes are the evidence, nothing more.
* **No accounting claim.** The oracle, the `accounted_target_consistency`
  join and the per front work assertions belong to the runtime's R1/R2 fix,
  which is in flight in the runtime owner's worktree.
* **No filesystem or security sandbox claim.** The packet proves network
  isolation inside the namespaces it owns. It does not claim a capability drop
  for arbitrary code, and it does not claim that the storage boundary is a
  sandbox: PostgreSQL runs outside the namespaces, as section 2.2 documents.
* **The Nix gate is open.** Section 7.

---

## 6. Bounded remaining gap

| Gap | Why it is open | What unblocks it |
|---|---|---|
| The runtime does not place its actors in the namespaces yet | the runtime owner is reworking `runtime.rs`/`engine.rs` (R1-R4 plus the non-blocking notes). This packet's modules are isolated so they can be merged after that freeze. | the merge hooks in section 8, applied to the runtime's final freeze |
| The two pool / miner wiring is not exercised in the namespaces | it depends on the runtime wiring above | the same |
| The storage boundary is a Unix socket plus the loopback of a storage endpoint, not a network link between namespaces | PostgreSQL refuses uid 0 inside the broker's user namespace, and this host cannot map a non-root uid into a user namespace that also owns a network namespace (the join fails with `EINVAL`) | a userns with a non-root uid mapping, or a storage endpoint per pool reached over its own link with a non-root storage process |
| The canonical observer is not yet wired into the runtime's evidence collector | it needs the runtime's node RPC client, which is being reworked | the `CanonicalProbe` implementation the runtime supplies (section 8) |
| The `accounted_target_consistency` join and the per front assertions are not in this packet | they belong to the runtime's R1/R2 fix | the runtime owner's revision |
| The provenance of a topology run is recorded by hand (the node binary and its sha256) | the runtime's R3 fix will bind a run to the frozen source | the runtime owner's revision |

---

## 7. The Nix gate (open, honestly)

The Nix tool realisation is **still blocked on this host**, and this packet
does not claim otherwise.

* The project's **locked** input resolves offline:
  `nix eval --offline --inputs-from /home/ethan/code/silent-pool --raw nixpkgs#lib.version`
  -> `26.11.20261101.0bb2eedr`.
* Realising the tools from it fails: the store holds only *derivations* for
  `iproute2` and `nftables`, so every build has to realise the stdenv
  bootstrap (bash 5.3 and its patches, bash-2.05b, acl, attr, autoconf,
  automake) from source, and this host cannot reach the source mirrors:
  `unable to download 'https://download.savannah.gnu.org/releases/acl/acl-2.4.0.tar.gz':
  Timeout was reached (28) Failed to connect to server`.
* All results in this packet are therefore **host tool** results: util-linux
  2.37.2 (`unshare`, `nsenter`), iproute2 5.15.0 (`ip`, `tc`, `ss`),
  nftables 1.0.2 (`nft`), jq 1.6, python3 3.10.12.
* **Merge note (not applied here):** `flake.nix`'s `devShells.default.packages`
  must gain `pkgs.iproute2`, `pkgs.nftables`, `pkgs.util-linux` and
  `pkgs.python3` (the broker and the fixtures are Python). `jq` is already
  there. The devShell must **not** make these mandatory for the core packages:
  the core `nix build` must keep working without a realisable network tool
  set, so the network tools belong in the devShell (and in an optional
  `checks.lab-topology` that is opt-in), not in `packages.default`.

---

## 8. Merge notes for the runtime owner

These are the hooks the runtime needs. They are deliberately small and
isolated, and none of them depends on a line position in `runtime.rs`.

1. **Start the broker and join its user namespace.**
   `spc_lab::namespace::NamespaceBroker::start(BrokerOptions)` starts the
   broker; `spc_lab::namespace::join_command` builds the
   `nsenter -U --preserve-credentials -t <broker-pid> -- ...` command. The
   runtime's CLI must re-exec itself under that command once, before the
   engine starts, and pass the broker's pid and run directory on. The
   controller's HTTP listener is bound **before** the join so it stays on the
   host network and remains reachable.
2. **Plan the topology.** `TopologyPlan::from_scenario(&scenario)` gives the
   endpoints, the links and the actor to endpoint mapping;
   `plan.broker_spec()` is the document the broker consumes.
3. **Place the actors.** For every actor, `ProcessSpec::in_namespace(netns,
   nsenter)` with the netns path the broker published for its endpoint. The
   supervisor's exec handshake makes the identity proof exact. The node, the
   front, the miner and the storage are placed per the plan's planes; the
   **storage stays outside the namespaces** unless the runtime supplies a non
   root uid mapping (section 2.2).
4. **Render the configs with the published addresses.** The node's
   `--rpc-listen` and `--listen`, the pool's `rpc_url`, the front's `listen`
   and the miner's front address must come from `Placement`, not from the
   scenario's `127.0.0.1:0` placeholders. The pool's `redis` and `postgres`
   URLs must name the storage endpoint's address or its private socket.
5. **Observe through the broker.** The front's API commands and the node's
   RPC are namespace local, so they have to be issued through
   `NamespaceBroker::exec` (or through a helper the runtime runs inside the
   namespace). The runtime's `RpcNodeControl` and its HTTP client need a
   namespace scoped variant.
6. **Drive the faults through the netns backend.**
   `NetnsLinkBackend::new(Arc<NamespaceBroker>)` implements the existing
   `LinkBackend` trait, so `config.link_backend = "netns"` selects it. The
   runtime's `backend_for` in `links/mod.rs` has to return it for the name
   `netns` (the fixture backend stays the default and stays honest: it
   cannot sustain a fault).
7. **Node scoped canonical observation.** Implement `CanonicalProbe` for the
   runtime's node RPC client (`hash_at` -> `getblockhash {"height": n}`,
   `tip` -> `getblockcount` + `getbestblockhash`) and feed the
   `CanonicalObserver` with it. The observer emits `yes`/`no`/`unknown` per
   observed height, so a displaced height is visible - the R4 gap the runtime
   review named.
8. **Publish the placement.** `TopologyPublication::new(&placement, &plan,
   broker_run_dir)` gives the shareable projection (addresses, links, planes)
   and the private projection (the endpoint to actor mapping and the broker
   run directory). The shareable manifest is where the addresses belong; no
   identity bearing field and no secret may enter it.
9. **Keep the runtime's crash semantics.** The broker dies with the run
   (`PR_SET_PDEATHSIG` on the namespace holders and the spawned fixtures).
   After the broker is gone the control socket stops answering and the
   runtime must mark the link resources **lost** and fail closed. There is no
   adoption of a dead broker's namespaces in this packet, and the manifest
   says so (`adoption_model: "pdeathsig"`,
   `supports_survive_supervisor: false`).
10. **Do not weaken the runtime's own fixes.** The pool delivery, per job
    and timeout fixes (the approved six file delta at `c9dfa986`), the
    identity tamper validation, the capability self assertion, the snapshot
    mutex during the miner requests and the observer stop/scanner gap and SQL
    identifier fixes are in flight in the runtime owner's worktree. This
    packet does not touch them and the merge must not drop them.

---

## 9. Reproduction

```sh
# the namespace test suite (real broker, real namespaces, real nftables)
cd /home/ethan/.local/share/opencode/worktree/ad8886/lab-topology
CARGO_BUILD_JOBS=2 cargo test -p spc-lab --test topology_namespace -- --test-threads=1

# the two real node run with the partition and the heal
./testing/lab-topology/run-two-nodes.sh --artifacts /tmp/opencode/LP-topology-nodes

# the scenario for the (not yet merged) runtime wiring
/tmp/opencode/LP-topology-target/debug/spc-lab validate \
    --scenario crates/spc-lab/scenarios/lab-topology-two-nodes.json
```

Artifacts of the last run: `/tmp/opencode/LP-topology-f1-final/two-nodes.json`.

---

## 10. Provenance and the snapshot

* The topology tree was bootstrapped from the runtime candidate snapshot
  `c9dfa986c2f6ad0daca44912906a0a2ec94129c25d465eee246f19a43ccd543d` with a
  patch that carries the file modes; all 172 files were verified against the
  candidate manifest after the patch, and the baseline is recorded separately
  in `/tmp/opencode/lab-topology-baseline.json` (file recipe
  `e8d9b50961709087e49a944ea7db88737ff947af002971eea0993da1d11099b5`).
* It was then reconciled onto the runtime's second frozen candidate
  `e891f34c69c70c2a6748be1ca2a77a71aba4a63377eda55a43afeb8613b68043`
  (section 1b): 9 changed and 2 added runtime files taken, **0 conflicts**,
  and no file this packet owns overwritten.
* That candidate is now the **approved** host baseline (the second Fable pass
  validated the Nix provenance, the 810 consistent accounted records and the
  per front assertions, and closed R1-R4). The only file this packet changed
  on top of it is `crates/spc-lab/src/runtime.rs`, and only for the required
  merge fix F1 (section 1c). The delta against the approved host is therefore
  exactly:

  | File | Change against the approved host `e891f34c` |
  |---|---|
  | `crates/spc-lab/src/runtime.rs` | the F1 fail-closed hash comparison, the `malformed_share_hashes` counter, the scope line and the three F1 regressions |

  The complete difference between this tree and the approved host is exactly
  four host files plus this packet's own new files, and it is verifiable
  from the manifest (the host files whose hash differs from the approved
  host's are listed here):

  | Host file | Why it differs |
  |---|---|
  | `crates/spc-lab/src/runtime.rs` | the required merge fix F1 (section 1c) |
  | `crates/spc-lab/src/supervisor.rs` | the namespace launch and the exec handshake (this packet's own work) |
  | `crates/spc-lab/src/lib.rs` | the new modules are declared |
  | `crates/spc-lab/src/links.rs` | moved to `crates/spc-lab/src/links/mod.rs` so the netns backend can live in `links/` (content otherwise unchanged) |

  Every other file of the approved host is byte identical in this tree.
* The candidate is **not** approved; its own review is open (R1-R4 plus the
  non-blocking notes). This packet inherits nothing from it.
* The LP-4a broker files used here are byte identical to the approved
  snapshot `bce93151` (verified file by file against the runtime candidate's
  copy, which the candidate had taken from the approved tree).
* The node binary used in the two node run is the one in the L2 worktree,
  with its sha256 recorded in the artifact.
