# T2 native bootstrap, contextual difficulty and traffic destinations: primary integration

Date: 2026-11-10
Owner: serial primary Rust/Cargo integration writer (sole writer for this
packet; other agents isolated in their own worktrees)
Status: complete; primary selected code frozen after the checks in section 5

Packet: the combined T2 (native bootstrap funding) + contextual native
difficulty + T1 traffic-destination extension layer, applied on top of the
packet-1 (T1/J1) primary state.

Candidate authorities (all sealed before transfer):

- Application patch `/tmp/opencode/T2-T1DEST-combined-from-J1.patch`
  (file SHA-256
  `acb9327befefff4a95a03f367780f08bfaca46730b9da6bb11653980e8defbc9`,
  31 paths against the J1 final primary state).
- Frozen full manifest `/tmp/opencode/T2-native-manifest-full.json`
  (file SHA-256
  `fec013468bb81e3d154a00972ccf3343733c3a86156086742bd75ff1f85dde18`,
  719 records, aggregate snapshot
  `9c8f9511774d4e3b77d828cff99ab4958f8c75363245c21717fa298ead6d53ee`,
  source worktree
  `/home/ethan/.local/share/opencode/worktree/ad8886/lab-native-bootstrap`,
  base revision `b0ab4ca34fda07c61a6335d05f598eaacc69678c`, freeze status
  "combined scoped checks and native bootstrap passed; external P2WPKH/P2TR
  raw transactions accepted, unconfirmed; full T3/Ark unclaimed").
- Sealed verification receipt
  `/tmp/opencode/T2-T1DEST-verification-receipt.json` (file SHA-256
  `eb8b4bb66c8537492ab6365cdb54c65015f04b89a96ee2b39f7c6439a2083e04`).
- Layer chain receipts: `/tmp/opencode/T2-T1DEST-layer-receipt.json`
  (`b03d33206186ab69c3020f25d09a8d7e2406892c8a248f033c4d993e238485cc`),
  `/tmp/opencode/T2-native-final-J1-layer-receipt.json`
  (`e4c90860fb14a04f6fb9a79bbd2a7692d63b5ca9f7a6c5445c6ac1190ebc4d0d`),
  `/tmp/opencode/T2-native-executable-source-receipt.json`
  (`90a37d84552c340e62caf7af442bdb1a601038648a3ed855027bb768502a3d7a`).

External manifest:
`/tmp/opencode/t2-native-primary-integration-manifest.json`.

This document is the sole unapproved evidence record of the transfer. It is
not a behavior change.

## 1. Pre-write verification (all before any write to primary)

- Manifest re-hashed; 719 records; the 31 patch paths' rows verified
  against the frozen candidate worktree bytes before transfer.
- Primary pre-state: all 13 added paths absent; the 18 modified paths at
  the packet-1 (J1 final) integrated state.
- Index hash before any edit: `git write-tree` =
  `44f21a926d4d065ac6937fb0f81b698c7b2d0627`, equal to `HEAD^{tree}`
  (nothing staged). Nothing was staged, reset, cleaned, stashed or
  committed at any point.
- The 719-record snapshot is the source-tree authority; only the 31 patch
  paths are in this packet's verification scope. Primary carries additional
  parent-side state outside the patch (packet-1 precedent), so full-tree
  equality with the snapshot is not expected and was not the criterion.

## 2. Transfer (functions tools only, narrow hunks)

- **13 new files** written byte-exact with the write tool and
  hash-verified against the manifest: `contextual_pow.rs` and its test
  module, `native_bootstrap.rs` and its test module,
  `tests/common/destinations.rs`, `tests/destination_outputs.rs`,
  `tests/traffic_destinations.rs`, the changelog fragment, three decision
  records and two review records (full table in the external manifest).
- **18 modified rows** applied via per-hunk edits, each ending hash-equal
  to its manifest row: `Cargo.lock`, `crates/spc-lab-txd/Cargo.toml`, the
  `spc-lab-txd` sources (`actor.rs`, `adapter.rs`, `admit.rs`, `builder.rs`,
  `error.rs`, `lib.rs`, `observer.rs`, `tests.rs`, `wallet.rs`),
  `tests/actor_control.rs`, `engine/traffic_runtime.rs`, the traffic
  controller files (`commands.rs`, `spec.rs`) and the two controller test
  suites (`traffic_completion.rs`, `traffic_contract.rs`); `docs/SPEC.md`
  applied semantically (section 3).
- **Four transcription errors were caught by post-edit hash-diff and fixed
  within the same path** (each final file then hash-equal to its row):
  `traffic_runtime.rs` (a dropped `.to_string()`),
  `contextual_pow/tests.rs` (a miscopied `0x200fffff` constant),
  `native_bootstrap.rs` (a `[0; 64]` array literal), and one dropped word
  in `docs/reviews/2026-11-08-native-bootstrap-implementation.md`
  ("interpret" restored to "interpretation", caught in the final
  verification pass).
- Final state: 30 of the 31 patch paths verified sha256-equal to their
  manifest rows; `docs/SPEC.md` verified per its documented semantic
  exception (section 3). Re-checked after the section 5 gate.

## 3. Documented exceptions (2)

- **`docs/SPEC.md` - semantic application.** Primary SPEC equals the T2
  candidate SPEC row
  (`5d286094170745fea5ef1cabfb74ab8a27d07e79ad86893557c725e170c9f551`)
  plus exactly the 12 AP4-era lines documented in packet 1 (1 frontmatter
  changelog line + 11 self-hosted-lab-Ark body lines), with zero
  candidate-only lines; re-verified mechanically post-transfer (diff = 12
  primary-only lines, no deletions). The full T2 SPEC delta is therefore
  present in primary. Final primary SPEC SHA-256:
  `69d58739c20c5acc1dcfe968e2950045f12fc6681fb8ad557afd53c8952f87d4`.
- **`crates/spc-lab-txd/src/keys.rs` - documentation-only correction**
  (QA finding, 2026-11-10). The T2 manifest row
  `c67e8e561e9a28905e4af66d8cee2a2a185f7c61a8e2e5a047dfc26cf6071a32`
  is preserved as the historical oracle binding: the sealed-run QA proved
  189 funding + 6 change scripts against that exact source. Primary now
  reads
  `35164cd9534b9eec2cff40a95758c45ecc9471c969aecc7235d7003918697eed`:
  header comment line 21 was corrected from `m/0'/{index}` /
  `m/1'/{index}` to `m/0'/{index}'` / `m/1'/{index}'` to match the actual
  double-hardened derivation (both levels hardened, source lines 324-326).
  Derivation logic, key version and golden vectors are untouched. No heavy
  rerun was performed for a comment-only change; the row is recorded as
  documentation-only in the external manifest.

## 4. Vendor-hash verification

The T2 `Cargo.lock` delta adds only the `"spc-cpu-miner",` line to the
`spc-lab-txd` dependency list. `spc-cpu-miner` was already a workspace
package block (line 4044) whose external dependencies were all already
locked; no new external crate or version entered the lock, so the
`fetchCargoVendor` set is identical and the flake vendor hash
`sha256-PvxybBOJ1f4X+nkf6DQCdHM6Le9Dr67v3nhoTdrLYBM=` is unchanged.
Verified once by lock-set comparison; no Nix rebuild was started.

## 5. Scoped merged checks

All checks ran under `flock -n -o /tmp/opencode/silent-pool-heavy-build.lock`
(never `-w`, no polling, no auto-retry; acquired at 65 GiB free, above the
50 GiB floor; 63 GiB free at completion) using the existing primary
`target/` directory. Logs: `/tmp/opencode/T2-primary-gate-20261110/`
(`txd-full.log`, `controller.log`, `lab-lib.log`, `fmt.log`, `clippy.log`,
`clippy-r2.log`). The broad 903-test storm was not repeated.

| Check | Result |
|---|---|
| `cargo test --locked --offline -p spc-lab-txd` (full crate) | 14 result lines, 12 non-empty suites all 0 failed, 2 ignored (pre-existing), 122 passed total; lib suite 53 (incl. the 7 contextual and 17 native tests); harness suites 26+2 ignored, 12, 6, 6, 5, 5, 3, 3, 1, 1, 1 (one with 27 filtered out); 2 empty suites; doc-tests 0 |
| `cargo test -p spc-lab --test traffic_completion` | 1 passed / 0 failed |
| `--test traffic_contract` | 49 passed / 0 failed |
| `--test traffic_destinations` | 1 passed / 0 failed |
| `cargo test -p spc-lab --lib` | 58 passed / 0 failed |
| `cargo fmt -p spc-lab -p spc-lab-txd -- --check` | clean |
| `cargo clippy --config /tmp/opencode/vendor-config.toml --locked --offline -p spc-lab -p spc-lab-txd --all-targets -- -D warnings` | exit 0; no code findings; only the pre-existing sqlx-postgres future-incompat notice |

Invocation note: the first clippy attempt placed `--config` before the
subcommand; the external clippy subcommand does not inherit a global
config option and failed offline resolution on `ar_archive_writer`
(candidate 0.2.0 vs locked 0.5.3). The corrected placement (config after
the subcommand, the T2 worktree gate precedent) passed. This was an
invocation error, not a code finding; both `ar_archive_writer-0.5.3` and
`psm-0.1.32` are vendored.

Post-check invariants:

- 32-path pre/post hash file
  (`/tmp/opencode/T2-primary-gate-20261110-pre-hashes.json`, the 31 patch
  paths plus keys.rs): 0 changed after all checks.
- 30 of the 31 patch paths sha256-equal to the frozen manifest rows;
  `docs/SPEC.md` per the documented semantic exception (section 3).
- Index hash before == after all edits and tests:
  `44f21a926d4d065ac6937fb0f81b698c7b2d0627`.

## 6. Sealed run facts (the evidence this packet integrates)

Case `/tmp/opencode/T2-native-funded-20261110-context-dest-v2` (receipt in
the candidate-authority list):

- Native funded acceptance TRUE on a real slice: 2 actual nodes, 2 wallet
  actors, real controller/node/probe; storage, front, miner and pool are
  explicit INERT fixtures in this slice.
- Bootstrap: 101 funding blocks, native mining mode, block height 105 at
  completion; 4 distribution transactions (fees 907 + 907 + 907 + 771 =
  3492 sats total) verified by the independent read-only oracle; bootstrap
  journal artifact SHA-256
  `2c735220daa272fa0de14244bdde9285f270cc2ecfd780abbdfdd4642af78263`.
- Hash budget: measured 1809 vs charged 1810 hashes at the configured
  40 H/s, 1798 batches, 128950 ms phase duration; funding observations
  149 (txd-0) + 40 (txd-1).
- External destinations: one P2WPKH (fee 672 sats, 223 vB) and one P2TR
  (fee 708 sats, 235 vB) spend of distribution outputs, node-accepted on
  both nodes, durable replay without duplicate send - both UNCONFIRMED.
- Cleanup: all 14 owned identities gone, durable stop succeeded.
- Independent verification (T3/T4b QA97, parent note 2026-11-10): the
  3492-sat fee total, the maturity-bound allocations and all hash budgets
  were re-verified independently against the sealed artifacts.

## 7. AP5 execution-dependency handoff (funded stock-Ark test)

For the funded stock-Ark executor (isolated `testing/lab-ark-funded/**`,
headless real APIs). These are execution dependencies sourced from sealed
receipts; this is not a request to rebuild the unchanged node or captaind,
and the active frozen invocation is preserved.

CORRECTION (2026-11-10, parent verification, re-verified in session): the
closure in 7.1 below is the OLD UNFUNDED AP3b baseline, NOT a corrected
joined funded execution closure. Six wallet-agent source rows differ from
the fixed AP3r2 manifest (R1-R6 audit fixes; table in 7.1). It is
HISTORICAL UNFUNDED ONLY: never use its `spc-agent` for funded tests, and
its CLI also predates the current T2 integration (no `spc-lab-txd`), so
merely rebuilding txd is NOT sufficient for a full join. The actual AP5
handoff is a NEW source-bound joined closure (CLI + txd + pool-ark +
agent-ark + miner + scanner) built from the manifest-verified combined
source after the AP3r2 and provider integrations pass their merged
checks (section 10 of the AP3r2 integration receipt). The pre-correction
version of this receipt had SHA-256
`e0b6caee1a3d9ad6dd9b5953829e31c173021ad5444daa2b59c8d0a812cbb409`
(preserved as the historical record; the correction changes only this
section's status labels and semantics, no other section).

### 7.1 Historical unfunded Nix closure (CLI, pool-ark, agent-ark, miner, scanner) - NOT for funded execution

- Output:
  `/nix/store/w52v4f6sr6pz32410y9la8sjkk02adga-spc-lab-ark-tools-0.1.0`
- Deriver:
  `/nix/store/bgzns420bafnhlbv2yhhjzng5yl4gs5q-spc-lab-ark-tools-0.1.0.drv`
- Source input: `/nix/store/vm95bb22061a626nd1si133xc9prjz0r-source`
- cargoBuildFlags: `-p spc-lab-cli -p spc-pool -p spc-cpu-miner -p
  spc-scan -p spc-agent --features spc-pool/rpc,spc-pool/ark,spc-agent/ark`

| Binary | Path | SHA-256 |
|---|---|---|
| spc-lab (CLI) | `<closure>/bin/spc-lab` | `e0f5897b5405a2a3b10887473c32e02c843e0f6be20f3ff6275153dd0a349e2a` |
| spc-pool (rpc,ark) | `<closure>/bin/spc-pool` | `7d0ab78af64e0c817ce34c3e0ce5cb0c52ce5e31cbb0ea50c6c41cd986d76aec` |
| spc-agent (ark) | `<closure>/bin/spc-agent` | `3fa96baa52a9a800b9ed0f96431be0d68714868b1d6f9fee84aa42d3db5526e5` |
| spc-cpu-miner | `<closure>/bin/spc-cpu-miner` | `0712d0f95194201ffc7e2af1b908bfadde3013e447a271a794e9ebd3a1d31d54` |
| spc-scan | `<closure>/bin/spc-scan` | `2956d3f1d210a903580b28fd9b26f79275380e9d20a3e1d8739b79c016407b3e` |

Wallet-agent source status vs the fixed AP3r2 manifest
(`0b0c5fd7d5b52e314a0a1aec818b13eb9d134aafa2ca93734310bed028949468` /
code `a270582ddfe2e0a20f11de5316b67b913f3a5a6404419f029633bd19b63a01a3`);
parent-verified and re-verified in session against
`/nix/store/vm95bb22061a626nd1si133xc9prjz0r-source`. Of the 15 wallet
source rows, SIX differ (the known R1-R6 audit-fix scope); the other nine
are byte-equal:

| Wallet source row | Closure source (old, keep as reference) | Fixed AP3r2 manifest |
|---|---|---|
| `wallet/audit_regressions.rs` | MISSING in closure source | `d59b3d81ec11176b05bf0ce0da697dc628a3ab4a54901445e14e31c8fadb2777` |
| `wallet/live_receive.rs` | `6cbf2fb397e1e7c06aa639ebb88c9bbdccff14100cee65338bca2c99f7df2440` | `867099a627ba3d4a1a7b58b99003ff86a07aadc7af8994c7cc8b38f798888247` |
| `wallet/maintenance.rs` | `c1e5d79e8719ee526d39d315c986683cdd96013da7a2882388c733393d96c759` | `d2fd32554a2b7132c462691d9b7c223f6cf25c8f3e765cc82d8e7a259521cc09` |
| `wallet/mod.rs` | `845d2b74fb4700b0433a928261377616ce028272f99a9be6c92a2f44e4c4bc55` | `50a305977e31980665dbfd2c4d81daad1f0e741f3a592ca90f6c58df5b4a1c3c` |
| `wallet/round.rs` | `45a3509d81562f805060fa438de6e4b429d7e948f56687a8f4968d0f5767cdc5` | `10da12d69435470e4920fa8dfb8bbfbf64f24918f4dd7b3597957b23f8eac3f6` |
| `wallet/scan.rs` | `acd06fe24ed3f7ce11f0821d83d2edb0baa00bc80ed7a1092969dfad7c3009f1` | `19b4b33b1d728a6e3effce3e1fdd5f0cc90c00b360c1f22fdf71fd4d7f2434dd` |

Old source and binary hashes are kept as reference only. No delete and no
rebuild of the unchanged node or captaind.

- This closure does NOT package `spc-lab-txd`, and the flake's
  `spc-lab-txd` package has no built store output. The sealed txd
  authority is the T2 sealed-run debug binary, SHA-256
  `173af943f1b485faf03ce0818f97400cd9563310b5c19d4b15cff6fe085755ca`
  (recorded in the verification receipt). DRIFT WARNING: the current
  `/tmp/opencode/T1a-target/debug/spc-lab-txd` now hashes
  `2652040bffd79ef1b88267578390182ade94b82a1944286844a7f825682cc823`
  (later rebuilds by other sessions; `spc-lab` there still matches
  `6bdf7f90...`, but `spc-lab-fixture` has drifted to `c9f8cd59...`). A
  funded run needing txd must rebuild from a manifest-verified source or
  use a hash-verified preserved copy; do not assume the shared debug
  directory still holds the sealed bytes. The closure CLI also predates
  the current T2 integration as a whole, so merely rebuilding txd is NOT
  sufficient for a full join; the actual AP5 handoff is the new
  source-bound joined closure from the manifest-verified combined source
  (correction note above).
- Two other ark-tools closures exist (`0c52hh...`, `kkd5y9...`, without
  `spc-agent`); `w52v4f6...` is the sealed one bound to the AP3b
  unfunded runs.
- Joined runtime launcher:
  `/nix/store/sslr7nzw0gp3qzb68is0p44aiqpn7a74-spc-lab-self-hosted-ark/bin/spc-lab-self-hosted-ark`.

### 7.2 Node and Ark server (unchanged; do not rebuild)

- Release node:
  `/nix/store/03pw2rf9wrxxnd9rl0l8ln0mnw9gcpx2-rbitcoin-node-ark-lab-0.8.99-profile-facts/bin/rbitcoin-node`,
  SHA-256
  `bba4752298aff258adb6cce99c6db849b02f50736a8ec32766ed1710045051aa`
  (byte-identical at
  `/nix/store/lbpv9zs0mdp36f912k63a31pg9qrzi54-rbitcoin-node-ark-lab-provenance/bin/rbitcoin-node`,
  verified equal today).
- Debug-profile node (the one the T2 sealed run used):
  `/tmp/opencode/node-profile-facts-r2/nix-gates-ea9TcMH8/rbitcoin-node-candidate-36e2387ada164e2c89b3d20f5d515bf19c33f30fb927e8ca11621eba3658d7d9`,
  SHA-256
  `36e2387ada164e2c89b3d20f5d515bf19c33f30fb927e8ca11621eba3658d7d9`.
- captaind (stock Bark c419159):
  `/nix/store/4mnskprw4qigq8i8xqf4pps7iak1bnly-captaind-0.7.1-c419159/bin/captaind`,
  SHA-256
  `4cac675bccc017608378382129613ed5202dd90e7300a92c4ed799869abaef9d`.

### 7.3 T2-style funded startup (funding barrier)

Actual sealed invocation (from `/tmp/opencode/T2-native-funded.py`), with
`<BIN>` the binary directory and `<case>` the run case directory:

```
SPC_LAB_BASE_REVISION=b0ab4ca34fda07c61a6335d05f598eaacc69678c \
SPC_LAB_DIRTY_TREE_HASH=<provenance source_sha256 of the actual source tree> \
<BIN>/spc-lab run \
  --scenario <case>/scenario.json \
  --config <case>/config.json \
  --run-dir <case>/run \
  --headless --keep --listen 127.0.0.1:<port> --duration-secs 4000
```

At seal time `<BIN>` = `/tmp/opencode/T1a-target/debug` (spc-lab
`6bdf7f90...`, spc-lab-txd `173af943...`, spc-lab-fixture `58ca6e94...`;
see the drift warning in 7.1).

Funding barrier (scenario, exact sealed values):

- `traffic.bootstrap`: `enabled` true, `mining_mode` "native",
  `funding_blocks` 101, `distribution_outputs_per_wallet` 8,
  `excluded_from_statistics` true.
- `traffic.driver.actors`: `txd-0` (4 network wallets, 1 user payment
  wallet, `hosts_bootstrap_funding` true) and `txd-1` (4 network wallets,
  1 user payment wallet, hosts false); bootstrap budget 40 H/s,
  `total_attempts` 4194304, deadline 3600 s, cancel grace 30 s.
- Network consensus profile: pow limit `207fffff`, initial target
  `200fffff`, no retargeting, coinbase maturity 100.
- One phase, one command: `bootstrap_fund` on driver `txd-0` / node
  `node-a`.
- Ark disabled in the T2 slice with `registration_before_mining` true
  (the funded Ark test enables ark; see 7.4).
- Node args: `--network regtest --regtest-pow-bits 200fffff --no-seeds
  --no-discover --max-outbound 1` with a paired `--connect`.
- In the T2 slice, storage/front/miner/pool/redis/postgres are INERT
  fixtures; the traffic probe is the frozen r4 producer probe (contract
  `754e1e61...`).

### 7.4 Registration barrier (AP3b precedent)

Actual sealed invocation (from `/tmp/opencode/AP3b-unfunded-fixture.py`;
`<tools>` is the 7.1 closure):

```
<tools>/bin/spc-lab runtime \
  --scenario <root>/scenario.json --run-dir <root>/run \
  --node <release node> --pool <tools>/bin/spc-pool \
  --miner <tools>/bin/spc-cpu-miner --wallet-agent-bin <tools>/bin/spc-agent \
  --captaind <captaind> --grpcurl <nix grpcurl> \
  --postgres-bin-dir <nix postgres 17 bin dir> \
  --redis-server <nix redis-server> --redis-cli <nix redis-cli> \
  --pow-bits 0x200fffff \
  --base-revision b0ab4ca34fda07c61a6335d05f598eaacc69678c \
  --dirty-tree-hash <source ingestion hash> --duration-secs 30
```

with `SPC_LAB_PORT_LOCK_ROOT=<root>/port-locks`. The scenario is
`crates/spc-lab/scenarios/lab-ark-wallets.json` from the AP3b
lab-ark-wallet-bridge source (the next packet to integrate). Its
`ark_backends` entry is `local-ark` / captaind, self-hosted, bound to
`node-a`, with `vtxo_lifetime` 432, `max_vtxo_exit_depth` 100,
`required_board_confirmations` 3, `min_board_amount_sats` 20000,
`round_interval_secs` 10. The unfunded precedent stripped miners and
observed four wallets registered and connected with live registration via
the agents' `/status` HTTP endpoints. Union startup semantics (corrected
2026-11-10; supersedes any "wallets before any mining" reading): cold
provision and front/agent registration MAY run before native funding -
the native bootstrap itself must mine to fund - and registration is
required before USER miner activation, not before all mining. The
MINER AddConnection step waits for BOTH the current verified registration
AND funding completion. The exact union scenario is 4 SP wallets, 4
miners, 8 network wallets, 2 nodes, 2 pools, 40 H/s and 0.2 tx/s; the old
AP3 small 8 H/s fixture is a per-run precedent, not the global target.

## 8. Scope and limits

- **T2 native honesty**: the sealed run is a two-node, 105-block, 40 H/s
  slice with storage/front/miner/pool inert; measured 1809 vs charged 1810
  hashes; the 4 distribution transactions are mined and oracle-verified;
  the two external P2WPKH/P2TR sends are node-accepted but UNCONFIRMED. No
  Ark funding, no witness spends (T4), no full T3 and no funded-pool claim
  is made by this integration.
- **J1/J2 honesty retained**: `traffic_j1_actual` still carries 1 ignored
  test; the J2 fresh-harness scope is not claimed.
- T2 test sources in the producer copy remain private evidence artifacts;
  budget authority stays in the existing CommandLog.
- No live run was touched: the sealed live controller (199214) and the
  parent inspection run (1441192) were not accessed; packaged Nix outputs
  were only read (hash checks), never rebuilt or garbage-collected.
- `docs/lab-build-report.html` is parent-owned and was not touched.

## 9. Freeze and queue

The primary selected code (the 31 patch paths of section 2 at the
section 5 verified hashes, plus the keys.rs documentation-only correction
of section 3) is frozen as of the completion of the section 5 checks.
Receipts: this document, the external manifest, and the sealed artifacts
cited above. Index unchanged:
`44f21a926d4d065ac6937fb0f81b698c7b2d0627`.

Queued next (not integrated here): the AP3b fixed r2 packet, the AP3 UI
read-model provider packet, and the corrected live wallet UI packet once
the UI author delivers the W1-W4 fixes (its eventual receipt must report
async review; no approval inferred).
