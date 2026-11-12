# Fable source-only audit, r4 narrow recheck (R1): pool wallet control packet

Date: 2026-11-10
Reviewer: Fable (same auditor; r1 and r3 reports preserved at
`/tmp/opencode/pool-wallet-control-source-review.md` and
`/tmp/opencode/pool-wallet-control-source-review-r3.md`)
Packet: `/tmp/opencode/pool-wallet-control-packet-r4/` (schema
`spc.pool.wallet-control.source-packet/v4`), delivery snapshot
`d00299c259aaa5e8098542f48f5c3c90c66737be3f965f5d65e5fd2a763914b2`, tested
code snapshot `f764900419db45901f558c367b5485938ed1b20b5ee5abdc039e3b66c230ae0c`,
predecessor r3 `dd9f0c37...` preserved.
Disposition: **SOURCE AUDIT - APPROVE (bounded)**. The single r3
counterexample R1 is closed by a minimal, correctly scoped change with a
genuine cross-process regression test; S3/S4/S5/`board_proof_v1` code is
untouched except the shared `check` helper split, which is
semantics-preserving. Source reading and receipt verification only: I ran
nothing, took no lock or build slot, and wrote no candidate, primary, cache,
pin or server file. This file is the only artifact. No funded, live-restart
or grant-union claim is made or implied.

## 1. Freeze and receipts (verified)

- `source-manifest.json` (690 rows) reproduces `d00299c2...` with the
  manifest row order; all 690 rows hash-match the worktree, and each of the
  5 changed rows hash-matches the packet `source/` copy.
- `/tmp/opencode/pool-wallet-control-r4-formatted-source.json` reproduces
  the tested hash `f7649004...`; the only row differing between tested and
  delivered is `docs/reviews/2026-11-10-pool-wallet-control.md`, so every
  executable row the receipts tested is the row delivered.
- Delta from r3 is exactly five files: one production Rust file
  (`bins/spc-pool/src/ark/wallet_control.rs`, now `53f5e789...`) and four
  docs. `wallet_control_io.rs`, `wallet_control_proof.rs`, `bark_stock.rs`,
  `board.rs`, `main.rs`, Cargo and Nix rows are byte-identical to r3.
- Five gate receipts (`fmt`, `check`, `tests`, `clippy`, `build-resume`)
  bind `f7649004...`, `source_unchanged: True`, exit 0; the tests log shows
  `28 passed; 0 failed` (focused filter, `258 filtered out`), i.e. the r3
  25 plus the three new lock tests. Two sealed binaries with `--help`
  exit 0. Not re-run by me.

## 2. R1 closure - read against the pinned stock lock manager

The production hunk is small and exactly targeted:

- `prepare_datadir` still establishes the owned root (`mkdirat 0700` for
  missing components only, no chmod of existing ancestors, uid check,
  `fchmod 0700`, `fsync`), unchanged from r3 - the original S1 case
  (`/var/lib/spc` 0755, fresh or loose datadir) stays fixed, and
  `Controller::open` still precedes `load_seed`/`open_bark_wallet`.
- Root entries are now opened `O_PATH | O_NOFOLLOW | O_CLOEXEC` and
  dispatched to `check_wallet_root_entry`:
  - name == `bark::lock_manager::pid_fcntl::LOCK_FILE` (the pinned public
    constant, i.e. `"LOCK"`): `check_owned_type` (same uid, regular file,
    `nlink == 1`) plus `mode & 0o600 == 0o600 && mode & !0o666 == 0` -
    owner read/write required, no exec/setuid/setgid/sticky bits, group and
    world bits tolerated inside the already-0700 root. Matches exactly what
    `FcntlPidLockManager::new` produces (`File::options().read.write.create`,
    `0666 & !umask`, `pid_fcntl.rs` 47-54). No chmod, truncate, read,
    replace or unlink of the stock file.
  - every other entry: the unchanged strict rule (exact `0600` regular
    single-link, or exact `0700` directory, same uid). `PrivateDir::check`
    is now `check_owned_type` + exact-mode `ensure!`, with the same
    acceptance set as r3 for every non-`LOCK` path.
- The r3 counterexample sequence (first start creates `LOCK` 0644; restart
  refuses) is therefore gone, and nothing else in the datadir policy was
  relaxed: `wallet.sqlite*`, `provision.json*`, unknown root files,
  `wallet-control/` children, faults, proofs and lineage all keep strict
  checks (the r3 private-queue tests still assert a 0644 request is refused).

**POSIX-lock preservation.** The switch to `O_PATH` is correct and
necessary: Linux `filp_close()` skips `locks_remove_posix()` for
`FMODE_PATH` files, so inspecting `LOCK` (or a live SQLite file) through an
`O_PATH` descriptor and closing it cannot drop this process's fcntl record
locks, whereas r3's ordinary `O_RDONLY` open-then-close would have. The
author's comment states this accurately.

## 3. The tests are genuine, not a manual touch

- `stock_pid_lock_subprocess` + `stock_pid_lock_child` run the test binary
  itself as a child with `--exact` on that test name. Action `create`
  forces `umask 022` in the child and calls the real pinned
  `FcntlPidLockManager::new`, so the `LOCK` file is produced by the stock
  writer (asserted `0644`), not by `std::fs::write`. Action `held` asserts
  the child sees `PidLockError::AlreadyHeld` and that the file contents
  (parent PID stamp) are unchanged - a true cross-process `F_SETLK`
  exclusion probe.
- `stock_0644_pid_stamp_survives_stop_reopen_without_replacing_or_unlocking_it`:
  `Controller::open` + `prepare_wallet_database` -> stock creator child ->
  `LOCK` 0644 with recorded inode/bytes -> drop controller -> reopen
  succeeds (the r3 failure point), inode and bytes unchanged -> the test
  process now holds the real stock lock -> child `held` -> `prepare_datadir`
  (the O_PATH inspection) -> child `held` again (lock NOT released) ->
  second `Controller::open` refused (owner lease exclusion) -> hard-link
  alias makes `prepare_datadir` fail (`nlink == 2`) while the lock stays
  held -> dev/ino/nlink/mode identical to the initial metadata -> clean
  reopen cycle. This is exactly the before/after guard I asked for.
- `stock_metadata_compatibility_does_not_relax_secrets_aliases_or_control_files`:
  0644 `mnemonic` refused; `LOCK` as a hard link to a secret refused; `LOCK`
  as a symlink refused (secret untouched); `LOCK` as a FIFO refused (O_PATH
  cannot block on it); a 0644 queue request refused by the strict queue
  path; a foreign-uid `O_PATH` file under the `LOCK` name fails
  `ownership/type`. Together with the retained r3 negatives this covers
  symlink, foreign uid, extra link, wrong type and bad private-queue mode.
- The full live stock-wallet process restart is correctly declared
  unexecuted (`full_live_wallet_restart_executed: false`); it is not needed
  for this narrow code fix, and remains AP5 recovery-proof scope.

## 4. S2/S3/S4/S5/proof - unchanged and not regressed

The only executable difference from r3 is the hunk above; `run()`, the
three independent lock spans, `protect`/`release_confirmed` ordering,
`reconcile`'s proof-before-receipt-before-release sequence, the fault/cursor
queue, `ReadRpc` budgets, the emitter-based read sync and the
`board_proof_v1` reader are byte-identical to the r3 text I reviewed.
`check_owned_type`/`check` preserve the exact r3 acceptance set for every
caller other than the `LOCK` entry. No generic no-effect release was added
(`generic_noeffect_release_added: false`; code confirms `start` `Err` still
yields `stock_board_outcome_unknown` with the reservation held).

Docs now state the 160-call/5 s read budget as a closed observation
capacity (about 150 owned UTXOs), the standard-BDK-emitter read sync with no
mempool scan, the `gettxout(include_mempool)` re-proof, the `LOCK`
classification with its O_PATH rationale, and the pre-stock-checkpoint /
ordinary-`Err` permanent `unknown` hold. No server fork, pin or vendor
change.

## 5. Residual notes (non-blocking)

- `prepare_wallet_database` still opens `wallet.sqlite` with an ordinary
  `O_RDWR | O_CREAT` descriptor and closes it. It is only called inside
  `open_bark_wallet` before `SqliteClient::open`, so no SQLite fcntl lock
  exists yet and nothing is released. Because the function is reachable
  from the crate, a one-line doc comment ("pre-open only; closing an
  ordinary fd on a live SQLite file releases this process's locks") or
  using `O_PATH` for the post-create check would make the hazard class the
  author identified fully closed by construction.
- The tolerated `LOCK` class accepts `0666` under `umask 000`; harmless
  inside the 0700 root and consistent with "stock mode as created", just
  noting it is deliberate.

## 6. Verdict

**SOURCE AUDIT: APPROVE**, bounded to this packet's source at code
`f7649004...` / delivery `d00299c2...`: R1 is closed by scoping the exact
mode rule away from the single stock PID stamp while keeping every secret,
database, control and unknown file strict, using O_PATH so the inspection
can never release a process-owned POSIX lock, and proving it with a real
pinned-helper, cross-process exclusion test. S1 (original), S3, S4, S5 and
`board_proof_v1` remain closed; S2 remains the documented accepted hold.
This is a source and receipt reading; AP5 funded use, the live restart
recovery proof and the grant union remain separately owned and unclaimed.
