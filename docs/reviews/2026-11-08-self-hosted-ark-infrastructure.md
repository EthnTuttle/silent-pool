# Self-hosted Ark infrastructure implementation receipt

Date: 2026-11-08
Worktree: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-self-hosted-ark`

**Final status (2026-11-09): the all-Nix-store-path unfunded infrastructure
launcher is built and runtime-verified.** The packaged release node passed its
scoped tests and both canonical r4 complete-profile checks. All bounded test
processes stopped. Funded flows and J2 engine-gate integration are not claimed.
Historical failures and the temporary disk block below remain as evidence;
the final completion section supersedes their pending status.

## Source provenance

- Base: `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.
- Bootstrap: all 658 manifest-listed files from `/tmp/opencode/AP2-nix-src`,
  10828681 bytes, verified SHA256 before copying. Manifest SHA256:
  `9fc3c7e14997de3cc22bf7e534d08398f2967377e99eccf854c50df8731a9987`.
- Stock Bark: `c419159fa54085572f8ca72d2ba1716556a9d3eb`, matching
  `UPSTREAM_DEPS.md` and Cargo.lock. The checkout's only untracked file was
  Cargo's `.cargo-ok`; `git archive` excluded it, `.git`, and build outputs.
- Upstream archive: 8417280 bytes; source files: 7638773 bytes. Archive SHA256
  `ccce6599cc74af0943abe979d3612ecc878b8c539ce08a65ae30c8a8a181797d`.
  NAR SHA256 `sha256-qmdyctM8mZ27IKruiGcZ42OmvHwYm0enlPqVWnw7WkU=` was
  independently accepted by Nix's pinned `fetchgit` build.

## Packaging

`nix/lab/ark.nix` builds stock captaind and exposes a separate Bark CLI package.
It uses the exact upstream Cargo.lock, checksummed registry dependencies and
git-revision fetches, native protobuf/pkg-config and OpenSSL/SQLite/Postgres.
No server source patch is applied. The exported source-revision, lockfile,
stock config template and protobuf files travel with the binary.

The first captaind build failed at upstream's version-detection build script,
which requires `SERVER_VERSION` when no Git executable/checkout is supplied.
The corrected package supplies upstream-supported `SERVER_VERSION`,
`BARK_VERSION` and `GIT_HASH` environment inputs; it does not patch build.rs.

Verified captaind result:

- Output: `/nix/store/4mnskprw4qigq8i8xqf4pps7iak1bnly-captaind-0.7.1-c419159`
- Deriver: `/nix/store/rwwk2780931zjfh0787ixwqws4l4ng41-captaind-0.7.1-c419159.drv`
- Binary SHA256: `4cac675bccc017608378382129613ed5202dd90e7300a92c4ed799869abaef9d`
- Version: `captaind 0.7.1-dev+c419159fa54085572f8ca72d2ba1716556a9d3eb`
- Source: `/nix/store/nmbj14m8zliwsy8af75q48f95rak97zn-bark-c419159`
- External receipt: `/tmp/opencode/self-hosted-ark-captaind-receipt.json`.
- Logs: `/tmp/opencode/self-hosted-ark-captaind-nix.log` (failed initial build),
  `/tmp/opencode/self-hosted-ark-captaind-nix-r2.log` (successful corrected build).

Every heavy invocation held the nonblocking shared build lock, checked the
priority ticket and actual `/nix/store` free space after acquisition, and used
a 22 GiB peak-growth budget with at least 20 GiB remaining. Before the corrected
captaind build, 69230571520 bytes were available. No cache cleanup or GC was run.

## Runtime implementation

- Explicit self-hosted backend with leased loopback gRPC/admin/Postgres ports.
- Separate provider cluster under `private/ark-server-local-ark/postgres`;
  neither pool's attribution database is reused.
- Stock `check-config`, `create`, `start`, actual `GetArkInfo`, and stock wallet
  RPC. The config is rendered from the pinned server's complete template.
- Both pool roles get independent CSPRNG seeds, wallet SQLite, order keys and
  height-indexed descriptors. Redis ACL selectors separate order consumption,
  receipt publication and identity-free operational keys. No pool Postgres or
  Noise-authority secret is rendered into an Ark-role config.
- Actual ArkInfo keys and fee-policy hash are persisted before front configs
  gain Ark delivery metadata. The pool wallet writes an observed funding address
  and spendable float after its stock-wallet sync, before readiness.
- Long-lived services use the existing supervisor and actor-exec parent-death
  path; bounded provisioning children also use actor-exec and are reaped.
- The inventory lists provider storage and both pool roles. Whole-run inventory
  remains operator-only. `--operator-ui` continues through the existing path.

The infrastructure scenario has one actual node, two pools, two pool Ark roles,
one stock provider and three Postgres clusters; zero user-wallet/miner actors.
It selects the explicit `bark-v31` RPC profile and requires the corresponding
node artifact through `--node`. The default L2 pin is unchanged.

## Remaining funded dependencies

The parent reported a concrete node-profile prerequisite during this packet:
both sealed L2 and AP-1b `getblockchaininfo.consensus` lack
`consensus_profile_id`, `profile_fingerprint`, `initial_target_bits` and
`subsidy_initial_sats`. The canonical probe's seven owned-lifecycle cases
passed, but its complete-profile CLI correctly refuses this incomplete data.
The additive metadata worker is
`ses_e42867233ffeZJwCoW2lDSSNIS`, candidate source
`/tmp/opencode/rbitcoin-lab-profile-facts-src` based on the immutable
AP-1b 2102-file source manifest. The parent subsequently froze its 2106-file,
30896946-byte source. This packet independently reverified every file, mode,
byte count and manifest aggregate before Nix ingestion:

- Full aggregate: `91f41d32c1bf1fc973792437c98dd6eafbb4eb5e9aba10aa5025ab988994b8d9`.
- Code aggregate: `517d35bbcd8e7f9351dcf981cf3de909aba7974da8d9a7a928a303c4130b9121`.
- Additive patch: `c958d22f2e527679af505c99e665f3b245be4268978f06d05c347bbebf2b2496`.
- NAR: `sha256-uLn1AJNu+cgB5yvUYQCEnq6SFR0s2+9JUOcSaPzsHnQ=`.
- Source store path: `/nix/store/sayd5lbjxl5hidx76yw32r0ws334p0vc-source`.
- `nix/lab/ark-node.nix` / `rbitcoin-node-ark-lab` evaluates to
  `/nix/store/l9nvcfk70gcvwcyxfvr3k29a0gwf8xkc-rbitcoin-node-ark-lab-0.8.99-profile-facts.drv`.
  It uses Rust 1.95, the exact source Cargo.lock and scoped real RPC/CLI tests.
  Evaluation is not a build/test pass. Only the frozen source directory was
  ingested, not the packet directory containing a 97 MB executable and runtime
  evidence. Receipt: `/tmp/opencode/self-hosted-ark-node-source-receipt.json`.

The source is unpublished, so this opt-in package binds the local frozen repo
snapshot with `fetchTree` and its actual NAR hash. It does not pretend that the
upstream reference pin alone includes the additive lab changes. The default
L2 package/source selection is unchanged. Actual provider initialization can be exercised
independently, but does not satisfy J2 complete same-chain profile binding.
No missing metadata receives guessed defaults or a relaxed probe verdict.

The downstream canonical probe contract is r4
`754e1e6131955f9e5bbce7ef81b6f9aabaa2f1379ec6409f1c3a7a9e88be91f9`.
It accepts literal `--rpc-compat-profile bark-v31` and its exact equals form as
digest-bound RPC policy, not isolation. Parent-reported 47 Python checks do
not prove a Rust process binding. Default and bark-v31 must have equal consensus
fingerprints for equal consensus, and different launch-comparison digests.
The actual combined positive check still requires the compiled candidate and
that canonical probe; this runtime records `not_validated_by_canonical_probe`.

This is not AP-5 acceptance. Native T1b/T2/DEST funding, the registered-agent
runtime bridge, four user wallets, boarding 115000000 sats per pool, and real
receive/refresh/offboard remain separate runtime work. The infrastructure
profile refuses direct unregistered miner startup/addition until that bridge
exists. No workflow approval is required to implement it.

`initialized_unfunded` comes from the actual server wallet's zero balance;
positive funds are `funding_observed_not_validated`. Unknown balances fail.
The 115000000-sat value is a target, never an observed float. Private native
funding handoff files contain real wallet addresses and observed balances.

No hosted Ark service, separate Core chain, SQL funding credit, or `generate*`
RPC is used by this packet.

### Exact downstream interfaces

- Public derivation input: `<run>/private/ark-info.json`, schema
  `spc.lab.ark-info/v1`, contains the local `endpoint`, `backend_id`,
  `source_revision`, `observed_at`, actual protobuf-JSON `ark_info`, and
  `bound.{ark_id,server_pubkey,mailbox_pubkey,fees_sha256}`. These are public
  server data in a private run artifact; there are no wallet secrets in it.
- Provider funding input:
  `private/ark-server-local-ark/native-funding-handoff.json`, schema
  `spc.lab.ark.native-funding/v1`, carries stock wallet RPC's
  `server_wallet.rounds.{address,total_balance,trusted_balance,untrusted_balance}`.
- Per-pool funding input:
  `private/pool-ark-<pool-id>/wallet/provision.json`, schema
  `spc.pool.ark.provision/v1`, carries the embedded stock wallet's
  `onchain_funding_address`, `float_spendable_sats`, `float_target_sats`,
  `server`, and `observed_at_unix`. This is startup evidence, not a live balance
  subscription. The separate `private/pool-ark-<pool-id>/provision.json` binds
  its descriptor/config digest. The funder sends real transactions to these
  addresses and returns transaction/confirmation evidence; it must not edit
  either SQLite or Postgres to make the float positive.
- AP-3b receives the local ArkInfo binding and existing AP-3a per-user branch
  xprv/child-seed provisioning files. It must use AP-3a's one-wallet foundation,
  not a new signer. The current runtime's empty identity roster deliberately
  avoids pretending its legacy random scan/spend-key records are BIP85 wallets.
- Agent startup belongs in `RunEngine::launch_all` after `launch_fronts()` and
  before `launch_miners()`. Replace the current explicit
  `ark_agent_bridge_unavailable` only with actual per-user AP-2 registration
  acknowledgment and AP-3b connection evidence; apply that same predicate to
  dynamic miner additions. No new registration codec or pool handler is needed.
- Node packaging handoff needs the metadata worker's immutable file manifest,
  exact patch/source hashes, Cargo.lock and tested binary receipt. This packet
  does not consume its live candidate directory.

## Merge boundaries

Do not copy bootstrap-era shared files wholesale into the primary worktree.
Transfer the new `nix/lab/ark*.nix`, `ark_service.rs`, infrastructure scenario,
Rust tests, Python lifecycle test and this packet's documentation. Apply only
the bounded hunks in `flake.nix`, lab `lib.rs`, `scenario.rs`, `resource.rs`,
`engine.rs`, `runtime.rs`, `inventory.rs`, `bins/spc-lab/src/main.rs`, and the
pool-wallet provisioning observation block in `bins/spc-pool/src/main.rs`.
The two in-tree `RuntimeOptions` test literals gain `ark: None`.

The SPEC writer applies the dedicated fragment in
`docs/changelog/2026-11-08-self-hosted-lab-ark.md`; no frozen plan is edited.

## Validation progress

The stock package build and its installed `--version` check passed. The Nix
launcher and hash-bound node derivations evaluate. Python syntax and scenario
JSON parse. The system Python is 3.10, so actual lifecycle testing uses Nix
Python 3.13.17 (`/nix/store/kp6p38x6zb21n7gdlnp9nsm2aagq7sg2-python3-3.13.17`).

The lab tools package passed 19 targeted integration tests in r3: Ark
infrastructure (3), lifecycle stop (3), operator inventory (5), supervisor
adoption (7), and two-run isolation (1). Its output is
`/nix/store/0c52hh2bzy89q1w7vy1mv0wabplkrwwm-spc-lab-ark-tools-0.1.0`, deriver
`/nix/store/6zx4cxjn7ayn0bm5703927cbjkq2cifp-spc-lab-ark-tools-0.1.0.drv`.
Earlier r2 also passed 52 library tests and 3 expanded-access tests. Those
passing checks were not rerun in r3. Initial scenario tests found and corrected
the empty-miner validation conflict and missing explicit pool-to-node links.

The same full-closure invocation compiled the node release product but failed
while compiling its test suite. Release-mode tests omit upstream debug-only
fault hooks; the Nix test phase is now corrected to use the ordinary test
profile while retaining a release product. There are also two genuine new
metadata test errors at `crates/rbitcoin-rpc/tests/methods/mod.rs:6712,6729`:
`ChainHub::new` expects `Query`, but the candidate passes `Arc<Query>`.
The node owner must correct/refreeze those calls before this Nix source binding
can advance. No node package output or complete launch command is claimed from
that failed derivation. Full log:
`/tmp/opencode/self-hosted-ark-runtime-nix-r3.log`.

### First real initialization

`/tmp/opencode/ark-live-r1/stop` used the built Nix lab/pool/server/storage tools
and the immutable preserved AP1b node with SHA256
`b75dad84bdd6663673a29c07352d10c4f78d2dcd14e885f62ad3209d214443e4`.
The actual stock `check-config`, `create`, `start`, `GetArkInfo`, and wallet
RPC succeeded. Observed network: regtest; rounds wallet total: 0;
state: `initialized_unfunded`; ArkId: `c41d55ec`; actual fee schedule SHA256:
`e09321914b842a94fa93c7251c795b915e4ba53b5cf8102bfd2acf234dc8933e`.
Receipt: `/tmp/opencode/self-hosted-ark-first-runtime-receipt.json`.

The run then failed because the ark-only config omitted the shared pool
parser's mandatory `listen` and `authority_public_key` fields. All seven
already-started actors were reported exited by the owned reaper. The render
is corrected with unused `127.0.0.1:0` / empty public-pin fields (no Ark-role
listener or Noise secret), and a new test checks the output through the real
pool parser. Source audit also corrected unconditional payout-key loading:
only a process actually running `payout` now attempts to read that SPc secret.
This first failed run is not reported as a passed lifecycle test.

### Corrected tools and actual lifecycle results

The r4 tools build passed all 20 selected tests, including the actual ark-only
pool-config parser regression:

- Output: `/nix/store/kkd5y9jy108f8ydp588xn804k177drib-spc-lab-ark-tools-0.1.0`.
- Deriver: `/nix/store/kmlc6pincy29dfvz4ynnn2f6flkm685v-spc-lab-ark-tools-0.1.0.drv`.
- `spc-lab` SHA256: `332b5ec2ce07d71cf72a44de4b0a480cd5bcd816e99ee98405d4514fc8e11e59`.
- `spc-pool` SHA256: `7d0ab78af64e0c817ce34c3e0ce5cb0c52ce5e31cbb0ea50c6c41cd986d76aec`.
- Compiled source: `/nix/store/jqd9mn6x2yghm2c39cgzps6kx2vyw90x-source`.
  SHA256 of compact JSON sorted `(relative_path, file_sha256)` pairs:
  `25b8b8d9a1548e4cbf7b81bfe8eaecb4b8a3aabdd8bc277a15ff6f0e7dc452fb`.
- Log/receipt: `/tmp/opencode/self-hosted-ark-tools-nix-r4.log`,
  `/tmp/opencode/self-hosted-ark-tools-r4-receipt.json`.

Runtime-only validation then passed all three cases in
`/tmp/opencode/ark-live-r2/receipt.json`. This used the built Nix daemon/tool
paths and the preserved AP1b `b75dad84...` node, not the unverified new node.
No heavy build ran during this runtime check; the heavy slot was released for
the node owner's bounded warm checks. Available space was 64718909440 bytes,
with a 2 GiB runtime-growth allowance.

| Actual case | Result |
|---|---|
| Start and duration-stop | PASS: 11 recorded actors and 35 captured OS children; all stopped. Actual ArkInfo and zero pool floats; three distinct PostgreSQL system identifiers. |
| Injected pool startup failure | PASS: all 7 already-started actors reaped. This deliberate negative is separate from the earlier accidental config failure. |
| Controller SIGKILL | PASS: all 11 actors and 35 captured children gone; owned reap succeeds. |
| Private storage and metadata | PASS: independent pool seeds, private config/cookie/mnemonic permissions, no pool Postgres/Noise-secret fields in ark configs; front ArkId/server/mailbox keys equal the actual ArkInfo binding. |
| Redis boundary | PASS: actual `ACL DRYRUN` allows order consumption and receipt publication; denies order publication and identity-stream/registration-ack access. No synthetic send order is written. |
| Operator inventory | PASS: unauthenticated requests rejected; authenticated inventory lists provider Postgres and both pool roles without seed values. |
| Payout-key read boundary | PASS: syscall tracing of the real startup records both ark-only config reads and successful payout-role signing-file reads, but **zero attempts** to open the absent `/run/secrets/spc/ark-order-key`. |

The read-boundary trace is `/tmp/opencode/ark-live-r2/signing-key-open.trace`,
SHA256 `2a312342b8772260898de50cc9d1406bcf15c9397ebecd125c1501d6ed09a0b4`.
It uses the host's `/usr/bin/strace` as a read-only test instrument, SHA256
`38a5c75cb29dd85ddd7780d54f5bf595554d7a1b5c42524b23065f5dc4c4b01d`.
The actual services remain the Nix-built binaries recorded above. The driver
traces file-open syscalls, not key contents. Tracer/controller failure cleanup
uses the test's own process group; daemon groups remain supervisor-owned.

### Corrected node source handoff (still not a binary pass)

After the tools invocation completed, the parent supplied the corrected r1
node source. This packet independently reverified every source row/mode/size,
the full aggregate and patch, and reproduced its NAR hash:

- 2106 files; 30897068 bytes.
- Full: `d06e366a57d04217d7096a2f28fd4daf381f15bc20366d3f9c68b79a5b74ec09`.
- Code: `1a3a62a6f4b4f8f2a9d6527b2e643b752ea7146284b9a180506997f5404f504d`.
- Patch: `abe0d65f33179d80e9f47c349f67a9136c01990c9d8b6786f379d141b21f3f48`.
- NAR: `sha256-JeoNicCbKC6Q+tPbc4la+qTI/tc1D4MwNQdX69/obhw=`.
- Receipt: `/tmp/opencode/self-hosted-ark-node-source-r1-receipt.json`.

`nix/lab/ark-node.nix` now binds this corrected source and ordinary test
profile, preserving the release product and old failed-source evidence.
The node owner's warm RPC/CLI tests get a bounded opportunity before another
expensive Nix node build. The new node package, full all-store-path launcher,
and canonical r4 positive pairs remain pending actual results. These successful
unfunded AP1b runtime cases do not satisfy those prerequisites or funded AP-5.

### Final r2 release node: built and tested

The final node handoff supersedes r1: full aggregate
`f1592c935e29b0b1fdf6429ad1b84e9ae925e9a9fcccd66b04f3d090680ff20f`, code
`9cbcb2a2051bd5da645872cc24d44b98048ce2b5892398d3f9b18f9611ad556d`, patch
`f54df78cb69b6417e4ee9d07e0cfb175faf249a336d82a109ead929aebe25e7c`, 2106 files /
30897163 bytes, NAR `sha256-t6LQYTA738Uzstg5AsakWCJS4yGanDBTn4FSpUmhiIU=`.
Every file/mode/hash and the NAR were reverified before the Nix build. Receipt:
`/tmp/opencode/self-hosted-ark-node-source-r2-receipt.json`.

The release product and ordinary-profile scoped tests passed:

- Output: `/nix/store/03pw2rf9wrxxnd9rl0l8ln0mnw9gcpx2-rbitcoin-node-ark-lab-0.8.99-profile-facts`.
- Deriver: `/nix/store/a6a6nq237i7hsgjflws2yq7hrms5p1h7-rbitcoin-node-ark-lab-0.8.99-profile-facts.drv`.
- Binary SHA256: `bba4752298aff258adb6cce99c6db849b02f50736a8ec32766ed1710045051aa`.
- 4 metadata tests and 22 CLI tests passed; no production test-hook patch.
- Actual version: `rbitcoin-node 0.8.99`, printed on stderr in the build log.
  The original metadata file captured stdout only and is empty; a lightweight
  provenance layer now captures both streams while reusing this exact tested
  release output. That layer has not yet been built.
- Log: `/tmp/opencode/self-hosted-ark-node-final-nix.log`.
- Receipt: `/tmp/opencode/self-hosted-ark-node-final-receipt.json`.

The parent has complete canonical r4 pairs for its debug binary `36e2387a...`;
those are **not** release-`bba47522...` proof. Fresh release-node canonical
checks and the all-store-path runtime remain pending.

To avoid recompiling unchanged tools, all 252 lab build-input files under
`bins`, `crates`, `config`, Cargo.toml and Cargo.lock were byte-compared to the
r4 tested source. `/tmp/opencode/self-hosted-ark-packet/launch-bindings.nix`
assembles the repository's actual `ark-runtime.nix` with that tested tools
output, the unchanged captaind, and the new node. It does not substitute a
different launcher implementation.

The canonical probe package (`nix/lab/ark-probe.nix`) has a closed 291991-byte,
14-file input, NAR `sha256-K1yy58S8HPIaxb/xmENcv26pVHsBCdWGn9fCJDItCao=`.
All 13 canonical code/schema rows match the frozen r4 manifest. The fourteenth
file is the parent's verified owned fixture (`e0be7504...`); packaging changes
only its canonical-code path and truthful release-node label. The parser,
normalizer and assertions are unchanged. A non-input review document has
drifted from the old 617-row manifest, so no current 617-row verification is
claimed. Exact selected-file hashes are in
`/tmp/opencode/self-hosted-ark-probe-source-receipt.json`.

### Current resource blocker

After acquiring the nonblocking shared lock, the lightweight launcher/probe
assembly found **53663698944 bytes (49.978 GiB)** available on `/nix/store`,
**23452256 bytes below** the mandatory 50 GiB floor. It stopped before invoking
Nix. No GC, deletion, repeated lock attempt, or captaind/tools rebuild occurred.
The existing tested release binary will be reused when that resource constraint
clears. `full_nix_launcher_verified`, release canonical binding and funded
readiness remain false until their actual checks run.

## Final completion: built closure and release-node runtime

The parent cleared the temporary disk block through its separately authorized
completed-cache cleanup (receipt
`/tmp/opencode/node-cache-cleanup-receipt-20261109.json`). This packet performed
no cleanup and did not recreate the removed Cargo target. The next nonblocking
locked invocation measured 57089814528 bytes available, with a 512 MiB assembly
allowance. It built exactly three lightweight derivations: node provenance,
canonical probe, and launcher. No captaind, tools or node Rust build was repeated.

### Exact outputs

| Component | Output | Deriver |
|---|---|---|
| Stock captaind | `/nix/store/4mnskprw4qigq8i8xqf4pps7iak1bnly-captaind-0.7.1-c419159` | `/nix/store/rwwk2780931zjfh0787ixwqws4l4ng41-captaind-0.7.1-c419159.drv` |
| Tested lab/pool/miner/scanner tools | `/nix/store/kkd5y9jy108f8ydp588xn804k177drib-spc-lab-ark-tools-0.1.0` | `/nix/store/kmlc6pincy29dfvz4ynnn2f6flkm685v-spc-lab-ark-tools-0.1.0.drv` |
| Release node source build | `/nix/store/03pw2rf9wrxxnd9rl0l8ln0mnw9gcpx2-rbitcoin-node-ark-lab-0.8.99-profile-facts` | `/nix/store/a6a6nq237i7hsgjflws2yq7hrms5p1h7-rbitcoin-node-ark-lab-0.8.99-profile-facts.drv` |
| Node provenance layer | `/nix/store/lbpv9zs0mdp36f912k63a31pg9qrzi54-rbitcoin-node-ark-lab-provenance` | `/nix/store/7qz9c30zfr57gf7xfwykgslzj37288gz-rbitcoin-node-ark-lab-provenance.drv` |
| Canonical r4 probe | `/nix/store/c5gh85rs220kc6g0yg9v2yk2nqd7szkx-spc-ark-canonical-probe-r4` | `/nix/store/nq3i2maayxvyblvszp3whxnby08691lv-spc-ark-canonical-probe-r4.drv` |
| Full launcher | `/nix/store/sslr7nzw0gp3qzb68is0p44aiqpn7a74-spc-lab-self-hosted-ark` | `/nix/store/x91p7xg5hfpaifcw73kah5yclv2hlpzb-spc-lab-self-hosted-ark.drv` |

The launcher also binds PostgreSQL 17.11 at `vync24csmxy5saql5ah77vkn59yg2x5h`,
Redis 8.10.3 at `nql1lir6650xigvf43n11zz4hh0lbw9z`, and grpcurl 1.9.4 at
`cxkyj4nmbbhhcmymfiss5476xrfb8xa0`, all under `/nix/store` with the full paths
in `package-bindings.json`. The provenance layer's executable resolves to the
tested release `bba47522...` binary; only its version capture changes.

The exact replay expression is frozen at
`/tmp/opencode/self-hosted-ark-packet/launch-bindings.nix`, referencing immutable
source `/nix/store/kpj9s8gyypip4sxjwjnmkyk9q449kfdr-source`. Evaluation reproduces
the actual `sslr7nzw...` output path. This explicitly reuses the byte-verified
r4 tools instead of rebuilding them because of documentation/packaging changes.

### Canonical r4 on the packaged RELEASE node

`/tmp/opencode/ark-nix-final/canonical-release-summary.json` binds node
`bba4752298aff258adb6cce99c6db849b02f50736a8ec32766ed1710045051aa`, helper
`332b5ec2ce07d71cf72a44de4b0a480cd5bcd816e99ee98405d4514fc8e11e59`, and contract
`754e1e6131955f9e5bbce7ef81b6f9aabaa2f1379ec6409f1c3a7a9e88be91f9`.

- Both CLI exits are 0, both reports schema-valid, nodes exit 0/reaped, and all
  controller/probe/node identities were verified gone.
- Both consensus fingerprints are
  `4634ab3e2ca66aa9d319b35d89131f7ad1fc48c3b315435652282fe9e20b79d3`.
- Default report SHA256:
  `1702284329e0e5a14e58c6147b18c22ff8ad65a45679a71451f15b2469482fa8`.
  Comparison digest: `4f2f61ee692172c3b29310861da2a0f14ec84003ae5ea7912aa1c4d41f870db4`.
- Bark-v31 report SHA256:
  `04d5da7d4cc2089e8295d34216f0b509fc618381ae0d94b061ec8a9f7d82fdbe`.
  Comparison digest: `1e10b83fc4327cb0d6e0b25d0d4e7dca260e5b05c20921b85acfcd2abd3ea5c6`.
- Same-environment policy-only counterfactual digests differ in both cases.
  Default retains `fail-version-gate`; bark-v31 is `surface_compatible`.
  Both remain `funded-unproven`, preserving the exact
  `maxdatacarriersize: expected JSON integer, got null` consumer-model failure.

### Fresh full launcher run and honest J2 boundary

The exact built launcher ran under a small explicit environment, with fresh
owned state at `/tmp/opencode/ark-nix-final-r2/run`, for 20 seconds. It started
11 actors, all executable paths resolving under `/nix/store`, and 35 captured
OS children. Three actual PostgreSQL system identifiers are distinct. Both
pool wallets reported zero spendable float; actual stock ArkInfo and fees were
recorded, front metadata matched the ArkId, and the real Redis ACL checks passed.
All actors/children stopped normally and the owned reaper succeeded.

The node's `/proc` executable SHA256 and start identity matched the manifest.
Its actual argv exactly matched the recorded arguments; its complete observed
ENV and argv are preserved privately in `actual-runtime-node-profile.json`,
SHA256 `7b349460b34a845f9b4d4ecbd286bdd1e682cd7db3742a039b355f4042c1c6a6`.
Actual RPC facts matched the canonical release case: height 0, empty mempool,
version 310000 with real rbitcoin subversion, and the consensus fingerprint above.

**J2 engine integration remains separate.** The existing lab engine's ownership
token contains `:`, rejected by canonical r4's token grammar. The first full-run
test aborted on this real rejection and cleaned up; that evidence remains at
`/tmp/opencode/ark-nix-final/run`. The corrected observation test preserves the
exact rejection `traffic_launch_profile_unsupported: missing or invalid ownership
token`, stores a null canonical runtime comparison digest, and binds raw observed
argv/ENV by its file hash. It does not rewrite the token, relax the normalizer,
or claim an engine gate. The successful canonical release pairs were reused,
not rerun. Full receipt:
`/tmp/opencode/ark-nix-final-r2/full-launcher-receipt.json`.

### Launch command

After checking the same resource budget, use a fresh directory:

```sh
/nix/store/sslr7nzw0gp3qzb68is0p44aiqpn7a74-spc-lab-self-hosted-ark/bin/spc-lab-self-hosted-ark \
  --run-dir /tmp/opencode/ark-next-run \
  --base-revision b0ab4ca34fda07c61a6335d05f598eaacc69678c \
  --dirty-tree-hash 616e4a787d4c341566495565aa7c7f84e3e9bf3c8026a536dfce26172fb8b473 \
  --operator-ui --listen 127.0.0.1:0 --duration-secs 120
```

For this frozen multi-source closure, the supplied dirty-tree identifier is
the launcher-script SHA256, binding its declared Nix store paths; individual
source manifests and binary hashes remain authoritative in the package bindings.
It is not represented as a Git tree digest. Fresh URLs are printed during the
bounded run. No network from these tests was left running. Native funding,
the registered-agent/miner bridge, board/receive/refresh/offboard and complete
mining-to-Ark acceptance remain outstanding; no funded proof is claimed.
