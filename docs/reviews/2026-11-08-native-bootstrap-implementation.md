# Native bootstrap implementation (T2-L)

Date: 2026-11-08
Updated: 2026-11-09
Status: COMPILED contextual-policy amendment; wallet/controller tests,
all-target clippy and formatting passed. FRESH NATIVE T2 FUNDING PASSED on
two actual nodes with two real wallet actors. Storage/front/miner processes
were explicitly inert fixtures; full T3 mining and Ark acceptance are separate.

## Combined T1-DEST verification (latest)

The tested T1-DEST packet was verified before applying its 17-path delta:
full `e4a81a06a126799d66b4c2eb2f0573d859c01c793cbfe4dfc22a69e054440fb9`,
code `ac7041578c229a2af33f82dc806db1f79b90798ebd2fad8c7dc5800a0e35efbe`,
patch `ef2d624cafa0b3e641a41d4e08492e2bd3e6d93d855b34361d43f833bcc92577`.
All 711 published source rows/modes and the five passing log/source bindings
were checked. The predecessor was preserved at
`/tmp/opencode/T2-native-before-T1DEST-d905c118/` (554,137 changed-source bytes,
plus prior freeze/receipt metadata). No whole upstream overlay was imported.

Only two overlaps existed: the actor-control fixture retained all four
required contextual policy fields, and SPEC received only the additive
T1-DEST bullet. Removing those four fixture lines reproduces the exact tested
destination-owner file hash. Every other destination row matches its tested
hash; native executor, installer, dependency edge, contextual observer and
proof code were preserved. Layer receipt:
`/tmp/opencode/T2-T1DEST-layer-receipt.json`. The destination owner's 154-test
attribution is retained in its handoff, not added to combined test counts.

The new guarded combined gate
`/tmp/opencode/T2-native-gate-20261110T034501Z-contextual` passed full txd
tests (121 top-level instances, two intentional child entry-point ignores,
fault child separately exercised), 62 selected controller tests, all-target
clippy, formatting and CLI/txd binary builds. The first combined live attempt
`T2-native-funded-20261110-context-dest-v1` encountered an HTTP port-allocation
race before bootstrap: exit 1, all 14 owned identities cleaned up. The
retained harness failure was corrected by kernel-assigned HTTP port 0 and
reading the controller's published URL; Rust source was unchanged.

Fresh combined run:
`/tmp/opencode/T2-native-funded-20261110-context-dest-v2`.
Authoritative receipt: `/tmp/opencode/T2-T1DEST-verification-receipt.json`.

- Native bootstrap passed again on both actual nodes: height 105, all 80
  allocation outputs credited, four signed distribution transactions, total
  distribution fees 3,492 sats. The independent completion verifier succeeded
  before activation. Actual search: 1,809 measured attempts, 1,810 charged
  attempts, 1,798 unique reconciled batches, 128,950 ms executor elapsed. The
  one reserved-but-unused attempt was NOT refunded or counted as worked.
- AFTER verified bootstrap, the existing `send_user_payment` controller route
  used the SAME already funded `txd-0` wallet set and UserPayment index 0 to
  pay 100,000 sats to each external output class. No new wallet/signer/miner,
  generated block or funding API was introduced. P2PKH inputs came from
  canonical distribution transaction
  `90f146e8e81353ecaefdb38a76d8e488f90f161eadcc6c92ab0eb12a173a84d9`
  at height 103, vouts 11 and 12 respectively.
- P2WPKH txid:
  `a56080674111d5773fe94938a39dbb19e490b021befed00a138b2cc55483e9dd`;
  actual fee 672 sats, final 223 vB.
- P2TR txid:
  `8049198fdc28cbfc7e27ea3e1946a0d4c9bf917f275405f5826e9eec68aef659`;
  actual fee 708 sats, final 235 vB. Its destination is the public stock-Bark
  address fixture published by T1-DEST; no Ark service was contacted.
- Both actual nodes showed each txid in their mempool and returned identical
  `getrawtransaction [txid,0]` bytes. The independent parser checked txids,
  exact destination script/value, legacy inputs, P2PKH change, canonical input
  provenance and exact input-minus-output fees against signer journal bytes.
  Durable command replay caused no duplicate send (one BroadcastSent per id).
- These two external transactions are explicitly UNCONFIRMED. The bootstrap
  window was not reopened to mine them. This is node-accepted raw-TX proof,
  not confirmed external/Ark funding, witness-input spending, board/receive/
  refresh/offboard or full T3 acceptance.
- Durable stop succeeded; all 14 owned identities and the controller were
  gone. Pre/post source and binary hashes matched; the shared slot is released.

The initial funding allocation remains P2PKH. The separately hash-bound
contextual join on the destination baseline is
`/tmp/opencode/T2-contextual-T1DEST-join.patch` with receipt
`/tmp/opencode/T2-contextual-T1DEST-join.json`; it excludes the native installer
for the T1 owner. The complete combined source freeze retains the two tested
upstream layers and the explicit T2/contextual delta separately.

## Passed fresh native T2 slice

Run: `/tmp/opencode/T2-native-funded-20261110-context-v1`.
Receipt: `/tmp/opencode/T2-contextual-verification-receipt.json`.

- Unmodified node binary `36e2387ada164e2c89b3d20f5d515bf19c33f30fb927e8ca11621eba3658d7d9`;
  canonical probe contract `754e1e6131955f9e5bbce7ef81b6f9aabaa2f1379ec6409f1c3a7a9e88be91f9`.
  Actual-node preflight/profile binding passed before the real executor ran.
- 105 raw canonical blocks on both actual nodes: 101 initial funding blocks,
  then four distribution-confirmation blocks. The first block legitimately
  uses 207fffff; subsequent short-gap blocks use 200fffff. The independent
  Python oracle recomputed contextual bits, PoW, txid Merkle roots, raw fees,
  subsidy bounds, input maturity and all destination output values/scripts.
- 1,632 actual SHA256d search attempts, 1,632 durably charged attempts, 1,623
  unique reconciled batches, no unresolved debit. The requested aggregate
  rate remains 40 H/s; complete executor elapsed time was 118,101 ms (including
  reads/scans/evidence, not a claim of sustained 40 H/s throughput). Every
  attempted raw header, target and authorization is retained privately.
- Four SAME-HOST-WALLET signed legacy distribution transactions confirmed:
  `cc3216e171d6e1f7c3980918632d4d2fc90a34147678fe2050675989ba80625b`,
  `392e0dac7182ef416635155762d668153bbf132bfdcaa21ed1012eb45c95ac99`,
  `9c9b4d9819c4b72cbf85239e91d01338637777b3f9eaad440498cc76d97786ac`,
  `9a106e9021e01d722e9ae1461ec268d854c4580a9cb142ee4375f61a1766d2a2`.
  Actual fees: 907, 907, 907 and 771 sats, total 3,492 sats.
- All 80 declared outputs were credited exactly once at their disjoint global
  role/index loci across both real actor journals. Host funding observations:
  149 (105 coinbases, 40 allocation receipts, 4 change outputs); second actor:
  40 canonical non-coinbase receipts. No shared signing wallet or synthetic
  credit API was used. The oldest coinbase was mature at the declared boundary;
  every spending input's creation/maturity was independently checked from bytes.
- The existing independent controller completion verifier succeeded, durably
  finishing `native-bootstrap-fe9c515ed8ce8aaa` at command-log seq 3251 with
  `bootstrap distribution independently verified; measured phase active`.
  Worker/scheduler activation remained behind that independent verification.
- Durable StopRun succeeded. All 14 owned PID/start-token identities, including
  probe descendants, and the controller were gone. CLI `--keep` was terminated
  only after its durable stop, so controller exit -15 is intentional fixture
  lifecycle cleanup, not an unobserved bootstrap outcome. Shared build lock is
  released. Pre/post source and executable hashes matched throughout the run.

Private native artifact SHA256:
`182b61afd4efb7eb06bd2342e2323095e40bb0d5115ee04b01aea208e5d0475f`.
The scope is actual T2 funding/controller/actor/node/probe behavior; inert
storage/front/miners do not prove pool fee-paying mining, T3 statistics, or
stock Ark send/receive/refresh/offboard behavior.

## Contextual policy correction (current)

The node owner confirmed the earlier node-mismatch diagnosis was wrong:
`no_pow_retargeting=true` does not disable independently enabled minimum
difficulty. Fresh genesis-to-wall-clock legitimately requires 207fffff;
the actual node rejected a genuinely hashed 200fffff block at that timestamp
with `bad-diffbits`. See `/tmp/opencode/node-target-semantics-r3/REPORT.md`.
The earlier failed runs below are retained as failed consumer attempts and
are not reclassified as passes.

The parent selected faithful contextual consumer support. Six starter-patch
before hashes were checked and changed rows archived at
`/tmp/opencode/T2-native-before-contextual-policy-v1/`. The required min flag
was applied first, then its blanket refusal was replaced by shared
`contextual_pow::VerifiedHeaders`. Required spacing, interval and overlay
facts come from actual bound NodeEffectiveProfile fields, with no defaults.
Both GBT and the raw observer/independent completion verifier now check each
header using the exact documented node rule order and complete ancestry.
Periodic retargeting still refuses before authorization. No node code/policy,
clock, hash budget, signer or journal format was changed.

Guarded Nix 1.93 gate `/tmp/opencode/T2-native-gate-20261110T033010Z-contextual`
passed: full txd tests (53 library tests including 7 contextual rule/proof
tests and 17 native tests, plus actor/core integration suites), all 61 selected
controller tests, all-target clippy, formatting and CLI/txd binary builds.
The independent live oracle was separately updated to derive contextual bits
from raw ancestors and actual live consensus facts. Source policy details:
`docs/decisions/2026-11-09-contextual-native-difficulty.md`.

## Preserved earlier verification and consumer failure (2026-11-09)

The final tested T1/J1 handoff was verified before consumption: all 706 rows
from `/tmp/opencode/traffic-runtime-j1-final-manifest-full.json`, full
`e53118dd3869db3722b4a28bf0028103604dcb7d162320c07fcf9607579dfaaa`, tested
code `8b1bbdc9267c4fd7a97592f2424e8e16e648e647cfa6e75033ad360be457be71`.
Only changed upstream rows were copied after checking their known local
baseline hashes; R3/R4 were not blindly reapplied. The one changed overlap,
`engine/traffic_runtime.rs`, was three-way compared: removing T2's installer
reproduced the old upstream hash, and the five-line J1 source-name addition
produced the final upstream bytes while preserving the installer. The
remaining T2 dependency/lib/lock/document changes stayed intact. Layer
receipt: `/tmp/opencode/T2-native-final-J1-layer-receipt.json`; small archive:
`/tmp/opencode/T2-native-before-final-J1-9b31b61b/` (759,967 source bytes,
changed rows only). The original 700-row R2 baseline remains immutable.

Actual Nix Rust/Cargo 1.93 verification, using the existing target and
complete read-only vendor directory:

| Gate directory under `/tmp/opencode/` | Actual result |
|---|---|
| `T2-native-gate-20261109T220736Z-native` | 14 initial native Rust tests passed |
| `T2-native-gate-20261109T220804Z-affected` | Full txd suite passed (104 top-level tests, 2 explicitly ignored child entry points, fault child exercised); controller completion/contract/history/RPC/runtime tests passed (61); controller budget tests passed (3). Its subsequent clippy invocation failed because the external Cargo subcommand did not inherit the global vendor option. |
| `T2-native-gate-20261109T221125Z-quality` | After correcting the invocation, compiler caught T2's missing `json!` qualification in the new private-provenance installer. Failure retained; fixed with `serde_json::json!`. |
| `T2-native-gate-20261109T221214Z-finish` | 15 native tests including the private-root negative passed; all-target clippy `-D warnings` for txd/lab/CLI, package formatting and real CLI/txd binary build passed. |
| `T2-native-gate-20261109T221619Z-finish` | After adding precise rejected-template diagnostics, the same 15 native tests, all-target clippy, formatting and binary build passed again. This is the live R2/R3 producer source. |

Every actual build/test invocation held nonblocking `flock -n -o` on the
shared heavy slot, checked both target/store filesystems inside the lock
against the 50 GiB floor and 20 GiB residual, and reused
`/tmp/opencode/T1a-target`. These gates started with 84-89 GiB available and
a conservative 15 GiB peak estimate. Toolchain executable hashes were
verified against T1's recorded Nix 1.93 paths. No pins/version bumps, target
copies, cache deletion, competing agent or workspace Nix build occurred.

Private artifact implementation now rechecks each private parent/root's
ownership, non-symlink directory type and 0700 mode at execution, enforces a
256 MiB evidence byte ceiling, and writes its path plus compile-time executor
source SHA256, executing controller SHA256 and manifest source claims into
`private/traffic/native-bootstrap-provenance.json` through the existing
private artifact channel. Raw work stays out of public TrafficObservation.
The root negative test rejects symlinks and 0755 directories. Rejected GBT
templates are retained privately, including their actual bits and target.

### Fresh actual-node attempts

The external, hashed acceptance harness `/tmp/opencode/T2-native-funded.py`
uses two real nodes, the canonical `754e1e61...` probe, two real txd actors,
the real CLI controller/long-operation path, and the installed native
executor. Storage/front/miner processes are explicitly labeled inert
fixtures: this is a T2 slice, not a full mining or Ark run. Nodes use freshly
leased loopback P2P/RPC endpoints and a new seed/run/datadir for each case;
their complete owned listen/connect groups pass production profile binding.
Probe Python uses `-B` to avoid writing into the read-only upstream source.

| Fresh owned directory under `/tmp/opencode/` | Node binary | Outcome |
|---|---|---|
| `T2-native-funded-20261109-r1` | `36e2387ada164e2c89b3d20f5d515bf19c33f30fb927e8ca11621eba3658d7d9` | Refused template before any hash; initial generic target-profile diagnostic |
| `T2-native-funded-20261109-r2` | same verified node | Exact private GBT proves `bits=207fffff`, target `7fffff0000000000000000000000000000000000000000000000000000000000`, height 1; bound policy declares `initial_target_bits=200fffff`, `no_pow_retargeting=true` |
| `T2-native-funded-20261109-r3-release` | Nix release `bba4752298aff258adb6cce99c6db849b02f50736a8ec32766ed1710045051aa` | Reproduces the same actual GBT/policy mismatch |

All cases passed actual read-only preflight recognition/profile binding and
reached the installed executor. They then failed
`bootstrap_target_profile_mismatch` BEFORE reservation or SHA256d search:
zero hash reservations, zero measured hashes, zero coinbases/submissions,
zero funding observations, zero distribution sends, and no worker/scheduler
activation. Every case durably stopped and all 14 owned process identities,
including probe descendants, were gone. Pre/post source and binary hashes,
raw rejected GBT, private provenance, command terminal, logs and cleanup
receipts are retained in those directories. The build slot was held during
each live slice to preserve shared-target executable identity and is released.

This is a concrete node-policy/interface blocker, not an old compiler hold.
The target field correctly matches `207fffff`; the mismatch is with the
declared fixed `200fffff` schedule. Read-only node source inspection locates
a plausible cause: `rbitcoin-consensus/src/header.rs:332-334` exempts a
min-difficulty time gap from the overlay, and fresh genesis-to-current-time
has that gap, despite the preceding comment describing a fixed overlay.
`rbitcoin-rpc/src/methods/mine.rs:981-1008` selects next bits at actual curtime.
Parent/node handoff: `/tmp/opencode/T2-native-live-target-blocker.md`.
Neither bits nor time were substituted, and no node/core/T1 policy was
modified to manufacture progress. A supported live fixed profile must be
resolved before real maturity/distribution acceptance can run.

The independent full raw-chain/distribution oracle is authored in the
harness, but it was NOT reached and is NOT a passed funded proof. Current
evidence receipts and final source manifests are external to avoid circular
report/source hashes. Earlier source-only notes below are retained as history.

## Original source checkpoint: ownership and provenance

Sole-owned worktree:
`/home/ethan/.local/share/opencode/worktree/ad8886/lab-native-bootstrap`
at `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.

The 700 upstream files were all hash/mode verified BEFORE copying from
`lab-traffic-runtime`, with 11,260,625 source bytes. No .git, target,
runtime data, generated cache, or second full temporary snapshot was copied.
Receipt: `/tmp/opencode/T2-native-upstream-receipt.json`.

- Upstream full: `b7d607e5c33f06c427de25b5a200eae08fe4d1db2989608989e9ed81949584b7`.
- Upstream code: `5f0522bd0ea45bdc74dc92ce8ebb27c0aa713e5d43b03ef028ffcc0ad720a92f`.
- Upstream layer is explicitly source-authored/unverified. No T1 test result
  is inherited by this packet. Earlier core approvals do not compile T1b.
- Approved plan, all 949 lines read: SHA256
  `44daaa2baa41a4b93eb46a9e69dfdcf2bba1e0ce73367e27e83a87ab3a6b3307`.

Current primary AGENTS and required README, DESIGN, SPEC frontmatter,
ARCHITECTURE, WIRE-FORMATS, ROADMAP, MAILBOX, RESULTS and pins were read.
The complete concrete bootstrap/actor bootstrap-control interfaces were
consumed. No primary/other-worktree edit, pin bump, commit or delegation.

## Implementation

`NativeBootstrapExecutor` implements the existing `BootstrapExecutor` trait.
All node calls use `BootstrapHost::actor_request("bootstrap_rpc", ...)` and
the exact T1 typed `RpcOutcome`; no credential/HTTP shortcut. The narrow
installer runs after the real funding host's launch, using its actual
incarnation and validated `ActorConfig`/effective chain policy. Existing
injected executors remain available for explicit fixtures.

Production behavior:

- Actual GBT `[{"mode":"template","rules":["segwit"]}]`; strict integer
  metadata, fixed-length hex, consensus roundtrip and returned txid checks.
  Witness-bearing templates fail immediately and honestly, without selection,
  dependency, fee or witness-commitment changes.
- Compact sign/zero/overflow/pow-limit checks, separate GBT target equality,
  and bound initial/constant schedule equality. Retarget schedules lacking
  necessary fields in the current `ChainPolicy` fail closed before mining.
- Independent subsidy/halving plus checked included integer fees must equal
  GBT coinbasevalue. Real minimal BIP34 height push and nonce, null prevout,
  maximum sequence, and one exact actor-returned P2PKH funding output.
- Public `spc_cpu_miner::{hash,budget}` helpers supply SHA256d, coinbase Merkle
  path evaluation, header serialization, `HeaderPreimage`, `HashBudget` and
  `Pacer` (including its public `pacing_delay`-backed delay). No hidden miner.
- At most 16 attempts per batch, before each of which the full durable actor
  reservation is validated and consumed locally once. Operation/incarnation/
  batch/count/cumulative debit all bind. Unknown/lost/duplicate/old permits
  execute no new work. Measured work is reconciled without refunds; one
  outstanding batch at most. Pacer and cumulative budget span all templates,
  rejected/stale work and blocks. Default controller limits remain 40 H/s,
  2^22 total, 3600 seconds and 30-second cancellation grace. Configured rate
  overrides are read from the bound budget rather than silently replaced.
- Cooperative pause/cancel and boot-bound absolute CLOCK_MONOTONIC deadline.
  RPC, scan and evidence work do not create new rate credit. No restart resets
  the deadline/debit; unresolved authorization or evidence-file re-entry
  rejects. This packet does not claim an automatic resumable funder.
- Actual submitblock response is retained as typed evidence, then raw blocks,
  txid Merkle root, PoW, exact coinbase value/script, canonical height mapping
  and common presence on every declared actual node are checked. Null is not
  canonical evidence. Actor observation owns the exact BootstrapCoinbase /
  CanonicalBlock(coinbase=true) journal pairing and real-tip admission.
- Mature funding is counted per outpoint; 101 blocks mature only the first.
  A displaced funding block causes an inconclusive failure before distribution.
  All distribution amounts and the entire bounded part sequence are computed
  before any send, with exact global wallet-role/index/output-count allocation,
  declared maximum fee reserve and retained integer rounding/change. The
  existing multi-output signer is used, not a second wallet or multisend RPC.
- Each distribution part is mined/confirmed before the next. The final common
  raw chain proves exact outputs, unspent destinations, rational fee bounds,
  conservation and native mature-coinbase/host-change ancestry. Only then does
  `BootstrapCompletion` name txids/common tip for the independent T1 verifier.
  This executor never activates miners or the ordinary scheduler.

## Approved private evidence interpretation

The T1 public `TrafficObservation` is deliberately a closed DTO excluding raw
bytes; no existing allowed BootstrapHost operation accepts raw hash evidence.
The exact interface gap was reported early in
`/tmp/opencode/T2-native-interface-handoff.md`.

The parent approved this narrow interpretation on 2026-11-08: create-new
mode-0600 `native-bootstrap.jsonl` under the validated, owned, non-symlink
mode-0700 driver root. It is observation-only, with no budget, signing or
replay authority. Every bounded batch retains actual raw headers, measured
count, target, height, authorization and monotonic time; submissions retain
raw blocks and typed outcomes. Write/flush/sync failure stops the operation.
The existing durable CommandLog controls all hash reservations even if the
artifact is lost or truncated; the original core funding journal is the sole
money authority. Existing-artifact restart refuses without deletion or
duplicate execution. No second ledger or new public schema is required.

Record the artifact path and source/hash provenance through private evidence
or the appropriate existing manifest channel; raw records must never enter
user/public `TrafficObservation`. The finite operation budget bounds artifact
growth. `source=bootstrap` and exclusion-from-statistics flags are claims:
the oracle must verify the actual measured boundary independently. The
interpretation is approved, with no further artifact approval hold.

This update changes only the decision and report. Prior freeze/report/decision
bytes and their hashes were preserved before editing in
`/tmp/opencode/T2-native-r9-preserved-9c39eca8/` (379,313 bytes, no full source
snapshot). `/tmp/opencode/T2-native-private-artifact-provenance.json` records
the private artifact location contract and exact frozen source hashes;
runtime artifact path/hash and executable provenance remain unavailable until
a real run produces them. Runtime root confinement, private manifest/evidence
binding, failure behavior and bounded growth remain conformance checks, not
claims of newly executed verification.

## Historical verification before the final T1/J1 handoff

1. Upstream copy: all 700 path/hash/mode rows verified before writing; exact
   full-manifest row digest checked. Receipt above.
2. Formatting: Nix-store Rust 1.93 rustfmt on both new Rust source files;
   `git diff --check` passed. The installer hunk was formatted narrowly.
3. Independent Python expected-vector checks passed: Bitcoin genesis header
   byte order and actual SHA256d/PoW, 101-block maturity boundary and halving
   boundary. `/tmp/opencode/T2-native-vector-receipt.json` explicitly says
   this does not execute or validate the Rust implementation.
4. Focused build attempted under nonblocking heavy flock, no priority file,
   ~55 GiB free on both actual target/store filesystems, 15 GiB conservative
   peak estimate and >20 GiB residual. Existing target
   `/tmp/opencode/T1a-target` reused. First offline resolution lacked an index
   entry; the next invocation used the existing complete vendor directory
   read-only, with unchanged pins/lock versions, and reached rustc.

Exact focused command inside `nix develop`:

```
cargo --config /tmp/opencode/vendor-config.toml test --locked --offline \
  -p spc-lab-txd --lib native_bootstrap
```

Result: exit 101. Three E0599 errors in frozen T1b
`crates/spc-lab-txd/src/actor/pump.rs` at lines 185, 206 and 426 reference
`CommandPhase::Begun`; the real core variant is `Began`. No T2 diagnostic
was emitted, but the test binary was not produced and NO Rust test ran.
Log: `/tmp/opencode/T2-native-check.log`; script and exit receipt use the
same prefix. Compiler fixes were handed to the parent/T1 owner; these
unowned sources remain byte-identical to the verified upstream layer.
Subsequent T2 source refinements have only format/source checks, not another
claim of Rust verification. No runnable T2 binary provenance exists yet.

Parent coordination update: T1 owner `ses_e4282accaffe53atnLNW5jzUw5` is
preparing the hash-bound three-line `Begun` -> `Began` correction and owns
the next guarded warm compiler opportunity. This documentation update does
not apply that correction or launch a competing check. Once supplied, its
before hash must match this frozen R2 layer; only those three lines may be
applied, recorded as a separately bound upstream-layer amendment rather than
T2 logic. No whole refreshed T1 overlay will be imported.

Authored tests cover known header/PoW and odd/even Merkle vectors, BIP34,
strict metadata/raw transaction/witness rejection, fees/overflow/halving,
per-outpoint maturity, complete disjoint allocation and integer rounding,
no-ack/duplicate/old-incarnation/cancel=zero hashes, actual production-loop
SHA256d under an explicit offline host, shared per-block accounting,
unknown/rejected submission retention, expiry/boot mismatch and paused
deadline. Fixture observation markers are not real journal funding credits.

## Serial merge notes and frozen artifacts

Apply only T2 delta against the exact upstream layer, not the worktree's
aggregate git diff (which includes the 700-file source overlay).

- `crates/spc-lab-txd/src/native_bootstrap.rs` and `native_bootstrap/tests.rs`:
  solely T2-owned new files.
- `crates/spc-lab-txd/src/lib.rs`: one module export.
- `crates/spc-lab-txd/Cargo.toml`: one explicit CPU-miner path edge.
- `Cargo.lock`: only `spc-lab-txd.dependencies += spc-cpu-miner`.
- `crates/spc-lab/src/engine/traffic_runtime.rs`: one installer block after
  host launch. Preserve all other T1/OP/ACCESS/profile facts and hunks.
- SPEC: prepend only the T2 changelog fragment. New dated decision and this
  report are packet-owned. No normative pool-body change.
- `flake.nix` vendor hash verification remains a serial integration check;
  no value was guessed and no workspace Nix build was launched.

External `/tmp/opencode/T2-native-manifest-{full,code,delta}.json` record
the exact full tree, code tree (both implementation reports excluded), and
only T2 before/after rows against the immutable 700-row parent layer.
`/tmp/opencode/T2-native-delta.patch` contains the verified narrow delta.
The external receipt carries hashes to avoid report/self-hash circularity.

## Remaining acceptance, precisely (current)

- T2 native funding is now proven in the explicit slice above. Integrate the
  source-bound contextual producer/observer/completion amendment with the
  native executor; the old constant-target consumer cannot verify this chain.
- Full T3 acceptance still requires real fronts/miners, measured nonzero-fee
  pool blocks, independent payout/statistics/pause/reorg oracles and their
  cleanup proofs. This T2 slice does not inherit or claim those outcomes.
- T1-DEST is integrated and node-accepted external P2WPKH/P2TR output sends are
  proven above. Confirmed Ark funding and stock self-hosted server operations
  remain AP-5/T3 work; T4 owns witness inputs. No Ark acceptance is inherited.
