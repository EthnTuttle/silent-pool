# Pool wallet control implementation and source audit

Date: 2026-11-10
Scope: AP5 ordinary-wallet funding, live one-owner executor and read model.

## R4 - stock PID-stamp restart correction (validated candidate)

The same auditor's `/tmp/opencode/pool-wallet-control-source-review-r3.md`
closed S3/S4/S5 and historical proof retrieval, accepted conservative S2,
and found R1: stock creates persistent `LOCK` with `0666 & !umask`, so r3's
blanket 0600 datadir-entry check refused restart. The `dd9f0c37...` r3 packet,
source, tests and sealed binaries remain immutable historical evidence.

The correction shares uid/type/single-link checks with the strict private-file
validator, classifies only the pinned PID stamp as stock root metadata, and
tolerates its stock permission bits under the owned 0700 root. Other files
remain private by default. Root metadata is inspected with `O_PATH|O_NOFOLLOW`:
ordinary I/O open/close would risk releasing an active F_SETLK lock belonging
to this process. The helper does not chmod, read the contents of, truncate, replace or unlink
the stock lock, and controller ownership still precedes Wallet::open.

New regression source uses the public pinned `FcntlPidLockManager` itself.
An isolated child with umask 022 creates the actual 0644 PID stamp; stop and
controller reopen retain its inode/content/mode. While the stock manager is
held in the parent, separate child probes verify actual POSIX exclusion both
before and after metadata checks and a refused second controller open. This
does not rely on the stock same-process registry, which could mask an
accidental kernel-lock release, including during hardlink rejection. Separate negative cases retain strict private
file, symlink, hardlink, FIFO and foreign-UID rejection.

R4 gates passed against the formatted source snapshot
`f764900419db45901f558c367b5485938ed1b20b5ee5abdc039e3b66c230ae0c`:

| Gate | Receipt |
|---|---|
| Format | `/tmp/opencode/pool-wallet-control-r4-fmt.log.receipt.json`, exit 0 |
| Ark all-target check | `/tmp/opencode/pool-wallet-control-r4-check.log.receipt.json`, exit 0 |
| Affected controller/private-guard/proof suite | `/tmp/opencode/pool-wallet-control-r4-tests.log.receipt.json`, 28 passed, 0 failed |
| Strict all-target clippy | `/tmp/opencode/pool-wallet-control-r4-clippy.log.receipt.json`, exit 0 |
| New pool/executor binaries, hash-sealed outside the cache | `/tmp/opencode/pool-wallet-control-r4-build-resume.log.receipt.json`, exit 0 |

The new actual-stock-helper regressions passed, including separate-process
probes after metadata inspection, a refused duplicate controller open and
hardlink rejection. The original source's initial fmt check requested only
formatting, which was applied and frozen before compilation. An outer
120-second harness timeout interrupted the initial binary-build command after
all earlier gates passed. A single process inspection found no surviving
matching build; only the incomplete build was resumed with no harness timeout
under a new nonblocking lock/source/disk check. No passing tests were repeated
for that interruption. The report-only delivery update after these gates is
identified in the final manifest; every executable source row remains the
tested row. The earlier r3 source, scope receipts and binaries are preserved.

R1 disposition: **implemented and regression-verified; ready for the same
auditor's recheck**, not a funded approval. The test uses the real stock lock
helper, not a full live wallet/server restart. Fresh unfunded/funded stock
runtime restart proof remains a joined-runtime check; no unit fixture is
claimed as live. S2's conservative unknown behavior and the proof/grant seams
are unchanged.

The decision and operations docs also name the closed ~150-UTXO observation
limit and deliberate same-wallet/persister emitter/maintenance adapter
divergence. No four-fact or generic-error no-effect release was introduced.

## R3 - tested source-audit fixes and historical proof interface

R3 producer code was validated at source snapshot
`8ec102f55081c58f2ff29aaadb5a78c56c9531a1245a74ffa77b484e077e0e42`.
The r3 delivery packet added only its evidence report after those code gates; its
manifest records the documentation delta and identical tested code rows.

| Gate | Result / receipt |
|---|---|
| S1/S3/S4/S5 and existing Ark behavior | 54 tests passed at `b9086375...`; `/tmp/opencode/pool-wallet-control-r2-final-tests.log` |
| Historical-proof follow-up and all wallet-control regressions | 25 tests passed on `8ec102f5...`; `/tmp/opencode/pool-wallet-control-r3-final-tests.log` |
| All-target Ark feature check | Passed; `/tmp/opencode/pool-wallet-control-r3-check.log`; final all-target clippy also typechecks every target |
| Strict clippy, `--features ark --all-targets -- -D warnings` | Passed on `8ec102f5...`; `/tmp/opencode/pool-wallet-control-r3-final-clippy.log` |
| Formatting | Passed; `/tmp/opencode/pool-wallet-control-r3-fmt.log` |
| Both producer binaries | Built and hash-sealed; `/tmp/opencode/pool-wallet-control-r3-final-build.log.receipt.json` |

The proof follow-up retains a separate immutable public object before the
first confirmed receipt refers to its digest. `board_proof_v1` uses a fresh
read correlation id and the unchanged original board id. It reads retained
bytes after stock refresh/spend/pruning, has no wallet/RPC call, does not
rewrite the original receipt, and returns typed missing/unavailable for
legacy/mismatched/damaged/unconfirmed data. Public policy/anchor/VTXO and the
terminal receipt context are bound by canonical byte hashes. Partial stock
round-import links are captured through the non-deprecated stock `history()`
API and never imply current-float equality from amounts.

The regression suite checks positive default `/var/lib/spc/ark` layout under
a 0755 parent; owned loose directory tightening and unsafe-file refusal;
maintenance/scanning during pending/unknown boards; actual acceptance proof
before releasing refresh protection; fair per-entry corruption/restart
behavior; bounded real loopback RPC and read deadlines; and historical proof
retrieval without wallet calls, including pruned-source, unknown, wrong-id,
legacy-missing-reference, immutable-object and symlink/partial-file cases.

All build invocations used the assigned nonblocking global flock, matching
session priority, the existing primary target, `CARGO_INCREMENTAL=0` and the
50/20-GiB checks. No cache cleanup, pin change, server build or hosted/runtime
inspection was performed. No funded captaind run is claimed. The ladder grant
implementation is still a separate serial union and is not covered by these
standalone control gates. The same source auditor must recheck the corrected
freeze, particularly S1/S3, and AP5 must use correctly joined binaries.

## R2 source-audit rework history

The immutable r1 packet `55a48819...` and its external grant seam note are
preserved. This worktree now contains S1/S3/S4/S5 fixes requested in
`/tmp/opencode/pool-wallet-control-source-review.md`. New regression coverage
includes default datadir startup, protected acceptance-before-refresh,
maintenance during pending/unknown boards, corrupted-entry fairness/restart,
stalled reads, RPC budgets and bounded scanning. The completed validation
evidence is above; historical r1 notes below record the earlier blocked iteration.

The ladder r3 handoff was read without overlaying its source. The one
`BarkWalletClient::operation_lock` and concrete onchain handle are preserved.
The later union must use its `reserve_send_capacity` / `send_authorized`,
`acquire_send_capacity` / `send_arkoor_guarded`, and `SendAuthorization`
post-lock/pre-cosign validation hooks without nested acquisition or regrant.
This candidate's own gates do not test that unmerged grant implementation.

## Source provenance

Owned tree: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-pool-wallet-control`.
Base: `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.
Bootstrap: exactly 682 individually hash-verified rows, 11105307 bytes, from
the immutable `/tmp/opencode/ladder-integration-baseline-src`, using
`/tmp/opencode/ladder-integration-primary-baseline-20261109.json`.
Baseline snapshot: `4561a2df12d9980ee1a16ee285e6667a94d807af7b9c5aa6d30bc81a7ef012b6`.
No primary or other worktree source was copied or modified.

Read authority: AGENTS, README, DESIGN, SPEC frontmatter and §11.3/§11.10,
ARCHITECTURE, WIRE-FORMATS, ROADMAP, MAILBOX, RESULTS and UPSTREAM_DEPS.
AP5's exact consumer contract was read from its owner tree README lines
107-131; integer money/null and queued polling were confirmed by its owner.

## Pinned stock call-phase investigation

Bark commit `c419159fa54085572f8ca72d2ba1716556a9d3eb`; the checkout's tracked
files are clean. The offline vendor copies match these checkout hashes:

| File | SHA256 |
|---|---|
| `bark/src/board.rs` | `ec9139eb5aa8c5d59d47f16d24b483e50a4ee3904ccc683c4d4ee183b5090143` |
| `bark/src/actions/board.rs` | `08d3c819fb56d0c2e81585bd69d1a8a1da0aaaa3e1002569ddb9552ed0a18ed0` |
| `bark/src/onchain/bdk.rs` | `f0305ec309739415d081a01008b233e48cae34bd76011ba0c48bb726e1bd24e4` |

- `board.rs:117-146`: stock board prepares with the onchain wallet, enforces
  the funding-input cap, signs without registering, then calls `board_psbt`.
- `board.rs:248-269`: actual board amount minimum and board fee; standard
  `RequestBoardCosign`. Server cosign occurs before any funding broadcast.
- `board.rs:276-316`: verifies/builds the signed VTXO, creates movement,
  checkpoints `Board::Broadcasting` with funding PSBT and signed VTXO.
- `board.rs:325-331`: initial drive errors are swallowed after the durable
  checkpoint. Therefore `Ok(PendingBoard)` alone is not submission proof.
- `actions/board.rs:227-278`: stock stores the locked VTXO, probes the funding
  tx and broadcasts through its chain source. Unknown stays pending.
- `actions/board.rs:286-386`: waits for actual confirmations, registers,
  retries/retains locks on registration errors, salvages near expiry.
- `board.rs:39-49,96-114`: public pending-board read/recovery; this is the
  recovery surface, not a hypothetical server GetByOrderId.
- `onchain/mod.rs:140-155`, `onchain/bdk.rs:214-255`: `sign_psbt` versus
  `finish_psbt`; use sign-only, never finish/register before the board action.
- `lib/src/lib.rs:53-119`: ArkInfo advertises min-board and max-VTXO but not
  server `max_board_amount`. The server's own cosign policy remains required.

## Implementation

New isolated module `bins/spc-pool/src/ark/wallet_control.rs` and native CLI
`bins/spc-pool/src/bin/pool-wallet-executor.rs`. Small shared hooks expose the
same concrete onchain wallet, add one operation mutex, hold it across sends
and automatic ticks, check pending-board capacity, and replace the old
uncoordinated automatic task with the controller. No aggregate-order paths
or authorization logic are changed. A narrow Nix install assertion ensures
the actual client is present in the joined Ark-tools package.

The receipt journal is not a wallet ledger. Queued is not submitted;
submitted is not confirmed; configured target is not a balance. Exact
request/result schema and recovery gaps are in `POOL-WALLET-CONTROL.md`.

## Validation status

- Initial check used the default Cargo cache and stopped before compilation:
  missing cached `ar_archive_writer 0.5.3`. No pin/lockfile version changed.
- Corrected command uses existing `/tmp/opencode/vendor-config.toml`, the
  immutable baseline Nix development shell, and the assigned shared
  `/home/ethan/code/silent-pool/target`. Its first attempt could not acquire
  the global nonblocking build lock; no waiting or polling was performed.
- The permitted offline `check --all-targets` reached this module and found
  two API spelling issues (`ark::address::ArkId` and the explicit
  `ProtocolEncoding::serialize` call). Both are corrected in source. Its
  before/after source manifests and compiler log are
  `/tmp/opencode/pool-wallet-control-check-r3.log*`; that check did not pass.
- `cargo fmt -p spc-pool -- --check` passed after module-order correction
  (`/tmp/opencode/pool-wallet-control-fmt-r2.log`). Final formatting check also
  passed: `/tmp/opencode/pool-wallet-control-fmt-final.log`, exit 0.
  The packet's later source refinements passed the same formatter check in
  `/tmp/opencode/pool-wallet-control-fmt-packet.log`, exit 0.
  `nix-instantiate --parse flake.nix` passed (syntax only, not a closure build):
  `/tmp/opencode/pool-wallet-control-nix-parse.log`.
- Ten focused tests cover real private queue/receipt files, immutable replay,
  a pre-stock-checkpoint crash, a lost broadcast reply and reopen, stock
  pending-capacity coordination, read-only snapshot/no-op behavior,
  insufficient/stale observations, exact fee inversion, path attacks and a
  real loopback Bitcoin RPC fixture (canonical tip, amount and coinbase
  maturity proofs). Wallet lifecycle states in these tests are explicit
  fixtures, not funded-server evidence.
  The existing automatic-board failure test is also updated to require a
  retained outpoint claim rather than generic error-based release.
- Focused test invocation was blocked before starting Cargo when the shared
  target/store filesystem had **43.36197280883789 GiB** available, below the
  required 50 GiB (`pool-wallet-control-test-r2`, exit 76). No retry below the
  floor, second target, cleanup or garbage collection was performed.
  Tests, final compile, clippy and executable build remain unexecuted pending
  a permitted shared build slot with sufficient disk.
- No funded captaind run is claimed. AP5 owns the real native 120M funding,
  both-node confirmation and observed 115M float proof on joined source.

The conservative pre-stock-checkpoint crash gap remains unknown rather than
inventing recovery. The signed proposal is retained, automatic new boards
are held, and mining has no dependency on this queue.

## Serial integration hooks

The packet carries diffs against the immutable 682-file baseline, not against
the ladder owner's moving files. Merge the new module/CLI directly and map
these small shared hooks into the joined `StockHandle`/wallet code:

1. Retain the existing concrete `Arc<RwLock<OnchainWallet>>` for read-only
   checkpoint/owned-UTXO inspection, while giving stock Wallet the same Arc as
   its trait-object onchain handle. Do not create another wallet.
2. Add the shared `operation_lock`. Every full send path (including the
   ladder owner's replacement send path) must take it before key selection
   through final persistence. Manual/automatic maintenance holds this same
   lock. Do not nest a second acquisition inside these calls.
3. Add `BarkClient::board_capacity_available` and the per-output automatic
   loop check. The stock implementation uses real pending board checkpoints.
   Keep the narrow automatic-error change: retain the outpoint claim on a
   generic error, stop that tick, and leave accepted boards to stock recovery.
4. Acquire `Controller::open(datadir)` before wallet open, use its guarded
   startup reconciliation, and spawn `Controller::run` instead of the old
   independent board loop. The new module has no aggregate authorization API.
5. Keep the Cargo binary/feature/default-run and existing-rustix membership
   additions; keep the Nix Ark-tools install assertion. Apply the SPEC fragment
   serially, alongside the dated decision, without altering other decisions.

Next validation command, once disk and global slot permit:

```text
flock -n -o /tmp/opencode/silent-pool-heavy-build.lock python3 /tmp/opencode/pool-wallet-control-build.py /tmp/opencode/pool-wallet-control-test-next.log test --offline --locked -p spc-pool --features ark --lib wallet_control::tests -- --nocapture
```

Also run `ark::board::tests::a_failed_board_retains_the_decision`. Then
check/clippy the joined Ark feature targets and build both `spc-pool` and
`pool-wallet-executor` from that same source before AP5 consumes them. A
source-only packet or formatter pass is not funded readiness.
