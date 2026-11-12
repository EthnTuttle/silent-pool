# Nix network-tool realization: root cause and remedy

Date: 2026-11-07
Author: TFF subagent (network-fetch root-cause/realization; independent of the packaging agent)
Status: complete - pinned tool closure realized online from the standard trusted cache
Scope: the open Nix integration gate of
`docs/reviews/2026-11-05-lab-lp4a-implementation.md` section 11 (LP-4a)
Evidence: `/tmp/opencode/nix-network-fetch-result.json` (frozen JSON,
sha256 `53a61a5b3ab2963457fff2a9b73a39a8c814ff30ebca3c0d37e74fd07a965cb4`),
scratch `/tmp/opencode/Nix-net-fetch-20261107/`
No repo file, lockfile, approved tree, or Nix configuration was modified.

## 1. Verdict

The limitation is resolved from the realization side. The exact pinned
nixpkgs input (rev `0bb2eedc20e4739f29e8045034f2828ad7054f8f`, narHash
`sha256-fiPJ88izTVucjeo2EQ3jk4aWmLg03I1D4vxnX2QjEgE=`, via
`--inputs-from /home/ethan/code/silent-pool` on the untouched lockfile)
provides the full LP-4a tool set as **binary substitutes from
`https://cache.nixos.org/`**. Nothing was compiled: 23 paths (12.74 MiB
download) plus 3 more (0.05 MiB) were fetched; the rest of the closure was
already in the store from the green core builds. Every realized output
carries a verifiable `cache.nixos.org-1` signature. The previously failing
command form now exits 0 when run online, and also offline now that the
store holds the closure (section 4).

The old blocker was caused by `--offline` **on an incomplete store**, not by
the pin, the cache, or (broadly) the network. Initial realization required
the online cache; cached offline execution is supported after all
inputs/closure are present (proved in section 4, exit 0). Two specific
corrections to the 2026-11-05 record:

* **"The binary cache does not cover the pinned inputs" is incorrect.** The
  online dry-run plans fetch-only realization (exit 0, zero derivations to
  build) for `iproute2 nftables util-linux python3 jq procps bash coreutils`.
* **"This host cannot reach the source mirrors" is true but irrelevant.**
  `download.savannah.gnu.org` indeed times out (TCP connect, curl 28 after a
  20 s bound), and `github.com` / `channels.nixos.org` do not even resolve,
  but `cache.nixos.org` resolves and serves narinfo and NARs over TLS. Source
  mirrors are only needed for from-source builds, which substitution avoids.

## 2. Root cause of the `--offline` failure

Reproduced once, bounded (`timeout 300`, returned on its own in about 90 s),
exit 1 (log `logs/reproduce-offline.stderr` in the scratch dir):

```sh
nix shell --offline --inputs-from /home/ethan/code/silent-pool \
    nixpkgs#iproute2 nixpkgs#nftables nixpkgs#util-linux \
    nixpkgs#python3 nixpkgs#jq nixpkgs#procps -c true
# -> "these 1743 derivations will be built", then
#    builder for '...-stage0-posix-1.9.1-source.drv' failed with exit code 1
```

The pinned nixpkgs (nixos-unstable, 2026-11) bootstraps its stdenv **from
source** via the stage0/live-bootstrap chain (hex0 seeds, M2-Planet, mescc,
bash 5.3 patches, bash-2.05b, acl, attr, autoconf, automake). With
`--offline`, substituters are disabled, so all 1743 derivations would have to
be built from source. That path fails **structurally**, before any question
of mirror reachability: the fixed-output derivation
`/nix/store/2kbbzfsz1hzd21yj0h989s3z40cgrp58-stage0-posix-1.9.1-source.drv`
has **no source URL at all** - its input archive is a manually created
bootstrap source that exists only as a binary-cache store path. Its builder
error states this directly ("Neither your store nor your substituters seems
to have: /1rz4g4znpzjwh1xymhjpm42vipw92pr73vdgl6xs1hycac8kf2n9"). The
`acl-2.4.0.tar.gz` savannah timeout observed on 2026-11-05 was a symptom of
the same forced source bootstrap, not the root cause.

Conclusion: on the store as it stood (missing the tool outputs and the
bootstrap input), `--offline` had no route to realization - the stage0
bootstrap source cannot be fetched from any mirror, and substituters were
disabled, so the store could not be populated offline. This is a property of
**that incomplete store plus disabled substituters**, not of the pin or of
any host: a fully populated store realizes and executes the same command
offline (section 4). The
cache-miss hypothesis was checked and rejected: the online dry-run is a
fetch-only plan, i.e. the narinfo for every needed path exists on the
standard cache.

## 3. How the closure was realized (commands and exit codes)

All realization commands ran from the scratch directory, against the repo's
locked inputs, with `--max-jobs 1 --cores 2` (nothing was actually built;
every path was substituted), `--no-link` (no `result` symlink in the repo).
The effective substituter set was read only and left untouched:
`https://cache.nixos.org/` only, `require-sigs = true`, daemon store,
untrusted user. No proxy variables were set in the shell; access-tokens and
netrc values exist in the config and were never read or logged.

| Command | Exit | Result |
|---|---|---|
| `nix build --dry-run --inputs-from ... nixpkgs#{iproute2,nftables,util-linux,python3,jq,procps,bash,coreutils}` | 0 | "these 23 paths will be fetched (12.74 MiB download, 45.59 MiB unpacked)", nothing to build |
| `nix build --max-jobs 1 --cores 2 --no-link --print-out-paths --inputs-from ...` (same 8 attrs) | 0 | all substituted |
| `nix build ... nixpkgs#grep ...` | 1 | attribute `grep` does not exist in the pinned nixpkgs (renamed upstream to `gnugrep`) |
| `nix build --max-jobs 1 --cores 2 --no-link --print-out-paths --inputs-from ... nixpkgs#{gnugrep,gnused,gawk,which}` | 0 | 3 paths fetched, rest already present |
| `nix shell --inputs-from ... nixpkgs#iproute2 ... nixpkgs#procps -c true` (the exact previously failing form, online) | 0 | success |
| `nix shell --offline --inputs-from ... nixpkgs#iproute2 ... nixpkgs#procps -c true` (same command, after the online realization) | 0 | warm-store offline pass, see section 4 |

The four extra packages exist because the LP-4a scripts
(`scripts/lab/*.sh`, `scripts/lab/lib/*.sh`, `testing/lab-isolation/run.sh`)
invoke `grep`, `sed`, `awk` and `which` in addition to the section 11 set;
everything else they use (`cat`, `env`, `timeout`, `od`, `sha256sum`,
`stat`, `readlink`, `mktemp`, `dirname`, `basename`, `date`, `sleep`, ...)
is coreutils.

## 4. Warm-store offline proof (exit 0)

After the online realization populated the store, the **exact** cold-failure
command form was re-run offline, bounded (`timeout 300`, returned in
seconds):

```sh
nix shell --offline --inputs-from /home/ethan/code/silent-pool \
    nixpkgs#iproute2 nixpkgs#nftables nixpkgs#util-linux \
    nixpkgs#python3 nixpkgs#jq nixpkgs#procps -c true
# exit 0; stderr contains only "warning: Git tree ... is dirty"
```

Log: `logs/warm-offline.stderr` (scratch dir). The cold-failure log
`logs/reproduce-offline.stderr` is preserved untouched (mtime
2026-11-07T10:31:26Z). Chronology of the three runs, all with locked inputs
and the same attr set: **cold offline failure (exit 1) -> online fetch
(23+3 substituted paths, exit 0) -> warm offline execution (exit 0)**.
`--offline` therefore fails only when the store lacks a required output or
the bootstrap input (which no mirror serves); once the closure is present,
offline realization and execution work.

## 5. Realized artifacts (provenance: all signed `cache.nixos.org-1`)

| Package | Version | Output path | Derivation |
|---|---|---|---|
| iproute2 (`ip`/`tc`/`ss`) | 7.2.0 | `/nix/store/x1pwsbz5cm23hk58b7fcpn3hhg9j7gwh-iproute2-7.2.0` | `.../jn8sg7z6rl6b350fdyqp8r7hx8ynbdhx-iproute2-7.2.0.drv` |
| nftables (`nft`) | 1.1.7 | `/nix/store/nwhaqj3i1lwvangwrfa3c7xswr3b79n1-nftables-1.1.7` | `.../5c3g3fp4slq7ybrl6kr70p1wvp6jdn7x-nftables-1.1.7.drv` |
| util-linux (`unshare`/`nsenter`/`flock`) | 2.42.4 | `/nix/store/2lgg8jqyzjp60svh7kmp39kqwnig31p2-util-linux-2.42.4-bin` | `.../w32d9vrkbzjsd17068ywhxmhj82fzjq1-util-linux-2.42.4.drv` |
| python3 | 3.14.8 | `/nix/store/xahxrjs30p3mw0a42bqg9ikgkikskw8m-python3-3.14.8` | `.../w1svn0lba9gv480i37z2q97yi4155mlr-python3-3.14.8.drv` |
| jq | 1.8.2 | `/nix/store/9l0j15jg8634hkb6d4c9y95mpxl0nhrd-jq-1.8.2-bin` | `.../2rkll8ncy5ldfv8zivk8ml6sga8zifgv-jq-1.8.2.drv` |
| procps (`sysctl`/`pgrep`) | 4.0.7 | `/nix/store/y733samykhd49crbck70qzsl315zq3hw-procps-4.0.7` | `.../pas0nxk4anj5g6iqwyqlgk4i2d9x6ccr-procps-4.0.7.drv` |
| bash-interactive | 5.3p20 | `/nix/store/kwyi4bq5s2qgwbhz6cs4gjy57c2zdxws-bash-interactive-5.3p20` | `.../mgicc2710bw3ffi1814yc82057fxka5d-bash-interactive-5.3p20.drv` |
| coreutils | 9.12 | `/nix/store/cb987zkxvsv0490753ilrvn5yk7ylgxi-coreutils-9.12` | `.../m7bmd2qz1zcnk4k25y5cg6wbb3y9qwcb-coreutils-9.12.drv` |
| gnugrep | 3.12 | `/nix/store/c3f45pxlb81gzz4w1nym7lpznfypva7s-gnugrep-3.12` | `.../1142izpla409fzbfwncwmmbs8y9ndc0k-gnugrep-3.12.drv` |
| gnused | 4.10 | `/nix/store/9wc73n5izv0m7ppjbibi4hbr68rba1lj-gnused-4.10` | `.../4fw9ksqs3knbrsfjr3g47i16vzr5v30a-gnused-4.10.drv` |
| gawk | 5.4.1 | `/nix/store/h96s16mjssibkvk62pvmfm3lwfc3wnwp-gawk-5.4.1` | `.../xmkywhcm19x46mxrc1ii56bfmg27nbkx-gawk-5.4.1.drv` |
| which | 2.25 | `/nix/store/bxaddvz98rw7pn24gcrjix69q6np2gq8-which-2.25` | `.../bcr3xmcmiq2fqs9z084b83cq7wnn9m5c-which-2.25.drv` |

Aggregate closure of the 12 outputs: **79 unique store paths, ~3.4 GiB total
store bytes** (sum of per-path narSize; multi-output packages overlap), of
which only the 26 paths above were fetched now - the rest was already
present from the prior green core builds.

Signatures were captured with `nix path-info --sigs`
(`evidence/provenance.txt`): every output path carries a
`cache.nixos.org-1:...` signature, so provenance is the trusted binary cache
of the pinned channel, verified against the default trusted key
(`require-sigs` stayed `true`; no signature or hash check was disabled).

## 6. Clean-PATH execution proof (exit 0)

`evidence/clean-path-exec-final.txt`: a store `bash` was launched with
`env -i` and a PATH built **only** from the 12 packages' `bin`/`sbin` store
directories. Every one of `ip tc ss nft unshare nsenter flock python3 jq
sysctl pgrep bash cat timeout env grep sed awk which od sha256sum stat
readlink mktemp dirname basename date sleep` resolved to a `/nix/store`
path (no host directory in PATH), the version checks passed
(iproute2-7.2.0 with libbpf 1.8.0, nftables v1.1.7, util-linux 2.42.4,
Python 3.14.8, jq-1.8.2, procps-ng 4.0.7, bash 5.3.20, coreutils 9.12,
grep 3.12, sed 4.10, gawk 5.4.1), and `jq`/`python3`/`grep`/`sed`/`awk`
smoke checks passed. Exit code 0. This is not a host-PATH pass: the shell
itself and every resolved binary are store paths.

## 7. Network facts recorded (bounded probes, 10-25 s each)

* `cache.nixos.org` resolves (IPv6 `2a04:4e42:8d::347`) and answers HTTPS
  (Varnish edge, TLS ok) - narinfo and NAR downloads work.
* `download.savannah.gnu.org` resolves (`2001:470:142::79`) but TCP connect
  times out (curl error 28, 0 bytes, 20 s bound).
* `github.com` and `channels.nixos.org` do not resolve at all; the latter
  only produces a non-fatal flake-registry warning under `--inputs-from`
  (inputs are locked, so no GitHub fetch is needed).
* DNS answers are IPv6-only for the reachable hosts; the host has working
  IPv6 to the cache.

## 8. Handoff to the packaging owner

No flake edit was made here (the flake and all approved trees stay
untouched). The bounded handoff for the section 11 devShell change:

1. Add to `devShells.default.packages` (exact attrs, all verified realizable
   online from the pinned lock): `pkgs.iproute2 pkgs.nftables
   pkgs.util-linux pkgs.python3 pkgs.procps pkgs.jq pkgs.bash
   pkgs.coreutils pkgs.gnugrep pkgs.gnused pkgs.gawk pkgs.which`.
   `pkgs.jq` is already in the devShell today; the rest are new. The four
   `gnu*`/`which` additions cover every external command the LP-4a scripts
   actually invoke beyond the section 11 list.
2. **Attr rename trap:** `pkgs.grep` does not exist in the pinned nixpkgs
   (upstream renamed it to `gnugrep`); using it fails evaluation with
   "does not provide attribute ... 'grep'" (exit 1, reproduced).
3. **Offline policy:** initial realization requires the online cache - the
   stage0 bootstrap source has no source URL, so an incomplete store cannot
   be populated offline. Once the closure is present, `--offline`
   realization and execution work (proved, section 4, exit 0). CI with a
   pre-populated store may therefore run offline; a cold CI store must
   allow the standard substituter (`https://cache.nixos.org/`, default
   trusted keys), which fully covers the closure (fetch-only plan,
   verified).
4. A `checks.lab-isolation` gate (section 11's last bullet) would realize
   the same closure plus the test driver's runtime; the same online-cache
   requirement applies, plus a kernel allowing unprivileged user namespaces.

Namespace functional testing is explicitly not run here; it stays with the
packaging/validator agent after this handoff.

## 9. Integrity and non-claims

* No repo file, `flake.nix`, `flake.lock`, approved tree, system or user Nix
  config was edited; no fetch URL override was needed (so none is proposed);
  no source hash was invented; the core pin was not changed.
* No TLS, hash or signature validation was disabled; no host `/usr` binary
  was imported as a Nix package; substituters and trusted keys are the
  untouched defaults.
* No commits, pushes, subagents, resets, stashes, cleans, GC or root
  privilege. Nothing was deleted anywhere.
* The failed `--offline` reproduction spawns fetch attempts at the
  configured default parallelism (it failed before any compilation
  started); all remedy commands were bounded to `--max-jobs 1 --cores 2`.
* Not claimed: running the LP-4a suite itself, `nix flake check`, or any
  namespace functional test from the realized closure.
