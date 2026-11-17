# Status

Live build board. `docs/IMPLEMENTATION-PLAN.md` defines the packets and gates.
A packet is Done only when its tests exist, `nix flake check` passes, both
reviews are recorded for risky packets, and the commit is pushed.

## Focus wave (2026-11-17)

Dispatched and merged per the MERGE-QUEUE focus-packet table. Base was master
after fast-forwarding `dev/testnet5`.

- **testnet5 + BIP54 (`dev/testnet5`, fast-forwarded):** consensus
  `ChainNetwork` distinct from SP/BIP32 encoding (Testnet5 code 4, BIP95
  genesis), endpoint/SQL/Redis/bootstrap chain bindings (migration 0013),
  BIP54 template negotiation on the rpc source, opt-in pinned
  `rbitcoin-node-testnet5-dev` package. Stock bark cannot represent Testnet5
  yet; agent Testnet5 runtime remains fail-closed.
- **Denomination ladder (`wp-ladder-audit`):** the 2026-11-03 decision was
  already fully implemented on master via the source union + ladder-entry-v2;
  the audit (`docs/reviews/2026-11-17-ladder-implementation-audit.md`) maps
  every checklist item to code and re-lands three union-dropped tests (era-1
  619 bound, coinbase-remainder scheduling, banded e2e assertion). The old
  `ladder` branch is superseded and archived as tag `archive/ladder-pre-union`.
- **SV2 Template Provider stages 1+2 (`wp-sv2-tp-port`):** `source = "sv2-tp"`
  is a live TDP client (`template_tp.rs`) with BIP54-over-TDP, witness
  transactions allowed, `SubmitSolution` behind the `BlockSubmitter` seam, and
  `rpc_url` as chain view only (decision
  `docs/decisions/2026-11-05-sv2-tp-chain-view.md`, ported).
  `testing/regtest-rbitcoin-tp-e2e.sh` passed live against the pinned
  testnet5-dev rbitcoin node in regtest. `source = "ipc"` is deferred, not
  stubbed: startup fails closed with an explicit error (new deps and an IPC
  node are a separate packet).
- **Prometheus families (`wp-metrics-families`):** roles publish identity-free
  JSON snapshots to TTL'd Redis keys; the api role exports `a_send` age/repeat,
  open-slice/sealed-slice, distribution slots/budget, channel and JD rejection
  codes, type-32 requests, ark float/rejects, overdue, orphaned settlements,
  registration lag and per-stream consumer lag. Absent snapshot = absent
  family, never zero. Outbox/tip-divergence/surplus stay with multi-region.
- **Cheat-slice statistical test (`wp-cheat-slice`):** SPEC §8.3 fixture
  landed; seeded 400-trial run catches the misstating pool within 2 windows
  (per-window 0.9425 vs 0.9375 model), release builds with the feature refuse
  to compile.
- **Grafana lab observability (`wp-grafana-lab`):** opt-in
  `spc-lab-observability` Nix wrapper (Grafana + Prometheus, loopback-only,
  run-dir state, scrape targets rendered from the run manifest), provisioned
  dashboards split exported/planned. LAB-RUNTIME §13.
- Repo hygiene: AGENTS.md no longer claims spec-only status and machine-local
  build policy (disk budget, lock files) left the repo contract; older
  harnesses pass explicit `--network regtest` to `spc-scan find`.

Known issues carried out of the wave: none remaining.
`ark::tests::held_order_stays_unacked_until_due` was diagnosed 2026-11-17:
the test predated the durable fair PEL cursor (decision
2026-11-10-pre-request-ark-readiness) and skipped the wrap pass the cursor
requires on exhaustion; the Held-stays-unacked property was never violated.
The test now covers the full held -> wrap -> sent -> no-double-send
lifecycle and still skips without `SPC_TEST_REDIS_URL`.

CI resurrection (post-wave): `nix flake check` had been failing at eval since
the ap5 intake exported a non-derivation (`ap5Packages`) under `packages`, so
the lab-era tests had never run in the sandbox. Resurrecting it surfaced and
fixed: workstation-convention paths in tests (`/tmp/opencode`,
`/usr/bin/python3`, `/bin/true`), the front chain-identity Redis dependency in
`main_sv2_pipeline` (now seed-or-skip, with a disposable redis-server in the
check derivations), and two real runtime bugs - the lab miner-profile lookup
breaking under bind mounts (now falls back to the supervisor's sha256
executable identity) and a supervisor fork/exec race that could record the
supervisor's own binary as a child's identity. `nix flake check` is green
again and is the merge gate it was always supposed to be.

## Session roster (merge wave 2, 2026-11-02)

| Role | Session | State |
|---|---|---|
| Orchestrator (merge gate, this board) | `ses_e60dd1e1` | active |
| PPLNS-JD/SLICE accounting + settlement | `ses_e61952d7` | checkpoint `186d99e`; gate/review fixes `e2e9943`..`c403b7c` |
| De-mock (metrics, per-height descriptor, ark role) | `ses_e61943da` | landed `3c3a19c`, `74807d3`, `65d87c1` |
| Tor transport (spec and docs) | `ses_e60e6aac` | landed `8aca4be`, `914509f`; spec-ahead-of-code, closed |
| Tor transport implementation | `ses_e5d9b9134` | landed `e919d35`..`57bd046`, merged `9be60a9`; review fixes `f9a046c`; scope narrowed to mining connections 2026-11-04 |
| Stock bark Ark adapter | `ses_e5d9b9133` | landed `bdfdab6`..`ec4b047`, merged `2358a63` |
| Amount-banded payouts analysis | `ses_e5d9b9192` | landed `116ed34`, merged |
| Settlement window target | `ses_e5cf0b2b9` | dispatched 2026-11-03 on branch `window-target` |
| JD fallback + template rebuild | `ses_e5cf0b27d` | landed `85db04d`..`18ac53f`, merged |
| Banded amounts + Ark overage design | `ses_e5ceb5ce6` | landed `bdd0be5`..`7be2541`, merged `d62bf23`; ladder decided |
| rbitcoin node fork (testnet4) | `ses_e6195c6a` | separate repo; 6 commits pushed to the fork; full sync to testnet4 tip verified once, re-syncing after a datadir wipe |
| rbitcoin RPC e2e harness | `ses_e592c174b` | landed `5f8d8b2`..`2d8d4ec`, merged `657c89a` |
| SV2 Template Provider source | `ses_e590d51b2` | stage 1 `f4015ed`; stage 2 in progress on branch `sv2-tp` |
| Merge / integration queue | `ses_e5394012e` | active; sole writer to the shared surface (`docs/MERGE-QUEUE.md`) |
| Denomination ladder impl (WP0-WP2) | `ses_e5393b4e2` | dispatched 2026-11-05 on branch `ladder` |
| Proxy/upstream analysis | `ses_e60f175c` | closed, `docs/analysis/2026-11-02-proxy-and-upstream-options.md` |
| Multi-identifier analysis | `ses_e638a2e6` | closed, `docs/analysis/2026-11-02-multi-identifier-payouts.md` |

## Wave 3 (2026-11-03)

- Tor transport (`e919d35`..`57bd046`, merged `9be60a9`): SPEC §3.8 lands in
  code. In-tree socks5h CONNECT dialer with per-connection isolation
  credentials (no new dependency crates, vendor hash untouched), `[reach]`/
  `[net]` config on both binaries, onion-published listeners forced to
  loopback at startup, `.onion` targets without a proxy rejected, and
  `GET /api/pool` publishing `endpoints` verbatim from `[reach] advertise`.
  The agent's SV2 upstream and JDP dials route through `[net]`.
  **Scope narrowed 2026-11-04:** Tor covers mining connections only. Ark
  egress is direct (the bark transport dials the server itself and mailbox
  reads are direct), `spc-pool` no longer has a `[net]` section, and the
  accepted residual is that the Ark server and mailbox endpoint see the
  network address. See `docs/decisions/2026-11-04-tor-scope-mining-only.md`.
- Amount-banded payouts analysis (`116ed34`): overage-to-Ark relocates exact
  amounts to the Ark server; band + overage-to-pending bounds the on-chain
  leak to `ceil(log2 L)` bits per block. No code change.
- **rbitcoin regtest harness merged (`657c89a`):** `testing/regtest-rbitcoin-e2e.sh`
  (front-only, RPC template source, miner, scan) and
  `testing/regtest-rbitcoin-reorg-e2e.sh` (full stack, `confirmation_depth=2`;
  settle, `invalidateblock`, competing branch). The reorg invariant is now
  tested for the first time: the invalidated block becomes `orphaned`, exactly
  one `block_found` row exists at the height, and ledger/liability are
  byte-identical to the pre-orphan state, so an orphaned payout can never be
  booked. rbitcoin reads an existing 0600 cookie (unlike bitcoind) and accepts
  a bare hex script for `generatetoaddress`. Validated against the local
  upstream-master build `e8cf9c9d6`; the fork pin is reference-only.
- **SV2 Template Provider (in flight):** `[template] source = "sv2-tp"` as a
  TDP client over Noise_NX against rbitcoin's TP, with `SubmitSolution`
  replacing `submitblock`, so rbitcoin becomes the native e2e node path.
- **Ladder decided and recorded (`d62bf23`, record `7be2541`):** coinbase
  amounts are multiples of a constant ladder `{500k, 1M, 3M, 5M}` sats, greedy
  largest-first, remainder to Ark. No era scaling; each halving doubles the
  bands' relative size and the Ark-only population, which is accepted. The
  constant ladder makes the band function pure over the amount alone.
  `max_payout_slots` 1024 (2048 fee headroom). Era-1 output bound 619 (26.6 KB,
  106k WU). Float = flow-per-find x 94 x hashrate share x 1.5 burst factor;
  about 0.34 / 1.9 / 3.5 BTC for 100 / 1000 / 10000 miners at 1 percent
  hashrate, worst case 2.9 BTC at 1 percent and 29.4 BTC at 10 percent. SPEC,
  DESIGN, WIRE-FORMATS and ROADMAP amended; no code yet.
- JD fallback merged: declare-bounded then hold-and-alarm, coded JDS declines,
  and a full-template rebuild on `SetNewPrevHash`.
- Stock bark Ark adapter merged (`2358a63`): stock bark at `c419159` behind a
  transport seam, per-order idempotency, float boarding from matured pool
  outputs, receipts, caps. The `command` transport is dev-only. The ark e2e
  over a stock `captaind` passes on the merged tree (boarded 1,000,000 sats,
  paid 500,000, receipt verified).

## Parallel integration (2026-11-05)

`docs/MERGE-QUEUE.md` sets the protocol: packets own disjoint modules and
tests; the shared surface (SPEC frontmatter, STATUS, config.rs, main.rs,
OPERATIONS, manifests) has one writer, the merge session. Normative entries
ship as `docs/changelog/*.md` fragments instead of editing the SPEC
frontmatter, `scripts/merge-check.sh` reports conflicts without touching the
tree, and `scripts/merge-gate.sh` runs the serial gate. The enabling refactor
(config and main split into modules, OPERATIONS split) is the next packet
after the in-flight branches land.

## Current state

The pool runs end to end on regtest. Native SV1 and SV2 listeners build the
same SPc coinbase, the front assembles and submits a block, `spc-scan` recovers
the miner's output, and the Postgres/Redis stack settles the block and serves
the API. Wave 2 closed the accounting/settlement/JD integration gaps and the
mock paths, and added the Tor design. No new protocol surface was added.

## Wave 2 merge log (2026-11-02)

- `ba2a9fb`..`186d99e` - accounting/JD/settlement: reward conservation under
  carried balances; front-event watermark (migration 0002) with idempotent
  `LedgerApplied`; settlement reconstructed from sealed slices; reachable and
  restart-safe share accounting; JDC + custom-job validation; share-target
  policy record.
- `3c3a19c`..`65d87c1` - de-mock: real `/metrics` exporter from the store;
  per-height `pool_payout_descriptor` derivation with no zero-key fallback;
  `rpc` feature shipped in the nix package.
- `89e3f83`, `914509f` - Tor: SPEC §3.8 and the decision record; shipped
  examples mark `[reach]`/`[net]` as spec-ahead-of-code (this build rejects
  the keys, so there is no silent clearnet path).
- `1ae2e9e` - proxy/upstream and multi-identifier analyses closed.
- `e2e9943`..`c403b7c` - gate fixes: bounded startup store connects (a dead
  Postgres no longer hangs the front), bounded ledger republish, SV1 rejects
  invalid `mining.set_difficulty`.

## Done

- Bark adapter decision record (commit `25f0a01`): stock `bark-wallet` pool
  transport, in-tree agent keystore, O13 resolved; follow-up doc edits applied
  (UPSTREAM_DEPS bark pin, SPEC §10 O13 row, ROADMAP Phase 7 note).
- `docs/OPERATIONS.md` and `docs/API.md` (commit `7fcbd64`).
- Protocol fixes: SV2 one-identity-per-connection, JD-only carrier,
  own-payout verification, `OP_RETURN` value guard, build-time budget check
  (commit `76df4f8`).
- Front accounting fixes: network-difficulty share `d`, template `job_fees`,
  slice `ref_job_id`/`ref_fees`, fee-band tests (commit `3b79e76`).
- Settlement/Ark fixes: chain-gated booking, null-fingerprint pending,
  settlement-time forfeiture, Redis-backed Ark dedupe, scrub excludes
  `failed_to_pending` (commit `af8a818`).
- Plan, decision records and spec amendments for native SV1, Nix-only builds
  and the `stratum-core` 0.6.0 baseline.
- `spc-crypto`: identity, BIP 352 address, ECDH, tier tweaks, P/U, mailbox
  blinding, carriers, zeroising `SendKey`; frozen vectors and BIP 352
  reference address.
- `spc-scan`: full-block and light-proof scanning, both carriers, H/H-1,
  tweak export, `gen-address`, file and RPC sources.
- `spc-accounting`: slices, sealing, window, SLICE scores, tier routing,
  pending, trim, autoscaler, distribution, JD weights, O11 claim. SPEC §4.5
  fixture and proptests.
- `spc-store`: SPEC §5.1 schema, migrations (0002 front-event watermark),
  settlement/orphan/Ark-failure paths, binary stream codecs, bootstrap frame,
  Redis stream helpers, store queries for the API.
- `spc-pool`: native SV1 session + TCP listener; native SV2 Noise listener;
  pool-template coinbase with a per-height descriptor output; block assembly;
  front state; payout role; read-only API with `b_scan` auth; JD distribution
  publication/validation; Ark order verification; identity-free share blobs;
  the API audit view; Prometheus `/metrics`.
- `spc-ext`: 0x0003 distribution/TLV/formula and 0x5350 registration messages,
  BIP340 signing, Ark order/receipt codecs.
- `spc-share-ext`: identity-free leaf, merkle proofs, sampling verifier.
- `spc-agent`: SV2 client, translator role, own-payout verifier, config, JDC.
- End-to-end: mock SV1, mock SV2, SV1 -> translator -> SV2, a real bitcoind
  regtest run, and a full-stack run with Postgres/Redis + `front,payout,api`
  that settles found blocks and serves the API.
- Live JDS/JDC: token allocation, `DeclareMiningJob` with the distribution
  TLV, JDS recompute-and-compare of the declared coinbase.
- Type-32 windows and share proofs served over SV2; `spc-scan audit` verifies
  sampled shares against a live server.
- `payout` emits signed, capped `ArkSendOrder`s for Ark-rail entries; the `ark`
  role verifies them.
- `nix flake check` green.

## Merge gate (2026-11-02, wave 2 final tree)

- `cargo fmt --all -- --check`: clean.
- `cargo test --workspace --all-targets --features spc-pool/rpc`: pass, all
  targets (25 test binaries, 0 failures).
- `cargo clippy --workspace --all-targets --features spc-pool/rpc -D warnings`:
  clean.
- `nix flake check`: pass.
- `testing/regtest-e2e.sh` (front only): pass, blocks 101 -> 103, `spc-scan`
  finds the miner output.
- `testing/regtest-services-e2e.sh` (Postgres/Redis, `front,payout,api`): pass,
  blocks 101 -> 103, block 102 settled and served by the API, scan finds the
  output.

Both harnesses bind bitcoind on 18443, so they must be run sequentially.

## Review

- Integrated review (2026-11-02): **APPROVE** at `b2a03c4`.
- Wave 2 normative review (`kimi-k3`, diff `5f36190..HEAD`): **APPROVE** with
  partial coverage. Verified: share target and credited `d`, `N x d` window,
  slice ids, sealed `high_water`, bootstrap restore and the `LedgerApplied`
  watermark, JD weights from the routing. Its two doc nits (the §3.4/§4.1/§4.3
  prose still stated the old share policy; the changelog named an internal
  field instead of `[accounting] retain_windows`) are fixed in `5cd79b4`.
- Wave 2 security review (`nemotron-3-ultra`, diff `5f36190..HEAD`):
  REQUEST CHANGES with 2 blockers. Both were challenged and resolved without
  code change: (B1) the Ark order's destination is what the Ark server must
  see; SPEC §11.3 already defines the order as carrying exactly that and names
  the fields, so the role boundary holds; (B2) the Tor keys are rejected by
  this build's parser, so an operator cannot get an onion address with a
  clearnet binding - the enforcement is a remaining packet, not a silent
  bypass. Verified in the review: identity separation of `spc:ark:orders`,
  fail-closed descriptor and target parsing, share-accounting idempotency and
  conservation, xpub-only descriptor, parameterized SQL, migration locking.
- Accepted residual risks from that review: bootstrap restore skips a slice
  whose stored difficulty disagrees with its credits (decision
  `2026-11-02-share-target-policy`, fail closed on the coinbase, window may
  shrink); the settlement window target is estimated from one slice and can be
  wrong across a difficulty retarget (needs the target persisted in
  `distribution_snapshot`); the interim `ark` send transport is an operator
  command (`sh -c`), to be replaced by the stock bark adapter; order-key
  rotation is operator-driven (documented in OPERATIONS).

## Remaining

1. `[template] source = "ipc"` (bitcoin-core Cap'n Proto): deferred
   fail-closed by the sv2-tp packet; needs the pinned `bitcoin_core_sv2`
   client, Cap'n Proto toolchain and an IPC-enabled Core node (SPEC §7.1 pin
   requirement stands). rpc and sv2-tp cover the node backends in use.
2. Testnet soak and third-party `spc-scan` recovery (SPEC §12 rollout gate:
   testnet4 `tsp` soak >= 24 h). Testnet5 chain identity is merged but its
   public-network soak additionally waits on ecosystem support (stock bark
   cannot represent Testnet5; agent runtime fail-closed).
3. rbitcoin upstream: the testnet4 branch is pushed to the fork and PLAN §7
   passes; the upstream PR is not opened.
4. Multi-region (Phase 3 multi-front, Phase 8) per MULTI-REGION.md — includes
   the deferred `spc_outbox_*`, `spc_tip_divergence_seconds` and
   `spc_ark_surplus_sats` metric families.
5. Bark/testnet5 upstream watch: no bark branch exists; the gate is
   rust-bitcoin PR #7160 (BIP95 Network variant), then a release, then bark's
   `bitcoin` major bump. Tracking section + re-check procedure in
   UPSTREAM_DEPS.md ("bark testnet5 tracking (2026-11-17)").

Done since the previous board: the 2026-11-17 focus wave (testnet5 + BIP54,
ladder audit, SV2 TP stages 1+2, role-sourced metric families, cheat-slice
statistical test, Grafana lab observability). Before that: bark transport,
JD fallback, window-target persistence, Tor scope decision, ladder decision,
rbitcoin RPC harness and reorg invariant, rbitcoin testnet4 acceptance.

## Considered, not adopted

- Proxy pool with an upstream pool, and p2poolv2 as a node mode
  (`docs/analysis/2026-11-02-proxy-and-upstream-options.md`). The pool is the
  payout authority; delegation removes the SPc and Ark rails.
- Multiple account identities (BTC/LN/Nostr usernames)
  (`docs/analysis/2026-11-02-multi-identifier-payouts.md`). Multiple delivery
  rails under one SP identity is the aligned future shape; multi-identity is a
  subsystem, not a parser.
- Tor for Ark/mailbox egress: rejected 2026-11-04 (mining connections only).

## Blocked

None. The remaining items are implementation.
