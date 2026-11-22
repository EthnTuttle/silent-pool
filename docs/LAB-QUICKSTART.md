# Lab quickstart: run the regtest lab with one command

## What you get

One command starts a private Bitcoin test network on your machine: one regtest
node, two silent-pool mining pools, four CPU miners with their own wallets
(alice, bob, carol, dave), a self-hosted Ark server, and a browser UI to watch
and control it. Everything listens on `127.0.0.1` only and uses worthless test
coins; a second command adds Grafana dashboards.

## Requirements

- x86_64 Linux, an ordinary user account (not root), Nix with flakes enabled.
- Disk: about 30 GB free for the first build (estimate), then 0.3 GB per run
  plus 1.7 GB for every hour it runs.
- Time: the first start compiles everything from source. That took 11 minutes
  on a 24-core machine (measured, downloads not included); allow 20 to 60
  minutes on a smaller one (estimate). Later starts reuse the build.

## Start

```sh
nix run github:EthnTuttle/silent-pool#lab
```

It stays in the foreground and creates a new run directory
`~/.spc-lab/qs-<date>-<time>/` each time.

## What you will see

After the build output (first start only) comes `starting the lab in ...`,
then nothing for 15 seconds to 2 minutes, then a banner (ports differ on
every run):

```text
================ silent-pool regtest lab ================
  Controls and wallets : http://127.0.0.1:45427/
  Operator directory   : http://127.0.0.1:45427/lab
  Pool page front-a1   : http://127.0.0.1:44117/
  Pool page front-b1   : http://127.0.0.1:38851/

  The pages ask for a run token. Open this file and paste the
  value of its "token" field ...
    /home/you/.spc-lab/qs-1122-065240/runtime/private/access/operator.json
  ...
```

Mining has started. If startup fails instead, the error is printed and the
launcher cleans up; just start again.

## Open the UI

Open the "Controls and wallets" address in a browser. When it asks for a run
token, open the `operator.json` file named in the banner in a text editor and
paste the value of its `token` field. Treat the token as a password. The
files `alice.json`, `bob.json`, `carol.json` and `dave.json` beside it hold
tokens that show one miner's view. To print the banner again:

```sh
nix run github:EthnTuttle/silent-pool#lab -- status
```

## Optional: Grafana

In a second terminal, while the lab runs:

```sh
nix run github:EthnTuttle/silent-pool#lab -- grafana
```

Grafana is at <http://127.0.0.1:13000>, Prometheus at
<http://127.0.0.1:19464>, no login. If those ports are taken, append
`--grafana-port 13001 --prometheus-port 19465`. Ctrl-C stops the dashboards
only. They add about 0.6 GB to the run directory.

## Stop

In the lab's terminal press Ctrl-C once (or type `stop` and Enter) and wait a
few seconds for `the lab stopped cleanly; nothing is left running`. From any
other terminal:

```sh
nix run github:EthnTuttle/silent-pool#lab -- stop
```

`status`, `grafana` and `stop` act on the most recent run. The banner also
prints them as plain program paths; prefer those if the repository may have
changed since you started, because `nix run` would rebuild first.

## Keep it running after you close the terminal

The lab stops when its terminal closes. On many desktop systems, closing a
terminal window also kills whatever was put in the background from it, even
with `nohup`. To keep a lab running, start it as its own systemd user unit
(run the plain start command once first, so the build is done):

```sh
systemd-run --user --unit=spc-lab --collect \
  -p KillMode=mixed -p TimeoutStopSec=300 \
  nix run github:EthnTuttle/silent-pool#lab

nix run github:EthnTuttle/silent-pool#lab -- status   # addresses, token file
journalctl --user -u spc-lab -f                        # launcher output
systemctl --user stop spc-lab                          # clean stop
```

To also survive logging out, run `loginctl enable-linger "$USER"` once.

## What to expect over time

- A block about every 40 seconds on average; gaps of two or three minutes are
  normal. To change it, append for example `-- --pow-bits 0x20008888` (about
  60 seconds) to the start command.
- Pools book a block once it is six blocks deep: the first booked block shows
  up after about 5 minutes. Mined coins mature after 100 blocks, about 67
  minutes.
- One run is useful for less than a day. Regtest halves the block reward every
  150 blocks: it falls below the pools' largest payout step (5,000,000 sats)
  after about 17 hours and below the smallest (500,000 sats) after about 23
  hours. Start a new run instead of extending an old one.
- The run directory grows about 1.7 GB per hour. Delete finished runs:
  `rm -rf ~/.spc-lab/qs-<date>-<time>`.

## Cleaning up after a crash

Lab processes end when the lab's controller process ends, and the launcher
checks for leftovers whenever the controller ends badly. If the launcher
itself was killed, the lab keeps running: use `stop` as above. If anything
is left after that, this ends the processes recorded by the most recent run
and deletes no files:

```sh
nix run github:EthnTuttle/silent-pool#lab -- reap
```

## Problems

- **`experimental Nix feature 'nix-command' is disabled`**: write
  `nix --extra-experimental-features 'nix-command flakes' run ...`, or put
  `experimental-features = nix-command flakes` in `~/.config/nix/nix.conf`.
- **`run directory path is too long`**: your home directory path is too long
  for the lab's 56-byte limit. Run `export SPC_LAB_HOME=/some/short/dir` (a
  directory you own) before the commands in this guide; for `systemd-run`
  also add `-E SPC_LAB_HOME="$SPC_LAB_HOME"`.
- **`... already exists; every run needs a new directory`**: a run directory
  is never reused. This only happens with `-- --name <name>`; pick a new name
  or leave it out.
- **The UI asks for a token or rejects it**: paste the `token` value from the
  `operator.json` named in the banner. A token works for its own run only.
- **`was not ready before the deadline`**: an occasional startup race. Start
  again.

More depth, the lab's limits and its HTTP API: [LAB-RUNTIME.md](LAB-RUNTIME.md).
