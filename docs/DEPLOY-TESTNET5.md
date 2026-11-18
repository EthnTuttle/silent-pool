# Deploying the public Testnet5 pool host

A fresh-VPS runbook for `nixosConfigurations.spc-testnet5-host`: the
pinned rbitcoin Testnet5 node, a self-hosted `captaind` (the adopted
testnet5 fork), and one `spc-pool` process running `front,payout,api,ark`,
with PostgreSQL, Redis and an nginx-published read-only API. Deployment
tooling only - nothing here changes normative behavior; `docs/SPEC.md` §9
stays authoritative and `docs/OPERATIONS.md` remains the general
operations guide.

**What this does NOT cover (read first).**

- **Authorization.** SPEC §9 "Testnet5 staging": identity support is not
  deployment authorization. Going public still requires owner lab
  approval and the separately approved public-network work/audit policy.
  This runbook is the mechanism, not the approval.
- The testnet4 soak / rollout gates (SPEC §9 "Rollout"), mainnet anything,
  and the v2 work-unit profile (regtest-only).
- Monitoring dashboards/alerting - a separate packet
  (`services.spc-monitoring`). This host guarantees the scrape contracts
  and feeds them automatically when that module is imported alongside the
  stack: pool metrics `http://127.0.0.1:8080/metrics` (Prometheus text
  0.0.4, loopback only, appended to
  `services.spc-monitoring.scrapeTargets` as job `spc-pool`) and node
  metrics `http://127.0.0.1:9332/metrics` (`rbitcoin_*` families from
  `--health-listen` + `--metrics`, fed as
  `services.spc-monitoring.rbitcoinTarget`).
- The block explorer - a separate packet (`rbitcoin-space-testnet5`). The
  node module exposes its hooks (`services.rbitcoin-testnet5.shIndex`,
  `esploraListen`, `electrumListen`); they default off here and the
  explorer deployment turns them on.
- Tor reach, multi-region fronts, remote JDCs (the JDP listener stays
  loopback by default), and publishing the Ark server itself (captaind
  gRPC stays loopback; only the co-located pool and local tools use it).

**Trust posture (testnet5 only).** The `ark` role is co-located and
`server_is_self = true`: the pool runs its own Ark server because no
third-party testnet5 server exists (SPEC §11.3/§11.9; decision
`docs/decisions/2026-11-17-bark-testnet5-fork.md`). Sender equals server -
the arkoor "one honest party" assumption is void. This is a test-network
posture, never a production pattern.

## 0. What runs where

| Unit | Binary (flake package) | Listeners |
|---|---|---|
| `rbitcoin-testnet5.service` | `rbitcoin-node-testnet5-dev` (pin `8af95b8`) | P2P `0.0.0.0:18353` public; RPC `127.0.0.1:18352` cookie-auth; health+metrics `127.0.0.1:9332` |
| `captaind-testnet5.service` | `captaind-testnet5` (fork `8c38e07`) | gRPC `127.0.0.1:3535` (public API), `:3536` admin, `:3537` integration - all loopback |
| `postgresql.service` | nixpkgs | local Unix socket only; DBs `spc-pool`, `captaind` (peer auth, no passwords) |
| `redis-spc.service` | nixpkgs | `127.0.0.1:6379` |
| `spc-pool.service` | `spc-pool-ark` (features `rpc,ark`), roles `front,payout,api,ark` | SV2 `0.0.0.0:34254`, SV1 `0.0.0.0:3333`, JDP `127.0.0.1:34264`, API `127.0.0.1:8080` |
| `nginx.service` | nixpkgs | `:80` (+`:443` with ACME) -> `127.0.0.1:8080`, GET/HEAD/POST only, `/metrics` refused |

Startup ordering is wired in the module: node -> captaind -> pool (after
postgres/redis). Chain identity: the **payout** role alone initializes the
SQL and Redis chain bindings; the stack asserts `payout` stays in the
single process's roles. If you ever split roles into separate processes,
the payout process must start first (SPEC §5.1/§5.3).

## 1. Provision

- VPS: x86_64-linux, >= 4 GiB RAM, >= 80 GiB disk (node store + postgres;
  testnet5 is young - revisit as the chain grows), NixOS installed (any
  recent channel; the flake pins its own nixpkgs).
- Edit `nix/deploy/testnet5/host-example.nix` ("EDIT ME" block): hostname,
  your SSH key, disk device/label for the image, and - once you have a
  domain - `domain` + `acme.*`.
- DNS: point the domain at the VPS before enabling ACME.

## 2. Secrets (before first switch)

Nothing below enters the repo or the Nix store. All paths are plain files,
so sops-nix/agenix users can target the same paths instead. The generator
is offline, stdlib-only Python that self-tests on every run:
`nix/deploy/testnet5/spc-testnet5-keygen.py` (henceforth `keygen`).

On the host (as root):

```sh
install -d -m 0755 /etc/spc
install -d -m 0750 -g spc-rpc /etc/spc/secrets   # group exists after first switch; see note
umask 077

# 1. Node RPC cookie (node + captaind read it; pool.toml carries the values)
python3 keygen rpc-cookie --out /etc/spc/secrets/rbitcoin-rpc.cookie
chgrp spc-rpc /etc/spc/secrets/rbitcoin-rpc.cookie
chmod 0640 /etc/spc/secrets/rbitcoin-rpc.cookie

# 2. SV2 Noise authority pair -> paste into pool.toml [pool]
python3 keygen authority

# 3. Ark seed (the ONE hot seed, SPEC §9/§11.3) + payout descriptor.
#    0600 file in a 0700 dir owned by the pool user; NOT under
#    /run/secrets/spc (the ark role refuses the SPc-side secret store).
install -d -m 0700 -o spc-pool -g spc-pool /etc/spc/ark
python3 keygen ark-seed --out /etc/spc/ark/seed.hex
chown spc-pool:spc-pool /etc/spc/ark/seed.hex
python3 keygen descriptor --seed-file /etc/spc/ark/seed.hex
#    -> paste pool_payout_descriptor into pool.toml [pool]. The ark role
#       cross-checks it against the seed at startup and refuses a mismatch.

# 4. Ark order-signing key -> order_pubkey into pool.toml [ark]
python3 keygen order-key --out /etc/spc/secrets/ark-order-key.hex
chown spc-pool /etc/spc/secrets/ark-order-key.hex && chmod 0600 /etc/spc/secrets/ark-order-key.hex

# 5. The pool config itself
cp nix/deploy/testnet5/pool.toml.example /etc/spc/pool.toml
chgrp spc-pool /etc/spc/pool.toml && chmod 0640 /etc/spc/pool.toml
$EDITOR /etc/spc/pool.toml   # fill every <PLACEHOLDER>; rpc_user/rpc_pass
                             # are the two halves of the cookie from step 1
```

The `[ark]` ArkInfo fields (`server_pubkey`, `mailbox_pubkey`, `ark_id`)
need the running captaind - fill them in step 5 below; the pool unit
fails fast with a pointer until `/etc/spc/pool.toml` exists and parses.

Note on ordering: `spc-rpc`/`spc-pool` users and groups are created by the
first `nixos-rebuild switch`. Easiest flow: switch once (services will
retry-loop on the missing secrets, loudly but harmlessly), provision the
secrets, then `systemctl restart` the units - or create the
groups/users first by hand if you want a clean first boot.

**Back up** (offline, per OPERATIONS §4/§5): `/etc/spc` (cookie, pool.toml
with the authority secret, ark seed, order key) and
`/var/lib/captaind-testnet5/data` (the captaind server seed/mnemonic -
also hot). The SPc side (Postgres/Redis) holds no spending key, only the
descriptor xpub; dump Postgres per OPERATIONS §5 and rotate old backups on
the scrub schedule (they are attribution data).

## 3. First switch

From a checkout of this repo (or `--flake github:...` once published):

```sh
nixos-rebuild switch --flake .#spc-testnet5-host --target-host root@<vps>
```

(Equivalently on the host itself: `nixos-rebuild switch --flake .#spc-testnet5-host`.)

## 4. Node sync

```sh
journalctl -fu rbitcoin-testnet5
```

Expect `rbitcoin-node starting version=0.9.99 network=testnet5` and
header/block progress from the bootstrap peer
(`lapis.erisian.com.au:18353`; no DNS seeds exist at the pin - discovery
grows the addrman from there). Testnet5 (BIP95, genesis
`00000000000008c1...d497`) is a young chain: initial sync is minutes-to-
hours, not days. Verify the tip against a second source before trusting
it. Sanity-check RPC + cookie + the captaind version gate in one shot:

```sh
curl -s --user "$(cat /etc/spc/secrets/rbitcoin-rpc.cookie)" \
  -d '{"id":1,"method":"getnetworkinfo","params":[]}' http://127.0.0.1:18352/
# -> "version": 310000  (config-level claim for captaind's v31 gate;
#    the node's real identity stays /rbitcoin:0.9.99/)
```

`captaind-testnet5` starts after the node, creates its wallet on first
boot (`captaind create`; mnemonic lands in /var/lib/captaind-testnet5/data)
and syncs from the node. The pool refuses to start until the chain view,
genesis and chain bindings check out - a restart loop during initial sync
is expected; it settles once the node is at tip.

## 5. Bind the pool to the Ark server (one-time)

Read ArkInfo from the local captaind with the bark fork CLI (any
directory; this creates a throwaway read-only wallet):

```sh
nix shell .#bark-testnet5 --command bark --datadir /tmp/bark-probe create \
  --testnet5 --ark http://127.0.0.1:3535 --bitcoind http://127.0.0.1:18352 \
  --bitcoind-cookie /etc/spc/secrets/rbitcoin-rpc.cookie
nix shell .#bark-testnet5 --command bark --datadir /tmp/bark-probe ark-info
```

From the JSON output fill `/etc/spc/pool.toml` `[ark]`:
`server_pubkey`, `mailbox_pubkey`, and
`python3 keygen ark-id --server-pubkey <server_pubkey>` -> `ark_id`
(startup cross-checks it). Then:

```sh
systemctl restart spc-pool
journalctl -fu spc-pool    # expect: SV2/SV1/JDS listeners ready, payout
                           # chain binding, bark wallet startup sync
```

## 6. Float funding (bark-testnet5 CLI against the local captaind)

The ark role pays sub-threshold balances from its own bark wallet float
(SPEC §11.3). On startup it writes a provisioning summary:

```sh
cat /var/lib/spc-pool/ark/provision.json
# { "onchain_funding_address": "tb1p...", "float_spendable_sats": 0,
#   "float_target_sats": 1000000, ... }
```

1. Get testnet5 coins (mine them, or ask on the BIP95 channels - no
   faucet is assumed here) into any wallet you control, e.g. a local bark
   wallet's onchain address (`bark onchain address` / `bark onchain balance`
   with the wallet from step 5 pointed at your own datadir).
2. Send to the pool wallet's `onchain_funding_address` above.
3. The pool's wallet-control/boarding loop boards matured funds toward
   `float_target_sats` (and later refills from matured pool outputs -
   SPEC §11.3 "Refilling"). Watch `spc_ark_float_sats` rise:

```sh
curl -s http://127.0.0.1:8080/metrics | grep spc_ark_float_sats
```

Boards need `required_board_confirmations` (3) plus coinbase maturity if
you mined directly; be patient on a slow test chain.

## 7. Verify /metrics and the API

```sh
# loopback metrics contracts (the monitoring packet scrapes exactly these):
curl -s http://127.0.0.1:8080/metrics | head          # spc_* (job "spc-pool")
curl -s http://127.0.0.1:8080/healthz
curl -s http://127.0.0.1:9332/metrics | head          # rbitcoin_* (rbitcoinTarget)

# public read-only API through nginx:
curl -s http://<domain-or-ip>/api/pool | jq .      # endpoints = [reach] advertise
curl -s http://<domain-or-ip>/api/pool/window | jq .
curl -si http://<domain-or-ip>/metrics | head -1   # HTTP/1.1 404 - by design
```

Point a miner (spc-agent, or any SV1 miner for a first smoke) at
`sv2://host:34254` / `stratum+tcp://host:3333` and watch
`spc_open_slice_shares`. Alarms and family semantics: OPERATIONS §7 -
scrape `/metrics` to discover what this build actually emits.

## 8. Backup / restore / upgrades

OPERATIONS §5 is the procedure (sqlx migrations at boot under advisory
lock; pre-migration `pg_dump`; **no downgrades across a schema version** -
restore the dump). Stack-specific pointers:

- Stopping `spc-pool` stops mining; pending balances and scheduled Ark
  payouts survive in Postgres and pay on resume (SPEC §9 Rollback).
- Secrets to restore on a rebuilt host: the whole of `/etc/spc` plus
  `/var/lib/captaind-testnet5/data` and `/var/lib/spc-pool/ark` (wallet
  db; recoverable from the seed but restoring saves a rescan).
- The node datadir (`/var/lib/rbitcoin-testnet5`) and
  the captaind Postgres DB are rebuildable from the network/wallet
  state respectively, but back up captaind's DB anyway - VTXO state is
  cheap to dump and tedious to regain.
- Old unbound SQL/Redis state is never adopted automatically (SPEC §9):
  a fresh deployment gets a fresh database; migrating prior data is an
  explicit, reviewed act.
