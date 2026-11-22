This packet makes no normative SPEC change: no `changelog:` bullet to splice.
Lab tooling is non-normative (AGENTS.md); `DESIGN.md` and `docs/SPEC.md` are
untouched, and no lab, pool, agent, node, scenario or Ark-server behavior
changes. Branch: `wp-lab-quickstart` (base `e70150b`). Packet-owned files:
`nix/lab/quickstart.nix`, `docs/LAB-QUICKSTART.md`, this fragment, the README
"Lab quick start" section and one pointer line in `docs/LAB-RUNTIME.md`.
Two additive `flake.nix` hunks (merge notes). No `Cargo.*` or Rust change.

## What changed

- New flake app `lab` and package `spc-lab-quickstart`
  (`nix run github:EthnTuttle/silent-pool#lab`). It runs the unchanged
  `spc-lab-self-hosted-ark` wrapper with a fresh run directory
  `~/.spc-lab/qs-<MMDD>-<HHMMSS>/runtime` (`--name`, `--home` or
  `SPC_LAB_HOME` override it), `--base-revision` set to the flake revision
  (`<rev>-dirty` from a dirty tree), `--dirty-tree-hash` set to the flake
  source NAR hash, `--listen 127.0.0.1:0 --operator-ui --interactive`. Other
  arguments pass through; a caller's own `--pow-bits`, `--listen`,
  `--base-revision` or `--dirty-tree-hash` replaces the launcher's.
- It refuses a run directory path over 56 bytes before starting anything, and
  after startup prints the UI addresses and the path of the operator token
  file. It never reads or prints a token.
- The controller runs in its own session with a FIFO
  (`<home>/<name>/control`) as stdin. Ctrl-C, SIGTERM and SIGHUP therefore
  reach only the launcher, which writes `stop` to the FIFO: the lab's existing
  interactive stop. If that has not finished after 120 s the launcher kills
  the controller. Whenever the controller exits non-zero the launcher runs
  `spc-lab reap` on the run and keeps the result in `reap.log`.
- Subcommands on the most recent run (`<home>/latest` symlink) or `--name`:
  `grafana` (execs `spc-lab-observability --run-dir`), `status`, `stop`
  (writes `stop` to the FIFO and waits), `reap` (refuses a live controller).
- `docs/LAB-QUICKSTART.md` is the hand-out guide. The README lab section leads
  with the command and no longer calls `98f0e6c` the current application
  reference; that revision remains the full campaign's pin
  (`docs/LAB-RUNTIME.md` section 2, unchanged).

## Why

Launching took about ten manual steps (build two packages, resolve two store
paths, compute a source hash, choose a short fresh directory, five required
arguments) and had three traps: a long run directory path fails at the Ark
PostgreSQL socket, Ctrl-C on the bare wrapper kills the controller instead of
stopping the run, and a lab started from a terminal dies with the terminal's
systemd scope.

## Validation (real processes)

Run directories are under `~/.spc-lab/`. "Left" is a scan of `/proc` for any
process whose command line or working directory names the run, taken after
the launcher exited. A started lab is 20 processes plus PostgreSQL workers.

Launcher from commit `640d5f1`, started as `nix run <worktree>#lab` from an
empty environment on a pseudo-terminal:

| Run | Test | Result | Left |
|---|---|---|---|
| `qs-1122-072904` | Quickstart followed in order: start, banner, UI `/` and `/lab` 200, both pool pages 200, health 401 without and 200 with the operator token, `status`, `grafana` (health 200, both pool targets `up`, two dashboards), 4 blocks, typed `stop` | exit 0 after 3 s, manifest `stopped` | 0 |
| `qs-1122-073111` | Ctrl-C on the terminal after 4 blocks | launcher printed the stop notice; exit 0 after 4 s, manifest `stopped` | 0 |
| `qs-1122-073643` | `systemd-run --user --unit=spc-lab --collect -p KillMode=mixed -p TimeoutStopSec=300 nix run ...`; launching shell exited; controller in `app.slice/spc-lab.service`; 4 blocks; `systemctl --user stop spc-lab` | clean stop in 3 s, manifest `stopped`, unit collected | 0 |
| `qs-pow` | `-- --pow-bits 0x200fffff --name qs-pow`; `-- stop` from a second shell | controller had one `--pow-bits`; both exit 0 | 0 |
| `qs-1122-074006` (under a temporary `SPC_LAB_HOME`) | systemd unit with `-E SPC_LAB_HOME=...`; `-- stop` | run stopped, unit ended by itself | 0 |

Same script built against the base `e70150b` packages (edge cases):

| Run | Test | Result | Left |
|---|---|---|---|
| `qs-a` | typed `stop`, Grafana on default ports | exit 0 after 3 s | 0 |
| `qs-b` | startup failed by itself after 16 s: `front/front-a1 was not ready before the deadline` | launcher reported it, ran reap (all `exited`), exit 1 | 0 |
| `qs-b2`, `qs-g` | `stop` subcommand; `qs-g` after `kill -9` of the launcher (lab kept serving, `reap` refused the live run) | clean stop, manifest `stopped` | 0 |
| `qs-c` | Ctrl-C | exit 0 after 4 s | 0 |
| `qs-d` | Ctrl-C twice during startup | lab finished starting, then stopped; exit 0 after 13 s | 0 |
| `qs-e` | `kill -9` of the controller | all lab processes gone within 0.5 s; launcher ran reap; exit 137 | 0 |
| `qs-f` | terminal closed (pty master closed, SIGHUP) | clean stop, manifest `stopped` | 0 |
| `qs-h` | systemd unit, `systemctl --user stop` | clean stop in 5 s | 0 |
| `qs-i` | controller frozen with SIGSTOP, then Ctrl-C | launcher killed it after 120 s, ran reap; exit 137 | 0 |

`nix build .#spc-lab-quickstart` and `nix flake check --no-build` pass at
`640d5f1`. Startup took 13-17 s in every timed successful run on this
machine. `qs-b` is the one failed start in 15; the pool front stalled after a
PostgreSQL migration-lock warning. It is not caused by the launcher and was
not investigated further.

Build time, 24 cores, each derivation alone: lab tools 4 min 23 s (fresh
build), Ark server 4 min 49 s and node 1 min 19 s (`nix build --rebuild`).
Downloads, toolchain fetches and crate vendoring were not measured; the
quickstart's 20-60 minute and 30 GB figures are estimates (the full build
closure present on this machine is 16.4 GiB).

Not verified: the literal `github:EthnTuttle/silent-pool#lab` reference (the
app is not on the public default branch yet; the worktree was used), a
machine without the build already in its store, the Nix error text when
flakes are disabled (this machine's Nix enables them unconditionally), and
`loginctl enable-linger` across a logout.

## Merge notes

- `flake.nix` (merge-session-owned) gains two hunks, both additive: the
  `spc-lab-quickstart` entry next to `spc-lab-observability` in `packages`,
  and a new per-system `apps` attribute with `lab`, Linux only. Exact diff:
  `git diff e70150b wp-lab-quickstart -- flake.nix`.
- `testing/lab-ark-funded/source-contract.json` records file hashes for
  `flake.nix`, `README.md` and `docs/LAB-RUNTIME.md`. All three change here
  and all three were already stale at the base; the contract is not
  regenerated by this packet.
- The launcher embeds the flake revision, so its store path changes with
  every commit; the lab tools it wraps rebuild on any tracked source change,
  as before.
