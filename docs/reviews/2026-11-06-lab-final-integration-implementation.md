# Lab final integration implementation report

Date: 2026-11-06
Author: TFF implementation agent
Status: **CANDIDATE COMPLETE - focused C1/C2 re-review required; no approval claimed**
Worktree: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-final-integration`
Base: `b0ab4ca34fda07c61a6335d05f598eaacc69678c`

This is a new source candidate, not a relabelling of the approved namespace
runtime `553cd4e8` / code `2d6c752d`. The approved tree and its evidence are
unchanged inputs. No commits, pushes, resets, stashes, cleans, subagents,
main-tree edits or approved-worktree edits were made. The v0.9.0 validation was
not duplicated or awaited, and the external L2 node was not upgraded.

## 1. Bootstrap and bounded scope

Bootstrap was performed from `/tmp/opencode/LP-network-runtime-manifest.json`
(Fable approved full aggregate `553cd4e8...4aa5692`,
code aggregate `2d6c752d24d4f4cc9ea30f6d5ced8b507dfd860acb8721ded85c758992705098`).

The approved input record is `/tmp/opencode/LP-final-approved-baseline.json`:
364 paths copied without `.git`, cache or artifacts, with every source hash and
Git mode verified before edits. `links.rs` remains removed in favour of
`links/mod.rs`; all other approved file moves are preserved. The approved LP-4a
broker remains byte-identical (`069a87e4...`). The external node remains
`eb257af7c10cdf0ded584adfadf2ae8124466304b13d26c17f855e76f9320f7c`
(`d2f37b91...`), an explicit non-Nix exception.

Only the requested merge-cleanup areas were changed:

- `namespace.rs`, `links/netns.rs`, `adapters.rs`, CLI helpers and tests;
- scenario/typed evidence, the new bounded `convergence.rs`, runtime extraction
  and topology-scenario declarations;
- the operator guides and the dated lab-only convergence decision;
- final-review documentation and provenance records.

Production share accounting, payout policy, SPc/Ark derivation and the
`DESIGN.md` authority boundary are unchanged. Network isolation does not grant
a filesystem/security sandbox.

## 2. Fable N1-N7 dispositions

### N1 - structured link errors and unknown policy

`NamespaceError::Rejected` now retains `op`, `code` and `detail`. Mapping uses
only the broker's actual structured code:

- `unknown_link` -> `UnknownLink`;
- `invalid_policy` -> `Unsupported`;
- `busy` -> `Busy`;
- `partial` / `policy_apply_failed` -> `Partial(detail)`;
- other codes -> `Unavailable(code + detail)`.

Message substrings do not determine a classification. Unknown or malformed
successful policy payloads fail as partial. `current()` inserts and returns only
valid decoded policies; partial/malformed/missing policy clears the cache and
returns `None`, never `Normal`. A dead broker also returns `None`.

Structured unit cases cover exact codes and a non-code containing `busy`.
Real broker cases prove zero-delay latency and >100% loss are
`Unsupported`, unknown links are `UnknownLink`, and the dead backend remains
unknown. No deprecated default-normal fallback remains.

### N2 - no RPC credential in helper argv/response

The namespace helper now uses `--auth-file <private-cookie-path>`. The existing
mode-0600 cookie is read only in the short-lived CLI helper and supplied to the
existing HTTP Basic encoder. Helper argv and broker responses can contain that
path but not its contents. The password is not moved to an environment variable
or rewrapped as different secret argv.

The parser requires a regular non-symlink file of at most 2048 bytes, no group
or other access, one nonempty `user:password` string, no whitespace/control/NUL
and no malformed or missing file. Errors never print the cookie body.
Namespaced controls fail if no cookie path is supplied; `NodeAccess` and startup
RPC use the exact rendered per-node cookie. Generic unauthenticated front API
requests are unchanged.

Unit tests cover valid/missing/malformed/group-readable files and exact Basic
base64. The opt-in broker test captures actual helper argv/response and proves
that a marker password and `--pass` do not appear. A full live node also proves
that real node authentication still works.

### N3 - strict helper inputs and private storage URLs

- `--params` must be valid JSON **and an array**. Invalid/null/object/scalar
  values are rejected instead of replaced with `[]`.
- HTTP status lines accept only HTTP/1.0 or HTTP/1.1 with an exact three-digit
  2xx code; no `contains(" 2")` or `2001` acceptance. Headers are capped at
  64 KiB, complete responses at 8 MiB.
- Credential-bearing `run-manifest.json` is explicitly **private**, mode 0600.
  Authenticated Redis connection state is preserved there for restore, while
  `shareable/` is a separate public projection. A new manifest test verifies
  mode, round-trip restore and absence of a marker password from shareable
  files; the live proof checks every node/Redis/PostgreSQL password field.

### N4 - typed topology evidence

`EvidencePayload` now has `topology_phase` and `topology_convergence` variants.
Both carry `schema_version = 1`, explicit phase/action, observation status,
journal outcome and optional unknown reasons. Ordinary phase records contain
controller/front/node views and bounded broker evidence with distinct
success/error objects. Convergence records additionally carry the complete
native convergence policy and durable control outcomes. No new event smuggles
its principal content through a stringified `UnknownEvidence.topic`.

Every atomic view retains the actual node, height, hash, atomicity and optional
peer/error fields. It also retains `previousblockhash` from the actual
`getblockheader` response. Phase/convergence events carry the
`pre_partition_common_anchor` when one has been observed. Differing heights do
not assert either a fork or a shared tip; parent-chain/phase validators can use
these values to distinguish descendants from actual divergent branches. A
serialization round trip plus unknown-field rejection and legacy
`unknown { topic: "topology_phase", detail: <string> }` loading is tested. The
existing generic SSE `message` handler needs no special case and remains green.

### N5 - native, bounded convergence

The old implicit resume-on-heal has been removed. `PhaseSpec` can declare one
optional native `convergence` action:

```json
{
  "version": 1,
  "timeout_secs": 90,
  "max_observations": 90,
  "allow_resume": true,
  "max_resume_attempts": 1
}
```

Absent fields retain old host/scenario behavior and no unexpected mining.
Version, 1..=90 timeout/observation bounds and resume combination are validated;
a disabled resume with nonzero budget is an error. A resume requires explicit
permission and counts independently of atomic view attempts. All pause/resume
operations are existing typed durable miner commands with journal outcomes.
A candidate common tip is not accepted until workers are paused and both nodes
are re-read atomically. Every attempted resume invalidates earlier quiescence
before the command is issued, including a partial or unknown outcome that may
have changed worker state. A later certificate therefore requires a new
successful pause followed by a fresh atomic observation. Pause alone performs
no hashing.

Timeout/exhaustion is `not_converged`; stop is `unknown` with an explicit reason
and no convergence claim. `timeout_secs` is checked between observation rounds,
not as a hard per-RPC wall deadline, and an action runs after all phases
regardless of declaration position (the scenario places it last); neither a
hard per-RPC deadline nor position-dependent phase ordering is claimed. The
convergence driver is injectable for bounded fake-view tests without replacing
real node/mining evidence in the live proof. Tests cover:

- a tied fork needing one permitted real-work resume then paused revalidation;
- a resumed common tip requiring controls `[pause, resume, pause]` and a fourth,
  freshly consumed view after the final pause (Fable C1 regression);
- disabled resume => no hash and no implicit recovery;
- timeout => `not_converged`;
- stop => interruptible `unknown`;
- different heights => no common tip or fork certificate.

The dated operational record is
`docs/decisions/2026-11-06-lab-convergence-v1.md`, with its retained changelog
fragment and the lab operator guide as the tool-only documentation homes. Under
approved docs decision 9, the lab-only entry is not included in `docs/SPEC.md`
frontmatter. This introduces no pool/share/payout change. The original 969/913
no-heal diagnostic stands; general convergence remains outside the claim.

### N6 - boundary language

Operator text and this report explicitly scope isolation to the broker-owned
**network** namespaces. Same-uid actors, descendants and credentials keep the
shared host filesystem/run-directory trust domain. There is no host `sudo`,
`modprobe`, `sysctl` or host network mutation, but this is **not a filesystem or
security sandbox**.

### N7 - provenance and guide delta

The following four operator documents remain a separately recorded delta over
approved docs snapshot `1d40b2b7`, not folded silently into that approval:

- `README.md`;
- `docs/LAB-RUNTIME.md`;
- `docs/OPERATIONS.md`;
- `docs/REGTEST-LAB-IMPLEMENTATION.md`.

`docs/SPEC.md` is byte-identical to its approved-docs copy (SHA-256
`5de914d7e2a38505beafa4410bd41798de5b2d2a65b46144832c3f769edbb788`); the
lab-only convergence frontmatter line was removed for C2 without reversing any
policy or dropping the dated decision/changelog. `docs/WIRE-FORMATS.md`,
historical share-target decisions and pool behavior are not rewritten. The new
decision and operator text describe the actual optional action, parent hashes,
pre-partition anchor, private manifests and native typed evidence. The previous
run/build/report associations are retained as historical inputs; this report
and the corrected source have a new manifest and source/code hash scope.

## 3. Corrected-source (v4) focused checks

| Check | Result |
|---|---|
| C1 fail-first regression | Failed before the fix with controls `[pause, resume]`, certifying at attempt 3 (`LP-final-c1-fail-first.log`). |
| C1 green regression | Passed with `[pause, resume, pause]`, attempts = 4 and all four scripted views consumed (`LP-final-c1-green.log`). |
| Convergence unit tests | **6/6**: tied work recovery, resumed-tip requiescence, disabled resume, timeout, interruption and different-height negative. |
| `spc-lab` package tests | All default targets passed, including **29/29** lib tests, typed topology **2/2**, private manifest **1/1**, HTTP/SSE and runtime/lifecycle tests; default topology **5 passed / 14 ignored**, plus two independently opt-in real-proof tests ignored (`LP-final-c1-spc-lab-tests.log`). |
| C2/frontmatter/docs | The lab-only entry is absent from SPEC frontmatter; `docs/SPEC.md` matches the approved baseline hash, and the dated decision/changelog/guide remain (`LP-final-c1-c2-doc-contract.log`). `scripts/check-lab-docs.py` and link/punctuation checks passed with the same three declared main-only inputs. |
| Formatting/clippy | `cargo fmt --all -- --check` clean; `cargo clippy --locked -p spc-lab -p spc-lab-cli --all-targets -- -D warnings` passed (`LP-final-c1-fmt.log`, `LP-final-c1-clippy.log`). |
| Historical opt-in host namespace suite | **14/14** on the v3 candidate (`LP-final-test-topology-netns-001.log`); not rerun for this convergence/documentation-only correction. No host namespace or live miner was started by the C1/C2 packet. |

The corrected gate source is `/tmp/opencode/LP-final-frozen-src-v4` (370 regular
files, git-free). `LP-final-code-manifest-v4.json` records the
mutable-report-excluded code aggregate
`30fedafdbba57b79fe4c13e2de0658aa3f05f527b778a242cbfab9c4ba253d76`.
Only four non-report files differ from the tested v3 source: `convergence.rs`,
`scenario.rs`, `LAB-RUNTIME.md` and `SPEC.md`; all other 365 non-report files are
byte-identical (`LP-final-c1-c2-vs-v3.log`). The final mutable report is
separately hashed by the final manifest below.

### Corrected-source Nix gate

From the v4 source, all four package builds completed and
`nix flake check --offline` passed all six checks
(`LP-final-nix-builds-v4.log`, `LP-final-nix-flake-check-v4.log`): `fmt`,
`clippy`, `doc`, workspace `test`, `ui-node` and `test-ark`. This is a fresh
source-bound gate, not a relabelled v3 cache result.

As with the earlier gate, `CARGO_TARGET_DIR=/dev/shm/LP-final-cargo-target` was
used for large Cargo outputs and `RUSTFLAGS=-Cdebuginfo=1` for check invocations
only. No source, artifact or Nix store output was deleted or garbage-collected.
The pinned Nix network-tool realization remains externally blocked by mirror
fetching; historical namespace evidence is host-tool evidence, not a claim that
those network tools were Nix-realized.

## 4. Historical v3 frozen-source native proof (retained evidence)

Fable verified that this older evidence remains valid and did not request a new
live mining run. It predates the C1 fix and used the v3 `spc-lab` binary SHA-256
`0d3d31f65a27ad571b33195210cf5197badf9a464d35551ce316e3fac483fcbc` with code
aggregate `37acb9aa5eb3ea23d2525dc4921ccec8d23e90e43b53ca6809bcee93af938bd1`.
It did **not** execute the corrected v4 source or binary. Its successful
convergence used attempts = 2 and controls `[pause, pause]`, never the defective
resume path. It therefore remains evidence for the previously verified historical
path, not regression proof of C1 requiescence after resume.

Run: `/tmp/opencode/LP-final-native-v3`
Console: `/tmp/opencode/LP-final-native-v3.log`
Independent validation: `/tmp/opencode/LP-final-verify-native-v3.py`
Summary: `/tmp/opencode/LP-final-native-v3-evidence.json`

Using the unchanged old L2 node and frozen-source Nix outputs:

| Evidence | Result |
|---|---|
| Assertions | All four passed, including independently accounted `front-a1` and `front-b1`. |
| Accounting | Scoped consistency `consistent`, 72/72; 0 unknown, inconsistent or malformed. |
| Per-front credits | a1 30, b1 42; `front_evidence` agrees (30/30 and 42/42 block-qualified). |
| Requested/measured | 20 req / 19 measured H/s per miner; attempted 74/77, stale-cancelled 35/43. |
| P2P fault | 34 dropped packets before heal; packet count only, never a block-count inference. |
| No bypass | `ok: true`, 276 checks, 0 violations. |
| Divergent view | node-a h13 `526224c7f6...68d46f`, node-b h14 `38a31a88...6b42a2d`; retained as source-typed views with real parent hashes, not used generically as a fork certificate. |
| Final convergence | Both nodes h44 `7d5e1ef006e87ad1b38f397a6fc86e53a9d3d5feca15b7a2d661aead006a207c`, after journaled pause and re-read. |
| Parent/anchor | `previousblockhash` preserved; partition common anchor is h9 `0210873edc283bd1bb0b8204c68d8e8f900593cb821007027e9af53bff2e9668`, also carried in typed convergence. |
| Storage | Both Redis return `PONG`; final Postgres counts a1 30|30|1 and b1 42|42|1. |
| Privacy | Cookie body/Redis/Postgres password fields are absent from shareable projections. Private manifest mode is 0600. |
| Shutdown | 8 stopped + 2 exited; no actor, broker holder or namespace-holder survivor; `/run/netns` empty. |

The native scenario has the explicit `converge-after-heal` action with the exact
bounds above. No `generate*`, `setmocktime` or fake accounting was used. A block
qualifying PoW submission is not automatically a canonical block. No causal
budget/retarget, full-window/header/payout, fee/witness, ladder, Ark, SV1,
stock-v0.9 compatibility or general-convergence claim is made.

The actor lifetime helper was not changed by the C1/C2 correction. The earlier
independently replicated controller-SIGKILL/PG-descendant cleanup proof was not
rerun or relabelled as a corrected-v4-binary proof. The historical v3 binary's
ordinary stop and owned terminal states were independently checked for its run;
no new crash or ordinary live run is claimed for v4.

## 5. Corrected v4 Nix provenance vs historical v3 run binaries

### Corrected v4 source-check outputs

Source: `/tmp/opencode/LP-final-frozen-src-v4`  
Code aggregate: `30fedafdbba57b79fe4c13e2de0658aa3f05f527b778a242cbfab9c4ba253d76`  
Record: `/tmp/opencode/LP-final-binary-provenance-v4.json`

| Binary | Derivation | Output | SHA-256 |
|---|---|---|---|
| `spc-lab` | `/nix/store/h1r50rysf6b38b8axxy37mwpy3v2vv2c-spc-lab-0.1.0.drv` | `/nix/store/9vkjx7mhxl46cq6wwd690rs8c7kmsinp-spc-lab-0.1.0` | `7249995c211fff1304925ab63e356c7b84ce4230b617aa608e19f01e9231dac3` |
| `spc-pool` | `/nix/store/82qka96iw3dz9908v2ffzxwppw9rf3xc-spc-pool-0.1.0.drv` | `/nix/store/y4k0q89r8b40hji4krfayz3zy1cw9swv-spc-pool-0.1.0` | `aac1278f870fbdd504f9c5846b5d38ebf52335853dcbc471932afb8b9c6c481d` |
| `spc-cpu-miner` | `/nix/store/dqk1srd1xa0kzkgyavnyfcz5bzhjzzaq-spc-cpu-miner-0.1.0.drv` | `/nix/store/rq65jn0aygdgcw0a34z9axj2dsm5zp0n-spc-cpu-miner-0.1.0` | `443de4380b50b48626556bfff3596e621723d54eea82f96d656af39959ffa472` |
| `spc-scan` | `/nix/store/naicb4jwwy0n16l4gvphvi5fkpz1pd2l-spc-scan-0.1.0.drv` | `/nix/store/v840arrn5vdd6p3z6bbx4y0i1l184886-spc-scan-0.1.0` | `f8000b43b3c7f12030be4d2eebb6fb5073cf0ae4b86830f595f24cad6bf14417` |

These v4 outputs were compiled and source-checked only. They did **not** execute
the historical native run in section 4; in particular that run is not evidence
that the C1-fixed `spc-lab` binary ran a live scenario.

### Historical v3 native-run outputs

Source: `/tmp/opencode/LP-final-frozen-src-v3`  
Record: `/tmp/opencode/LP-final-binary-provenance-v3.json`

| Binary | Derivation | Output | SHA-256 |
|---|---|---|---|
| `spc-lab` | `/nix/store/pkx4bm8a37j5l9h2byh8h81zqhyq5wqz-spc-lab-0.1.0.drv` | `/nix/store/dcxi8d1kmqn87y014fi9aq3lvf6rjfk3-spc-lab-0.1.0` | `0d3d31f65a27ad571b33195210cf5197badf9a464d35551ce316e3fac483fcbc` |
| `spc-pool` | `/nix/store/dk2qpclqk9fm48p4l3gncgvmh18ajzrj-spc-pool-0.1.0.drv` | `/nix/store/h2h63b32z4pfnhgkijci5rvaphk6yp2y-spc-pool-0.1.0` | `aac1278f870fbdd504f9c5846b5d38ebf52335853dcbc471932afb8b9c6c481d` |
| `spc-cpu-miner` | `/nix/store/0qqw31nw63zn4s1i7ya7dnfydijb2m68-spc-cpu-miner-0.1.0.drv` | `/nix/store/hxzfjm0wgvn48w6xj4fzzkfsdkqv5k6n-spc-cpu-miner-0.1.0` | `443de4380b50b48626556bfff3596e621723d54eea82f96d656af39959ffa472` |
| `spc-scan` | `/nix/store/clqfdw933rdws42bc1mvpyk9jl61gm64-spc-scan-0.1.0.drv` | `/nix/store/c6x1820i737k1p8v0gj13clh0a9dacjr-spc-scan-0.1.0` | `f8000b43b3c7f12030be4d2eebb6fb5073cf0ae4b86830f595f24cad6bf14417` |

Records `/tmp/opencode/LP-final-nix-builds-v3.json` and
`LP-final-nix-flake-check-v3.log` preserve the historical Nix build/check inputs.
They are retained for provenance only; no v3 binary hash is reused as the
corrected v4 source identity.

## 6. Final corrected-source manifest

`/tmp/opencode/LP-final-runtime-manifest.json` is the new source-only review
manifest generated by `/tmp/opencode/LP-final-build-manifest.py`. It records:

- every source path, SHA-256 and normalized Git executable mode;
- the approved 364-file baseline and input hashes, LP-4a broker, approved docs,
  namespace-runtime approval and L2 external-node exception;
- full 370-file aggregate, mutable-report-excluded code aggregate and the
  separate report hash;
- the formula `sha256(compact sorted JSON of {base, files[path, sha256, mode]})`;
- `.git`, target, result, cache, temp and generated regtest data exclusions.

The final source copy is `/tmp/opencode/LP-final-runtime-src-v4`. All 369
non-report files match the v4 Nix gate source byte-for-byte, and the code
aggregate is `30fedafdbba57b79fe4c13e2de0658aa3f05f527b778a242cbfab9c4ba253d76`.
The report is the only mutable file excluded from that code aggregate. Final
full/code/report hashes are independently recomputed after report generation;
they are not placeholders. The native evidence remains bound to v3 code and
binaries as section 4 states, never to this corrected v4 source label.

## 7. Historical v3 native-run command

No live or crash command was run for the C1/C2 correction. The following is the
exact historical v3 invocation retained for provenance, not a command executed
against v4.

Working directory: `/tmp/opencode/LP-final-frozen-src-v3`.

```sh
LAB=/nix/store/dcxi8d1kmqn87y014fi9aq3lvf6rjfk3-spc-lab-0.1.0/bin/spc-lab
POOL=/nix/store/h2h63b32z4pfnhgkijci5rvaphk6yp2y-spc-pool-0.1.0/bin/spc-pool
MINER=/nix/store/hxzfjm0wgvn48w6xj4fzzkfsdkqv5k6n-spc-cpu-miner-0.1.0/bin/spc-cpu-miner
SCAN=/nix/store/c6x1820i737k1p8v0gj13clh0a9dacjr-spc-scan-0.1.0/bin/spc-scan

"$LAB" validate --scenario crates/spc-lab/scenarios/lab-network-runtime-two-pools.json

"$LAB" runtime \
  --scenario crates/spc-lab/scenarios/lab-network-runtime-two-pools.json \
  --run-dir /tmp/opencode/LP-final-native-v3 \
  --node /tmp/opencode/rbitcoin-lab-retarget-profile/target/debug/rbitcoin-node \
  --pool "$POOL" --miner "$MINER" --scanner "$SCAN" \
  --postgres-bin-dir /nix/store/vync24csmxy5saql5ah77vkn59yg2x5h-postgresql-17.11/bin \
  --redis-server /nix/store/nql1lir6650xigvf43n11zz4hh0lbw9z-redis-8.10.3/bin/redis-server \
  --redis-cli /nix/store/nql1lir6650xigvf43n11zz4hh0lbw9z-redis-8.10.3/bin/redis-cli \
  --pow-bits 0x207ffffe \
  --base-revision b0ab4ca34fda07c61a6335d05f598eaacc69678c \
  --dirty-tree-hash 37acb9aa5eb3ea23d2525dc4921ccec8d23e90e43b53ca6809bcee93af938bd1 \
  --topology --broker-script scripts/lab/lib/lab_backend.py \
  --listen 127.0.0.1:0 --duration-secs 3
```

For an optional new v4 reproduction (not performed in this packet), use the same
scenario/options with a fresh non-v3 run directory and these corrected settings:

```sh
LAB=/nix/store/9vkjx7mhxl46cq6wwd690rs8c7kmsinp-spc-lab-0.1.0/bin/spc-lab
POOL=/nix/store/y4k0q89r8b40hji4krfayz3zy1cw9swv-spc-pool-0.1.0/bin/spc-pool
MINER=/nix/store/rq65jn0aygdgcw0a34z9axj2dsm5zp0n-spc-cpu-miner-0.1.0/bin/spc-cpu-miner
SCAN=/nix/store/v840arrn5vdd6p3z6bbx4y0i1l184886-spc-scan-0.1.0/bin/spc-scan
DIRTY_TREE_HASH=30fedafdbba57b79fe4c13e2de0658aa3f05f527b778a242cbfab9c4ba253d76
```

Those v4 settings have no association with the historical v3 result; use
`--dirty-tree-hash "$DIRTY_TREE_HASH"` and a new `--run-dir` if independently
reproducing. Reproduction requires a **fresh** run directory; change the
`--run-dir` instead of deleting evidence. Read `shareable/result.json`,
`shareable/topology.json`, `private/evidence.jsonl` and `private/commands.jsonl`.
Normal duration or
`stop_run` cleanup leaves all ten actor records stopped/exited. A killed
controller can be reaped by verified identity using the same binary, never a PID
alone. The actual run above printed HTTP endpoint `http://127.0.0.1:38257`.

## 8. Remaining limits

No approval is claimed by this report. Parent Fable review should inspect the
focused C1/C2 delta, its red/green regression and Nix source checks, the retained
historical evidence boundary, and the new source manifest. The path is not
modified by an older approved snapshot, the queued v0.9 validator, a later
mutable report revision, or any upstream node release. Nix network-tool
realization, full production protocol/payout/Ark behavior and general node
convergence remain outside this candidate.
