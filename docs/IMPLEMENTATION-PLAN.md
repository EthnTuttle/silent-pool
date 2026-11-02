# Implementation Plan and Orchestration Protocol

Status: active. This file is the build plan. `docs/STATUS.md` is the live
status board. `docs/SPEC.md` remains normative; `DESIGN.md` outranks it for
payout derivation, tiers and Ark delivery. Where this plan is wrong, fix the
plan, not the spec.

Added 2026-11-02. Two requirements beyond the original spec are recorded as
`docs/decisions/2026-11-02-native-sv1-nix-and-protocol-baseline.md`:

1. `spc-pool` serves **SV1 and SV2 natively** (the translator remains for
   remote miners, but the pool no longer requires it).
2. Every build, test, CI and deployment artifact is a **Nix flake output**.

## 1. Target

A pool that:

- accepts SV1 and SV2 connections directly, with the BIP 352 address as the
  only identity and fail-closed parsing on both listeners;
- pays coinbase SPc outputs (pool-template and JD modes) and sub-threshold Ark
  payouts from a third-party server;
- runs SLICE/PPLNS-JD accounting with the spec's pending, trim, tier and
  idempotency rules;
- serves the type-32 audit extension and the `0x5350` delivery-capability
  extension;
- builds, tests, deploys and runs from `nix build` / `nix flake check`;
- is proven end-to-end on regtest, with the Ark leg runnable against a stock
  `captaind`.

## 2. Operating model

### 2.1 This thread

This session is the orchestration/status tab. It does not hold long
implementation context. Its jobs:

1. dispatch work packets to child sessions with an explicit scope, spec
   references, owned paths, acceptance tests and model;
2. merge/verify their commits, run `nix flake check`, and update
   `docs/STATUS.md`;
3. resolve ambiguity by decision record, never by asking the user;
4. keep the branch pushed.

### 2.2 Session roster

Each work packet gets a fresh child session (or a resumed one when iterating on
review). Models are chosen for the task, not for uniformity.

| Session role | Model | Context it gets | Output |
|---|---|---|---|
| Orchestrator (this) | `tff/deepseek-v4.1-flash-1017` | `AGENTS.md`, this plan, `STATUS.md`, diffs | dispatch, verification, status, commits |
| Spec architect | `tff/kimi-k3` | `DESIGN.md`, `SPEC.md`, `ROADMAP.md`, relevant decision records | decision records, spec edits, ambiguity rulings |
| Core implementer A | `tff/kimi-k3` | one work packet + cited spec sections + frozen vectors | `spc-crypto`, `spc-accounting`, `spc-scan` |
| Core implementer B | `tff/glm-5.3` | same, disjoint paths | `spc-store`, `spc-pool` roles, `spc-agent` |
| Protocol implementer | `tff/kimi-k3` | SPEC §3, PR #203 text, `WIRE-FORMATS.md`, `stratum-core` API | SV1/SV2 listeners, 0x0003, 0x0020, 0x5350 |
| Nix/CI engineer | `tff/glm-5.3-flash` | this plan §7, `UPSTREAM_DEPS.md`, repo tree | `flake.nix`, modules, CI workflows |
| Code reviewer (normative) | `tff/nemotron-3-ultra` | packet diff, cited spec sections, test list | blocking review findings or approval |
| Code reviewer (security) | `tff/nemotron-3-ultra` | diff plus `SPEC.md` §1/§2.2/§11.3/§11.9 | adversarial findings |
| Docs writer | `tff/glm-5.3` | code + spec + decision records | `docs/spc-spec.md`, deltas docs, operator docs |
| E2E engineer | `tff/kimi-k3` | §8.4, regtest harness, built binaries | harness, CI job, soak scripts |

Rules for every child session:

- Read `AGENTS.md` first, then the packet's cited sections. Do not read
  `docs/research/**` unless the packet says so.
- Own a disjoint path list. Never edit another packet's files without saying
  so in the final report.
- Test-first: write the failing test from the spec citation, then the code.
- No `unsafe`, no `unwrap()` in library paths except proven invariants with a
  comment, no silent fallbacks on identity/address/amount parsing.
- End with: tests run, commit SHA(s), files touched, open questions (recorded,
  not asked).
- Commit subjects state **why**. Body states the behavioural delta and the
  spec section. One logical change per commit, tests green.

### 2.3 Review gates

Every packet passes two gates before the orchestrator marks it done:

1. **Normative review** (`claude-opus-4.6`): does the code do what the cited
   spec says, including failure modes and privacy invariants?
2. **Security review** (`gpt-5.4`) for packets touching keys, identity parsing,
   settlement, Redis ACL boundaries or the Ark stream separation.

Reviewers may commit fixes only in the packet's owned paths. Normative
disagreements are resolved by the spec architect and recorded; they are never
settled by majority.

### 2.4 Decisions

Any behaviour the spec leaves open, or any new requirement, gets a dated
`docs/decisions/*.md` record plus a `SPEC.md` frontmatter changelog entry in the
same commit. No decision is deferred to the user.

## 3. Definition of done

The effort is complete when all of these hold:

- `nix flake check` is green on a clean checkout: fmt, clippy `-D warnings`,
  unit tests, proptests, integration tests, doc build.
- `nix build .#spc-pool .#spc-agent .#spc-scan` produces the three binaries.
- `nix develop -c ./testing/regtest-e2e.sh` runs the SPEC §8.4 scenarios
  1-8 (Ark leg 3 and §11.9 behind `SPC_ARK_E2E=1`, with a stock `captaind`).
- SV1 and SV2 clients can both open a channel with a `tsp` identity and be paid
  in the same block.
- Pool-template and JD modes both produce scannable coinbase outputs.
- The type-32 verifier catches the seeded `cheat-slice-difficulty` build.
- `spc-pool --roles ark` passes the §11.3 separation tests: it reads only
  `spc:ark:orders`, writes only `spc:ark:receipts`, and the captured stream has
  no identity field.
- `docs/spc-spec.md`, `docs/share-accounting-ext-deltas.md`, operator docs and
  `docs/STATUS.md` are current.
- All work is committed with why-first messages and pushed to origin.

Explicitly out of v0.1: mainnet, BIP 352 labels, covenants, Lightning rail,
solo-pool profile, finder bonuses.

## 4. Work packets

Packets are ordered by dependency. Wave numbers say what can run concurrently.
Every packet's "Tests" list is the packet's gate; the packet is not done until
those tests exist and pass.

### WP0 - Workspace, Nix, CI, protocol baseline (wave 0)

- **Spec**: §7.1, §9; `UPSTREAM_DEPS.md`.
- **Deliverables**: Cargo workspace with empty-but-compiling crates; pinned
  `rust-toolchain.toml`; `flake.nix` (devShell, packages, checks, NixOS
  modules); `.github/workflows/ci.yml` calling `nix flake check`; `justfile`
  (or `Makefile`) for local parity; `README` build section.
- **Protocol baseline**: `stratum-core` 0.6.0 (codec, messages, Noise, SV1
  codec) from crates.io; `spc-ext` owns `0x0003` and `0x5350` message structs
  so no fork of `extensions_sv2` is needed. Recorded in the 2026-11-02
  decision.
- **Tests**: `cargo test --workspace` on the skeleton; `nix flake check`
  executes it; a smoke test that each binary prints `--version`.
- **Session**: Nix/CI engineer, then normative review.
- **Commit**: `build: establish the workspace and make nix the only build path`
  - why: one reproducible toolchain for every later packet.

### WP1 - `spc-crypto` and frozen vectors (wave 1)

- **Spec**: §2.1-§2.3, §8.1; `DESIGN.md` "Derivation scheme".
- **Deliverables**: `parse_sp_address`, `sp_id`, `SendKey` (zeroising), `ecdh`,
  `tier_tweaks`, `P`/`U` derivation, spend secrets, mailbox blinding
  (`M`, `D`, static/epoch variants), carrier emit/parse (`scriptSig`, `OP_RETURN`),
  `ark1` address builder behind the `ark` feature.
- **Port**: `spc_ark/src/lib.rs` and `spc_ark_mailbox/src/lib.rs` logic and
  frozen vectors unchanged; add negative tests (bad HRP, wrong version, bad
  checksum, odd-Y, zero scalar, `B_ark == B_spend`, tag separation).
- **Tests**: every §8.1 vector, including negative controls, as `#[test]`;
  property test that pool-side and miner-side ECDH agree for random keys.
- **Session**: Core implementer A.
- **Review**: normative (`claude-opus-4.6`) + security (`gpt-5.4`).
- **Commit**: `feat(crypto): make the spc derivation executable and freeze it`
  - why: every pool and agent path keys off this crate; vectors are the
  contract.

### WP2 - `spc-scan` (wave 2)

- **Spec**: §2.4, §2.5, §6.2, §8.4 steps 1 and 7.
- **Deliverables**: carrier parser precedence, `scan_block`, tweak export,
  light-proof input format, `find`/`light`/`export-tweaks`/`vectors` CLI,
  Bitcoin RPC block source behind a feature and a file source for CI.
- **Tests**: carrier round-trips; scan finds `H` and `H-1`; wrong merkle branch
  rejected; a block with five outputs found by five scan keys; light path equals
  full path.
- **Session**: Core implementer A.
- **Commit**: `feat(scan): recover spc outputs from a coinbase alone`
  - why: miners and third parties must verify payouts without trusting the pool.

### WP3 - `spc-accounting` (wave 2)

- **Spec**: §4.1-§4.8, §8.2, §11.1, §11.6; O11 decision record.
- **Deliverables**: `Slice`, sealing, window selection with `high_water`,
  SLICE scores, pending ledger, tier routing, trim and autoscaler, distribution
  builder (sats and weights), conservation and claim formula.
- **Tests**: §4.5 worked example fixture; proptests for §4.6 (a)-(f) and
  §11.9 conservation; `delta = 0` equals an independent plain-PPLNS oracle;
  deterministic `sp_id` ordering; budget/slot bounds; tier disjointness.
- **Session**: Core implementer A (or B if WP1 slips).
- **Commit**: `feat(accounting): implement slice accounting as pure functions`
  - why: correctness must be provable without Redis, Postgres or SV2.

### WP4 - Store, wire formats, migrations (wave 3)

- **Spec**: §5.1, §5.2, §5.3, §7.4; `WIRE-FORMATS.md` §3.
- **Deliverables**: `spc-store` models and sqlx migrations for every table;
  Postgres writer with advisory-locked migrations; Redis stream codecs for
  `spc:blocks`, `spc:slices`, `spc:stats`, `spc:arkreg`, `spc:front:events`,
  `spc:ark:orders`, `spc:ark:receipts`; bootstrap RPC types; trim discipline.
- **Tests**: migration up/down; idempotent settlement; orphan path writes no
  ledger rows; redis codec golden bytes match `WIRE-FORMATS.md`; trim never
  passes a live cursor or the 600 s floor.
- **Session**: Core implementer B.
- **Commit**: `feat(store): make settlement idempotent at the database boundary`
  - why: redelivery and restarts must never pay twice.

### WP5 - `spc-pool front` with native SV1 and SV2 (wave 4)

- **Spec**: §3.1-§3.4, §3.6, §6.3, §9, §2.2, §2.4.1; 2026-11-02 decision.
- **Deliverables**: config loader with all invariants; SV2 listener
  (SetupConnection, extensions negotiation, channel open fail-closed, jobs,
  shares); native SV1 listener (`mining.subscribe/authorize/notify/submit`,
  username = identity, worker suffix stripped); one `a_send[H]` per height,
  zeroised; coinbase prefix/suffix builder with the `push32` carrier; slice
  aggregation; `spc:blocks` event with `entries`; metrics.
- **Tests**: the §3.1 table on both listeners; no coinbase built for a rejected
  channel; share replay and stale-job rejection; slice sealing; restart
  mid-height yields a new scannable `A_send`; a `push32` budget failure at
  channel setup.
- **Session**: Protocol implementer + Core implementer B, disjoint files
  (`src/sv2/**`, `src/sv1/**` vs `src/front/**`).
- **Commit**: `feat(pool): accept sv1 and sv2 miners with one fail-closed identity`
  - why: the address is the account on every ingress, and a silent default is
  the defect the design exists to avoid.

### WP6 - `payout` role and HTTP API (wave 5)

- **Spec**: §3.3, §4.6, §5, §6.1, §7.4, §11.1.
- **Deliverables**: settlement worker (slice persistence, confirmation gate,
  orphan discard, ledger apply, pending, scrub); tier booking; `ark_payout`
  scheduling; public and `b_scan`-authenticated API; `healthz`/`metrics`.
- **Tests**: §7.4 redelivery/orphan/down-up; scrub leaves no scripts; API
  rejects unsigned per-identity reads; liability endpoint returns to zero after
  a send; `pending` carried into the next distribution.
- **Session**: Core implementer B.
- **Commit**: `feat(payout): settle only from the mined coinbase and an event`
  - why: JD revenue differs from the snapshot, so settlement must read chain
  data and be idempotent.

### WP7 - JD mode: `spc-jds` module and `spc-jdc` role (wave 5)

- **Spec**: §3.5, §3.6, §4.6, §8.3, §8.5; PR #203 at `47dbedd`.
- **Deliverables**: `0x0003` codec in `spc-ext`; JDS publication/validation,
  `distribution_id` (`front_id << 56 | local_seq`), same-height grace, forced
  republish; JDC job construction, own-payout check, refusal rules; both
  coinbase-only and full-template paths.
- **Tests**: publication matrix; mutated/reordered/missing carrier rejected;
  height-straddling rejected; same-height previous id accepted; JDC refuses
  when negotiation fails on either connection; outputs scannable via
  `OP_RETURN`.
- **Session**: Protocol implementer.
- **Commit**: `feat(jd): publish and verify payout distributions per height`
  - why: JD miners build the coinbase, so the pool must prove the outputs
  before accepting a share.

### WP8 - `0x5350` delivery capabilities (wave 6)

- **Spec**: §11.4, §11.9; `WIRE-FORMATS.md` §3.3.
- **Deliverables**: `SetArkIdentity`, `RegisterEpochMailboxes`,
  `ArkPayoutNotice`; signature verification over the exact field layout;
  fail-closed error codes; `ark_delivery` upsert via `spc:arkreg`; front cache
  and rev application.
- **Tests**: full §11.9 unit matrix; another miner's `sp_id` rejected; nothing
  derived for a rejected registration; `spc:arkreg` recomputes `sp_id`.
- **Session**: Protocol implementer.
- **Commit**: `feat(ext): register ark delivery without weakening identity`
  - why: delivery data is private and miner-signed; it must never become an
  account key.

### WP9 - `ark` role (wave 6)

- **Spec**: §11.3, §11.9; `WIRE-FORMATS.md` §3.4.
- **Deliverables**: signed order verification and caps; bark wallet adapter;
  float, send, receipt, retry and `failed_to_pending`; matured pool output
  discovery by outpoint; board only the shortfall; own-host secret loading and
  refusal rules.
- **Tests**: §11.3/§11.9 tests with a mock Ark transport; no identity field in
  a captured order; reorg cancellation; float target behaviour; refusal to run
  self-hosted on mainnet or with a shared secret store.
- **Session**: Core implementer B + security review.
- **Commit**: `feat(ark): keep the float separate from every identity secret`
  - why: compromising the hot float must not deanonymise a single coinbase
  payout.

### WP10 - `spc-agent`: translator, JDC, wallet, mailbox (wave 7)

- **Spec**: §1, §3.6, §11.5, §11.8, §11.9; `MAILBOX.md`.
- **Deliverables**: one binary with roles; `sp_address` validation; translator;
  JDC; own-payout verifier; both-tier scanner; SP-aware keystore; registration
  before shares; mailbox modes; refresh/offboard; shutdown warnings.
- **Tests**: stock SV1 miner through the translator to a `tsp` payout; JDC
  happy path; no shares before `SetArkIdentity.Success`; mailbox read schedule;
  keystore accepts `U`; shutdown warning.
- **Session**: Core implementer B + Protocol implementer.
- **Commit**: `feat(agent): make one seed serve both tiers without holding b_spend`
  - why: the miner side is where key separation is actually enforced.

### WP11 - Type-32 share accounting (wave 7)

- **Spec**: §3.7, §8.3, §8.4; `extension.md` @ `c661e93`.
- **Deliverables**: message structs on `stratum-core`; server from slice store
  plus bootstrap blobs; sampling verifier; `spc-scan audit`;
  `docs/share-accounting-ext-deltas.md`.
- **Tests**: `GetWindow` threshold; merkle paths; no identity fields; seeded
  cheating build caught within three windows at 4x8; deltas doc complete.
- **Session**: Protocol implementer.
- **Commit**: `feat(audit): let a miner recompute the window it was paid on`
  - why: accounting the miner cannot check is a trust assumption, not a pool.

### WP12 - Regtest e2e, metrics, deployment artifacts (wave 8)

- **Spec**: §8.4, §9, `ROADMAP.md` Phase 8.
- **Deliverables**: `testing/regtest-e2e.sh`; bitcoind/redis/postgres Nix
  services; scenarios 1-8; full-mempool scenario 9; metrics assertions;
  third-party `spc-scan` verification; testnet runbook; NixOS modules for
  front/payout/api/ark; secrets layout.
- **Tests**: the whole script, in CI and locally.
- **Session**: E2E engineer + Nix/CI engineer.
- **Commit**: `test(e2e): prove both tiers on regtest before any testnet claim`
  - why: the spec's acceptance is behavioural, not unit-level.

### WP13 - Final audit and release (wave 9)

- **Deliverables**: full spec-to-test traceability check, two independent
  reviews, `docs/spc-spec.md`, README status update, CHANGELOG, tagged
  `v0.1.0`, pushed.
- **Session**: orchestrator + both reviewers + docs writer.

## 5. Test matrix (traceability)

| Spec | Test | Location | Gate |
|---|---|---|---|
| §2.1 identity | parse table | `spc-crypto/tests/identity.rs` | unit |
| §2.2 `a_send` | zeroise, no persistence, repeat alarm | `spc-pool/tests/a_send.rs` | integration |
| §2.3 derivation | frozen vectors + negative controls | `spc-crypto/tests/vectors.rs` | unit |
| §2.4 carriers | byte round-trip | `spc-crypto/tests/carriers.rs` | unit |
| §2.5 scan | find/light/tolerance | `spc-scan/tests/scan.rs` | unit |
| §3.1 channel open | both listeners, full table | `spc-pool/tests/identity_e2e.rs` | integration |
| §3.2 extensions | negotiation matrix | `spc-pool/tests/extensions.rs` | integration |
| §3.3 job build | prefix/suffix, carrier, event | `spc-pool/tests/jobs.rs` | integration |
| §3.4 shares | replay, stale, slice seal | `spc-pool/tests/shares.rs` | integration |
| §3.5 JD | publication/validation/grace | `spc-pool/tests/jd.rs` | integration |
| §3.6 own payout | warn/refuse/abort | `spc-agent/tests/verify.rs` | integration |
| §3.7 type 32 | server + verifier | `spc-share-ext/tests` | integration |
| §4 accounting | proptests + fixture | `spc-accounting/tests` | unit |
| §5 store | migrations, idempotency, scrub | `spc-store/tests` | integration |
| §6.1 API | auth and public split | `spc-pool/tests/api.rs` | integration |
| §7.4 settlement | redelivery/orphan/missing event | `spc-store/tests/settle.rs` | integration |
| §8.4 e2e | scenarios 1-9 | `testing/regtest-e2e.sh` | e2e |
| §11 tier | routing + conservation | `spc-accounting/tests/tiers.rs` | unit |
| §11.3 separation | ACL + payload schema | `spc-pool/tests/ark_sep.rs` | security |
| §11.4 registration | error matrix | `spc-ext/tests/identity.rs` | unit |
| §11.9 Ark | mock + real captaind | `testing/ark-e2e.sh` | e2e |

## 6. Local development

```sh
nix develop                      # rust, postgres, redis, bitcoind, jq, sqlx-cli
just check                       # fmt + clippy + test
just test                        # cargo test --workspace
just e2e                         # regtest scenarios (starts services)
just ark-e2e                     # requires captaind in PATH, SPC_ARK_E2E=1
```

Services for integration tests:

```sh
nix run .#services               # postgres + redis on localhost
export DATABASE_URL=postgres://spc:spc@localhost:5432/spc
export REDIS_URL=redis://localhost:6379
```

## 7. Nix and CI design

- `flake.nix` inputs: `nixpkgs`, `flake-utils`, `rust-overlay` (or fenix) for
  the pinned toolchain.
- `packages`: `spc-pool`, `spc-agent`, `spc-scan`, plus `spc-tests` and
  `spc-e2e`.
- `checks`: `fmt`, `clippy`, `test`, `doc`, `e2e` on Linux.
- `devShells.default`: rust toolchain, `sqlx-cli`, `postgresql`, `redis`,
  `bitcoind`, `captaind` (optional, from the pinned bark source), `jq`, `curl`.
- `nixosModules`: `spc-pool` (front/payout/api/ark role options), `spc-agent`,
  `spc-postgres`, `spc-redis`; secrets via `environmentFile`, never in the Nix
  store.
- CI: GitHub Actions with `cachix/install-nix-action`, then
  `nix flake check --print-build-logs`. No non-Nix build path is maintained.
- Reproducibility: `Cargo.lock` committed; `flake.lock` committed; upstream
  git dependencies pinned by SHA. If a crates.io baseline is used, it is
  recorded in `UPSTREAM_DEPS.md` and the decision record.

## 8. Risk handling

Spec risks R1-R15 apply unchanged. Build-specific risks:

| Risk | Mitigation |
|---|---|
| `stratum-core` 0.6.0 API drift vs pinned SRI | isolate protocol code behind `spc-pool::sv2`; a failing upstream API costs one module, not the accounting core |
| Native SV1 scope creep | minimal SV1 subset (subscribe/authorize/notify/submit/set_difficulty), one file, spec-tested |
| Ark e2e needs external `captaind` | mock transport for CI; real server behind `SPC_ARK_E2E=1`; pin bark commit |
| Long build times | workspace split; `sccache` in devshell; CI cache via Cachix |
| Context loss across sessions | `docs/STATUS.md` plus one commit per packet; every session begins by reading them |

## 9. Waves and parallelism

- Wave 0: WP0.
- Wave 1: WP1.
- Wave 2: WP2 + WP3 (disjoint crates).
- Wave 3: WP4 (depends on WP3 types).
- Wave 4: WP5 (depends on WP1-4).
- Wave 5: WP6 + WP7 (payout/API vs JDS/JDC, disjoint modules).
- Wave 6: WP8 + WP9 (extension vs ark role, disjoint).
- Wave 7: WP10 + WP11 (agent vs type-32, disjoint).
- Wave 8: WP12.
- Wave 9: WP13.

Each wave ends with `nix flake check`, both reviews for the wave's risky
packets, a status update and a push.

## 10. Status protocol

`docs/STATUS.md` is append-only per wave and has exactly these sections:

- Current wave and packet.
- Done (packet, commit, test command, evidence).
- In progress (owner session, paths).
- Blocked (reason, decision needed, who decides).
- Next dispatch (packet, model, prompt summary).

A packet is only "Done" when its tests are in the repo, `nix flake check`
passes, review is recorded, and the commit is pushed.
