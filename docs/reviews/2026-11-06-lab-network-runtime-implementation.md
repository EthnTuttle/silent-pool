# LP-network-runtime implementation report

Date: 2026-11-06
Author: TFF implementation agent
Status: **CANDIDATE COMPLETE, awaiting exact-snapshot Fable review**
Worktree: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-network-runtime`
Base: `b0ab4ca34fda07c61a6335d05f598eaacc69678c`

The runtime wiring, bounded host-tool namespace tests, Nix builds/checks, real
native-SV2 partition/heal run and controller-SIGKILL cleanup proof are complete.
No Fable approval is claimed for this combined candidate. The older host runtime
is approved; the unfinished topology helper review is not being relabelled as an
approval. No commits, pushes, resets, stashes, cache cleans or edits to approved
worktrees were made. No v0.9.0 validation agent was spawned and the node was not
upgraded.

## 1. Frozen inputs and implementation architecture

### Inputs

- Approved host runtime: full snapshot
  `e891f34c69c70c2a6748be1ca2a77a71aba4a63377eda55a43afeb8613b68043`,
  tested code `d75ab391b0ca1c8ab5b03af27dd5cf45a8a384ef8ac056a1ceeb2b2f456d8684`.
  Bootstrap manifest `/tmp/opencode/lab-network-runtime-baseline.json`, recipe
  `90a4c78474a4c4983c0b6770baaa56ad42f0f641f7c05687473f374c507d3034`.
- Approved LP-4a broker: `bce93151...`; this tree's
  `scripts/lab/lib/lab_backend.py` remains byte-identical at SHA-256
  `069a87e497af0630916a995fb4bf083f745191de9c21b2dc4bfcd8c2b2107359`.
- External L2 node: source snapshot `d2f37b91...`, recorded binary SHA-256
  `eb257af7c10cdf0ded584adfadf2ae8124466304b13d26c17f855e76f9320f7c`.
  It remains an explicit non-Nix exception. No pin or release was changed.
- Approved docs delta `1d40b2b7` was hash-verified and only its ten owned files
  were applied in the recorded order. The before/input/applied hash record is
  `/tmp/opencode/LP-network-kimi-docs-input-hashes.json`. The actual namespace
  operator scope was then added to `README.md`, `docs/LAB-RUNTIME.md`,
  `docs/OPERATIONS.md` and the runtime appendix.
- F1 is integrated here: malformed/non-32-byte/non-hex share hashes increment
  `malformed_share_hashes` and make the record inconsistent; a missing hash is
  unknown. The older approved host's version-bound limitation is documented,
  not hidden.

### Controller and actor ownership

The controller stays in the host user and network namespaces for its entire
lifetime. One in-memory engine/resource plan, run lock, listener and supervisor
owns the run. It never re-execs and does not adopt controller FDs, trust an
environment `lock held` flag or unlock/relock the controller.

Topology placement and published addresses exist before configuration rendering.
Node/front/miner/Redis actors enter through:

```text
nsenter --user=<broker-userns> --net=<endpoint-netns> \
  --preserve-credentials -- <actual-binary> [args...]
```

`EngineOptions` and `ProcessSpec` carry both namespace paths and both owner
identities to launch. The exec handshake canonicalizes the expected program,
checks launcher exit with `try_wait`, and kills *and waits* an owned failed or
timed-out launch. Listener readiness runs in the same user/net namespace and
verifies the listener's socket inode against the real child's FD table.

Every real actor, including host PostgreSQL and the broker where configured by
this runtime, goes through the Rust `actor-exec` helper. The helper calls Rustix
`set_parent_process_death_signal(Some(SIGKILL))`, verifies the expected parent
PID *and start-time token* after setting the signal, then execs the real actor.
There is no unsafe Rust `pre_exec`. Ordinary miner JSONL stdin/stdout stays on
the supervisor's owned pipes. Run-token/executable/start-time checks remain in
place; `reap` never signals by bare PID.

Actors can mutate only their broker-owned namespaces. There is no host
`sudo`/`modprobe`/`sysctl`, arbitrary host route or arbitrary host nft mutation.

### Storage and control plane

- PostgreSQL remains non-root in the host user/network namespace and is started
  exactly once by the engine after resource planning. It does not appear as an
  actor in a Redis storage endpoint. A front uses the run-private Unix socket
  with the exact allocated port, role, database and password. The SQLx URL has
  a nonempty `localhost:port` authority for parsing and an explicit
  `host=<socket-dir>` query; this is not cross-namespace TCP loopback.
- Each pool's Redis runs in its own `storage-<pool>` namespace and binds the
  broker-published storage veth address. Its front URL is
  `redis://:<password>@<published-address>:<allocated-port>/0`. There is no
  cross-namespace `127.0.0.1`. Namespace-collector reads use the same
  credentials/address and check the protocol answer.
- Node startup/observer/scanner RPC and front API reads use bounded broker
  `exec` helpers in the correct endpoint. Host HTTP/SSE control and durable
  commands remain host-local. Miner control remains the approved owned pipe,
  not an invented RPC.
- Consensus faults do not partition storage. No storage-IP-partition or Ark
  privacy-boundary claim is made.

The operator guide, including the node contract required by a separate future
v0.9.0+lab-patch validation wave, is `docs/LAB-RUNTIME.md` sections 10-12.

## 2. T1-T9 and F1 disposition

| Finding | Implemented/check |
|---|---|
| T1 links | Scenario IDs are preserved verbatim. Declared miner/front and pool/node links satisfy the planned data route; a duplicate generated route is removed. Consensus links are a `Vec`. Invalid/unplaced IDs and generated-ID collisions fail closed. |
| T2 broker identity | Fresh broker directories are required. Tokens are eight hex chars; manifest token and live `ping` must match. Up-error payloads are propagated and `Drop` performs down/reap. The runtime writes its topology outside the broker's empty private run directory. |
| T3 state/errors | Partial/busy/unsupported/unavailable are distinct. Failed apply drops the cache; `current()` asks the broker, so a dead backend is unknown, not normal. |
| T4 exec/identity | Expected executable canonicalization, early status reporting, owner token checks, failed-launch reaping and the listener parser regression are implemented. The new combined host-controller user+net test checks final user and net namespace inodes. |
| T5 tips | Runtime and helper probes accept only `getbestblockhash` + `getblockheader(hash)` + matching `getblockhash(height)`. A mismatch is `unknown`, never a displacement. Last-K displacement reports remain bounded tip sampling, not full reorg coverage. |
| T6 default green | Process-level namespace proofs are `#[ignore]` and require `SPC_LAB_NETNS_TESTS=1`; storage negatives also require explicit Redis binary paths. Default topology: 5 pass/13 ignored. The full default Nix check is green. Opt-in results are explicitly host-tool, not Nix-network-tool proofs. |
| T7 tools/bounds | Tools resolve from options/`PATH`. Failed address enumeration is fatal rather than a vacuous pass. Broker responses are capped at 8 MiB, broker exec has deadlines, and empty/failing answers do not count as RPC/storage success. |
| T8 storage | Redis is per-pool storage-NS TCP with the published authenticated address; PostgreSQL is host-local over private Unix sockets. Both were used by the real fronts, not just a helper test. |
| T9 accuracy | The host runtime is APPROVED. The old node-only artifact used RPC `generate` and shell `nsenter`, not the Rust handshake; its nft counters count packets, not blocks. This candidate instead uses runtime-journaled native SV2 CPU mining; no `generate*`, `setmocktime` or fake credit appears in measured mining. |

## 3. Checks actually completed

| Check | Actual result |
|---|---|
| Cargo checks | `cargo check -p spc-lab -p spc-lab-cli` passed after simplification and after the final runtime wiring (`check-001`, `003`, `004`, `005`, `006` logs). |
| Formatting/clippy | `cargo fmt --all -- --check` clean. `cargo clippy --locked -p spc-lab -p spc-lab-cli --all-targets -- -D warnings` passed (`clippy-002`; final control iteration also `clippy-003`). |
| Default topology | 5 passed, 13 ignored. Pure storage URL/P2P/storage-placement/unit regressions are included. |
| F1/runtime units | 11 passed (including both malformed hash classes, missing hash, P2P peer mapping, Redis URL/auth/database and PostgreSQL socket/port checks). |
| Opt-in host namespace suite | **13 passed, 0 failed** in the final run (`LP-network-kimi-test-topology-netns-final.log`), including combined user+net identity, stale broker/token/exec errors, reconnect/heal/no-bypass and negative storage auth/loopback/port controls. One earlier negative correctly exposed redis-cli's exit-0-on-NOAUTH behavior; the check now inspects the protocol response. |
| Documentation | `scripts/check-lab-docs.py`: OK, 6 files/ASCII/links/vectors; the three main-only merge inputs are explicitly expected. The namespace link/punctuation check passed. |
| Frozen Nix binaries | All four packages built from `/tmp/opencode/LP-network-kimi-frozen-src-v2` (`nix-builds-v2.log`). |
| Default Nix check | `nix flake check --offline` on frozen source v2: **all six checks green**: fmt, clippy, doc, workspace test, UI node and test-ark (`nix-flake-check-v2.log`). Network proofs remain separate opt-in host proofs; they were not silently counted as default Nix coverage. |

The build/check commands used `NIX_BUILD_CORES=2 CARGO_BUILD_JOBS=2` and
`--max-jobs 1`; targeted Cargo checks used `CARGO_BUILD_JOBS=2` and unique logs
under `/tmp/opencode/LP-network-kimi-*`.

## 4. Genuine native SV2 partition/heal proof

### Run and provenance

Run directory: `/tmp/opencode/LP-network-kimi-final-live-v2`
Console log: `/tmp/opencode/LP-network-kimi-final-live-v2.log`
Independent check/output summary: `/tmp/opencode/LP-network-kimi-final-live-v2-evidence.json`

The run used the frozen-v2 Nix binaries below and the unchanged external L2
node. Working directory: `/tmp/opencode/LP-network-kimi-frozen-src-v2`.

```sh
LAB=/nix/store/ac546m1g3z90fsp67vv3mrp4q31wrv4z-spc-lab-0.1.0/bin/spc-lab
POOL=/nix/store/39rsjr4drdfw1y1xwcdzcpzpiy3y5x9v-spc-pool-0.1.0/bin/spc-pool
MINER=/nix/store/kfiw6pylnrkk14vmbvk677vx58932wpp-spc-cpu-miner-0.1.0/bin/spc-cpu-miner
SCAN=/nix/store/0fdnzpnlcwkkl1mpgmjl01qyvmmc339q-spc-scan-0.1.0/bin/spc-scan

"$LAB" runtime \
  --scenario crates/spc-lab/scenarios/lab-network-runtime-two-pools.json \
  --run-dir /tmp/opencode/LP-network-kimi-final-live-v2 \
  --node /tmp/opencode/rbitcoin-lab-retarget-profile/target/debug/rbitcoin-node \
  --pool "$POOL" --miner "$MINER" --scanner "$SCAN" \
  --postgres-bin-dir /nix/store/vync24csmxy5saql5ah77vkn59yg2x5h-postgresql-17.11/bin \
  --redis-server /nix/store/nql1lir6650xigvf43n11zz4hh0lbw9z-redis-8.10.3/bin/redis-server \
  --redis-cli /nix/store/nql1lir6650xigvf43n11zz4hh0lbw9z-redis-8.10.3/bin/redis-cli \
  --pow-bits 0x207ffffe \
  --base-revision b0ab4ca34fda07c61a6335d05f598eaacc69678c \
  --dirty-tree-hash 2d6c752d24d4f4cc9ea30f6d5ced8b507dfd860acb8721ded85c758992705098 \
  --topology --broker-script scripts/lab/lib/lab_backend.py \
  --listen 127.0.0.1:0 --duration-secs 3
```

The HTTP endpoint printed during this run was `http://127.0.0.1:37525`. The
scenario executes baseline work, pauses, P2P partition, genuine independent
work on both independent pools, heal, resumed real mining to break any tied
fork, pause and all four assertions. The full no-bypass call runs with workers
paused; its IPv6 negative probes are many one-second checks, not fabricated
latency evidence.

### Accounting/rates

Both front assertions passed (`assert-front-a1-accounted` and
`assert-front-b1-accounted`), along with `assert-jobs` and
`assert-pool-evidence`. All four final assertions report `passed`.

| Scope | Value |
|---|---|
| `front-a1` | 39 submitted/block-qualified, 37 acknowledged/credited, 2 rejected/stale |
| `front-b1` | 40 submitted/block-qualified, 37 acknowledged/credited, 3 rejected/stale |
| Transport unknown | no records observed (`0`) in `front_evidence`; this is not a claim that unknown outcomes are impossible |
| `miner-a` | requested 20 H/s, measured 19 H/s, 77 attempted, 39 stale-cancelled |
| `miner-b` | requested 20 H/s, measured 19 H/s, 79 attempted, 40 stale-cancelled |
| Scoped consistency | `consistent`, 74/74 records checked against their front/job/generation target and credit; 0 unknown/inconsistent/malformed |

Block-qualified is not equated with canonical inclusion. The scoped check does
not recompute full headers, independently derive family targets, verify DB
credit sums or prove payout/settlement.

### Real P2P fault, no bypass and canonical anchors

The before-heal fork was:

- node-a: height 12, `11eb68925e7884aac64fde80340cff4e3d9ab705f0d9ea32e8d78d9cf57ade9b`;
- node-b: height 9, `3707a1b2bba259b93f54d00fd969ad9bb708fda2590dbdab7741566fa87d1b76`.

The final real, paused-view convergence is both nodes at height 45, hash
`40c17738494e099ff8e1d214194a49f2ca0b2237f4cea08955e4b9e6ecdc8a09`.
Before publishing this anchor, the runtime pauses workers through the durable
command journal and re-reads both atomic views; a momentary tie is not called
convergence. Resume/pause controls are also journaled if an equal-height fork
needs additional real work.

The raw `link-p2p-nodes` summary had **45 dropped packets** before heal and
returned to normal afterward. These are packets, not “45 rejected blocks.”
`no_bypass.ok = true`, 276 checks, 0 violations. Every phase observation had
host controller health and both front APIs reachable. Both Redis instances
answered `PONG`; both authenticated Postgres queries succeeded with final
`slice|slice_credit|sp_identity` counts `37|37|1`. Requested/measured and
per-front outcome/rate values derive from `runtime_metrics`, not ACK assertions.

The independent verification additionally checked each node's distinct
listen/peer addresses, private Redis/PostgreSQL URLs, real Nix binary hashes,
absence of `generate*`/`setmocktime` in node API logs and absence of surviving
process identities. Eight process records are `stopped`, two are `exited`;
there are no actor or namespace-holder survivors. `/run/netns` is empty.

### Earlier diagnostic failures retained, not hidden

Run 008 exposed self-connect P2P arguments (peer address and peer port mapping
was wrong; this was fixed and regressed with a unit test). Run 009 did not heal
within 90 seconds: 969 vs 913 with logs showing the baseline node's low-work
compact/stale-fork behavior. Run 010 tied at equal height/different hash with
paused workers. No claim is made that rbitcoin v0.9.0 fixes those issues.
Actual resumed mining and paused revalidation, rather than a fabricated status
or an upgrade, is what the final proof checks. The final candidate retains the
old node; any release evaluation belongs to the separate parent-managed wave.

## 5. Controller-SIGKILL lifetime proof

Driver: `/tmp/opencode/LP-network-kimi-crash-proof.py`
Evidence: `/tmp/opencode/LP-network-kimi-final-crash-v1-evidence.json`
Run log: `/tmp/opencode/LP-network-kimi-final-crash-v1.log`

The driver started all ten actors with frozen-v2 Nix binaries, captured the
executable/start-time identity of every actor and eight namespace holders,
rechecked the direct controller `Popen` identity, and issued a real `kill()`
(Signal 9). Controller exit was `-9`. Within the bounded 25-second check there
were:

- zero surviving actors;
- zero surviving namespace holders;
- zero remaining `/proc/*/cmdline` matches for this crash run directory.

A subsequent `spc-lab reap --run-dir ...` successfully acquired the released
old run lock and reported all ten records exited. This proves actual
parent-death cleanup and lock release; `Drop`'s graceful behavior and reap's
fallback are not being passed off as proof of automatic crash cleanup.

The namespaced negative test also verifies the bounded combined launch: the
final actor's user and network namespace inode values match the broker user
namespace and endpoint network namespace, and stop/reap leaves no actor.

## 6. Frozen source, Nix outputs and hashes

Frozen source: `/tmp/opencode/LP-network-kimi-frozen-src-v2` (364 regular source
files; `.git`, targets, result links, caches, temp and generated regtest data
excluded; symlink inputs rejected by the source-manifest builder).

- Frozen source aggregate: `3ac3925274002fdc078a9557d4f860aaba697dc487836c4b6eb5abd8d7aafb3a`
- Code-only snapshot excluding this mutable report:
  `2d6c752d24d4f4cc9ea30f6d5ced8b507dfd860acb8721ded85c758992705098`

| Binary | Derivations/output | SHA-256 |
|---|---|---|
| `spc-lab` | `/nix/store/vl2rxvk8pkrydbk2bsk6vsz7hbxdzi7w-spc-lab-0.1.0.drv`, `/nix/store/ac546m1g3z90fsp67vv3mrp4q31wrv4z-spc-lab-0.1.0` | `6a3b61f51a8f60a89ff2a70078dd3109fad77a6a55d7caa9078f7e3712bf95d2` |
| `spc-pool` | `/nix/store/9yq9gnv1nywg4calg3zc4lpsyiqv0ckk-spc-pool-0.1.0.drv`, `/nix/store/39rsjr4drdfw1y1xwcdzcpzpiy3y5x9v-spc-pool-0.1.0` | `aac1278f870fbdd504f9c5846b5d38ebf52335853dcbc471932afb8b9c6c481d` |
| `spc-cpu-miner` | `/nix/store/ihis8gj6xxcyga9xf6sp1jz7jhkh8b8z-spc-cpu-miner-0.1.0.drv`, `/nix/store/kfiw6pylnrkk14vmbvk677vx58932wpp-spc-cpu-miner-0.1.0` | `443de4380b50b48626556bfff3596e621723d54eea82f96d656af39959ffa472` |
| `spc-scan` | `/nix/store/cn9r1plymrdlhdklmhikyxfhw0k2j9n7-spc-scan-0.1.0.drv`, `/nix/store/0fdnzpnlcwkkl1mpgmjl01qyvmmc339q-spc-scan-0.1.0` | `f8000b43b3c7f12030be4d2eebb6fb5073cf0ae4b86830f595f24cad6bf14417` |

The run manifest's pool/miner/node/PostgreSQL/Redis paths and hashes were
matched to the corresponding actual binaries. `/tmp/opencode/LP-network-kimi-nix-binary-provenance.json`
is the compact machine-readable record; `/tmp/opencode/LP-network-kimi-nix-builds.json`
keeps Nix's drv/output records.

Host network/storage tools were used explicitly, not relabelled as Nix tools:
util-linux 2.37.2, iproute2 5.15.0, nftables 1.0.2, Python 3.10.12,
Redis 8.10.3 and PostgreSQL 17.11. The pinned Nix network-tool realization
remains externally blocked by unavailable mirrors/bootstrap downloads. This is
not silently skipped when claiming a network gate: both host-tool proofs and
the separate default Nix checks are named above.

## 7. Source-only manifest and review scope

The final machine-readable source manifest is
`/tmp/opencode/LP-network-runtime-manifest.json`, generated by
`/tmp/opencode/LP-network-kimi-build-manifest.py`. It contains every annotated
source path, SHA-256 and Git mode, the base/inputs, the build-source full
aggregate, the code-only hash and a separate mutable-report hash. Its recipe is
compact sorted-key JSON equivalent to:

```text
sha256(json.dumps({'base': base, 'files': files}, sort_keys=True,
                  separators=(',', ':')))
```

`code_snapshot_sha256` covers every source file except this report. The full
aggregate in the final manifest covers that same code plus the report; no
`.git`, cache, target or runtime artifact is admitted. The separate `files`
array makes the code-versus-report split independently reproducible.

The final candidate is scoped to the documented two-node/two-pool/two-native-
SV2-miner regtest, its exact source and recorded binaries. It does not show
causal hash-budget response, a full header/window/payout oracle, fee/witness or
TDP/ladder coverage, stock-release compatibility, SV1 interoperability, Ark
settlement, or general SV2 interoperability. Production `DESIGN.md`
derivation/tiers/Ark delivery and the SPc/Ark privacy boundary are unchanged.
No normative share-policy amendment was introduced.

## 8. Validation handoff and cleanup

Use `docs/LAB-RUNTIME.md` sections 4 and 11 plus the exact command above for
reproduction. That contract includes the L2 regtest profile flags/effective
`pow_limit_bits`, the authenticated RPC methods used by runtime evidence and
published-value listen/peer-port handling. Stock v0.9.0 lacks the current L2
`--regtest-pow-bits` and `getblockchaininfo.consensus` surface and must not be
hot-swapped. Validators must have their own impact assessment and exact binary
binding; no claim about v0.9.0 behavior is inherited from this run.

Normal run cleanup is already verified above. The crash directory and failed
predecessor diagnostics are preserved for audit; they are no longer live.
Nothing in the queued v0.9.0 wave needs this packet to spawn agents. Parent
review can hand Fable the final source manifest plus the run/crash evidence as
one combined snapshot.
