# Regtest lab implementation and Fable review board

Date: 2026-11-05
Updated: 2026-11-12
Status: self-hosted Nix Ark/rbitcoin infrastructure and a native-funded two-node slice are verified. The complete registered-miner, ladder and funded Ark experiment remains unfinished; availability of earlier inspection launches is not guaranteed.

## Current integration snapshot (2026-11-12)

- Native public exchange retention passed 33 targeted tests, including all
  seven retention cases and N1/N2, plus CPU/runtime checks, strict clippy,
  formatting and build. Parent verified 713 archived rows, 12 invocation
  histories and CPU `fbef4e1c` at source `4d14e213`. Actual component artifacts
  retain request/response/manifest data before hashing; signed wallet responses
  and startup metadata are fixtures. Independent retention review/replay are
  assigned. The final wallet joint fixture will target this CPU and capture
  configuration, without inheriting older joint results.
- Historical full-rail observations now own the build turn. The primary owner
  is closing the remaining readiness timing window inside the command-log
  mutex and aligning deadline equality with reservation/consumer rules.
- The newly authorized preservation-first cleanup removed 156 verified test
  cache copies and skipped one changed/unverifiable browser test host. Every
  removed executable has a verified raw or compressed preservation copy.
  Free space rose from 47.711 to 58.916 GiB; metadata for 128,145 other files
  was unchanged. Receipt/restore helper:
  `/tmp/opencode/preserved-rust-test-executables-20261112-r1/`.
- Parent verified all 87 ladder and 40 native packet coverage rows against
  primary: no missing or mismatched rows remain for those approved packets.
  The newly added native exchange-retention and historical full-rail evidence
  hooks are source-complete but await their compile/test turns. Native retention
  holds the next reservation, then historical observations and final packaging.
- Readiness v3 retains the required raw bracket observations, but the current
  time can still be ignored after on-time acquisition. A fresh deadline check
  at the actual pre-HashStart transition and a corrected green gate receipt
  remain required. The primary owner has the exact consumer source findings.
- 2026-11-11: parent source reconciliation found required ladder and native
  modules still absent from primary despite a reported complete union. Missing
  paths include aggregate/economic store modules, migrations 0007-0011,
  `ark/send_fence.rs`, and the CPU native gate/outbound writer. The green
  workspace check therefore describes a partial union. The integration owner
  is applying cumulative approved packets with explicit per-path coverage.
- Readiness v3 source and fixtures now exist, but its final joined/live gate
  and canonical AP5 descriptor remain pending. The proposed package descriptor
  must retain the established Nix source, binary, probe and test bindings; a
  differently shaped file named `bundle.json` does not establish admission.
- Latest parent disk check found 47.77 GiB free, below the 50 GiB build floor.
  All heavy builds are blocked. The approved older-test cleanup preserved 67
  binaries and recovered 3.60 GiB, skipping 46 uncertain entries; later test
  builds consumed that headroom. Further preservation-first options are being
  prepared, with no new deletion authorization assumed.
- QA integrated exact provider-checkpoint retention, FIFO capture and final
  mining/source/raw/SQL drain. The r17 packet passed 95 affected tests; parent
  verified its 98-file archive and receipt seals. Read-only replay of old r5
  remains unconverged, including its missing H23 economic row. The new path
  awaits a live joined campaign; runtime and AP5 owners have the shutdown and
  capture contract so final evidence is retained before roles stop.
- Wallet aggregate/native-service audit gave bounded approval for code
  `2a022552`, with no concrete privacy/funds/signing counterexample. Parent
  reproduced full `2ceb09fc` and code `2a022552` hashes from the frozen
  `{base_manifest_sha256, files}` recipe (728 full / 467 non-Markdown rows).
  The audit's earlier recipe-attribution note is being corrected. The primary
  join will confirm strict grant/status revision ordering and the native
  request-stream keep-open requirement. Full controller/history/funded
  acceptance remains separate.
- Same-auditor native lifecycle recheck APPROVED source `ccf53fa9` and sealed
  CPU `7b62234f`, closing N1/N2. Bounded writer handling reaches revocation
  before network close, and the original monotonic/proof deadlines cannot be
  extended by clock rollback. The 22-test evidence retains its original scope.
  Corrected-CPU wallet joint verification, controller lifecycle and full-history
  funded acceptance remain pending.
- Native N1/N2 lifecycle corrections passed 22 targeted tests, including eight
  new blocked-write/clock regressions, plus check/clippy/fmt/build. Parent
  verified 702 archived source rows, 11 invocation histories and sealed CPU
  `7b62234f` at source `ccf53fa9`. Public codec and SourceFeed v2 are unchanged.
  Same-auditor recheck is assigned. The wallet joint fixture is being rebound
  to this exact CPU; its old result is not attributed to the new binary.
- The primary owner now holds the build reservation for IBD-aware readiness
  verification and the corrected execution bundle, with merged controller
  dependency checks as required. Full native lifecycle/history and funded
  acceptance remain outstanding.
- Wallet native service and aggregate consumer are compiled and tested at code
  `2a022552`, with sealed Ark agent `aee146fb`. Feature-scoped suites passed:
  48 default-agent, 170 Ark-agent, six codec, nine conservation, two order,
  three bridge and seven runtime-helper tests, plus fmt/clippy. Parent verified
  728 archived source rows and 17 passing receipt seals. A joint actual-service/
  sealed-CPU fixture proved zero hashes before COMMITTED, followed by commit
  event 5, activation 6 and first hash 7. Startup metadata/history were explicit
  fixtures; full controller lifecycle and funded acceptance remain pending.
  Wallet source audit is assigned. Native N1/N2 corrections own the next build.
- The harness-only checkpoint-capture/drain correction passed 13 lightweight
  sequencing tests. Parent verified its five-file archive and logs. Exact
  provider checkpoints are retained before activation, and final sealing waits
  for mining drain and source/tip/economic convergence. It has not run live;
  the old H23 gap remains unchanged. QA is integrating these collection hooks.
- Independent QA replay passed the corrected native H16/17 Rule B pair and
  H1-H8 audited warm-up checks, with exact-checkpoint entitlement through H18.
  Parent verified the 94-file consumer archive, receipt seals and 64 affected
  test results. H17's aggregate delta is -1 sat, while per-identity debit
  reservations correctly total 260,870 sats. QA matched 21,840 SPc outputs.
  H19-H22 lack exact frozen checkpoints; H23 also lacks its SQL economic row,
  and final SQL remains anchored at H22. The producer owner is correcting
  next-run capture/drain sequencing. Old artifacts remain unchanged; full
  native-audited and Ark acceptance remain inconclusive.
- Native activation audit requested two CPU-local corrections on `a0d1d04`:
  blocked outbound writes can defer candidate/stop revocation, and a later
  wall-clock rollback can extend the original admission deadline. The native
  owner is implementing independent pre-I/O revocation and an original
  monotonic-deadline cap, with focused regressions. Public codec/signing
  formats remain unchanged. The 56-test packet is not final audit approval;
  wallet-service compilation continues under its existing build reservation.
  Review: `/tmp/opencode/native-activation-audit-a0d1d04.md`.
- Native CPU activation gate passed 56 distinct tests, CPU/scanner checks,
  strict clippy, formatting and binary build. Parent verified the 697-file
  archive and 21 invocation histories at source `a0d1d04f`; sealed CPU is
  `aee91a91`. The native tests use real Noise/CPU work with simulated signed
  wallet responses. Actual wallet/collector acceptance remains unrun. The
  runtime helper passed production check/clippy, but its unit test requires
  the corrected pool-dependency union. A bounded source audit is assigned.
- The wallet service now owns the build reservation, using the exact corrected
  conservation helper and unchanged native codec. Explicit default/Ark tests,
  real service authorization/durability checks and a sealed agent build are
  assigned before the joined activation experiment.
- Ladder source `2314f6e2` passed the production-installed restricted Ark ACL
  regression and final clippy/builds. The same auditor closed the cursor ACL
  defect; earlier A1-A5 closures stand. Parent verified 723 archived source
  rows, four green gate receipts, three binaries and 147 native evidence seals.
- A fresh 40 H/s rbitcoin component run observed consecutive reservations at
  heights 16/17, both booked by tip 23. Producer-invoked frozen QA replay passed
  the pair over 428 signed events, 23 blocks plus genesis and 283 SQL history
  events. Independent QA replay is assigned. Early warm-up certification,
  exact checkpoints at heights 19-22 and wallet-audited native activation are
  not claimed. The tested packet is queued for primary integration.
- The native CPU activation gate now has the build reservation. Its public
  receipt codec and pre-hash activation checks will be compiled and tested;
  the wallet service and IBD-aware readiness observer remain separate pending
  integration work.
- Pinned node-source analysis resolves the inbound-only readiness blocker:
  IBD owns outbound sockets outside PeerHub, and genesis prevents IBD completion
  before the follow-outbound path starts. Requiring those outbound RPC rows
  before hashing is cyclic. Parent verified all 17 cited source/evidence seals.
  The primary owner is implementing completed inbound-handshake evidence joined
  to both owned processes' reversed ESTABLISHED TCP tuples, bracketed by process
  and namespace identity checks. A fresh exact-profile pre-hash readiness and
  authority-only first-block propagation regression remains required. The old
  run lacks socket joins and is not retroactively accepted. Source report:
  `/tmp/opencode/node-ibd-peer-readiness-source/REPORT.md`.
- User approved another scoped cache cleanup after checking that the current
  floor remained 50 GiB and free space was 49.41 GiB. All 65 selected inactive
  Rust test executables were preserved and SHA256-verified before removing
  cache copies. Free space rose to 64.01 GiB; metadata for 47,767 other build
  files was unchanged. Receipt and hash-based restore helper:
  `/tmp/opencode/preserved-rust-test-executables-20261110-r1/`.
- Net-pool/readiness correction passed 91 distinct affected tests and final
  checks, but its binary build was disk-blocked. Same-auditor recheck closes
  the admission logic and identifies one cursor ACL mismatch. A dedicated
  GET/SET selector and real restricted-user retry test are assigned; the
  ladder owner has the restored build turn and fresh native proof task.
- Canonical 350 admission passed and a fresh run exercised the new readiness
  gate. Both nodes reported inbound-only peers at genesis, and the gate refused
  bootstrap before any hash reservation or transfer. Node-source investigation
  is tracing PeerHub versus IBD connections for an exact endpoint/ownership
  proof. Reap now succeeded, with all 66 tracked identities and recorded
  listeners cleared. Global unreadable-process coverage stays inconclusive.
- Network/reap diagnostic bundle `2484b1e8` is built with its measured
  350-check inventory. Parent verified 838 actual Nix source rows, 13 binaries
  and 19 passing check-log seals; CLI is `ddd7dd27`. AP5 has the completed
  evidence to qualify the source-specific inventory revision, run canonical
  admission and execute a fresh diagnostic. Nine P2P tests cover rendered
  argv/ownership; live propagation and confirmed funding remain separate.
- The next heavy turn is assigned to the frozen net-pool/pre-grant admission
  correction (`64feeaaf`). A2-A5 are audit-closed; A1 still needs the typed
  retryable pre-request capacity/float outcome verified, preserving all
  post-request ambiguity pins. Native consecutive-pending proof remains
  failed on the preserved pre-fix source, pending a fresh corrected run.
- Corrected Runtime diagnostic bundle is built and canonically admitted:
  `/tmp/opencode/runtime-fix-diagnostic-r1/bundle.json` (`4d3778e8`), CLI
  `52e45eab`. Parent verified 833 actual Nix source rows, 13 binaries, the
  common application derivation, 17 passing check-log seals and exact probe
  tuple. Three real `Runtime::execute_phases` regressions cover same-ID wait,
  cancellation and failure; library tests passed 77/0. AP5 is assigned a fresh
  confirmed-funding run with this bundle, never a g105 retry.
- The heavy ticket is reassigned to AP5's standalone VTXO verifier: six
  unconditional signed conformance cases, frozen lock and Nix source binding.
  The live diagnostic can run independently of that build. Full control/grant/
  ladder/native-audit integration remains outstanding; the diagnostic bundle
  does not claim those features.
- Canonical admission passed for the actual joined Nix bundle. Three fresh
  AP5 diagnostic attempts exposed a runtime bug: `execute_phases` rejects the
  declared bootstrap's valid asynchronous Running result; its same-ID wait
  was placed in the StopRun branch. The primary owner is moving the wait into
  phase execution and adding a runtime regression, then rebuilding the bound
  executable. The declaration remains required for route admission; no second
  bootstrap or new ID is a workaround. Zero user payments executed. Recorded
  processes are gone/endpoints released; four unreadable same-UID processes
  leave global cleanup inconclusive. Parent verified the 40-file/69-test
  handoff and execution receipt seals.
- Same-auditor R4 recheck APPROVED pool-wallet source `d00299c2` / tested
  `f7649004`, closing the stock-lock restart finding without relaxing other
  private-file checks or releasing active fcntl locks. Primary and AP5 have
  the exact approval. Merged grant-path and funded behavior remain separate.
- Native activation implementation and public r3 codec are frozen but
  uncompiled. Parent verified the codec, patch and runtime-hook artifact seals
  and forwarded exact types to the existing-wallet service and QA owners.
  QA's audited-warm-up packet passed 262 tests; parent verified all 74 archived
  source rows and referenced evidence. Its Unix exchange uses synthetic inputs
  and does not establish live native activation or funded Ark acceptance.
- Pool-wallet R4 passed 28 tests, including the actual pinned stock lock
  manager's restart and cross-process exclusion cases; fmt/check/strict clippy
  and sealed builds passed. Parent verified 690 source rows and five green
  receipts at full `d00299c2` / tested code `f7649004`. Same-auditor R1 recheck
  is assigned, and the primary owner is integrating the tested control source.
  Full live-wallet restart and funded execution remain unclaimed. The heavy
  build ticket now belongs to the combined ladder/Rule B/SourceFeed gate.
- Joined diagnostic Nix closure is built. Parent independently verified 833
  files from its actual Nix source root, 13 binary hashes, the common six-app
  derivation and 14 passing check-log seals. Bundle:
  `/tmp/opencode/AP5-joined-bundle-r1/bundle.json` (`f2c96b21`). Apps share
  `g105903s...-spc-lab-ark-tools` and source `wr1ix2s3...-source`. Vendor hash
  was refreshed against the unchanged current lock; node/server outputs were
  reused. Canonical diagnostic admission and the fresh confirmed-funding run
  are assigned to AP5, not claimed passed yet.
- The primary omission/Ark-feature gate is verified green: 296 Rust tests
  (including 162 with the agent Ark feature), 36 Node tests and 15 compiled-
  browser workflows. Browser provider fixtures remain explicit. The build
  ticket is now assigned to pool-wallet R4's stock-lock restart regression.
- Boarding-control r3 recheck closed maintenance, queue handling, bounded sync
  and historical proof retrieval, but found one restart blocker: stock Bark's
  persistent `LOCK` file is 0644 under umask 022 and the new root-directory
  check rejects it. The owner is correcting stock-file initialization while
  retaining strict control-file checks. `dd9f0c37` is not final funded/recovery
  approval. Review: `/tmp/opencode/pool-wallet-control-source-review-r3.md`.
- AP5 retained-board-proof consumer passed 54 Python tests, including 13 new
  cases. Parent verified its 33-file frozen packet and artifact/contract seals.
  Historical reads use same-wallet `board_proof_v1`, with separate read IDs,
  immutable board ID and canonical proof/terminal-context binding. Partial
  lineage does not replace current-float proofs. QA has the updated contract;
  the Rust verifier and actual funded execution remain unrun.
- Corrected pool-wallet controls are tested at full `dd9f0c37` / tested code
  `8ec102f5`: parent verified 690 source rows, 16 delta rows, both sealed
  binaries and green final test/clippy/build receipts. There were 54 earlier
  Ark tests and 25 final focused tests, with scopes kept separate. The packet
  includes `board_proof_v1` and retained original public board proofs. Same-
  auditor recheck is running; grant-path union and funded AP5 remain unclaimed.
- The build ticket is reassigned to the primary integration owner. The focused
  omission-correction gate has acquired the lock and is running explicit Ark-
  feature tests/clippy plus rebuilt HTTP/UI/browser checks. No result is yet
  claimed for that running gate. AP5 has the tested read-only board-proof
  contract for its consumer integration.
- QA's source-feed/checkpoint/AP5 consumer passed 218 tests. Parent verified
  its 68-file archive, patch/manifest and 13 reference seals. It implements
  durable Unix ACK handling and initial-checkpoint replay but correctly does
  not claim native CPU activation verification.
- Actual native routing still bypasses the translator's verifier:
  `Worker.publish` can start work before the wallet/collector ACK. Native gate
  implementation is assigned in `lab-native-audit-gate`; the existing wallet
  process will verify/sign, and CPU miners receive only public receipts. The
  ladder owner is exposing the shared verifier and validating complete-history
  empty/underfilled warm-up windows, with no fabricated shares or Q coverage.
  The full campaign must reach a verified full window. Direct native SV2, four
  wallets/four miners and the existing hash budget are preserved.
- Original UI base omission is corrected in source. Parent checked the actual
  24-path original patch: 18 rows match the approved final manifest, six are
  intentional unions/forward changes, and none are absent; app remains
  `468ac395`. The restored routes/assets/SSE checks passed lightweight
  formatting and 36 Node checks. The next primary gate explicitly includes
  Ark-feature agent tests/clippy and rebuilt compiled-browser checks. It has
  not run; the boarding-control owner retains the current build reservation.
- AP5 v2-r2 passed 41 Python tests and reproduced its verifier lockfile
  byte-for-byte (`ac24ee3b`); offline locked metadata passed without compiling.
  Parent verified 29 frozen source rows and artifact/contract/lock seals.
  Admission now requires explicit Ark-feature coverage and the complete
  compiled UI/browser gate. The pool-control owner has the exact request for
  read-only original-board proof retention after refresh/pruning. The verifier
  will use valid signed offline fixtures for compilation tests and separately
  validate the first actual board's live proof; no funded pass is claimed.
- The merged eight-packet checkpoint passed 266 Rust/Node tests, fmt, clippy
  and builds, but its compiled-browser step failed: the original UI base layer
  was omitted (HTTP routes, DOM/assets and wallet UI tests). The integration
  owner is applying that already-authorized dependency while preserving later
  fixes and app `468ac395`. This is not a passing full integration gate.
  Parent also verified that the 48-test agent command omitted `--features ark`;
  explicit merged Ark-feature tests/builds remain required.
- Ladder source `0515a6a8` supplies the signed `/order` API, live proof-feed
  acknowledgement, wallet grant fences and migration 0011's initial economic
  checkpoint. Parent verified all 717 source rows and archive/patch/API seals;
  compilation and PG/native checks have not run. Aggregate client R3 now binds
  the exact codec/API and preserves SourceFeed plus wallet configuration, but
  its 708-file packet remains uncompiled. Boarding controls hold the next
  reserved build turn; all heavy checks still require the global lock.
- AP5 v2 passed 35 Python tests and a sealed preparation replay. It now has
  explicit raw-component, historical-board and cleanup contracts. Its offline
  VTXO cryptographic verifier, original-board-proof retrieval, joined binaries
  and fresh funded execution remain unverified. Neither preparation nor these
  fixture tests establish a funded AP5 pass.
- Pool-wallet control source audit requested corrections before compilation/
  funded use: default datadir initialization must retain baseline behavior,
  and regular maintenance/scanning must continue while boards are pending.
  Bounded queue-error isolation and synchronization are included in the same
  rework. The conservative Unknown hold is retained; absence from the mempool
  is not a generic no-effect proof. External source review:
  `/tmp/opencode/pool-wallet-control-source-review.md`. No compiled or funded
  approval is attributed to the original `55a48819` packet.
- Acceptance-consumer A1-A4 are closed by the same auditor. The final A1
  recheck is bound to `T3-A1-late-acceptance-fix-r4` (`f6452174`, 177 tests):
  every RPC acceptance needs its own actual unsuppressed success, matching
  intent/transaction/node and history-prefix proof. The later working Rule B
  consumer packet (`bc4ba68a`) passed 196 offline tests; parent verified both
  archives and test receipts. Producer approval and live acceptance remain
  inconclusive, including the newly identified initial economic-state
  checkpoint gap.
- User explicitly authorized incremental-cache cleanup to unblock verification.
  Parent removed 149,555 cache-exclusive files under the two approved cache
  paths, retaining 631 externally hard-linked metadata inodes. Available space
  increased from 43.29 to 92.36 GiB; metadata for all 41,831 sibling files was
  unchanged. Receipt: `/tmp/opencode/silent-pool-incremental-cleanup-20261110.json`.
  The primary merged gate was resumed; subsequent builds use
  `CARGO_INCREMENTAL=0`, the existing targets and the same 50/20 GiB safeguards.
- T1 early-hook and before-sign version-3 follow-up is verified at full
  `6740857f` / code `e1ae700d`. Parent checked all 730 source rows, the 23-path
  patch, sealed CLI/txd binaries and 63 evidence files. Retained receipts cover
  114 core test instances plus two HTTP/schema tests, a fault child, clippy,
  fmt and builds; invocation failures and final test-only pattern attribution
  remain documented. Integration and QA have the exact constructor/record API.
  The native T2 installer must be retained during the merge. Fresh native-funded
  rejection is still unrun; synthetic signatures and node-route evidence are
  not relabelled as that result.
- T1 frontend join passed 14 Node checks. Parent verified nine artifacts and
  exact patch reconstruction from approved app `8548baf6` to `468ac395`.
  It joins authenticated user/operator projections to wallet rows, supports
  T1-only accounts, and enforces source-age expiry at rendering and clicking.
  The approved 697-file UI tree remains immutable. The single-file patch is
  with serial integration for compiled real-provider checks; the prior browser
  proof is not attributed to these newly changed app bytes.
- Aggregate source audit requested five fixes on `415d21fa`: authorization
  scheduling, full receipt policy/server binding, legacy Sent exclusion for
  reserved members, unrelated booked-effect reversal during reorg, and fair
  receipt processing. These are assigned alongside Rule B, with real Redis/PG
  and proof-boundary regressions. Review:
  `reviews/2026-11-10-ladder-aggregate-review.md`. The old aggregate path is
  excluded from funded use; the 990-output component result retains its scope.
- AP5 orchestration is implemented in `lab-ark-funded`. Parent verified seven
  source rows, the 12-test receipt and the actual stale-closure refusal before
  startup. Default preparation succeeded; funded execution has not started.
  Besides joined binaries, AP5 needs a live same-wallet pool executor to board
  external native transfers. This is assigned in `lab-pool-wallet-control`
  (`ses_e3bdf9734ffec9NcGHb0G7QQwj`), with private command/status handling and
  the existing stock wallet writer. No competing wallet or Ark-server change
  is authorized. Exact AP5 bundle/API contracts are in its README.
- Ladder component freeze `415d21fa` is verified: parent checked all 50 delta
  rows, eight receipts and three sealed native-test binaries. It passed 260
  pool tests and three real PostgreSQL aggregate tests. A native-SV2 run on
  stock Bitcoin recovered 990 outputs / 4,950,000,000 sats, exported all 990
  tweaks and settled one logical row with Q=9. This is a component proof,
  not the final rbitcoin/Ark experiment.
- Provisional accounting is resolved for implementation by net-debit
  reservations: freeze available prior pending P, derive actual-mined resulting
  error pending E, reserve max(0, P-E); use fresh raw plus booked pending minus
  canonical unbooked reservations. Booking applies E-P additively and retires
  the reservation atomically. Full unbooked payout subtraction is incorrect.
  The ladder owner is recording the dated clarification and implementing frozen
  settlement inputs, ancestry-complete publication, lifecycle and recovery.
- Aggregate client work is assigned in `lab-aggregate-wallet`, preserving the
  approved AP-3b/provider baselines. It consumes the concrete fresh-challenge
  status and SPRP/1 receipt API and retains anchor watches until actual verified
  receipt. An independent audit of the frozen aggregate send/receipt/privacy
  path is running. QA has the actual publication/window/history bundle API.
- T1 payment-provider completion is verified: 19 tests and seven green steps,
  22 evidence seals and all 724 frozen source rows (including archive/patch
  reconstruction). Full `1647d831` / code `eb9383cb` and the earlier tested
  acceptance layer are queued for integration. A separate small UI provider
  join is assigned against the approved UI bytes. The 23-path before-sign
  version-3/early-hook follow-up is undergoing its own focused gate.
- Same-auditor UI recheck APPROVED full `a964f157` / code `b360b332`:
  user/operator replay, durable terminal results, bounded pending controls and
  wallet journal failure guards are closed. The launch/attach registry patch
  was reviewed read-only and still needs the actual serial join. Integration
  proceeds with merged-provider tests; funded wallet flows remain AP5 work.
- Corrected UI final freeze is verified: full `a964f157`, code `b360b332`,
  CLI `98a03d2e`. Parent checked 697 source rows, 111 evidence-file seals,
  three binaries and seven green source-stable steps. The latest journal
  guard passed 14 narrow Rust tests, fmt/clippy and builds; prior 15-browser-
  workflow evidence applies to unchanged assets, with latest CLI responses
  checked separately. Same-auditor W1-W4 recheck is dispatched, and serial
  integration has the final correction plus launch/attach registry patch.
  Real-provider and funded-flow acceptance remain separate.
- T2/contextual/T1-DEST primary integration is closed. Parent verified all
  31 primary patch rows, authority receipt hashes and unchanged index, then
  checked logs for 122 txd tests (two pre-existing ignored), 51 controller
  tests and 58 library tests passing; corrected clippy invocation completed.
  Receipt: `reviews/2026-11-10-t2-native-primary-integration.md`. Serial
  integration proceeds directly to corrected AP-3b r2 and its input provider.
- The T2 handoff's `w52v4f6` Ark-tools closure is historical unfunded evidence,
  not the corrected joined runtime: parent compared its 15 wallet source rows
  with fixed AP-3b r2 and found five changed production files plus the missing
  audit regression file. Its CLI also predates the T2 join and it has no txd.
  The serial and AP5 owners have this correction. Funded execution requires
  freshly bound combined binaries; rebuilding only txd is insufficient.
- Node source review proves exact `(-26, "version")` is pre-admission and
  cannot publish the transaction in that invocation. Parent verified nine
  cited node files, both separate release/debug binaries and four historical
  profile reports. Existing uncertainty/reservation history still governs
  earlier possible publication. T1 is assigned an explicit acceptance-only
  version-3 choice before the production signer; mutating version bytes after
  signing is not valid-signature evidence. The actual native-funded signed
  rejection case remains unrun. Source proof:
  `/tmp/opencode/node-definitive-rejection-source/REPORT.md`.
- UI W1-W4 corrections reached the audit-r2 checkpoint: 63 Rust tests,
  36 Node tests, fmt/clippy/builds and 15 compiled-browser workflows passed.
  Launch/attach registry reconstruction has an exact separate join patch.
  Parent verified all 697 current source rows (`c130b02e` / `fc94b8d1`).
  A later torn-journal-tail guard is explicitly outside the tested r2 binary;
  its narrow verification is resumed after observing a free build slot and
  61.74 GiB available. Final audit approval and real-provider integration
  remain pending.
- A read-only audit of the frozen 97-test acceptance consumer found four
  false-pass cases: send-outcome/journal contradictions, reused send intents,
  omitted high-water fronts and ignored reaper failures. Fixes and regressions
  are assigned to QA before joined acceptance. Review:
  `reviews/2026-11-10-funded-acceptance-consumer-review.md`. The archived T2
  raw-chain results retain their scoped meaning; full acceptance remains
  inconclusive.
- Independent QA verified the archived 105-block T2 chain, 3,492 sats of fees,
  80 allocation outputs, 189 funding scripts, six change scripts and the
  1,809 measured / 1,810 charged hashes. Parent verified 25 consumer source rows,
  the 97-test log, oracle receipt and 43 input-artifact seals. Full acceptance
  remains inconclusive: old node logs cannot prove complete forwarding/pause
  behavior. T1 has the precise bootstrap-stage hook and rejection-catalog gaps;
  the ladder owner has the exact immutable proof-bundle requirements.
- AP5 funded stock-Ark orchestration is assigned to the existing self-hosted
  infrastructure owner (`ses_e429358c5ffeV7N1lyDn76dAVl`), independently of UI
  completion. It will use fresh owned runs and the joined production native
  funder, with actual board/receive/refresh/renewal/offboard evidence. Existing
  inspection data and pinned node/server binaries remain preserved.
- Live wallet UI audit found a concrete replay bug: a missing registry after
  attach can falsely terminal-reject an already queued operation. Parent also
  identified the same early rejection in the non-operator authenticated path.
  UI author is fixing both paths, durable nonterminal-to-terminal journal
  updates, and unrelated controls while wallet operations are running, with
  focused Rust/browser regressions. The previous UI freeze is not final;
  other reviewed security/SSE/path boundaries remain sound. Integration keeps
  this known defect out of funded UI use while independent work continues.
- Live wallet UI compiled-server verification passed: 64 targeted Rust tests,
  six wallet regressions after the lint fix, clippy/builds, and 13 Chromium
  workflows with 14 exact static responses. Parent verified 693 source rows
  at full `19aa61a0` (code `0f559b5d`) and sealed CLI `363dbb19`. The exact
  UI/registry-hook packet is queued after corrected AP-3b and its tested private
  input provider; T1 balance/address wiring is a separate remaining join.
  A focused asynchronous access/path-safety audit is running. Browser provider
  fixtures are not funded-wallet evidence; the final HTML remains a draft.
- Private wallet UI input provider passed its focused gate: nine provider
  tests, five status tests, check/fmt/clippy. Parent verified all 694 rows
  and five green log bindings (full `e97ade12`, code `7119fd63`). Its exact
  `spc-agent-private-inputs/v1` DTO and eight-row delta are with the UI and
  integration owners. The fixed AP-3b baseline remains unchanged. T1's
  separate per-user balance/address provider UI-v2 gate is running; no
  funded UI/provider acceptance is claimed yet.
- Live wallet UI progressed to 64 targeted Rust tests, formatting and nine
  focused JS tests passing. It now has the exact AP-3b registry caller patch,
  homogeneous derived/normal offboard support and definite no-effect terminal
  handling. Parent found the shared lock free with 77.27 GiB available and
  resumed only clippy, final CLI build and compiled-server browser proof.
  Real wallet-input and T1 balance providers remain explicit integration joins.
- Same-auditor AP-3b recheck APPROVED the fixed `0b0c5fd7` / `a270582d`
  packet: R1-R6 are closed. Funded maintenance and aggregate-status/ladder
  integration remain outstanding. A separate `lab-wallet-ui-provider` task
  now adds the private validated input-summary read model without modifying
  that fixed integration baseline.
- Live wallet UI passed 33 Node tests and 13 Chromium fixture checks. Parent
  assigned the existing primary target cache for its Rust/backend validation;
  browser fixture evidence is not a compiled-server or funded workflow proof.
  AP-3b input/registry and T1 per-user balance/address providers are explicitly
  assigned. T1's frozen acceptance-adapter API was forwarded to the independent
  QA owner for real forwarding/barrier evidence collection.
- Combined native T2/contextual difficulty/T1-DEST is verified at source
  full `9c8f9511` / code `d33e88fc`: parent checked 719 source rows and
  33 artifact hashes. A real two-node/controller/two-driver slice mined 105
  canonical blocks and funded 80 allocation outputs (1,809 measured hashes,
  1,810 charged, 40 H/s). Storage/front/miner actors were inert fixtures, so
  this is not full pool-mining acceptance. External 100,000-sat P2WPKH/P2TR
  payments reached both nodes without replay duplication but remain unconfirmed.
  Combined integration and independent raw-chain replay are dispatched.
- AP-3b audit fixes passed a source-bound six-step gate: 11 focused regressions,
  153 agent tests (one ignored), three bridge tests, fmt/clippy. Parent verified
  690 rows at full `0b0c5fd7` / code `a270582d` and all six green log bindings.
  The same auditor is rechecking R1-R6; the integration owner has this corrected
  packet instead of the old defective candidate. Funded maintenance is not run.
- Authenticated live wallet UI work is dispatched in `lab-live-wallet-ui`
  (`ses_e3c670affffeBwvH6vNd799DJY`): token-entry/bootstrap, scoped reads/SSE,
  real controller command outcomes and wallet-provider integration. The
  independent T3/T4b harness has 51 offline tests passing and is adding actual
  payout/forwarding/witness/storage evidence adapters; full live acceptance
  remains incomplete. Raw fixtures are never counted as production evidence.
- User requested continuing until all components and acceptance are complete.
  Parallel work now includes the independent T3/T4b funded oracle/harness
  (`lab-funded-acceptance`, `ses_e3c817d93ffe4m9Cg1i37GS0X3`) and production
  ladder completion (`lab-ladder-completion`, `ses_e3c8b74eaffeqxYab5ewFv2fD2`).
- T1-DEST output support passed scoped verification. Parent verified all 711
  rows, full `e4a81a06`, code `ac704157`, patch/archive and five green check
  receipts. It is handed to T2 and serial integration: standard regtest
  P2PKH/P2WPKH/P2TR outputs, while witness inputs remain a separate path.
- Native T2 passed its scoped tests but live funding exposed a consumer-policy
  error: it dropped `allow_min_difficulty_blocks` and treated no-retargeting
  as constant difficulty. The node's long-gap `207fffff` template is correct;
  a native `200fffff` header at the same time was rejected `bad-diffbits`.
  T2 is implementing contextual expected-target/observer checks using complete
  bound policy and chain history. No node policy, binary or NAR change is needed.
- AP-3b audit found six concrete defects: durable output proofs before
  forfeits, command fairness, due-epoch filtering, renewal of stock-child
  outputs, release of definite no-effect reservations, and mailbox continuation.
  The author is fixing them before funded use. Unfunded registration evidence
  stands, but the old candidate is not final wallet integration.
- Ladder source A/C exists but still needs execution and audit-source wiring.
  Parent assigned the existing primary target cache under the shared lock and
  authorized a documented aggregation policy: oldest covered stored anchor,
  frozen membership/destination/value/order, all-member fresh canonical checks,
  pinned uncertain/reorg orders until definitive outcomes, atomic receipts,
  and ownership-authenticated client status with mailbox retention until actual
  receipt verification. No aggregation identity data may enter Ark-role streams.
- AP-3b has authored wallet opening, native pool registration, epoch receive
  adapter and cold provisioning, but remains incomplete and uncompiled.
  Parent assigned the existing `/tmp/opencode/T1a-target` cache for scoped
  verification (5 GiB estimate, normal nonblocking lock/disk checks) and
  directed continued implementation of runtime readiness/miner bridging,
  scan/reorg scheduling, refresh and offboard. Funded lifecycle proof remains
  separate; no existing inspection run may be used for destructive tests.
- Self-hosted Ark primary integration is closed: all 22 source rows match
  the tested packet, merged checks passed (20 focused lab tests, seven
  history-3 conformance tests, fmt/clippy), and Nix evaluation rebound the
  existing captaind/node outputs without a cold rebuild. J2 engine and funded
  boundaries remain explicit. `docs/lab-build-report.html` now exists as an
  interim draft with a short how-it-works summary; it is not the final user
  deliverable until the remaining funded and wallet/miner work completes.
- Serial owner closed journal-v3 merged validation: 137 passed, zero failed,
  two ignored (including the separately reported fault child), fmt/clippy
  zero; audit approval is recorded. Self-hosted Ark source transfer is
  complete (22 rows verified against candidate, SPEC 55 -> 56) and its
  merged targeted validation is running. No current inspection run changed.
- AP-3b wallet/miner bridge implementation is dispatched to
  `ses_e41b5a67dffe2ntnvsQImzAz2j` in `lab-ark-wallet-bridge`: extend the
  existing wallet foundation to stock wallet opening and live pool
  registration/receive/lifecycle APIs, then gate miner startup on actual
  per-identity registration. This proceeds alongside native funding work;
  funded receive/refresh/offboard acceptance remains required. The user
  inspection stack must not be used as a destructive test target.
- Full all-Nix unfunded infrastructure is verified. Parent checked nine
  outputs/derivers and 53 executable hashes, then launched inspection run
  `/tmp/opencode/ark-inspection-20261109T025651Z/run` (controller 1441192,
  two-hour duration). Authenticated inventory confirms 11 live actors;
  tokenless inventory returns 401. Provider and pool balances are zero.
  Controller: `http://127.0.0.1:33579`; operator pages:
  `http://127.0.0.1:41225/` and `http://127.0.0.1:37983/`.
  There are no user-wallet/miner actors in this infrastructure scenario.
  Exact source integration is dispatched; funded and J2 engine claims remain
  separate. The inspection run must not be used as a worker test/reap target.
- T1 reports 70 atomic J1 tests passing and is running fresh J2 validation
  with actual process/ENV/profile token equality. New traffic runs already
  generate 64-hex ownership tokens; colon-token rejection is explicitly
  tested. No existing run token is rewritten. Fresh acceptance is pending.
- Final user deliverable: once the full lab implementation and acceptance are
  complete, produce a self-contained HTML write-up at
  `docs/lab-build-report.html`. Lead with a short how-it-works summary, then
  document exactly what was created: actual services and responsibilities,
  architecture/data flows, pinned source/Nix packages, launch and operation
  commands, wallet/funding lifecycle, and verified test/acceptance results.
  Describe implemented behavior rather than planned features; omit secrets
  and use sanitized evidence references. Do not label an interim report final.
- Nix release node built and passed four metadata plus 22 CLI tests (reported
  binary `bba47522`); final launcher/probe assembly hit the 50 GiB floor.
  Parent reclaimed only the completed AP1b node cache after owner confirmation
  and process/fd/maps/environment checks. All 123 executable/shared-library
  records were preserved by hash; source/receipts stayed unchanged. Recovered
  3.212 GiB, reaching 53.169 GiB available. Receipt:
  `/tmp/opencode/node-cache-cleanup-receipt-20261109.json`.
  The Nix owner is resuming only remaining provenance/launcher/release-profile
  verification, with no repeated node/server build or relaxed disk floor.
- The remaining T1/J1 producer-consumer mismatch is assigned for atomic
  integration: the runtime still consumes historical `f4fc3e04` while the
  verified producer emits `754e1e61`. Parent authorized the T1 owner to merge
  the selected-consumer table, metadata, lineage and tests together, preserving
  history-3/schema/OP/access hunks. New activation is recorded as parent/user
  authorization with actual evidence, not a fabricated Fable approval.
  Full-model failures stay visible and funded behavior stays unproven;
  joined Rust tests and fresh actual-run profile checks remain required.
- Node metadata blocker is cleared for sealed binary `36e2387a`. Parent
  verified all 2,106 final source rows (full `f1592c93`), binary identity,
  and both canonical r4 default/bark-v31 complete-profile reports and process
  receipts. Both CLI checks exited zero; consensus facts match and the
  policy-only comparison digests differ. The full-model mempool diagnostic
  and funded-unproven status remain visible. Final source/NAR handoff
  `node-profile-facts-r2/NIX-HANDOFF-complete.json` is with the self-hosted
  Nix owner to finish the packaged release node and all-store-path launcher.
  The devshell-built binary is not a completed Nix package or full lab proof.
- Readiness check recovered the node gate's timeout receipt: RPC metadata,
  full RPC suite, node CLI and fmt completed with exit 0; a 120-second shell
  timeout interrupted clippy before a binary build. Parent dispatched only
  the two remaining steps, preserving the four passing results. Full Nix
  launcher/profile binding and funded T1/T2/Ark paths are still unverified.
  The earlier bounded mining demo's HTTP endpoint is no longer reachable;
  the successful self-hosted Ark lifecycle cases were tests that stopped
  their owned processes, not a currently hosted complete expanded lab.
- T1 compile fixes R3/R4 are published: three `Begun` -> `Began` references
  and one ambiguous `Path::as_ref()` removal in CommandLog opening. Parent
  verified both patch hashes, resulting source rows, and matching T2 bases;
  T2 is applying only those two upstream rows. No interface or serialization
  change. T1's guarded check/tests/clippy sequence is running; success is not
  yet claimed, and T2 will not start a duplicate build concurrently.
- T2 native executor source is authored (nine-path delta over the 700-file
  T1 layer). Its first compile reached three T1 actor errors: `Begun` is not
  the core's `Began` variant. The T1 owner is correcting those references
  and producing a narrow source-bound handoff; no Rust test ran in T2 yet.
  Parent accepted the private, create-new bootstrap raw-evidence JSONL as
  observation-only: existing journals retain budget/signing/recovery authority,
  and write failure stops execution. Native funding remains unproven.
- Traffic-runtime R2 source is authored and parent hash-verified: 700 rows,
  full `b7d607e5`, code `5f0522bd`, 59-path delta. It includes the atomic
  recovery read API, actor pump, raw admission, completion verification,
  budget wire accounting and restart/evidence joins. Formatting passes;
  compilation/tests/clippy remain pending. Native bootstrap implementation
  is dispatched to `ses_e4220291effeJCDinxPtFrpAvy` in `lab-native-bootstrap`,
  consuming that frozen source interface. It will implement real coinbase
  mining and distribution through the existing signer; no funding result is
  claimed. The running node-metadata gate keeps the current build slot.
- The first corrected-node warm gate selected rustup instead of the required
  pinned Nix toolchain and was stopped with exit 143 before any tests ran.
  Source and approved binaries are unchanged. Parent inspected the corrected
  `node-profile-facts-r1/run-gates-nix.sh`, which verifies Nix-store compiler
  paths and versions before compilation, and authorized one fresh nonblocking
  attempt. The setup-failure receipt is preserved; no test success inferred.
- Self-hosted Ark infrastructure now passes 20 targeted tests and three
  real unfunded lifecycle cases (normal stop, partial-start failure, controller
  death), with distinct provider/pool storage and zero observed Ark-role
  payout-key read attempts. Parent verified the 20-file source delta and
  runtime case outcomes in `/tmp/opencode/self-hosted-ark-packet/manifest.json`
  and `/tmp/opencode/ark-live-r2/receipt.json`. Full Nix launcher/node-profile
  binding and funded flows remain unfinished. On completion, parent found
  the build lock free with 59.43 GiB available and granted the corrected-node
  warm compile/test/build sequence; no cold node-package retry is concurrent.
- Node metadata test-constructor fix is source-verified: exactly two argument
  replacements in one test file, with production metadata unchanged. Parent
  verified 2,106 rows / 30,897,068 bytes, full `d06e366a`, patch `abe0d65f`
  and NAR `sha256-JeoNicCbKC6Q+tPbc4la+qTI/tc1D4MwNQdX69/obhw=`.
  The Nix owner has the immutable r1 handoff. Rust tests/build and positive
  canonical profile checks remain pending; current tools rebuild continues.
- Self-hosted stock captaind is built with Nix. Parent verified its store
  output/deriver, binary `4cac675b`, version at pinned `c419159`, and upstream
  Cargo.lock hash. The first local run reached real check-config/create/start,
  ArkInfo and zero-balance wallet RPC (`initialized_unfunded`), then failed
  on Ark-role pool-config fields; that run is not a passed lifecycle test.
  Corrected lab tools are rebuilding. The node package exposed two metadata
  test constructor errors (`Arc<Query>` where `Query` is required), now
  assigned back to the node owner for a narrow fix/refreeze. No hosted Ark
  service or server-source patch is involved; funded lifecycle remains open.
- T1b/controller source checkpoint is incomplete and uncompiled (44 changed/
  new paths). Parent confirmed the recovery API gap: journal state has the
  last sent txid and command/transaction maps, but lacks a coherent public
  read view. The runtime owner is authorized to add an atomic detached
  recovery snapshot, with no format/write-rule change, and finish the actor
  pump, pending-send recovery, completion verifier and evidence/wire joins.
  Known source formatting must also be fixed. Lock contention still blocks
  compilation; no source-only checkpoint is treated as a working runtime.
- Probe r4 explicit `bark-v31` policy grammar is implemented. Parent verified
  617 source rows and 47 distinct affected Python tests, including four new
  cases: full `71b4ee6f`, code `22fd2fcd`, candidate contract `754e1e61`.
  The contract and policy goldens were forwarded to node-metadata, self-hosted
  Nix Ark and traffic-runtime owners. Rust tests and complete CLI binding to
  the metadata-enabled node remain pending; no candidate promotion is claimed.
- Node metadata source fix is delivered and parent-verified: 2,106 files,
  30,896,946 bytes, full `91f41d32`, code `517d35bb`, patch `c958d22f`
  against preserved AP1b. Four fields now derive from effective consensus
  parameters; independent fingerprint vector and format/source checks passed.
  Compilation and complete canonical-probe positives remain pending because
  the shared slot was busy. The exact source/patch binding was handed to the
  self-hosted Nix Ark owner; no new node binary or successful binding is claimed.
- Journal-v3 asynchronous audit APPROVED exact `ff6353c2` / `df0b0545`;
  no concrete bug found. Candidate checks remain green; merged validation
  still needs an available nonblocking build slot. T1b actor/controller/J2
  implementation is dispatched to `ses_e4282accaffe53atnLNW5jzUw5` in
  `lab-traffic-runtime`, consuming the indexed integrated baseline and the
  audited journal APIs. This covers real control/RPC, allocation, staging,
  observer and durable budget/command joins; native T2 funding and full Ark
  lifecycle remain subsequent implementation/acceptance work.
- Probe/profile r3 completed seven owned process-lifecycle cases, including
  SIGKILL and partial-start cleanup. Parent verified all 614 source rows and
  seven receipt hashes (full `1c62691a`, code `e654b382`). The complete
  profile CLI correctly fails because the pinned node lacks four required
  consensus fields; that case proves failure cleanup, not successful binding.
  A narrow additive node-metadata implementation is dispatched to
  `ses_e42867233ffeZJwCoW2lDSSNIS`, based on preserved AP1b source, and
  coordinated with the self-hosted Nix Ark owner. The probe's Rust gate and
  merged journal-v3 gate remain lock-blocked; no waiting lock is authorized.
- A second concrete profile join is being implemented: the probe's closed
  grammar must accept `--rpc-compat-profile bark-v31` as literal, digest-bound
  policy. It must survive probe isolation unchanged; profile-on/off digests
  must differ. The probe owner is updating grammar, contract lineage and
  focused tests atomically, coordinated with the node-metadata owner.
  Earlier lifecycle and missing-field failure receipts remain preserved.
- Journal-v3 source transfer is reported complete, with the operator
  constructor preserved and SPEC changelog 53 -> 55; no dependency changes.
  Merged validation remains pending. Parent corrected an unauthorized
  `flock -w 1800` wait: cancel it if still waiting, or finish an already-held
  invocation normally, then use nonblocking acquisition only. Removing
  workflow holds does not relax the build/disk policy.
- Journal-v3 scoped verification passed: 136 top-level test executions,
  zero failed, two ignored, plus the passing fault-test child; fmt/clippy
  exited zero. Parent verified all 608 rows and five log/exit bindings:
  full `ff6353c2`, code `df0b0545`. Tested Rust bytes are unchanged; only
  report/workflow wording changed afterward. Exact-delta serial integration
  is dispatched with asynchronous code audit, following the user's removal
  of workflow holds. The live mining/UI run uses sealed binaries and stays
  separate from these source/build changes.
- Fresh live host run launched from sealed merged binaries:
  `/tmp/opencode/live-lab-20261108T231027Z/run`, controller PID 199214,
  one native node, two pools and two CPU miners at 40 H/s aggregate.
  Hub `http://127.0.0.1:36387/lab`, controls on the same origin; pool operator
  pages `http://127.0.0.1:43525/` and `http://127.0.0.1:34009/`.
  Live inventory and both operator observation endpoints responded; both
  pools show real confirmed block records. Run duration is bounded to two
  hours from startup. Source/binary/startup/live observations are recorded
  beside the run. This run has no funded Ark or ordinary traffic actors.
- Self-hosted Ark infrastructure implementation dispatched to
  `ses_e429358c5ffeV7N1lyDn76dAVl` in `lab-self-hosted-ark`: pinned source
  Nix packages and lab-owned server/storage/config/lifecycle, with actual
  funding dependencies stated. Journal-v3 worker now has permission to run
  its prepared scoped checks under the single-build lock. Workflow review
  holds are removed; actual test outcomes and runtime validation remain.
- User removed workflow holds and clarified deployment: Ark infrastructure
  must be self-hosted inside this lab, built from pinned code repositories
  with Nix and supervised like the other daemons. The lab owns its Ark server,
  PostgreSQL, wallets, datadirs and lifecycle; no hosted-provider endpoint is
  the intended deployment. Standard unmodified Ark interoperability remains
  the protocol target. Parent cleared the completed serial-window priority
  reservation. The single-heavy-build lock and disk-budget rules still apply.
- Parent recovered and independently verified the completed serial window:
  hub, history-v2, access, operator-runtime wiring and AP-2 all have primary
  integration receipts. The final Nix test output exists and its log records
  903 passed, 0 failed, 18 ignored across 103 result lines. All 658 frozen
  source rows match; 246 current code/build-input rows match that snapshot;
  all eleven AP-2 focused exit files are zero and final root hashes match.
  Current CLI hash is `0949b949`, production RPC pool hash `e2953368`.
  This closes the merged-code gate for a fresh basic host native-mining run
  with hub/operator pages. That fresh live run has not yet occurred; traffic
  driver/native funding, history-3/J1 acceptance, witness/ladder and full
  vanilla-Ark lifecycle remain separate unfinished work.
- Serial owner reports the approved operator-runtime source transfer complete,
  preserving access routes, history-v2 and existing root dependency edges.
  Its vendor probe and ten-step merged validation are running; results remain
  pending. Parent corrected the proposed changelog omission: the candidate's
  untouched SPEC does not waive the merge-time frontmatter splice. The owner
  must splice the operator-wiring fragment after the current checks, recording
  the documentation-only difference without restarting code checks for it.
- Operator runtime wiring is Fable APPROVED at code `a9b6e4fc` / full
  `fd198c44`, with documentation-only D1-D3 merge conditions. It is queued
  after hub/history-v2/access: correct float-versus-liability wording, document
  the opt-in operator link, and annotate stale report metadata. No code rework
  or new candidate binary proof is required by that verdict.
- AP-2 registration is Fable APPROVED at full `d94bb158` / code `de0e2158`;
  R2.1 and all earlier findings are closed. Exact narrow-hunk integration is
  queued after the UI/access packets, with migration-slot reconciliation and
  merged RPC/Ark-feature checks. No stock-Ark lifecycle claim is implied.
- The serial owner has launched the merged hub validation under its reserved
  build window. Its result remains pending. History-v2/access maps are ready;
  queued source changes must wait until the current tested snapshot and
  receipt are closed. Operator wiring and AP-2 approval do not waive this gate.
- AP-2 R2.1 final verification passed: 22 store and eight registration tests,
  fmt/clippy/docs exit 0. Parent verified all 379 source/archive rows, nine
  log hashes and tested-code equality: full `d94bb158`, code `de0e2158`.
  The same Fable reviewer is checking the confined replay fix. No AP-2 merge
  or Ark lifecycle approval is implied.
- Parent transferred the build reservation to the serial integration owner
  (`ses_e458119eeffe4Jl3MZjJ4J4HJR`) and dispatched actual hub validation,
  followed by approved history-v2 and access integration/verification. The
  runner must accept only its explicitly granted owner and recheck the ticket
  after locking. Operator-wiring/J1/history-3/AP-2 code remains gated on its
  own review; no fresh native experiment is authorized by this build window.
- Host operator wiring verification completed: reported 175 spc-lab tests
  and three CLI tests passed, clippy/source-format clean, JS 12/12. Parent
  verified all 604 source rows/raw modes, full `fd198c44`, code `a9b6e4fc`,
  and sealed CLI binary `7af444a9`. Fable is reviewing the packet at
  `docs/reviews/2026-11-08-operator-runtime-wiring-fable.md`; no live run yet.
- AP-2's original worker failed at model startup (provider 404), with no
  verification result. Replacement `ses_e42f6aeb7ffeFD76hA9245WKg5` now owns
  the transferred build reservation and the existing registration worktree's
  final R2.1 PostgreSQL/Redis tests and fmt/clippy. Old sources/receipts remain
  intact. After this window, serial hub/history-v2/access integration is next.
- EXP-ACCESS Fable re-review APPROVED full `b89f74d2` / code `8085616e`;
  F1-F8 are closed. Serial integration is queued after hub validation and
  history-v2 integration. Required merge adaptation: hub inventory is
  operator-only under expanded access (user 403, tokenless 401, tested);
  only verified data-free hub assets may join the exact public static list.
  Primary Cargo.lock remains authoritative. The separate operator listener
  and future traffic commands are outside this access approval. Host wiring
  retains the current build reservation.
- EXP-ACCESS rework is finalized and back with the same Fable reviewer
  (`ses_e44612566ffe3LCJTjtpruLe0c`) for F1-F8 closure. Parent verified all
  589 rows, modes, inventory and binding: full `b89f74d2`, code `8085616e`.
  Receipt `/tmp/opencode/EXP-ACCESS-rework-freeze-binding.md` binds the green
  24 access / 33 legacy / 42 unit tests and fmt/clippy exits. Cargo cache
  maintenance warnings are preserved and explained. Original pre-rework
  packet bytes are hash-only; recovered baseline originals are archived.
  Host wiring keeps the build reservation; review starts source/receipt-only.
- Access gate completion recovered from actual exit/log files: 24 access,
  33 legacy and 42 unit tests passed; fmt and clippy record exit 0. The lock
  was free and available disk was 81.78 GiB. Access owner is finalizing the
  tested-source freeze and explaining the preserved clippy cleanup diagnostic;
  parent has not yet claimed final source-bound approval. The build reservation
  is now assigned to host operator runtime wiring (`ses_e444fa08bffes1FApjGS7mp54I`).
  Its focused Rust/route/link verification is dispatched. Next: AP-2's affected
  DB regressions, then merged hub/history-v2 validation; newly authored traffic
  prerequisites remain queued. No build reservation was bypassed.
- Traffic-runtime plan `44daaa2b` is APPROVED by the independent reviewer
  (Round 5); all plan findings are closed. Two source-implementation packets
  are dispatched against the approved history-v2-r1 source layer:
  - `lab-traffic-probe-profile`, `ses_e43876decffeA3fiuqwCH6FmlA`: J1 selected
    consumer/table lineage and the canonical-probe half of J2 (profile
    normalization, binding and parent-death launch).
  - `lab-traffic-journal-v3`, `ses_e4386c6b5ffevCnI2y8xQOxPuJ`: observer
    snapshot and proof-bearing rollback/maturity amendments, with coherent
    history-3 core/T0 activation and history-2 refusal tests.
  Primary history-v2 integration remains a prerequisite for their acceptance.
  Both packets need implementation verification and Fable review. Host operator
  wiring now holds the build reservation; new workers may author source/tests but may
  not acquire a Rust/Nix build slot until the parent grants a window.
- Journal-v3 prerequisite source is delivered, with compilation/tests pending.
  Parent verified all 608 source rows, modes, inventory and receipt hashes:
  full `1d84f881`, code `d106e200`; exactly eight modified and six added files
  against the approved history-v2-r1 layer. The packet includes observer
  snapshots, atomic rollback/maturity handling, a queued-send guard, and paired
  core/T0 history-3 activation. Source-only freeze:
  `/tmp/opencode/traffic-journal-v3-recipe.json`. No Fable-ready, merge or
  runtime-activation claim; verification awaits a parent-granted build window.
- Probe/profile prerequisite source is delivered with 129 Python tests
  reported passing. Parent verified its 613-file freeze: full `aebe80a1`, code
  `0b3c5020`, twelve modified and eleven added files. Rust and process-death
  acceptance remain pending. Before verification, the author is completing
  J1's stale Rust contract fixtures and moving the packet's changelog fragment
  into the standard `docs/changelog/` location; the original checkpoint is
  preserved. Candidate lineage `da87e2b6` remains unapproved/fail-closed.
- Probe/profile r1 source packaging is complete: parent verified all 613
  records at full `3ea6f04d`, code `64f75a68`. J1 Rust fixtures are authored,
  history-2 sections preserved, and the unchanged changelog bullet is in
  `docs/changelog/`. The author is executing only the new pure-Python RPC
  regressions and affected tests; this uses no heavy-build slot. Rust,
  owned-process acceptance and Fable remain pending; lineage is unchanged.
- Probe/profile r2 lightweight gate passed: two new tests passed, then all
  seven tests in the affected module passed (seven distinct tests, not nine).
  Parent verified both logs, source bindings and the 613-file refreshed
  manifest: full `014a8ef2`, unchanged code `64f75a68`. Only the report changed;
  prior reports/freezes remain archived. Rust/process-death/Fable gates remain
  pending; no broader suite rerun or build-slot acquisition occurred.
- Serial owner completed history-v2 read-only pre-merge verification: all
  602 candidate rows match, existing T1a core needs no retransfer, and the
  LAB-RUNTIME amendment fits the post-hub context. Mapping/evidence:
  `/tmp/opencode/T0-history-v2-integration-prep.md` and
  `/tmp/opencode/T0-history-v2-verify-prep.log`. No merge or build occurred.
  Hub validation/receipts still precede this merge. Parent checked the live
  priority ticket: EXP-ACCESS owns it; the handoff's AP-2 attribution was stale.
- Independent traffic-runtime peer review is dispatched to
  `ses_e43be64ecffe5uyhTgnlnrnxZO` against frozen plan `e54b7c77` at
  `/tmp/opencode/traffic-runtime-plan-review-v1/2026-11-08-traffic-runtime-wiring.md`.
  It checks exact RPC forms, actual-node preflight/startup ordering, native
  funding and multi-driver allocation. This is source-only review and uses
  no build slot; implementation dispatch awaits its verdict.
- Traffic-runtime peer review returned REQUEST_CHANGES (F1-F8): exact raw/
  canonical RPC contracts, resolved preflight profile binding and cleanup,
  deferred worker activation with cancellable bootstrap, global allocation/
  per-driver storage, recurring output discovery and reorg reconciliation,
  typed RPC uncertainty, durable pause/restart barriers, and bootstrap coinbase/
  witness economics. The original author is revising the source-grounded
  contracts; frozen v1 stays unchanged. Same-reviewer approval is required
  before implementation dispatch. This work consumes no build slot.
- Traffic-runtime v2 is frozen at `d749b125` (578 lines), with v1 preserved:
  `/tmp/opencode/traffic-runtime-plan-review-v2/2026-11-08-traffic-runtime-wiring.md`.
  Parent verified both hashes and returned v2 to the same independent reviewer
  for F1-F8 closure. Recheck includes pause-interval enforcement, controller-
  death cleanup, definitive rejection classification and per-driver path/
  allocation binding. Author-reported resolution is not yet peer approval.
- Traffic-runtime v2 peer recheck returned REQUEST_CHANGES. RPC forms (F1)
  and the existing-package binary choice are closed. R2-1 through R2-8 require
  exact pause intervals/WAL handling, parent-death/profile normalization,
  bootstrap admission and finite budgets, encoded allocation paths, state-aware
  funding rescan and lower-tip reorg support, definitive send-error classification,
  typed GBT fields and dependency/admission-site corrections. The author is
  revising those contracts. Lower-tip reorg support is now an explicitly required
  reviewed core amendment; existing core approval does not prove this behavior.
- Traffic-runtime review-v3 (internal revision 2) is frozen at `caf4f352`
  (789 lines) and with the same reviewer for R2-1 through R2-8 closure:
  `/tmp/opencode/traffic-runtime-plan-review-v3/2026-11-08-traffic-runtime-wiring.md`.
  Parent verified its hash and both prior freezes. The revision specifies pause
  intervals, launch-profile/parent-death handling, bootstrap limits, encoded
  roots, observer amendments and a send-error table. Those contracts still
  require independent approval; no implementation or build was started.
- Traffic-runtime review-v3 returned REQUEST_CHANGES with five localized
  corrections. Pause intervals, parent-death mechanism, startup stages,
  allocation roots/order, observer algorithms, GBT types and dependency/
  destination dispatch are cleared. Remaining: canonical profile encoding,
  durable batch reservations and boot-bound deadlines, concrete observer/
  rollback APIs with send-time maturity checks, two schema references, and
  exact error literals bound to the selected node artifact. The author is
  applying the review's bounded replacements; prior freezes remain intact.
- Traffic-runtime review-v4 is frozen at `cc0a4b34` (891 lines) and back
  with the same reviewer for the five localized corrections:
  `/tmp/opencode/traffic-runtime-plan-review-v4/2026-11-08-traffic-runtime-wiring.md`.
  Parent verified this snapshot and all three prior freezes. Recheck includes
  canonical profile equality, durable batch authorization/replay, and the
  rollback-record format/header compatibility amendment. Approval and
  implementation dispatch remain pending; no build slot is used.
- Traffic-runtime review-v4 returned REQUEST_CHANGES with exactly three
  blockers: normalize both validated P2P forms to one comparison value and
  remove raw-position metadata from the digest; poison/replay hash-budget
  authorization safely across failures and duplicate delivery; and activate
  the proof-bearing rollback format coherently as history 3 / journal protocol
  3 across all consumers, keeping control protocol v1. The author is applying
  the supplied localized replacements. History-v2 approval remains preserved
  as the baseline, not authorization to write the new record format.
- Traffic-runtime review-v5 is frozen at `44daaa2b` (949 lines):
  `/tmp/opencode/traffic-runtime-plan-review-v5/2026-11-08-traffic-runtime-wiring.md`.
  Parent verified the new hash and all four prior freezes. The same reviewer
  is checking only R4-1 through R4-3 closure: normalized comparison equality,
  durable/idempotent hash authorization, and coherent history-3 activation.
  No implementation approval, build or live-readiness result is claimed.
- Primary now contains Fable-approved T4a witness primitives/publication,
  AP-3a wallet persistence foundation, AP-1a r2 read-only RPC probe, T0 traffic
  contracts, T1a signing/journal core, and the pool operator page. Each completed
  integration has its own receipt. Hub source is transferred but its merged
  validation remains pending; operator runtime opt-in wiring is not yet verified.
- Parent independently verified 126 transferred paths against approved hashes
  and modes, reconstructed the combined SPEC exactly from approved hunks and
  documented changelog reflow, and confirmed the unchanged Git index.
- Targeted merged tests and the final combined six-check Nix gate passed.
  Parent verified all six output/log bindings and 523 store-source records;
  `/tmp/opencode/primary-four-packet-nix-gate.json` has exit 0. The earlier
  pre-T0/AP-1a `test-ark` result remains separately scoped.
- Subsequent integration receipts record T1a's merged workspace Nix test gate
  green and EXP-OP's later gate at 815 passed, 0 failed, 18 ignored. These do
  not substitute for live funded traffic, witness-block or Ark lifecycle proof.
- Live Ark registration/delivery/refresh/offboard, funded traffic, native mixed-
  witness acceptance, ladder integration and the live hub remain open gates.
  The standard node pin/profile is unchanged; no commits have been made.

## Live Lab hub pivot (2026-11-07)

- User found the mockups too information-dense and requested a running experiment
  workspace with a hosted explanation/directory of its real roles and daemons.
- Draft-2 work is paused: 14 HTML screens and one U4 test render exist, but no
  completed render/validation/manifest packet. Draft-1's 49 files, gallery port
  18481 and user feedback remain unchanged; no render processes remain.
- Two independent TFF models approved the minimal hub plan before implementation:
  [plan review record](reviews/2026-11-07-lab-hub-plan-reviews.md).
- TFF Kimi `ses_e46b967adffeM95EnJ79kzc2xx` is implementing a small `/lab` page
  in isolated `lab-live-hub`: actual role/process inventory, purpose, recorded
  state/freshness, real UI/API availability, and links to existing controls.
  No raw private manifest, secret-bearing URLs or arbitrary RPC proxy.
- Current mining experiment functionality is ready; dedicated pool operator,
  miner-client and funds-management UIs are not. The standard lab does not run
  `spc-agent`; its wallet role is explicitly unimplemented. Current roles will
  be documented in `docs/LAB-ROLES.md` against source rather than stale plans.
- Fable checks precede integration and a new live manual experiment. The broader
  product mockup implementation remains on hold. Nix tooling closeout continues
  independently and must not disturb the review gallery.

### Expanded live-lab planning dispatch

- User requested agents plan, implement and review missing lab components, with
  simulated network transactions and population controls. User explicitly added
  **Ark services for SPc users as required core scope**, not an optional deferral.
- Planning worktree: `lab-experiment-expansion`, with disjoint documents:
  - TFF Kimi `ses_e46b3f034ffej2SoSmAtX2BN2Z`: role-owned minimal UIs, actual
    `spc-agent` integration, funds/status surface and integration contracts.
  - TFF GLM `ses_e46b2954affeTU8vy9e9iA4Pwn`: real signed regtest transaction
    actors, funding, rate/fee controls, population presets and acceptance gates.
  - TFF DeepSeek `ses_e46b21c1fffeIEl5Aw2Sj4lDY3`: Ark server, separate pool
    Ark delivery/float roles, SPc registration/receive/refresh and funds lifecycle.
- Starting profile proposal: two nodes, two pools, four SPc identities with one
  worker each and Ark capability, eight background network wallets, one shared
  independent Ark server, and 0.2 ordinary transactions/second aggregate.
  These are UX defaults, not scale/performance/privacy claims.
- Identity count, worker/process count, aggregate hash budget, transaction rate,
  fee/size and Ark traffic remain independent controls. Increasing users must not
  silently increase mining power or alter node-authoritative consensus settings.
- Transaction traffic must be real signed regtest transactions, with explicit
  funding/maturity bootstrap and no fake credits or statuses. Nonzero fees,
  nonempty templates and Ark Bitcoin-RPC compatibility require new real tests.
- Plans receive independent cross-model review before implementation packets
  are dispatched; each concrete packet receives Fable approval/rework and a
  combined live experiment check. No existing unsupported wallet/Ark feature is
  relabelled implemented by these plans.
- Ark plan delivered: `docs/plans/2026-11-07-lab-ark-services.md` in the planning
  worktree. It identifies substantial pool execution code but missing front
  registration/destination wiring, persistent client-wallet lifecycle and lab
  server/process orchestration. The primary ladder implementation is also absent.
- Source inspection reports a concrete Bark/rbitcoin gate: rbitcoin advertises
  RPC version 190000, while pinned Bark requires 310000 server-side and 290000
  wallet-side. The v0.9 lab port retains that value. Compatibility must be
  verified honestly, not bypassed by an unsupported version claim.
- TFF GLM Flash `ses_e46a4fd00ffe5Oa5XwLP50434A` is independently reviewing
  AP-1 through AP-5, including RPC/chain compatibility, key/privacy boundaries,
  real funding/maturity, registration, receive/refresh and offboard acceptance.
  Role/UI and traffic planners received these dependencies; no code is adopted
  from the plan until review.
- Ark peer review returned APPROVE-plan with six bounded clarifications. The
  author is specifying mailbox derivation from the BIP85 child, missing wire
  acknowledgments/retries, exact error/lifetime contracts, named RPC compatibility
  profile and shared-file ownership. The version gate and delegated-refresh
  APIs were independently verified in source.
- First safe implementation dispatched: AP-1a read-only Ark/node capability
  probe, TFF GLM `ses_e469bb926ffeVwXphP8aizG45M` in `lab-ark-rpc-probe`.
  It must distinguish method recognition, response shape and tested behavior,
  report the real version-gate failure and never spoof readiness. Node/pool
  production code remains unchanged by this packet.
- Full compatibility-profile and registration/wallet implementation waits on
  the precise reviewed contracts and probe evidence; Ark remains required core
  scope. Fable reviews concrete implementation/evidence before integration.
- Ark author delivered the clarified contracts: versioned registration replies,
  request IDs and retryable commit-unknown state, BIP85-child mailbox derivation,
  named regtest-only `bark-v31` compatibility profile, explicit lifetime/float
  settings and shared-file ownership. These are proposed contracts, not code.
- Peer reviewer is checking the changes, including request-ID scope/durability
  beyond the Redis ack TTL, seed recovery boundaries, actual server-sourced VTXO
  expiry and whether the float covers the declared acceptance workload.
- AP-1a retains its already-dispatched standalone read-only probe ownership;
  a future Rust integration must absorb or consume it explicitly rather than
  silently requiring a second conflicting probe implementation.
- Ark clarification re-review approved the direction but left R1/R2 blocking
  registration implementation: per-user durable idempotency beyond the Redis
  cache TTL and explicit AP-1a probe absorption. R3/R4 require actual server-
  reported VTXO expiry and a cumulative float/fee budget before acceptance.
  The author is resolving all four with concrete cases and bounds.
- Transaction/population plan is delivered at
  `docs/plans/2026-11-07-lab-transaction-traffic.md` in the planning worktree.
  Independent TFF Kimi `ses_e468f57f4ffeFoToVNaA0Gyngf` is reviewing it before
  transaction-driver and funding implementation.
- Critical cross-plan dependency: current pool templates filter witness
  transactions, but Ark boards/rounds/offboards require them mined. Witness-
  capable block construction is now a required prerequisite for Ark acceptance;
  legacy P2PKH traffic alone cannot close that gate.
- Integration planning must align four identities/wallet daemons/workers with
  the chosen process layout and use one ladder implementation owner. Default
  preference is one miner process per user, with fixed aggregate 40 H/s and
  0.2 ordinary tx/s; cohosting remains a later deployment option, not a population
  change. No capacity/anonymity claim follows from these small UX defaults.

- AP-2 durable registration is dispatched to TFF Kimi
  `ses_e4683467dffeSvKkshwNsMjADp` in `lab-ark-registration`: versioned codecs,
  per-identity durable request journal, actual handler/advertisement, destination
  derivation, DB/stream/wire tests and normative fragments for Fable review.
- AP-3a wallet foundation is dispatched to TFF GLM
  `ses_e4682e8a7ffeRXSDct2er76CSZ` in `lab-agent-wallet`: provisioned child-key
  material, persistent keystore/stock bark wallet, request counter, recovery and
  honest status. It consumes AP-2's codec; full live refresh/offboard follows
  the registered wallet and server/witness integration gates.
- Ark acceptance budgeting is still being refined: pausing all mining until
  board replenishment can prevent the required confirmations. The plan must
  explicitly fund continuous headroom or declare chain-progress work, with no
  hidden mining or ignored liabilities; breakers must cover real remainder sizes.
- Ark author replaced the all-miner pause with a declared 110-height prefund:
  `110 * 2 * 499999 + 5000000` rounded to 115,000,000 sats per pool, plus
  explicit send-only shortage handling. Max order now covers the full possible
  sub-rung remainder; four per-user miner actors and witness prerequisite agree.
- Peer review is checking that this is a bounded-window claim, including fee,
  reorg and delayed-board assumptions, and that the separate low-float fixture
  bases its replenishment/health result on actual matured output value. Funding
  planner received the updated two-pool requirement; no amount was fabricated
  in a runtime or database.
- Final Ark peer check clears AP-2 registration and AP-3a wallet foundation
  dispatch. Their contracts remain stable while those implementations proceed.
- AP-5 retains three bounded plan refinements: unique-order committed outflow
  must retain reorged sends without double-counting state transitions; fee
  reserve must be derived/bound to the actual Ark fee profile; replenishment and
  health assertions must use actual matured board value and configured thresholds.
  These acceptance details are being updated without blocking AP-2/AP-3a.
- AP-5 refinements are now incorporated in the plan: committed outflow/reservations
  are unique-order scoped, actual sent payments survive reorg accounting, fee
  reserve is bound to the declared Ark fee profile, user-paid fees are separate,
  and refill/deficit/status derive from actual board value. The continuous profile
  prefunds 115M sats per pool; the 5M low-float fixture is separate.
- Funding and role/UI integration planners received those concrete values and
  state semantics. They remain acceptance contracts to implement and verify,
  not claims that Ark services are already running. AP-2/AP-3a protocols stay
  unchanged and their implementation continues.
- AP-1a read-only probe is delivered in `lab-ark-rpc-probe/testing/ark-rpc`.
  Parent froze its 38-file source/fixture/evidence packet as
  `810236fbd5f52c952b994fb595fcc15bc94b7dd3c62ddd3a2f777279ed363c18`
  in `/tmp/opencode/lab-ark-rpc-probe-review-manifest.json`.
- Both approved node binaries report RPC version 190000 and correctly fail
  Bark's version gates. Funded transaction/mempool/fee behavior remains
  unknown or not-run; version 310000 alone cannot produce full compatibility.
  Implementer reports 49 unit tests and ten self-test scenarios passed.
- Fable `ses_e4662a668ffe1iR36XBtVz562W` is reviewing the probe code, exact
  consuming-type contracts and raw results at
  `docs/reviews/2026-11-08-ark-rpc-probe-fable.md`. This review does not approve
  a node compatibility-profile change or claim Ark end-to-end readiness.
- AP-1a Fable verdict: REQUEST_CHANGES. The real version-gate failure is sound,
  but RPC form/encoding coverage, numeric response-ID/HTTP handling, pinned-serde
  fidelity and report/verdict completeness need correction. The old contract
  hash `48894548` is not approved for AP-1b absorption.
- Author is adding BDK's verbose-block and sync request forms, exact protocol
  metadata, source-backed serde vectors and canonical check/verdict rules, then
  capturing new read-only evidence without altering the nodes. Misstatements
  about Core getindexinfo/genesis behavior are also being corrected.
- Named RPC compatibility changes remain gated on the corrected probe and
  funded/witness behavior checks; version 190000 alone is not the whole readiness
  assessment. Other independent registration/wallet foundation work continues.
- Transaction author staged the dependent funding/process amendments without
  modifying the plan under peer review. Parent requested one canonical funding
  table with references rather than repeated constants across the document.
- Default process census must preserve two existing per-pool PostgreSQL
  instances plus the Ark instance (about 22 primary services including the lab
  controller for the proposed mapping). Shared-cluster cohosting, namespace
  holders and PostgreSQL worker children must be stated separately.
- The standalone Bitcoin Core Ark prototype remains supporting evidence only;
  it cannot substitute for witness-capable native mining on the expanded
  rbitcoin lab's canonical chain.
- Traffic peer review returned REQUEST_CHANGES with eight bounded amendments:
  complete witness serialization, shared probe projection, one funding signer,
  deterministic schedule/amount arithmetic, durable reservation/replay/reorg
  handling, independent exact-payment proofs, census and serial merge ownership.
  Review: planning-worktree `docs/plans/reviews/2026-11-07-traffic-plan-kimi-review.md`.
- The traffic author is applying those amendments and the staged canonical Ark
  funding table. A confined peer re-check precedes the legacy traffic dispatch.
- The author delivered the revised 1346-line traffic plan with all eight
  dispositions: native bootstrap default, T2-L ordinary-only funding, one
  signer/journal, complete witness and exact-payment proofs, and serial shared
  integration ownership. These are plan changes, not implementation results.
- The same Kimi peer reviewer is checking the actual revised document against
  the eight findings and binding the verdict to its content hash. T0/legacy
  implementation awaits that verdict; T4a continues independently.
- Confined re-review of plan `ca4e520730692e84c06f07fad594831a38ffc1ccdc05b17e2106898fef9b100e`
  clears T0 independently. Two plan corrections remain: name the native P2PKH
  bootstrap block-production adapter (T2-L), and fix larger-preset worker budget
  allocation plus stale daemon wording (T5). Author is applying only those edits;
  reviewer permits parent verification/new hash without another architecture pass.
- T0 is dispatched to TFF GLM `ses_e4604a9d0ffeHEKwnwhfH0S61J` in
  `lab-traffic-contract`: complete traffic schema/validation, protocol/history
  contracts and canonical AP-1a result projection. No signer, probe execution,
  service launch or mining scope. Shared scenario/runtime edits are merge hunks.
- AP-1a owner received the traffic projection's five-method interface and
  `getmempoolinfo` coverage requirement. Its corrected canonical report remains
  a dependency; neither an unfinished contract nor the old hash is approved.
- Traffic plan is now APPROVE-plan under the peer's confined R1/R2 condition:
  `d64a423570509f0974f3853453942e530ec0e21e0c5c3d1904d4d1427022ad18`.
  Parent verified the current hash, correction regions and public CPU APIs;
  author reports exact reverse-hash reproduction of the reviewed base. Frozen
  plan/review/receipt: `/tmp/opencode/traffic-plan-approved-r2/`.
- T2-L now owns `crates/spc-lab-txd/src/bootstrap.rs`, with explicit native
  template/search/submit and measured-phase termination interfaces. Larger
  presets preserve 40 H/s through declared allocation; stress rotation remains
  T5 work, not an already-supported runtime capability. T0 types are unchanged.
- T1a is dispatched to TFF GLM `ses_e45fe3e45ffe2yR0qc3Arkzs94` in
  `lab-traffic-wallet`: real P2PKH signing and one durable reservation/provenance
  journal, crash/replay/conservation tests. T0 remains the control/schema owner.
  T1b actor/RPC integration, T2-L native funding and T3 live acceptance follow;
  T1a runs no services, mining or broadcasts. Fable reviews before integration.
- AP-3a wallet foundation candidate is delivered. Parent froze 383 source files
  with 22 changes versus the approved runtime (9 modified, 13 added including
  report): `/tmp/opencode/AP3a-review-manifest.json`, full
  `ec8edf5778dacd2865ba0c1e72774a975bb33a54577501e2ee1b13274e2f72d9`.
  Author reports 96 agent Ark tests and four Nix checks passed.
- Fable `ses_e45f878d2ffe47m663ndC7VDy9` is reviewing the exact candidate,
  including derivation, durable counters, persistence/recovery and test-source
  provenance. Scope issue: the candidate persists a Bark WalletSeed/fingerprint,
  but defers full stock Wallet opening; this must not be called a completed
  persistent stock wallet without review. Live receive/refresh/offboard remain
  unimplemented. AP-2 received its semantic registration seam for coordination.
- T4a witness primitives are delivered and frozen: 375 files, 13 changes versus
  the approved runtime (8 modified, 5 added), manifest
  `/tmp/opencode/T4a-review-manifest.json`, full
  `72c2ffd5f8cdb835e76ef77a71feeae88ae8a238f948f955de878308ef57946a`.
  Author reports 267 pool tests passed; the RPC path is compile-checked only.
- Fable `ses_e45f15b2cffeumsClu5MdysSYk` is reviewing serialization,
  commitment/weight accounting, job identity, settlement-mode recognition and
  normative fragments. AP-2 received the additional narrow payout.rs merge
  touchpoint alongside front.rs. No candidate integration has occurred.
- T4b must retain native-PoW canonical mixed-witness acceptance plus independent
  raw-header, txid/wtxid, commitment, block and payment checks. The candidate's
  node-only oracle wording and generic node-pin handoff are review items;
  the approved L2 lab baseline remains unchanged.
- AP-3a Fable verdict is REQUEST_CHANGES. Derivation, source identity and tests
  were independently reproduced; blockers are absent stock Bark persistence,
  terminal registration retry loops, incomplete branch validation, payload-blind
  registration state and missing normative/config documentation. Author is
  implementing offline stock persistence and the bounded state/document fixes,
  including honest dormant/recovery status. The promised scope is retained.
- AP-1a r1 is delivered and with the same Fable reviewer: 59-file packet
  `/tmp/opencode/lab-ark-rpc-probe-r1-review-manifest.json`, snapshot
  `620e42e32d744b8ac5b0ca18d20fc0a9e3532d57ac6fcf7bdb3ffb6da26108dd`.
  Candidate contract `11bdb1007e130a749720f87482f287c1fb65d1e743c142fc4b1ca15a782d89e7`
  supersedes prior hashes but is not approved yet. Author reports 87 unit tests,
  15 scenarios and 12 real-type devcheck vectors passed.
- New read-only finding: both nodes emit `getmempoolinfo.maxdatacarriersize`
  as null, rejected by the selected pinned v30 parser. This is a specific
  consumer-shape mismatch, not a Bark requirement or automatic failure of every
  traffic consumer. T0 received the candidate projection and this distinction;
  Fable is checking it alongside F1-F9 dispositions and evidence provenance.
- T4a Fable verdict is REQUEST_CHANGES. Independent parser/Core transaction
  tooling reproduced all primitive vectors. Blockers: commitment-only job
  changes do not pass main's publication key, strict decode accepts a zero-input
  blob it claims to reject, and BIP141 descriptions/SPEC carrier prohibition
  conflict with the new behavior.
- T4a author is fixing publication with a narrow main.rs hunk and ingress-level
  test, explicit zero-input rejection, truthful consensus-versus-pool policy
  text, real JD-carrier vectors and settlement overhead consistency. AP-2 has
  the expanded main/front/payout merge touchpoints. Independent live oracle and
  exact approved L2 baseline remain mandatory in the corrected T4b handoff.
- AP-1a r1 Fable re-review reproduced source, real-type tests and node evidence,
  but remains REQUEST_CHANGES: HTTP 500/404 response-ID mismatches are recorded
  without failing the protocol check, and several canonical request encodings
  disagree with pinned callers' trailing-null handling/JSON-RPC versions.
- Author is closing C1/C2 and required provenance/projection documentation in
  a fresh r2 packet, preserving v1/r1 evidence. Contract `11bdb100` remains
  unapproved. T0 received the continuing gate and global protocol/chain checks;
  its independent schema work can proceed. Selected v30 `getmempoolinfo` shape
  failure is confirmed, with actual traffic-consumer requirements kept explicit.
- AP-2 returned a propagation description rather than a complete review packet.
  Parent confirmed the implementation report is absent and resumed the same
  worker to finish code, targeted DB/stream/SV2 checks and source freeze. Existing
  successful AP2 receipts are retained. Registration is not marked complete or
  sent to Fable until the concrete report, exact delta and manifest are ready.
- T0 candidate delivered: traffic schema/protocol/history/command types and
  canonical-probe projection. Parent verified 377 files, 11 changes (4 modified,
  7 added), frozen at `/tmp/opencode/T0-review-manifest.json`, full
  `a9f841cb7236f21c4d4dca70cef5f8ee249f8b2eba51337eec468bc42cab353c`.
  Author reports 139 package tests including 34 new tests passed.
- Fable `ses_e45bd8d8affeurg5r1JS5alcwq` is reviewing T0, particularly the
  unwired runtime guard, exact projection coverage and unapproved contract-hash
  boundary. T1a received the candidate type handoff for coordination only;
  actor capability registration belongs to later T1b integration.
- Parent reproduced the disputed AP-1a r1 aggregate `620e42e3` exactly using
  sorted-key compact JSON `{base, files}`. The manifest file hash `daf62d7e`
  is a different hash target; no evidence-integrity discrepancy was found.
- T0 Fable verdict is REQUEST_CHANGES. Schema/vocabulary/provenance checks
  passed, but actual engine prepare can ignore traffic, the gate accepts an
  unapproved lineage, record validation admits forged/misassigned check rows,
  and one readiness bit circularly requires funded proof before bootstrap.
- T0 author is wiring prepare/adopt guards before effects, keeping approved
  lineage empty, binding exact check IDs/methods/provenance, and separating
  recognized preflight from later funded behavior proof using canonical flags.
  Missing checks and actual shape failures still block; read-only reports never
  prove funded behavior. Typed IDs/history paths and decision fragments join
  the same bounded rework.
- Manual payment ambiguity is resolved as an explicit mandatory
  `destination_address`, preserving exact amount and sender ownership. T0 will
  record the contract decision; T1a received the signing-boundary requirement.
  Candidate types remain unapproved until the revised Fable gate passes.
- Role/UI planner `ses_e46b3f034ffej2SoSmAtX2BN2Z` ended with a provider
  socket timeout. Its 299-line `2026-11-07-live-lab-components.md` is preserved
  at `/tmp/opencode/role-ui-plan-recovery/e93a1e27bc12b3b007e12d48d7ce8905933a69c019a7bf41bbe4c6f92009c17f.md`.
  The referenced integration-contract companion is absent.
- Fresh TFF GLM `ses_e45981208ffeLOuq6YXAMySADO` has sole ownership of those
  two planning documents to complete the missing contract and align current
  candidate/implemented states. Two independent plan reviews follow the freeze;
  the first intended UI slice is the small read-only pool operator page.
  Active hub and design-feedback files remain under their existing ownership.
- Role/UI recovery is complete. Parent verified and preserved both documents
  under `/tmp/opencode/role-ui-plan-review-v1/`: components hash
  `1e44c2d449114ef87c21d61d15582f1e1f86fe2fb4e52eddb6b2678ce868593c`,
  integration-contract hash
  `f65c78794b0a742ff13f4d37056d3cca04feb477ae239037c00c442aa894201e`.
- Independent plan reviews are running: TFF Kimi
  `ses_e458d1120ffedHq5rDfRfkcSuR` and TFF DeepSeek
  `ses_e458d0d84ffeqrdY63e1o83yMm`. Each must give an explicit EXP-OP verdict
  so the small read-only operator page can proceed after both approve its
  contract, independently of later access/agent/Ark implementation dependencies.
- AP-3a fix-r1 is frozen and back with the same Fable reviewer. Parent verified
  385 file hashes/modes/inventory and 15 changes versus the prior candidate:
  `/tmp/opencode/AP3a-fix-r1-manifest.json`, full
  `cefc18226fac0363654417486c51f22cc2d1509d0e41cf528faa45a2b780feee`, code
  `e6af84a91714a9f85cbf04fcbb12cf976d02431b38fd144e7366d0ec174fdd5c`.
  Rework includes stock Bark persistence, bounded terminal handling, exact
  branch metadata checks and effective-payload registration binding.
- During AP-3a validation the disk filled. The worker reports removing about
  38 GB of shared Nov-3 `m64-target-{guardian,main,runtime}` build caches;
  receipt: `/tmp/opencode/AP3a-fix-r1-space-freed.txt`. Parent confirmed 37 GB
  free afterward and instructed the worker to stop shared-cache deletion.
  Source hashes are verified; Fable is checking final test/source provenance.
  No additional cleanup or cache rebuild is authorized by this receipt.
- Cache incident clarification: the three deleted paths were
  `/tmp/opencode/m64-target-guardian` (14G), `m64-target-main` (13G), and
  `m64-target-runtime` (11G). The worker checked filesystem owner and mtime,
  not per-session ownership; it acknowledges the scope error. Candidate and
  proofs remain frozen, with no further cleanup or cache restoration attempted.
- T4a rework is frozen and back with the same Fable reviewer. Parent verified
  all 375 records, modes and inventory: 9 modified and 5 added versus baseline,
  `/tmp/opencode/T4a-rework-manifest.json`, full
  `b9961746533632eaaa64eb30a508640cc95d9e234521cd81fdebc1a3795439f9`.
  Rework adds real in-process SV1/SV2 publication regression coverage, explicit
  zero-input rejection, corrected BIP141/SPEC text and settlement overhead
  consistency. Author reports 270 tests passed. No live T4b claim or integration.
- T1a signing/journal candidate is delivered and with Fable
  `ses_e45860f12ffeCAvgGm9bItHZB9`. Parent froze 385 files, 17 changes
  (2 modified, 15 added): `/tmp/opencode/T1a-review-manifest.json`, full
  `5f00c62609a4a46a87de219059e50201f76569af9274404d20eef15a06a776d8`.
  Author reports 41 tests passed. T0 received the nine adapter differences;
  these must be reconciled before T1b without silently changing either contract.
- DeepSeek role/UI review returned conditional EXP-OP clearance after OP-1/OP-2:
  sanitized, scoped operator endpoint and one consistent liability snapshot
  with Ark-due labeled as a subset. Later access/agent/QA packets retain separate
  blockers. Author is applying only the two operator corrections; Kimi continues
  independently against the immutable original pair. No UI dispatch yet.
- Live hub worker returned an evidence-ready acknowledgment, not a completed
  report. Parent confirmed `docs/LAB-ROLES.md` and browser/runtime artifacts
  exist but the implementation report is absent. The same worker is finishing
  report/source freeze only, including the rejected source-filter check and
  existing successful receipts. No repeat builds, browsers or mining requested;
  Fable review and integration still await the concrete packet.
- T4a Fable re-review is APPROVE for `b9961746` / code `c3f9b53a`.
  B1-B3 and R4/R5/R6/R8/R9 are resolved; R7 stays in the live T4b handoff.
  Parent checked every affected primary path against the approved baseline:
  all match, so T4a can integrate first while AP-2 remains isolated.
- Sole T4a primary integration owner: TFF GLM
  `ses_e458119eeffe4Jl3MZjJ4J4HJR`. It is transferring exact reviewed code,
  reconciling the approved SPEC frontmatter fragment, and recording targeted
  validation/source/index receipts. AP-2 has the witness-first merge order.
  Integration is in progress, not yet complete; no T4b/Ark readiness claim.
- AP-1a r2 is frozen for the same Fable re-review: 74-file packet
  `/tmp/opencode/lab-ark-rpc-probe-r2-review-manifest.json`, snapshot
  `f0aa504aa6f247474eb247b4c25d12f286bb3f88abb1d42e77c3ba643f65ce12`.
  Parent verified 16 historical evidence records remain unchanged. Candidate
  contract `f4fc3e043f420870675095ab24144ba14e0afba6cb9ad69d818d99b1bccb2f62`
  remains unapproved pending C1/C2 and documentation disposition checks.
- AP-3a fix-r1 re-review closed findings 2-11 in code but found one remaining
  persistence defect: missing/empty Bark DB plus surviving initialization
  marker is recreated. Author is adding pre-mutation refusal and regression
  tests, moving the SPEC changelog to its required fragment, and correcting
  residual recovery/count claims. Only targeted fix-r2 checks are requested;
  full stock wallet opening and live Ark lifecycle remain separate dependencies.
- EXP-OP plan corrections are frozen under `/tmp/opencode/role-ui-plan-review-v2/`:
  components `150d3f5b23345ee7f41265719f8b6c3c7805f1b7cb045630ec84bbf4b1b5492b`,
  contracts `0a0d27969143b822950ca44de835c0e1acdcdb3a41f03bb7a41dad5bde861125`.
  Parent verified both hashes. DeepSeek is checking only OP-1/OP-2; Kimi will
  give its independent first-page verdict against v2 while retaining the
  original later-packet review. EXP-ACCESS/AGENT/QA text remains unchanged.
- DeepSeek v2 disposition: APPROVE-EXP-OP, no remaining operator-page blocker,
  bound to the exact two hashes above. Kimi's independent v2 verdict remains
  required before implementation dispatch. Access/agent/QA are not approved
  by this scoped operator-page clearance.
- AP-1a r2 Fable verdict: APPROVE for the bounded probe and exact contract
  `f4fc3e043f420870675095ab24144ba14e0afba6cb9ad69d818d99b1bccb2f62`.
  C1/C2 and D1-D7 are closed; both nodes still honestly fail the Bark version
  floor and funded behavior remains unproven. T0 may bind this reviewed lineage
  and exact check table in its decision-backed rework; T0 itself is not approved.
- Isolated named-profile candidate implementation is dispatched to TFF GLM
  `ses_e4578e2b0ffecRY7O8BchbJK9r` in `lab-node-compat-profile`, with a new
  node source under `/tmp/opencode/AP1b-bark-profile` derived only after exact
  approved L2 source verification. Scope: opt-in regtest-only `bark-v31`, real
  rbitcoin identity, existing Core package dialect, failing-first tests and
  unchanged approved read-only probe. Separate Fable review precedes adoption;
  no primary node replacement, version override or pin change has occurred.
- T4a integration worker confirms the source candidate is untouched and
  primary verification tests are running. Report/manifest closeout follows the
  actual results; this is not yet a completed integration receipt.
- T1a Fable verdict is REQUEST_CHANGES: signing/arithmetic checks passed,
  but reproduced journal defects include uncertain-input release, reorged
  outputs remaining spendable, retry bypasses, concurrent-owner double spend,
  and unvalidated funding/maturity. Author is fixing these plus digest/input
  binding, direct key-index resolution, write-failure recovery and wrappers.
- T1a core/header corrections are coordinated with T0: persisted configurable
  retry caps, run/driver binding, invalidation state and disjoint wallet-index
  allocation with one bootstrap owner. Lab deterministic keys will explicitly
  include run_id and be labeled publicly reproducible from the declared seed;
  private-file permissions are not a secrecy claim for that mode. New decision
  and independent fixtures accompany the rework. No T1a integration approved.
- Hub closeout again returned only an acknowledgment; the report is still
  absent. Sole report ownership is transferred to fresh TFF GLM
  `ses_e456ee982ffevxunpd6RL6z7GX`; the prior Kimi session is not resumed.
  Parent froze the existing 377-file source (17 changes vs runtime baseline)
  at `/tmp/opencode/LP-hub-pre-report-manifest.json`, hash
  `1a1c74661cb9c15c8d8aca0749e038465cdcb98abb5a24ab010cb47a8ae80012`.
  The new worker may write only the report and external manifest, reuse existing
  evidence, and identify gaps honestly. No builds/tests/browser runs requested.
- AP-3a fix-r2 is back with the same Fable reviewer. Parent verified all 386
  records/modes/inventory and exactly 8 changes versus fix-r1:
  `/tmp/opencode/AP3a-fix-r2-manifest.json`, full
  `ae7e99e6d77b4be598bcf9a4e9d1466ebf46d7f31d49a398898c564dfabda218`, code
  `cd09ed78b7b82c795a43b4798608228f300437e38fc50533638eb8b9b86d0530`.
  Rework rejects damaged prior Bark state before mutation, adds the actual
  changelog fragment and corrects recovery/count claims. Author reports 131
  agent tests passed, one ignored; prior Nix/workspace receipts remain scoped
  to fix-r1. Approval and primary integration are still pending.
- AP-3a fix-r2 Fable verdict is APPROVE for that exact bounded candidate.
  Parent checked primary delta paths: all baseline/absent except SPEC, which
  now carries T4a's edits. Wallet integration is queued with the sole primary
  writer `ses_e458119eeffe4Jl3MZjJ4J4HJR` after its T4a receipt. SPEC and
  Cargo/flake hunks must preserve preceding approved changes; merged checks and
  an explicit source-bound Nix gate precede the wallet integration receipt.
- T0 rework is frozen for the same Fable reviewer: 380 files, 15 changes
  versus baseline, `/tmp/opencode/T0-fix-r1-review-manifest.json`, full
  `3304dffa84c32d546ee783bd15beec09bbfe5fa6755a71c9c5721bc6241d41a8`.
  Parent verified hashes/modes/inventory and the separately labeled Rust-delta
  hash. Rework wires prepare/adopt guards, binds approved AP-1a r2 with exact
  check metadata, validates full records and separates preflight from funded
  proof. T1 header/caps/allocation alignment remains a pre-actor dependency.
- Fable's non-blocking wallet notes remain carried: whole Bark-directory
  contents loss with directory retained, a weaker fresh-case SPEC phrase,
  and status-listener teardown on fatal stop. Exact transfer adds no optional
  code changes. Full Wallet opening/live Ark/AP-2 transport are not proven.
- T0 re-review Fable verdict is APPROVE for `3304dffa` and its exact approved
  AP-1a table/lineage binding. Parent checked all T0 primary delta paths remain
  baseline/absent. T0 and the approved AP-1a tooling are queued with the same
  sole primary integration owner after T4a/AP-3a, with separate receipts and
  source-bound combined Nix verification where work has not already begun.
- T1's mandatory history header requires version 2 before its first journal
  write; this is an explicit pre-actor integration condition, not permission
  to omit run/driver/network/key-mode/cap fields from v1. T1a owner received
  the requirement. No runtime traffic capability or funded proof exists yet.
- Hub report closeout is delivered and verified: 378 files, full
  `503ec69595f3fd4283e0e67e47985e86a89209969de3f068efa67c93c8c6512f`, code
  unchanged at `1a1c74661cb9c15c8d8aca0749e038465cdcb98abb5a24ab010cb47a8ae80012`,
  `/tmp/opencode/LP-hub-review-manifest.json`. Existing live/recorded evidence
  is source-bound, but Rust execution and mobile/console receipts are missing.
- Hub owner is closing only those verification gaps on frozen code: targeted
  Rust/Node/doc checks and an owned read-only recorded-run browser check.
  Existing native-mining proof is reused; no new mining or full-workspace
  rebuild is requested. Report/provenance wording will be corrected and a new
  manifest frozen before Fable. Old source/evidence/manifests stay preserved.
- AP-1b named-profile candidate is delivered: default 190000 versus opt-in
  310000, preserved rbitcoin identity, existing package dialect, source-bound
  read-only counterfactuals. It is not adopted or Fable-approved. Parent froze
  the initial 27-file packet at `/tmp/opencode/AP1b-initial-packet-manifest.json`
  (`016b90b8`).
- Before review, author is removing the added `RBITCOIN_RPC_COMPAT_PROFILE`
  environment knob (separate node ask-first approval was not obtained), keeping
  CLI/config activation, and adding required node changelog/owner documentation.
  The approved probe can inspect an explicitly launched node via `--rpc-url`;
  an environment activation knob is not needed for verification. Revised
  explicit-launch receipts and source freeze precede node Fable review.
- Four-packet primary transfers are complete and independently source-verified.
  T0's titled changelog fragment was mechanically reflowed into one frontmatter
  bullet; parent reproduced the exact SPEC result (`3f669947`) with no wording
  change. The bullet hash `9b3abff5` excludes its terminal newline; saved-file
  hash including the newline is `ed9325a4`.
- Final Nix closeout remains: current flake has six fmt/clippy/doc/test/ui-node/
  test-ark checks, not a live namespace/mining hook. The same integration owner
  is running one clean-source combined gate with explicit result/source/deriver
  receipts. The earlier pre-T0 `test-ark` result is preserved with its scope.
- Combined gate started on `/tmp/opencode/four-packet-nix-src` (523 files,
  9.8 MB; worker-reported source aggregate `92dc2e25`), max-jobs 1 / cores 2,
  with 22 GB free at launch. Completion receipt and per-check output bindings
  are pending; source evaluation or a quiet log is not a pass.
- AP-1b CLI/config-only r1 candidate is frozen for its first Fable code review,
  reviewer `ses_e4532be38ffeCXaI7w3D77nbxa`. Parent verified the 41-row packet
  and 25 node-source records and froze all 82 packet/history/log files at
  `/tmp/opencode/AP1b-r1-full-review-manifest.json`, hash
  `9375653ca90158446b99ae5c02aaee8482153984c68ba53aef447b115df40050`.
  This includes 27 gitignored proof logs omitted by the initial Git-based walk.
- AP-1b r1 removed the new environment knob, added owner docs/changelog, and
  captured explicit-CLI default/profile-on read-only runs with external binary
  bindings. Candidate binary is `b75dad84`; profile-on reports surface-compatible
  with funded behavior unproven. No approved L2 or primary node adoption occurred.
- Hub verification worker ended with a provider 500 after saving receipts.
  Parent recovered actual passing Rust tests (29 lib, 3 HTTP/SSE, 6 hub),
  Node tests and doc-check exits. Browser attempt 3 is a genuine failed proof:
  mobile scroll width 868 at client width 390, and failed keyboard-disclosure
  assertions on both viewports; it must not be reported as passed.
- Fresh TFF Kimi `ses_e451c559cffeCc5XyCePj06jWj` now owns bounded hub
  recovery: diagnose/fix responsive layout and disclosure behavior or harness
  error, preserve all failed evidence, rebuild only affected assets/binary,
  repeat browser proof and finish the report/freeze. Passing unrelated tests
  are reused. No mining or changes to the feedback app are authorized.
- AP-1b Fable verdict is APPROVE for patch `97f84755`, node aggregate
  `f8a51111` and binary `b75dad84`, including independent default/profile
  counterfactuals. Approval is bounded code/probe evidence, not a baseline-node
  replacement, pin change or funded Bark acceptance.
- Parent authorized preparation of a separate opt-in Ark development profile.
  TFF GLM `ses_e45178b81ffeaN10b9kNhMcxlm` owns fresh
  `/tmp/opencode/AP1b-lab-integration` source, additive approved packet transfer
  and the conditional lab decision/receipt. Standard L2 remains unchanged.
  Node/RPC full suites and a fresh probe on the integrated binary remain
  required; heavy validation is queued after the running primary Nix gate.
- AP-1b preparation is complete. Parent independently verified all 82 primary
  packet paths/modes and all 2,102 prepared node-source records, reproducing
  source index `da21f56dd6ab581e0d8353fe7431cc44efa9eb29bffb998459cbd7ac29105a9e`.
  Git index remains `44f21a92`. Receipt:
  `/tmp/opencode/AP1b-lab-integration-prepared.json`; conditional decision:
  `docs/decisions/2026-11-08-lab-bark-v31-profile.md`. No node build or probe
  ran in this step; validation still waits for the primary Nix completion.
- T1a journal rework is frozen and with the same Fable reviewer. Parent
  verified all 387 records/modes/inventory and 14 modified + 2 added versus
  the original candidate: `/tmp/opencode/T1a-fix-r1-manifest-full.json`, full
  `fef50f3437df50a52e143442541f47a70df4bba7bb896f7ff48a9f0470b1eb0e`, code
  `a5d3318f61683ec8cf06ee7d1237fb4c3acefaa71250c5a1e6198a42ce7074ce`.
  Review covers lifecycle/cascade/locking/maturity/write-failure regressions,
  v2 header and allocation enforcement, plus remaining report/fragment precision.
  It is not yet approved or integrated; live traffic acceptance remains open.
- Kimi's independent EXP-OP v2 review has one remaining blocker (OP-3):
  recent-block rows promised by Details lack a DTO field, and module-export/
  store-test ownership needs completion. Author is adding a bounded nullable
  recent-block collection, explicit operator module export and focused store
  snapshot tests, keeping the four-route contract. Final OP-only reviews follow;
  later access/agent/QA findings will be amended separately after this freeze.
- EXP-OP v3 is frozen at `/tmp/opencode/role-ui-plan-review-v3/`: components
  `a07b8ab30e2875c457ed540cd2894588d90f87003ba1b5c9e80af463a773dc2f`, contracts
  `ace881da5100d9d4e9b4056c067d53aaff9d322544b3bcc8a9d8caa079756b2c`.
  Parent verified both hashes. Both reviewers are checking only OP-3 and
  preservation of OP-1/OP-2; later packet sections remain unchanged.
- DeepSeek has approved EXP-OP v3 with no remaining OP blocker. This confirms
  its own v2 approval on the revised pair; Kimi's separate v3 verdict has not
  yet returned to the parent and is still required before dispatch.
- Kimi has now independently APPROVED the same EXP-OP v3 pair. Both plan
  gates are met. TFF GLM `ses_e44f03a1cffe5EjIbGPld4kKc2` is implementing
  the first operator page in `lab-pool-operator`, from a fresh 583-file primary
  source snapshot `/tmp/opencode/EXP-OP-baseline-src`, manifest hash
  `3e2a31be506a36355aee80aef8be60457ca1a1ad68192dc02af80bfbda7c9cb0`.
  Scope: optional same-process loopback listener, single liability snapshot,
  ten recent blocks, simple read-only UI, real DB/browser tests and Fable gate.
- Role/UI author is applying one combined later ACCESS/AGENT/QA amendment,
  preserving the approved OP sections. Authentication-before-replay, per-user
  reads/SSE, browser/namespace routing and real field/effect providers remain
  plan gates for those later packets; OP implementation does not wait on them.
- T1a r1 re-review remains REQUEST_CHANGES, now confined to real-I/O rollback,
  restart-in-flight uncertainty, canonical re-inclusion and enforced allocation
  type-state, plus changelog/report precision. The same author is fixing those
  items; previously verified lifecycle/signing changes are retained.
- AP-2 registration packet is finally delivered and with Fable
  `ses_e44ea4744ffeHG3mTTzBN5l93T`. Parent verified 379 files and the declared
  ordered-array hash recipe (octal permission strings): 21 modified, 9 added,
  full `f8bf8c80e61d3156ab6a20efd5c87f4cc65b2a79e61819fc4972f889ba644f03`,
  code `5e21d48b93d7c7c4529a58e54b2c31d532ca1436e9325bc21852af052e5fcbd2`,
  `/tmp/opencode/AP2-review-manifest.json`.
- AP-2's explicit contract amendment signs request_id to prevent signature
  transplant to a newer counter. Fable is checking that amendment, durable
  replay/cache supersession, full bootstrap/destination publication and privacy.
  AP-3 adapter must consume the canonical signer and supply epoch-derived D;
  that live client integration is not yet proven or authorized by this packet.
- Later ACCESS/AGENT/QA v4 amendment is frozen at
  `/tmp/opencode/role-ui-plan-review-v4/`: components
  `67a0d8ea3d41be9ec132253f3f6c47e9e48e4e4c35f8a00e9bd1f026fc1de285`, contracts
  `f44d9e627ce818ebd9e389f7c427b7aba4ff5e590a234b0342a33dd85295c197`.
  Parent verified both hashes and byte-identical preservation of approved OP
  sections. Kimi and DeepSeek are independently reviewing only later packet
  amendments; the operator implementation continues without a new plan gate.
- DeepSeek v4 later review leaves four bounded corrections: explicit public
  static token-entry exception, run/principal-scoped browser token state,
  Bearer-capable fetch/SSE transport, AP-3b live funds as the fact-4 provider,
  and scoped/versioned command hashing preserving legacy replay (the token
  exception and scoping are one finding). Author is applying those corrections;
  Kimi's independent frozen-v4 review continues. Approved OP remains unchanged.
- Later-plan v5 is frozen and hash-verified: components
  `cd2185389b41d08d1b16fa763b6bdac513c333d4cd0b78634af19d5a7a0f1830`, contracts
  `05c0d4f8f0ac362f404e3a0d9958dc3240839246bd7cdce3325fc446b1527a53`, under
  `/tmp/opencode/role-ui-plan-review-v5/`. Parent again verified OP preservation.
  Both reviewers are checking the four clarifications and must give an explicit
  EXP-ACCESS foundation disposition independently of later live-Ark acceptance.
- DeepSeek has APPROVED the EXP-ACCESS foundation against exact v5; all four
  listed clarifications are closed. Kimi's independent verdict is still needed
  before dispatch. This scope does not approve the agent UI or final QA and
  does not require a live Ark run to implement the access foundation.
- User explicitly requested disk discipline and authorized stale-temp cleanup.
  At 16.6 GiB available, parent removed only the completed AP-3a, T4a and T0
  worktrees' `target/` caches, after source/ownership/process checks. Six
  top-level executables were preserved by hash via hard links. Reclaimed
  38.39 GiB; 54.99 GiB available afterward. All 1,141 frozen source records
  re-verified unchanged. Receipt:
  `/tmp/opencode/disk-cleanup-20261108T124252Z/receipt.json`.
- `AGENTS.md` now requires 50 GiB starting headroom, at least 20 GiB projected
  residual space, and one heavy build under nonblocking
  `/tmp/opencode/silent-pool-heavy-build.lock`. Active workers received the
  policy; no new revision/reviewer target trees or unapproved shared cleanup.
- Final combined Nix gate completion was recovered from the worker and
  independently verified: six checks passed, all log hashes/output paths and
  523 source rows match. Receipt hash `abaabc23`; workspace 724 passed and
  Ark-feature 507 passed (ignored cases remain recorded). No repeat gate needed.
- AP-2 Fable review returned REQUEST_CHANGES: stale cached acknowledgment,
  missing SQL bind, terminal-invalid stream entries stuck pending, and job
  identity on destination-only changes. Author is addressing these plus the
  bounded documentation/error-path issues under the new shared build lock.
- Kimi has independently APPROVED the later v5 plan and EXP-ACCESS foundation;
  both foundation dispatch gates are met. TFF GLM
  `ses_e44bc72bfffefwuJrKdTOymUjk` is implementing it in `lab-expanded-access`.
  It reuses the existing `EXP-OP-baseline-src` snapshot: parent verified all
  203 Rust/Cargo/flake records still match primary, avoiding a duplicate snapshot.
  Current primary AGENTS disk rules apply despite the older snapshot copy.
- EXP-ACCESS scope is opt-in claims/authentication, principal-bound legacy-
  compatible replay, per-user read/SSE projection and closed relay plumbing.
  Actual wallet/traffic providers remain unavailable until their owners wire
  them. Builds require the shared nonblocking lock and headroom; Fable review
  precedes integration. Operator implementation and later live QA stay separate.
- T1a round 2 is frozen and with the same Fable reviewer. Parent verified
  388 files, 11 modified + 1 added versus r1:
  `/tmp/opencode/T1a-fix-r2-manifest-full.json`, full
  `ed0d100a9ef44d97d6cbb4df0206839cd3f47adb2d5f568943fd25366e4004b0`, code
  `47158c2fa2e2964b1fa5dbbca0113e747193b228ea82db179495c6122c23c5c1`.
  Re-review is confined to real-fault rollback, restart uncertainty, canonical
  re-inclusion, enforced allocation and packaging. Builds remain disk/lock gated;
  T0 v2 header alignment and live traffic acceptance are still separate gates.
- EXP-OP implementation is delivered and with Fable
  `ses_e44a0c9b1ffeuXyD30Z1T0FU9V`. Parent verified the 15-file implementation
  delta plus report and froze all 592 source files at
  `/tmp/opencode/EXP-OP-parent-review-manifest.json`, full
  `ed00ba4826f7ce096fa409f8af1ebdb121ae1bd9c0027eb4b8b9a00005023ad9`.
  Reported evidence: 16 operator tests, 4 real-DB snapshot tests, and 29 browser
  checks including mobile, exact amounts, keyboard interaction and stopped DB.
  These are test-fixture UI proofs, not native payout/Ark acceptance.
- EXP-OP source and binary are frozen for review. Parent requested the exact
  recipe for the author's separate `96ed7895` delta hash; per-file bytes already
  verify and the parent full-manifest recipe is explicit. No integration yet.
- EXP-OP delta-hash recipe is resolved and independently reproduced: SHA256
  of the ordered concatenation of each path and its hex file hash (15 files,
  1463 characters, no separators or modes). Fable received the clarification;
  no source or evidence changed and no additional tests were needed.
- T1a round-2 review closed real-I/O rollback, restart uncertainty, allocation
  enforcement and new-block re-inclusion. Two small items remain: same-block
  canonical reappearance after a reorg back (with newer chain evidence), and
  recording genuine late rejections while guarding only command completion.
  Author is implementing S3/S4 and their replay/cascade regressions, retaining
  all verified fixes and the shared build/disk-budget guard.
- EXP-OP Fable verdict is REQUEST_CHANGES: listener isolation and most
  behavior verified, but the race test needs a conserved-total oracle, one
  test leaks an extra fixture DB, and polling can overwrite newer data with
  an older response. Author is fixing R1-R3 and refreshing browser evidence
  with mobile Details open, strict wrapper containment and exception capture.
  Bounded stale-data/header formatting fixes join this pass; no public API
  redesign or primary integration is authorized.
- Hub recovery fixed layout, recorded labels/links and disclosure focus/state.
  Reported current tests: Node 24/24 and Rust 29+3+6 pass; first fixed binary
  reached 80/84 browser checks. The last row-disclosure fix is frozen at code
  `712fa8cb` but its identical-binary browser proof was lock-blocked once.
- Parent checked the build lock once after that blocker (no holder) and
  authorized one guarded final hub build/browser attempt. No passing Rust/Node
  tests are restarted; the old failed/partial browser receipts stay preserved.
  Final 84/84 proof and a fresh full manifest still precede Fable and integration.
- T1a round 3 is with the same Fable reviewer for S3/S4 only. Parent verified
  all 388 records and exactly seven modified files versus r2: full
  `65a16584fb89429ef2f59e88285fbfdd587a5892bd51c3f3489e2c7ad8eb3748`, code
  `a8d28ceac3a9bd63fc72886ca5040544431c1235b31a98e902290ee4ff834628`,
  `/tmp/opencode/T1a-fix-r3-manifest-full.json`. Same-block reorg-back and
  genuine late-rejection regressions are reported green; approval remains pending.
- T1a round-3 Fable verdict is APPROVE for the exact core. Serial primary
  writer `ses_e458119eeffe4Jl3MZjJ4J4HJR` is integrating only that delta,
  preserving AP-3 Cargo edges and current SPEC entries. Core integration does
  not enable a driver or waive the version-2 first-runtime-write gate.
- T0 history-v2 alignment is dispatched separately to TFF GLM
  `ses_e4473a19effeWS3JcEvSRsSrC2` in `lab-traffic-history-v2`, reusing the
  existing source snapshot plus approved T1a overlay. It must conform to the
  actual first journal record and allocation digest, keep control protocol v1
  distinct, and pass Fable review before integration.
- AP-1b integrated-node validation is now dispatched because the primary Nix
  gate passed. It remains subject to the shared nonblocking build lock and disk
  headroom. Required node/RPC suites and fresh integrated-binary probe will be
  recorded separately; default L2 and live Ark acceptance remain unchanged.
- AP-1b validation acquired the shared lock with 90 GiB free. Fmt and clippy
  passed; node/RPC suites and the integrated build are running before probes.
  Parent corrected a proposed harness restriction: an integrated binary may
  legitimately hash identically to the candidate; path/source/build receipts,
  not forced hash inequality, establish provenance. No gate result inferred.
- EXP-OP rework is with the same Fable reviewer. Parent verified all 592
  records and exactly six changed files versus the original candidate: full
  `d3710c0c3100c86251abd377043b80d1ffcec5b9ba1eca64bf6f97c69e91d1d4`, code
  `365e0bf200a98252278dec495bbb68d341c0f118e1bf4acbb8de175807a80614`,
  `/tmp/opencode/EXP-OP-rework-parent-manifest.json`. Reported proof is 46/46
  browser checks plus the strengthened real-DB tests; approval remains pending.
- T1a primary transfer is complete, validation still running. Parent separately
  verified all 18 new files byte-exact, reverse-removed the new Cargo member/
  lock block to reproduce AP-3 roots exactly, and reverse-spliced the T1a
  changelog to reproduce the prior SPEC. Cargo checks and the source-bound
  Nix workspace-test gate await actual completion receipts under the shared lock.
- EXP-ACCESS foundation is delivered and with Fable
  `ses_e44612566ffe3LCJTjtpruLe0c`. Parent verified 589 files and the exact
  12-record delta (6 modified, 6 added):
  `/tmp/opencode/EXP-ACCESS-review-manifest.json`, full
  `7a7087e070d2be2172934bc18860b3a3ed2579d5363f85e0ade355a5185e0020`, code
  `d3a5f078a5f3369a0c4f8258809f7a926f805b7995c4550428c0f4b172da3d73`.
  Review covers auth across lifecycle paths, cached-claim changes/revocation,
  principal-bound replay, filtered reads/SSE and closed relay routing. The
  reported 14 access, 33 legacy and 40 unit passes are fixture foundation proof,
  not live wallet, traffic or Ark acceptance. No integration yet.
- EXP-OP rework Fable verdict is APPROVE for code `365e0bf2` and sealed
  test-feature binary `1684cc84`. Serial integration is queued after T1a checks.
  Production build hashes may differ by feature unification; sealed copies,
  not mutable `target/debug` paths, bind the browser proof.
- Small host-mode operator runtime wiring is dispatched to TFF GLM
  `ses_e444fa08bffes1FApjGS7mp54I` in `lab-operator-runtime-wiring`: explicit
  opt-in flag, existing port allocation, generated config and known loopback
  URLs. It reuses the operator-family build cache under the shared lock and
  needs its own Fable gate before a fresh manual run. No new mining in this slice.
- AP-1b integrated-node validation PASSED. Parent verified five exit receipts,
  14 probe artifacts and all 2,102 source rows. Node suite 118/118 and RPC
  suite 214/214 passed; integrated binary legitimately reproduces `b75dad84`.
  `/tmp/opencode/AP1b-lab-integration-validation.json` hashes to `95a3d131`.
  Profile-alone dialect enforcement is proven with all RBITCOIN env removed;
  funded behavior remains unproven. The node-profile SPEC fragment is queued
  for the serial writer; standard L2 is unchanged.
- Hub final identical-binary proof is 84/84. Parent verified all 378 source
  rows, binary `a577aea7` and browser receipt `555e81f4`; full snapshot
  `3896b132e8accda44fa4aa8efd7eccbfe37c07d0be4fb3c397e733f5b4034168`, code
  `712fa8cb`. Fable `ses_e445425e8ffeh6YLtDeu2FM4n0` is doing its first
  concrete review. This is recorded-mode browser proof, not a new live experiment.
- T0 history-v2 contract is with Fable: 602-file layered snapshot
  `dc2c865854d899a6fe013aa69c1a88b418798b650f8ad1ab4d60abf0019736a7`, code
  `df8cedb6cbd47cff3632d0774d85144467389d3071f28d2e9af1be316c5ed4b8`,
  `/tmp/opencode/T0-history-v2-manifest-full.json`. Parent verified the freeze
  and all 15 unchanged approved T1a core files. Real-writer header conformance
  is reported; actor activation and funded traffic remain separate gates.
- EXP-ACCESS Fable verdict is REQUEST_CHANGES: whole-run artifacts leak to
  user tokens, directory-mtime caching misses in-place claim revocation, and
  formatting/lint gates need completion. Author is also closing reserved-name/
  file-hygiene cases, operator SSE revocation and empty-worker scope handling,
  with real legacy-byte tests and precise relay/static-page limitations.
  Existing auth/scope/replay fundamentals were verified; no integration yet.
- Hub Fable verdict is APPROVE for code `712fa8cb`, with doc-only merge
  conditions: correct the remaining blanket wallet-unimplemented sentence and
  add a lab-scope decision/changelog. Hub integration is queued after T1a/OP
  with semantic merges preserving T0. Embedded assets stay exact; no new browser
  seal is required for those documentation conditions.
- History-v2 Fable review found only false serde rationale: internally tagged
  enums can reject unknown fields; T1a's current leniency is a chosen annotation.
  Author is correcting the four wording/test-rationale sites without changing
  core behavior or digests. Reviewer permits a manifest-only recheck.
- AP-2 rework compiled and passed docs checks, but real PG/Redis regressions
  were lock-blocked (43) and final fmt/clippy were not claimed. Parent checked
  the lock once afterward (free) and authorized one guarded validation attempt,
  including the modified PostgreSQL suite. Integration/Fable re-review still
  waits for actual runtime-test receipts and the final frozen snapshot.
- AP-2's first real post-rework run exposed the missing seventh bind,
  omitted error_code SELECT field and a job-test initialization error; bounded
  fixes are applied but their rerun was lock-blocked. Parent checked once
  afterward (free) and scheduled formatting plus the complete affected test
  group in one guarded attempt. Failed 101/43 receipts remain preserved.
- History-v2 wording-only r1 is back with Fable for its permitted manifest-only
  check. Parent verified 602 rows and exactly four changed files: full
  `e2dc40c7d78527536998e7f9bd793273bbef4698fa700300e2d15f1c4c211859`, code
  `4c1bce8c0d12193ae1243c1166e13812798906c2f17e6748aa0b9842993ffb84`.
  Types, validation, digests, T1a core and changelog fragment are unchanged;
  no new build/test was run for the rationale correction.
- AP-2 again lost a nonblocking lock race without executing checks or creating
  services. Parent created `/tmp/opencode/silent-pool-build-priority.json`
  reserving the next slot for AP-2 and notified all active builders to finish
  their current invocation, then yield. AGENTS now documents priority tickets;
  no repeated lock polling or bypass is permitted.
- History-v2 r1 Fable verdict is APPROVE. The exact amendment is queued with
  the serial integration owner; no duplicate T1a overlay/member/lock block may
  replace already integrated core files. Build validation waits for AP-2's
  reserved window, while safe source/receipt work can continue.
- EXP-ACCESS rework code/docs are prepared, but its new lint/test receipts
  remain pending behind the AP-2 priority ticket. It holds no build lock.
  Parent queued its verification window next after AP-2 and requested a real
  preserved pre-change legacy command fixture for the byte-replay proof.
  No green rework freeze or integration readiness is claimed yet.
- Host operator runtime wiring source is prepared (opt-in config/ports,
  route-presence probes, typed inventory interface and narrow JS URL guard).
  JS tests report 12/12; Rust tests/compilation are not yet run because the
  AP-2 priority ticket is active. It holds no lock. Verification is queued
  after AP-2 and EXP-ACCESS; no Fable-ready or live-runtime claim yet.
- User reaffirmed that silent-pool must interoperate with vanilla Second.tech
  Ark servers. This matches SPEC §11.3/O17: no captaind/server fork, plugin or
  custom server RPC; SPc changes are pool/client-side. The regtest rbitcoin
  `bark-v31` profile changes the Bitcoin backend, not Ark server software.
  AGENTS now makes this explicit. Full stock-server send/receive/refresh/
  offboard acceptance remains required and is not yet proven in the expanded lab.
- EXP-ACCESS now has a preserved pre-access StopRun journal fixture, inspected
  as non-secret and copied without modifying the original run. Its new tests
  target exact old-byte replay/write compatibility; execution and re-freeze
  remain queued behind AP-2's build reservation.
- AP-2 final targeted gate is green: store integration 22, registration 7,
  publication/privacy 3, pool lib 229 and ext 14 passed; fmt/clippy/docs exited 0.
  Parent verified all 379 records at full
  `7a8aea5069747c6e6099962f09f7ee5f7ef3f1fd65c07f476f9047dff17f703e`, code
  `e0ad690da836be525f43930f6f937ce585f178c0c717a8b0b2be491517d962ba`,
  `/tmp/opencode/AP2-finalgreen-manifest.json`. The same Fable reviewer is
  checking the rework; no integration or stock-server E2E claim yet.
- Parent transferred the build-priority ticket from AP-2 to EXP-ACCESS for
  its final lint/access/legacy verification and freeze. Other builders still
  yield their next acquisition; AP-2 review can proceed source/receipt-only
  until execution is available without bypassing the reservation.
- AP-2 round-2 Fable review closed all six prior findings, but found one new
  blocking rejected-ID replay case: a journaled rejection without advancing
  the last applied counter can be mistaken for a new request and hit a PK
  violation. Author is adding journal-first replay/mismatch resolution and its
  three-step regression. Tests wait for the reserved build queue; no Ark-server
  changes are involved. Prior final-green receipts remain separately scoped.
- AP-2 R2.1 source and the static-ID1/rejected-ID2/mismatched-ID2/valid-ID3
  regression are authored, including preserved rejected replay and supersession
  behavior. No new tests or freeze are claimed. The two affected real-DB suites
  plus fmt/clippy await the parent-granted window after current queued work.
- Source-only traffic runtime handoff planning is dispatched to TFF GLM
  `ses_e43d49d09ffekI0NY6OjhQW3cP` (one new bounded plan document). It resolves
  actual-run preflight staging, the minimal real mempool consumer, T1b actor/
  RPC plumbing, native T2-L funding and T3 proof ownership using the approved
  core/header contracts. No build slot, new signer, server patch or second
  probe is authorized. This prepares the remaining real-traffic implementation.
- Peer-cleared T4a witness primitives are dispatched to TFF GLM
  `ses_e463308d8ffeQPpMjPUNcF9fRf` in `lab-witness-primitives`, based on the
  frozen approved runtime. Scope: stripped txid/full wtxid separation, BIP141
  commitment/reserved value, full-block versus miner coinbase serialization,
  weight accounting and independent deterministic vectors. Shared front wiring
  is a serial merge hunk coordinated with AP-2.
- T4a has no services or mining scope. Fable reviews its implementation before
  integration; subsequent T4b must prove real native-PoW canonical mixed-witness
  acceptance, including P2TR, before Ark A4/A7/A8. AP-5 also needs the single
  production ladder. AP-1a/AP-2/AP-3a contracts remain unchanged.

## Nix completion handoff

- User requested a fresh-context handoff after the packaging worker appeared
  stalled. The predecessor `ses_e4960a7b9ffem4e4Izur6q1wjk` has been told to stop
  writes/tests, checkpoint and confirm cleanup; it must not resume afterward.
- Fresh TFF Kimi recovery session `ses_e47c0beeaffevistP84atHufpS` is inspecting
  `lab-nix-network-tools` read-only until the predecessor confirms it is paused.
  Only then will write ownership transfer. No concurrent writers are authorized.
- Preserve completed evidence: 28/28 isolation tests, 14/14 Rust namespace tests,
  and an 80/80-consistent stopped real run. Recover the missing final report,
  exact source/provenance binding and gate receipts rather than rerun successful
  work by default. Fable review and primary integration still follow.
- The design gallery and user feedback at port 18481 remain untouched.
- Read-only recovery confirmed that the remaining issues are closeout records:
  missing implementation report, verifiable receipts for all seven Nix checks,
  and precise binding of the native run's approved runtime binary to the newly
  packaged Nix-tool closure. Existing successful tests are preserved.
- The predecessor remained active after the checkpoint request. Parent used the
  documented session interrupt with `resume=false`; it returned
  `interrupted=true` and the harness confirmed cancellation. No service restart
  or other session interruption was performed.
- Fresh session `ses_e47c0beeaffevistP84atHufpS` now has sole write ownership to
  finish those records, freeze the candidate and hand it to Fable. The predecessor
  must not be resumed. New tests are required only for unresolved evidence gaps.
- Its closeout response was interrupted by a provider connection/payload error,
  not a reported Nix failure. Parent resumed the same fresh session once to
  recover any partial report/receipts and finish, preserving completed tests.
- The retry also ended with a provider socket timeout. Parent confirmed that
  session was idle and assigned sole closeout ownership to fresh TFF GLM 5.3
  `ses_e46972ce1ffeeEfWBvEgfyb60n`; no further Kimi retries are scheduled here.
- Completed receipts were recovered before reassignment:
  `Nix-net-tools-proofs/flake-check-realization-receipt.json` proves all seven
  checks realized, and `flake-check-final-offline-exit-receipt.json` records
  exit 0 against code `ad03c7e0`. No new build/test run is needed for this gap.
- Remaining closeout is the missing implementation report and accurate final
  manifest/provenance scope, followed by Fable review and integration.

- Closeout is now delivered: report and 373-file final manifest at
  `/tmp/opencode/Nix-net-tools-final-manifest.json`, full
  `3b4f66c24058f1828b972573f313951ed40cd6230da909ba1ee2a4ee7b6926ab`,
  code `ad03c7e0d23ba83b5d82448fcf5a60754583ad83c5033569b2f3cd309692d5c3`.
  Parent verified all content hashes/modes and manifest hash.
- Fable reviewer `ses_e46849c5effeDFqHcOSSImIrNk` is reviewing the exact packet.
  Initial provider response was blocked before a verdict; a narrower legitimate
  review request was retried once. No candidate or test evidence changed.
- The report explicitly discloses that native-r1c ran approved application
  binaries with new Nix tools, while a small new supervisor helper has only
  compilation/unit coverage. Fable must resolve that scope before integration.
- Fable review returned REQUEST_CHANGES confined to the supervisor helper;
  the Nix packaging, closure provenance, normalized scripts and seven check
  receipts were approved as-is. The helper injects an argument rejected by
  direct coreutils applet launches, so it is a real production regression.
- Closeout owner is applying the minimal fix: restore the approved supervisor
  byte-for-byte, rename the T4 symlink fixture to retain the `sleep` applet name,
  and rerun the focused test with pinned Nix tools. No new helper mechanism or
  host-tool fallback is planned. Re-freeze and a confined Fable re-review follow.
- Fix progress: supervisor is restored byte-identically to the approved file;
  T4's renamed `sleep` fixture passed in the Nix environment, and direct applet
  launch checks passed. The fresh seven-check flake rebuild is still running;
  evaluation/derivation creation is not a completed gate.
- The author will record the actual check exit receipt and final report/manifest
  after completion, then return the frozen correction for Fable re-review.
  No duplicate test or build run is requested.

## Product mockups and review app (2026-11-07)

- User requested a design/layout pass with at least ten mockups covering pool
  operator, miner/client-daemon, and desktop/mobile end-user funds/status/daemon
  management. No production UI implementation before multi-model design review
  and user feedback, followed by Fable's final check.
- Manual QA at port 18480 is stopped: durable `stop_run` succeeded, controller
  exited and all recorded processes are terminal. Browser-harness work is paused
  before file writes; that agent launched no browser/runtime processes.
- Designer: `tff/kimi-k3`, `ses_e48e2d10affefAJO5LaBPFXwH2`, worktree
  `lab-design-mockups`, owns `design/mockups/**` and its brief/index docs.
  Target: 14 distinct PNG/HTML mockups, including two operator layout directions,
  a contact sheet and a stable screen catalog. All data is synthetic.
- Independent reviewers: `tff/glm-5.3` (`ses_e48e28425ffeYRv1iiOdMhWJMo`) for
  role workflows/layout, and `tff/deepseek-v4.1-flash-1017`
  (`ses_e48e22cf0ffeE02EDzgSk5r6cX`) for funds/daemon UX and accessibility.
  They prepare criteria independently and will review the frozen rendered draft.
- Review-app owner: `tff/glm-5.3-flash`, `ses_e48e129dbffe5xO1k1BcuLdBLr`,
  same worktree but disjoint `design/review-app/**` and own doc. The user asked
  for a web gallery with persistent per-screen/overall notes readable by agents;
  JSON plus Markdown exports, stable screen IDs/version and saved-state feedback
  are required. State stays outside the frozen source/git tree.
- The review app is a design-feedback tool, not the proposed production UI.
  It will be served when mockups are ready; no mining services are required.
  Manual QA stays down until the mockup review is finished.
- GLM's independent criteria brief is complete at
  `/tmp/opencode/design-review-glm-brief.md` and handed to the designer. It covers
  role separation, lab/production context, first-run connection, financial/mining
  state distinctions, degraded states and readable hierarchy. This is not an
  approval of unseen mockups. Model-specific tier statistics must not be shown
  as universal facts; DESIGN/SPEC remain the policy authority.
- DeepSeek's independent criteria brief is complete at
  `/tmp/opencode/design-review-deepseek-brief.md` and handed to the designer.
  It covers classified funds, booking versus maturity, units/fees, daemon and
  identity bindings, permissions, command outcomes, mobile ergonomics and
  accessibility. Rail-specific lifecycle disclosures apply only to relevant
  screens; planned-feature scope must be clear without excessive badge clutter.
- Both reviews so far are criteria-only. Actual rendered mockups, user notes
  and Fable's final check still precede production UI implementation.

### Review app candidate

- Eight-file tool snapshot:
  `dc63be6ba58e93bf7513a0f6bfe617ebac325b339a7fc25b56b1ec957c07a087`,
  parent-verified in `/tmp/opencode/lab-design-review-app-review-snapshot.json`.
- Candidate implements local gallery/filter/compare, per-screen and overall
  notes/statuses, explicit saves with draft preservation, revision conflicts,
  persistent `feedback.json` and derived `feedback.md` exports outside git.
- Implementer reports 26 stdlib tests and three headless Chrome DOM smoke tests
  passed. Screenshots demonstrate the review shell with fixture data, not the
  still-pending rendered product mockups.
- Fable tool reviewer: `ses_e48d89d7dffe2hAN52AoIjWJUe`, report destination
  `docs/reviews/2026-11-07-design-review-app-fable.md`. This is a review of the
  feedback tool, not product-design approval.
- Port 18481 is not serving yet. Parent will bind the review app to the actual
  mockup catalog when ready and provide the user its URL and persistent note paths.

### Review app first verdict

- [Fable tool review](reviews/2026-11-07-design-review-app-fable.md):
  REQUEST_CHANGES on `dc63be6b`. Independent server probes and real Chrome note
  CRUD verified core persistence, conflicts, restart, literal rendering and
  containment behavior, but found three blockers.
- B1: filter/compare rerenders append duplicate main panels and stale editors.
  B2: separate server processes can overwrite notes without a cross-process lock.
  B3: zoom crops the left edge and 200% is defeated by flex shrink.
- Tool owner is fixing those plus GET Host validation, SVG script containment,
  visible catalog/version staleness, export/commit error reporting and stronger
  browser regressions. Designer was told to preserve versioned published assets.
- Review app remains offline; no real note state has been created and manual QA
  remains stopped. Mockup design and production implementation approvals are
  distinct from this tool review.

### Review app second candidate

- Nine-file snapshot:
  `123c2eceb77da1b52973e7b62a1a76ff49c112a65afcc6faf9538f1b2af6e518`,
  parent-verified at `/tmp/opencode/lab-design-review-app-r2-review-snapshot.json`.
- Candidate fixes duplicate rendering, adds lifetime kernel locking, intrinsic
  image zoom, all-request Host checks, SVG containment, visible catalog/draft
  staleness and accurate post-commit warning semantics.
- Implementer reports 35 server tests, two DOM smoke tests and five real Chrome
  CDP flows passed, including note persistence/conflicts and patterned-image
  zoom. Fable's original duplication probe now passes all four checks.
- Same Fable reviewer resumed for independent rechecks. Port 18481 remains
  offline and no real user-feedback state has been created yet.

### Review app second verdict

- Fable independently verified the major rendering, locking, zoom, Host/SVG,
  conflict and persistence fixes. Verdict remains REQUEST_CHANGES for two small
  issues affecting actual design review.
- R3: permanent per-entry catalog warnings must not disable status controls for
  every screen; gate writes on explicit stale/last-good catalog state instead.
- R4: after a design-version bump, saved local drafts need a visible recovery
  path without retyping or silently rebinding feedback to a changed mockup.
- Tool owner is revising those and bounded startup/commit-warning/version-lock
  handling, with real-browser regression tests. Manual QA and review port remain
  off while the mockups/tool are prepared.

### Review app third candidate

- Nine-file snapshot:
  `68a4d8edd96c775669a9477095a727d3b8c6b80bcd91af80764d7926b0f1c267`,
  parent verified at `/tmp/opencode/lab-design-review-app-r3-review-snapshot.json`.
- Candidate separates catalog warnings from stale fallback state and offers
  clearly labelled draft carryover across design versions without retyping.
  Commit warnings, first-start recovery and freshness locking are also tightened.
- Implementer reports 38 server tests, two DOM tests and six real Chrome flows
  passed; the version-change flow types its sentinel only once and verifies
  persistent content after explicit save against the new draft.
- Same Fable reviewer resumed for the confined third check. The actual review
  server and manual mining QA remain off pending the coordinated gallery handoff.

### Review app approval

- Fable APPROVE, review tool only, snapshot
  `68a4d8edd96c775669a9477095a727d3b8c6b80bcd91af80764d7926b0f1c267`.
- Independent probes and the 38/2/6 candidate suites verified note persistence,
  single-writer locking, version conflict/carryover, warnings and reachable zoom.
  This does not approve any product mockup or authorize production UI changes.
- Tool remains frozen for gallery delivery. Designer has the handoff; parent
  will serve port 18481 when the rendered catalog is complete and frozen, before
  accepting notes against that version. Manual mining QA remains stopped.
- Non-blocking follow-ups remain recorded in the tool review (note-path warning
  toast, carryover-label revisit, warning concurrency, reserved `overall` id).
  Current planned mockup IDs do not use the reserved name.

### Draft-1 gallery live

- Designer delivered 14 distinct PNG/HTML screens: O1-O4 operator, M1-M3 miner,
  D1-D3 desktop user, U1-U4 mobile user. Parent verified all 49 manifest files,
  exact catalog dimensions/count and catalog SHA256
  `330b673485ec07fad9d49206966a1d1100308c0df04b5ca9a600d4d7efe67b7f`.
- Mockup manifest: `/tmp/opencode/lab-design-mockups-manifest.json`, SHA256
  `e3cf7004e02f1e109bce91c37d188e5c25bf271902380e78fef5ee62ceb4e414`.
  Assets are preserved under `design/mockups/draft-1/` in the design worktree.
- Approved review app is serving **http://127.0.0.1:18481/**. Parent verified
  gallery/catalog/feedback/Markdown endpoints and rendered the actual gallery
  in headless Chrome. It shows all 14 screens; no mining services were started.
- Persistent agent-readable user feedback:
  `/home/ethan/.local/share/silent-pool/design-review/feedback.json` and
  `feedback.md` (private state directory 0700). Initial state has no notes;
  agents must not insert model-review findings as if they were user feedback.
- GLM and DeepSeek are now independently reviewing the actual frozen PNG/HTML
  draft. Reports will be separate under `docs/design/reviews/` in the design
  worktree. No production UI implementation is authorized by the draft delivery.
- Manual QA remains stopped. Publish revisions as draft-2 or later, preserving
  draft-1 assets and the user's version-bound notes for later Fable/user review.
- GLM's actual-screen review encountered a provider HTTP 500 and was resumed
  once in the same session. No completed GLM design verdict is claimed yet;
  DeepSeek's review remains independent. Gallery service and user notes are
  unaffected, and production UI implementation remains on hold.
- The GLM 5.3 retry also failed with provider HTTP 500. Replaced it with
  `tff/glm-5.3-flash`, session `ses_e48153840ffe66w9slOOOuLB9l`, for the independent
  role/layout review. No further retries of the failed model are scheduled.
  The replacement must report whether findings use pixels or HTML/DOM evidence;
  draft assets, the review app and user feedback remain unchanged.
- GLM 5.3 Flash completed its independent pixel/HTML/CSS review of all 14
  screens: `docs/design/reviews/2026-11-07-glm-flash-draft-1.md` in the design
  worktree. Verdict: bounded REQUEST_CHANGES (two MUST, three SHOULD).
- Findings for draft-2: remove gallery links from product navigation; clarify
  the applicable Ark-registration/forfeit choice; label offboard versus general
  sending coverage; make disclosure targets 44 px and mobile navigation fluid.
  Financial policy wording must match DESIGN/SPEC and the chosen flow.
- Reviewer prefers O1/Direction A for on-call operations and O2 for daily
  accounting, recommending O1 as the single console direction with O2's scoped
  change-log concept. This is model feedback, not the user's design selection.
- Draft-1 remains frozen while the second review and user notes are collected.
  Model reviews are kept separate from user-authored feedback in the app.
- DeepSeek completed its independent rendered-screen review at
  `docs/design/reviews/2026-11-07-deepseek-draft-1.md` in the design worktree.
  Findings include O4's incorrect `pay_P` fee wording, cross-screen permission
  contradictions, enabled authorization for an invalid recipient, app-spendable
  versus cold-signer funds, and mobile action/nav obstruction.
- Parent read the user feedback store at revision 0: no saved notes/statuses.
  Both model reviews are consolidated in
  `docs/design/reviews/2026-11-07-draft-1-synthesis.md`; neither model preference
  is treated as a user design selection.
- Designer is staging draft-2 separately with stable IDs, corrected states and
  mobile layout, preserving both directions and every frozen draft-1 asset.
  The live gallery remains on draft-1 until coordinated publication. No model
  findings are inserted into the user's note store; no production UI is changed.
- Designer's interim return did not include a completed draft-2 packet. Parent
  found generator scripts and styles only, with no rendered screens, catalog,
  change report or manifest. The same designer was resumed to finish those
  concrete deliverables before review; no draft-2 completion is claimed.
- A second return again described future tool work without delivering rendered
  screens. Parent confirmed the old designer was idle and transferred sole
  draft-2 ownership to fresh TFF Kimi session `ses_e472cb032ffeMSKcV5Tk2BMwah`.
  It will finish the existing generators and actual render/validation/manifest
  handoff. Draft-1, the live gallery pointer and user-feedback files stay frozen.

## Browser QA follow-up (2026-11-07)

- Dedicated manual QA run previously started at `http://127.0.0.1:18480`, using the approved
  final-v4 Nix binaries and old L2 node. Two real host-mode pools/miners; private
  run directory `/tmp/opencode/qa-do1exv_g/run`, receipt
  `/tmp/opencode/lab-manual-qa.json`. HTML, run discovery, snapshot and catalog
  returned HTTP 200. It has since been stopped at the user's design-review request.
- This is a user QA session; automated tests must not mutate or stop it. It is
  separate from the network-tool packaging tests and does not claim namespace
  coverage. The receipt records controller identity and cleanup information.
- Browser harness owner: `tff/kimi-k3`, session
  `ses_e48e9f7e5ffeh914Z5ukGuId5v`, isolated `lab-browser-qa` worktree.
  Implement real browser clicks plus state assertions, screenshots and traces
  against disposable owned runs, with a read-only attach/screenshot option.
- No existing Playwright/Puppeteer harness is present; current UI tests exercise
  JavaScript helpers rather than a rendered browser. Browser automation must
  demonstrate an actual browser run and retain failures, not substitute screenshots
  for command/outcome assertions. Fable review precedes integration.

## Nix network-tool completion (2026-11-07)

- User requested agents address the remaining limitation. Existing integrated
  code and approvals remain intact; new implementation is isolated for review.
- Fetch/root-cause owner: `tff/glm-5.3`, session
  `ses_e496106d1ffe8q1En37Obi3EiB`. Diagnose current cache/offline/mirror behavior
  and realize the pinned tool closure or supply a hash-verified narrow remedy.
  Report: `docs/reviews/2026-11-07-nix-network-fetch.md`.
- Packaging/validation owner: `tff/kimi-k3`, session
  `ses_e4960a7b9ffem4e4Izur6q1wjk`, isolated `lab-nix-network-tools` worktree.
  Implement explicit Nix tool outputs/environment and run namespace proofs with
  Nix-resolved userspace tools, preserving core default checks.
- Evidence must distinguish evaluated derivations from realized executables and
  verify no host-tool fallback. Existing host-tool artifacts are not relabelled.
- No persistent Nix trust/config changes, node upgrades, commits or pushes are
  authorized. Any needed pin/source override must be explicit and reviewed.
- Fable reviews the completed implementation and actual Nix-tool proof before
  the limitation is marked closed or changes are integrated into primary.

### Fetch diagnosis and realization handoff

- Fetch agent reports the unchanged pinned tool closure successfully substituted
  from the standard trusted `cache.nixos.org` cache. No source compilation, pin
  changes, persistent Nix configuration edits or weakened verification were needed.
- The prior cache-unavailability conclusion was incorrect: the original
  `--offline` invocation disabled substitution while required paths were absent,
  causing fallback to an unavailable bootstrap/source-build path. Some source
  mirrors still fail, but the trusted binary cache supplies these outputs.
- Evidence: `docs/reviews/2026-11-07-nix-network-fetch.md` and
  `/tmp/opencode/nix-network-fetch-result.json`. Realized package paths,
  derivations, signatures and clean store-only PATH smoke tests are recorded.
- Packaging owner has the handoff and now runs actual namespace validation with
  Nix userspace. Realization alone does not close the functional tooling gate.
- The fetch agent is qualifying the cold/incomplete-store diagnosis and checking
  warm-store offline use; an absolute claim that offline can never work is not
  accepted. Historical approved artifacts remain unchanged.
- Qualification complete: the exact earlier offline command now exits 0 with
  the populated store. Recorded sequence is cold offline failure, successful
  online substitution, then successful warm offline execution.
- Corrected fetch evidence SHA256:
  `53a61a5b3ab2963457fff2a9b73a39a8c814ff30ebca3c0d37e74fd07a965cb4`;
  parent verified it and handed the precise cache/offline semantics to the
  packaging owner. Historical failure logs and approved records are preserved.

## Final delivery

- Primary checkout now contains the reviewed browser/headless runtime, native
  SV2 CPU miner, v2 accounting/persistence, namespace partition/heal support,
  operator documentation and independently approved v0.9.0 validation gate.
- Final runtime approval: full `9ffff6da`, code `30fedafd`; release-validation
  approval: `5473dbc8`. Node v0.9.0+lab-patch source approval: `0e037d35`.
- Primary transfer independently rechecked: 368/370 final-input paths exact
  (README preserves two primary links; merge queue preserved), 10/10 release-
  validation paths exact, frozen input sources intact and staged diff empty.
- Formatting, locked RPC-feature workspace tests/clippy, all six explicit-source
  Nix checks, driver/release self-tests and read-only namespace replay passed.
  The initial offline dependency-cache failure is retained, followed by a
  successful locked registry-enabled run; no source or lockfile drift occurred.
- Integration record: [primary integration](reviews/2026-11-06-lab-primary-integration.md).
  Operator guide: [LAB-RUNTIME.md](LAB-RUNTIME.md). All work is uncommitted;
  no pushes or node-pin/deployment changes were made.
- Historical namespace proofs used explicitly recorded host tools. The follow-up
  above has now realized pinned Nix network tools; packaging and new Nix-userland
  namespace proofs are still pending. Existing evidence scope and node-suite
  limitations remain as recorded, not broadened by source transfer.

## Current execution state

- Namespace worker `ses_e4def8af7ffehCm53JCnmeFu56` completed and froze the
  approved final revision in `lab-final-integration` and completed primary
  transfer/checks. All approved input worktrees and existing main review records
  remain intact; no implementation or validation agent is awaiting another task.
- The chosen host-controller/per-actor namespace architecture below governs
  the resume. Older storage-prepass/re-exec notes are historical, not current
  implementation requirements.
- The user-requested TFF v0.9.0 port and independent live validation are complete
  with bounded Fable approvals. Current node pins remain unchanged. Final runtime
  review and primary integration are complete.

## Resume checkpoint

- User explicitly requested continuation. The existing TFF worker has resumed
  from the preserved storage pre-pass checkpoint; Fable review remains required.
- Worker `ses_e515750bbffeLFOVlZkDkLHdgZ` confirmed PAUSED in
  `lab-network-runtime`: no owned live processes remain, `/run/netns` is empty,
  and other sessions' processes were left untouched.
- Preserve all uncommitted changes, worktrees, approved snapshots and evidence.
- Resume point: namespace runtime wiring and topology T1-T9 fixes, then genuine
  two-pool/two-miner partition/heal proof and final combined Fable review. The
  approved documentation delta `1d40b2b7` has already been handed off.
- Existing component, host, pool-delta, LP-live and documentation approvals
  remain recorded below; the unfinished namespace runtime is not approved.
- Detailed checkpoint: `docs/reviews/2026-11-06-lab-network-runtime-implementation.md`
  in `lab-network-runtime`. T1-T7 helpers and initial namespace wiring are in
  place. Last checked namespace tests: default 4 passed / 8 ignored, opt-in
  12 passed. The last two storage patches have not been compiled.
- Current next step: finish the storage pre-pass
  (`RuntimeSecrets::for_plan` is missing; `prepare_storage_for_join`,
  `external_storage` and engine adoption are uncompiled), then check/build and
  rerun the real topology proof. Latest live attempt hit the PostgreSQL root /
  user-namespace UID mapping boundary. Apply approved docs and regenerate
  source-bound Nix provenance and the final manifest after implementation.
- Resume scope: compile the unfinished storage/ownership wiring, then complete
  the real two-node/two-pool native-SV2 partition/heal run, apply the approved
  documentation delta, run combined checks and freeze a candidate for Fable.
  Prior pause cleanup facts above are historical; new test resources must again
  be cleaned up by their owner.

### Implementation handoff after stalled resume

- The previous MiniMax worker returned architectural deliberation instead of a
  completed revision. It is no longer the implementation owner and will not be
  resumed. Existing uncommitted changes and diagnostics are preserved.
- New sole writer: `tff/kimi-k3`, session `ses_e4def8af7ffehCm53JCnmeFu56`, in
  `lab-network-runtime`.
- Chosen architecture: controller remains in the host user/network namespaces;
  isolated actors enter the broker user namespace and their endpoint network
  namespace at launch. PostgreSQL stays host-local with private Unix sockets;
  Redis runs in each pool's storage namespace over its published address.
- This removes the need to transfer the controller's lock/listener/engine across
  a re-exec. Conditional earlier re-exec guidance is superseded; ownership,
  cleanup, private storage, durable controls and real isolation requirements
  still apply. New code and actual namespace mining evidence require Fable review.

Input: [proposal](REGTEST-LAB.md) and
[checked five-model feedback](reviews/2026-11-05-regtest-lab-review-summary.md).
The user requested implementation by Tokens for Friends (TFF) models, with
Fable reviewing their work and approving it or returning actionable feedback.

## Workflow

1. Each implementer owns a topic worktree and explicit paths/deliverables.
2. Implementers supply tests, actual command outcomes, changed paths, unresolved
   limitations, and a reviewable uncommitted diff. No commits or pushes are
   authorized by this task.
3. Fable reviews the concrete diff, requirements and test evidence. Its result
   is APPROVE or REQUEST_CHANGES, with reproducible findings. Approval is tied
   to the reviewed diff; later material changes require another review.
4. REQUEST_CHANGES returns to the same TFF implementer. After fixes and tests,
   Fable reviews again. Approval is not a substitute for the integration checks.
5. Approved work is integrated with the repository's shared-file/merge protocol;
   cross-packet checks run against the combined result. Source baselines and
   concurrent TP/ladder work are tracked explicitly.

The original review proposal and independent review records remain the evidence
baseline. Subsequent implementation contracts and decision records incorporate
the checked recommendations, including corrections to reviewer claims.

## Active packets

| ID | Model | Worktree / branch | Deliverable | State |
|---|---|---|---|---|
| L1 | `tff/kimi-k3` | `lab-sv2-conformance` | Fix verified SV2 wire/session defects and add independent conformance regressions | Fable APPROVED snapshot `fa532607` |
| L2 | `tff/glm-5.3` | rbitcoin `lab-retarget-profile` | Exposed regtest-only retarget profile, effective-profile introspection, and node tests | Fable CLEARED_WITH_RECORDED_BASELINE_LIMITATIONS `d2f37b91` |
| L3 | `tff/deepseek-v4.1-flash-1017` | `lab-contracts` | Resolve/document work-unit and window design; concrete runner/evidence/topology contracts and implementation packets | Fable APPROVED snapshot `3edcd4e7` |
| LP-1a | `tff/glm-5.3-flash` | `lab-share-math` | Approved pure v2 work arithmetic and independent vectors; full policy wiring follows L1 | Fable APPROVED snapshot `45df0eb3` |
| LP-1 | `tff/kimi-k3` | `lab-share-integration` | Full v2 job/work/window/accounting/persistence integration on approved inputs | Fable APPROVED snapshot `c16032a7` |
| LP-2 | `tff/kimi-k3` | `lab-cpu-miner` | Persistent native SV2 CPU miner, pacing, controls and protocol tests | Fable APPROVED snapshot `5e27e918` |
| LP-3 | `tff/deepseek-v4.1-flash-1017` | `lab-runner` | Headless supervisor, evidence, process adapters and thin browser | Fable APPROVED snapshot `6a97dff0` |
| LP-3 HTTP | `tff/glm-5.3-flash` | `lab-runner` (disjoint coauthor paths) | Bounded HTTP parsing, SSE/browser recovery and correct control payloads | Helper complete; handed to core owner for combined gates |
| LP-4a | `tff/minimax-m3.1-flash` | `lab-link-preflight` | Isolated-link capability probe/backend precursor and real TCP fault tests | Fable APPROVED snapshot `bce93151`; Nix-tool gate blocked |
| LP-live | `tff/kimi-k3` | `lab-live-e2e` | Reusable real rbitcoin/v2-pool/miner/settlement proof | Fable APPROVED snapshot `aa3b37b6` |
| LP-runtime | `tff/deepseek-v4.1-flash-1017` | `lab-runtime` | Combine approved inputs and implement real service/UI runtime | Fable APPROVED bounded host `e891f34c`; code `d75ab391` |
| LP-stale | `tff/kimi-k3` | Read-only approved sources; scratch analysis | Diagnose observed stale-work and job-update delays | Diagnostic complete; bounded pool fixes assigned to runtime owner |
| LP-topology | `tff/minimax-m3.1-flash` | `lab-topology` | Namespace helpers, supervisor launch and F1 hash guard | Fable REQUEST_CHANGES; F1 guard accepted independently |
| LP-network-runtime | `tff/kimi-k3` | `lab-network-runtime` | Complete namespace runtime and real multi-pool SV2 partition/heal | Fable APPROVED full `553cd4e8`, code `2d6c752d` |
| LP-final | `tff/kimi-k3` | `lab-final-integration` | Final N1-N7 merge cleanup and combined validation | Fable APPROVED full `9ffff6da`, code `30fedafd` |
| LP-primary | `tff/kimi-k3` | Primary checkout | Transfer approved final source and release-validation gate, preserving local records | Complete; hashes independently verified and all integration gates passed; uncommitted |
| Nix-net fetch | `tff/glm-5.3` | Read-only primary inputs, scratch realization | Resolve pinned cache/mirror/bootstrap failure | Complete: online realization and warm offline execution verified |
| Nix-net tools | `tff/kimi-k3` | `lab-nix-network-tools` | Nix environment/package and namespace validation | Realized closure handed off; packaging and functional proof running |
| Browser QA | `tff/kimi-k3` | `lab-browser-qa` | Screenshot/click smoke harness with real runtime assertions | Paused before file writes; manual QA stopped for mockups |
| Product mockups | `tff/kimi-k3` | `lab-design-mockups` | Multi-role desktop/mobile mockups and catalog | Draft-2 paused for live-lab pivot; draft-1 and user notes preserved |
| Live Lab hub | `tff/kimi-k3` | `lab-live-hub` | Minimal live role/process directory and current functionality docs | Plan approved by GLM Flash/DeepSeek; implementation running |
| Design review app | `tff/glm-5.3-flash` | `lab-design-mockups` (disjoint paths) | Web gallery with persistent agent-readable notes | Fable APPROVED tool `68a4d8ed`; serving gallery on port 18481 |
| LP-docs | `tff/deepseek-v4.1-flash-1017` | `lab-docs` | Final SPEC/WIRE/OPERATIONS and host lab documentation splice | Fable APPROVED snapshot `1d40b2b7` |
| L2-v0.9 port | `tff/glm-5.3` | `/tmp/opencode/rbitcoin-v0.9-lab` | Port approved lab patch and run node build/regression gates | Fable APPROVED `0e037d35`; bookkeeping corrections complete |
| L2-v0.9 validation | `tff/kimi-k3` | `lab-v09-validation` | Independent profile/mining/retarget/partition compatibility evidence | Fable APPROVED bounded release validation `5473dbc8` |

Implementation sessions:

- L1 author: `ses_e5258c453ffeMqfacHi0ZKs5sT`; approved L1 directory remains
  `/home/ethan/.local/share/opencode/worktree/ad8886/lab-sv2-conformance`.
  The session now continues LP-1 in
  `/home/ethan/.local/share/opencode/worktree/ad8886/lab-share-integration`.
- L2: `ses_e5258c1aeffe7F3DsAIGH3yoMd`, directory
  `/tmp/opencode/rbitcoin-lab-retarget-profile`.
- L3: `ses_e5258bfafffejUargeBZAhECdq`, directory
  `/home/ethan/.local/share/opencode/worktree/ad8886/lab-contracts`.
- LP-1a: `ses_e521764e9ffeMzTDlV9ba0K24a`, directory
  `/home/ethan/.local/share/opencode/worktree/ad8886/lab-share-math`.
- LP-2: `ses_e5218de5bffeiJF7DeD8fIVmAN`, directory
  `/home/ethan/.local/share/opencode/worktree/ad8886/lab-cpu-miner`.
- LP-3 author: `ses_e5218d9f7ffecv5wCYeS2R74dU`; approved runner remains in
  `/home/ethan/.local/share/opencode/worktree/ad8886/lab-runner`. The session now
  continues LP-runtime in
  `/home/ethan/.local/share/opencode/worktree/ad8886/lab-runtime`.
- LP-3 HTTP: `ses_e51363fb7ffebqov2U0qnNuZFX`, same worktree, only
  `http.rs`, static browser assets, HTTP/browser tests and its own report.
- LP-4a: `ses_e515750bbffeLFOVlZkDkLHdgZ`, directory
  `/home/ethan/.local/share/opencode/worktree/ad8886/lab-link-preflight`.
- That author now continues LP-topology in
  `/home/ethan/.local/share/opencode/worktree/ad8886/lab-topology`; the approved
  primitive worktree remains frozen.
- The topology helper snapshot is now frozen too; the author continues actual
  runtime wiring in `/home/ethan/.local/share/opencode/worktree/ad8886/lab-network-runtime`.
- LP-live: `ses_e50f01463ffeNOfEF3HXLvSM8r`, directory
  `/home/ethan/.local/share/opencode/worktree/ad8886/lab-live-e2e`.
- LP-stale diagnostic: miner author `ses_e5218de5bffeiJF7DeD8fIVmAN`;
  approved component sources stay frozen. Report destination:
  `docs/reviews/2026-11-06-lab-stale-work-diagnostic.md`.
- LP-docs: runtime author `ses_e5218d9f7ffecv5wCYeS2R74dU` now works in
  `/home/ethan/.local/share/opencode/worktree/ad8886/lab-docs`; the approved host
  runtime stays frozen in `lab-runtime`.

Reviewer model: `amazon-bedrock/us.anthropic.claude-fable-5-1`. Review sessions
are dispatched when an implementer returns a concrete diff and test evidence.

Silent-pool packet bases: `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.
rbitcoin packet base: `e8cf9c9d61f7c63ae59cc11ab1179cace50ca142`.
Node TDP work already exists separately on `sv2-tp` (`1ec1dd0`); the ladder
implementation is a separate in-flight packet. These are integration inputs,
not files for the first-wave implementers to overwrite.

### L1 ownership

SV2 server/client protocol state and their tests; coinbase Merkle-path producers
and consumers; tightly required pool config/role wiring for stable Noise keys.
The topic worktree may modify its local shared-file copies to test a complete
packet, but supplies explicit shared-file merge notes. It does not decide new
share-work units or accounting-window policy; L3 supplies that design.

### L2 ownership

In the separate rbitcoin worktree: regtest profile composition and CLI/config
surface, effective profile introspection, necessary consensus tests, node/RPC
journeys, and owner documentation/changelog fragment. Existing non-lab profiles
retain their defined behavior. Follow that repository's AGENTS and test-first
rules; no mainnet datadir, schema wipe, environment-knob shortcut, or publish.

### L3 ownership

New lab implementation/design documents and proposed decision/changelog
fragments in its worktree. Decisions require Fable review before code consumes
them. The unit representation and N-share versus network-block-equivalent
window question are distinct decisions; do not encode contradictory semantics
under the same stored unit. The implementation contracts name concrete evidence
states, failure handling, process/link isolation, statistical scope and gates.

## Following work

### User-requested v0.9.0 validation

- Trigger: the current namespace-wiring worker completes its implementation and
  hands back its checkpoint/candidate. Do not start this release-validation wave
  while that worker is still completing the wiring.
- Trigger satisfied by namespace candidate `553cd4e8`. Port session
  `ses_e5258c1aeffe7F3DsAIGH3yoMd` owns the isolated node clone at release
  `9614b27ecd832a04070a5226da26c8844284f2c4`; validation session
  `ses_e50f01463ffeNOfEF3HXLvSM8r` owns `lab-v09-validation`. Both are TFF.
- Use TFF agents for the validation work. Target upstream release commit
  `9614b27ecd832a04070a5226da26c8844284f2c4` plus the local L2 patch currently
  approved at snapshot `d2f37b91` on base `e8cf9c9d`.
- Dispatch isolated patch-port/build validation and independent compatibility /
  regression work. Prepare independent checks in parallel; live checks of the
  patched node depend on the new frozen candidate and binary provenance.
- Cover CLI/effective-profile parity, retarget boundaries and real mining,
  multi-node partition/heal, and the previously flaky node tests. Compare with
  the preserved baseline; distinguish fixes observed in tests from hypotheses.
- Keep current approved node/source and run artifacts untouched. No stock-binary
  replacement, dependency-pin bump, commit or push is implied by validation.
- Source audit so far: stock v0.9.0 lacks our retarget/profile extensions;
  Cargo.lock/flake.lock and schema 26 match the old base. Actual patch application
  and runtime compatibility remain to be tested.

Approved L3, L1 and LP-1a snapshots now feed the full LP-1 integration packet in a
new worktree. The original approved worktrees remain frozen. Miner protocol tests
and runner fixtures proceed independently; live non-block-share and end-to-end
claims wait for the combined approved packets. Topology faults and economic
scenarios follow the runner and node-profile gates. Every packet uses the same
TFF implementation -> Fable feedback/approval loop.

### Integration still required for the user's live lab

Component approval is not completion of the live lab. After the runner revision
is approved, the next serial integration packet must:

- Combine approved pool, arithmetic/policy, miner, runner and script inputs in
  an integration worktree; retain the original approved snapshots and reconcile
  shared Cargo/flake/config/SPEC/OPERATIONS changes explicitly.
- Render complete private real pool/miner configurations, initialize actual
  Redis/Postgres resources and wire the live pool/node/miner evidence collectors.
  A fixture launcher or a private authority fragment alone is insufficient.
- Connect the browser's add-miner and other live controls to the same durable
  command path, not just display fixture state or expose pause/budget controls.
- Integrate namespace placement/address publication and control/RPC access with
  the approved link primitives, accounting for the runner's executable/control
  lifecycle and the backend's parent-death-only model.
- Run approved-binary real-node v2 and retarget proofs plus multi-pool/node
  partition/heal scenarios, and retain honest capability failures for unsupported
  TDP/ladder/witness/Ark features rather than fabricating results.
- Complete combined tracked-source Nix checks; keep the network-tool realization
  blocker explicit if the required external sources remain unavailable. The
  rbitcoin quiet-window check is complete and Fable has cleared lab integration
  with the recorded baseline limitations.

No commits or pushes are authorized by this task.

## Review records and integration evidence

A dispatched or compiling packet is not done.

### LP-final - merge revision first review

- Full snapshot: `6271865396b5312616ddef5448f75845f5ca8fc218abfd39db05e4dcd2fde9a2`;
  code: `37acb9aa5eb3ea23d2525dc4921ccec8d23e90e43b53ca6809bcee93af938bd1`.
  Manifest `/tmp/opencode/LP-final-runtime-manifest.json`; parent verified all
  370 source hashes/modes, code/full aggregates and manifest-file hash.
- Candidate addresses structured link errors/current unknown state, private
  auth-file helper credentials, strict inputs/status parsing, typed topology
  evidence and explicit bounded convergence, with operator/provenance updates.
- Implementer reports all six default Nix checks, 14 opt-in namespace tests,
  five convergence tests and supporting typed/privacy/HTTP/SSE checks passed.
- Source-bound native run `/tmp/opencode/LP-final-native-v3` reports four
  assertions passed, 72/72 consistent records, 34 dropped packets, 276 no-bypass
  checks with no violations and matching final paused tips at height 44. Parent
  hashes and the pre-partition anchor are retained; all actors stopped cleanly.
- Same Fable reviewer `ses_e512efa05ffeZbiKXB9sLD6n3A` is checking the narrow
  delta and evidence at `docs/reviews/2026-11-06-lab-final-integration-fable-review.md`.
- Approved namespace inputs remain frozen. This candidate uses the old L2 node;
  v0.9 gate corrections and any adoption remain separate.

### LP-final - focused review outcome

- [Fable final review](reviews/2026-11-06-lab-final-integration-fable-review.md):
  REQUEST_CHANGES on code `37acb9aa` / full `62718653`, confined to C1/C2.
- C1: a tie-break resume leaves `quiesced` true, allowing convergence to be
  certified while miners run. Reset it after resume and require another
  successful pause plus fresh atomic views; add the reproduced control-order
  regression (`pause`, `resume`, `pause`).
- C2: remove the lab-only convergence entry from SPEC frontmatter, retaining
  its operational decision/changelog and the approved documentation policy.
- N1-N4/N6-N7 and recorded live/Nix evidence were verified. The live run did not
  take C1's problematic path, so no new mining run is required for these fixes.
- Same TFF author is revising the candidate and recording actual regression
  results before a confined Fable re-review. Approved inputs stay frozen.

### LP-final - corrected candidate

- Full snapshot: `9ffff6dae933fad03ec0884092da1d97fcc65a571b5b573a43e67d4432a62465`;
  code: `30fedafdbba57b79fe4c13e2de0658aa3f05f527b778a242cbfab9c4ba253d76`.
  Parent verified all 370 records, full/code aggregates and manifest-file hash.
- C1 resets quiescence before attempted resume. The reproduced counterexample
  now requires pause/resume/pause and consumes a fourth fresh observation.
  C2 removes only the lab-only SPEC frontmatter entry.
- Implementer reports six convergence tests, 29 library tests/default targets,
  formatting/clippy/docs, four Nix packages and all six default checks passed.
- Corrected source was built and checked, not used for a new live run. The
  previous native-v3 evidence remains explicitly tied to its older binary and
  verified execution path. Fable is reviewing the confined correction on hash.

### LP-final - final merge approval and primary transfer

- Fable APPROVE, final bounded merge revision, code
  `30fedafdbba57b79fe4c13e2de0658aa3f05f527b778a242cbfab9c4ba253d76`
  and full `9ffff6dae933fad03ec0884092da1d97fcc65a571b5b573a43e67d4432a62465`.
- C1's pause/resume/pause plus fresh-view regression and C2's approved SPEC
  restoration verified. All six Nix checks and corrected source/binary binding
  verified; historical native-v3 evidence remains bound to its own binary.
- Same TFF author is assigned serial primary-checkout integration of this exact
  approved source plus the separately approved v0.9 validation gate `5473dbc8`.
- Existing primary README links, progress/merge boards and Fable review histories
  must be preserved. No node pin adoption, commit or push is authorized. Any
  production-file mismatch or new semantic change must stop the transfer for
  investigation rather than inherit the reviewed snapshot's approval.

### L2-v0.9 - independent live validation review

- Validation-gate snapshot:
  `bb8e800932a457fefb17b780f55ff4f7337c423ce6a38b8c2be740890773ac7b`,
  eight files, manifest `/tmp/opencode/lab-v09-validation-manifest.json`.
  Parent verified every source hash, aggregate and manifest-file hash.
- TFF reports real native-SV2 baseline work, canonical acceptance, fee-zero
  settlement/scanner recovery, all 73 retarget headers and two-pool namespaced
  partition/heal passed on node `bcb6d5ee` / source `0e037d35`.
- Retarget boundaries were `1e7fffff`, `1e608887`, `1e5ae6f6`; each matched the
  independent oracle. Namespace final paused tips matched at height 47, with
  71/71 consistent records and 276 no-bypass probes without violations.
- Candidate CLI/RPC checks passed, including seven negative cases. The profile
  stage remains explicitly `incomplete` because no unpatched stock binary was
  supplied. A TFF worker is attempting a bounded official-asset retrieval and
  checksum verification for a supplemental comparison, without changing source.
- Fable reviewer `ses_e5057f85affebQdtmo6uWTJfRc` reviews the gate and raw
  evidence at `docs/reviews/2026-11-06-lab-v09-validation-fable-review.md`.
- Source/port approval is already recorded; independent live approval, stock
  comparison and any final adoption decision remain separate. Current pins and
  approved old binaries are unchanged.

### L2-v0.9 - port candidate and independent validation handoff

- Candidate base: official v0.9.0
  `9614b27ecd832a04070a5226da26c8844284f2c4`; source snapshot
  `0e037d358c057d382adcca8d6c094bcf4a46eb1e02758777d20f5d70eb4ddcda`.
  Parent verified HEAD, all 19 file hashes and aggregate in
  `/tmp/opencode/lab-v09-source-manifest.json`.
- Old L2 delta applied cleanly; upstream release changes retained. Cargo.lock
  additionally records workspace self-version updates from 0.8.99 to 0.9.0;
  no third-party dependency or flake input change is claimed.
- Binary `/tmp/opencode/rbitcoin-v0.9-lab/target/dev/debug/rbitcoin-node`, SHA256
  `bcb6d5eefe0dd0bd96e2ac79e173b4b99a91f67f7717dae6620c30786378a171`,
  independently re-hashed by parent and handed to the TFF live validator.
- Targeted node/consensus/RPC/profile and static gates reportedly pass. Broader
  tests remain mixed: standalone test suite 93/4, later workspace test crate
  97/0 with a net-suite `tip_accept` failure. Shared load is disclosed; no
  all-green, quiet comparison or proven upstream flake fix is claimed.
- Fable port reviewer `ses_e5223ce14ffeYm7aoPv0MNREcB` reviews source and node
  gates at `docs/reviews/2026-11-06-lab-v09-port-fable-review.md`; independent
  TFF live compatibility/mining/retarget/topology validation runs separately.
- Existing node pins, approved old sources and historical artifacts remain
  untouched. New-source approval and live acceptance are still pending.

### L2-v0.9 - source/port/node approval

- [Fable port review](reviews/2026-11-06-lab-v09-port-fable-review.md): APPROVE
  for source, port fidelity and node gate, snapshot
  `0e037d358c057d382adcca8d6c094bcf4a46eb1e02758777d20f5d70eb4ddcda`,
  binary `bcb6d5eefe0dd0bd96e2ac79e173b4b99a91f67f7717dae6620c30786378a171`.
- Reviewer verified exact approved-hunk preservation against upstream changes,
  profile semantics and all targeted gate evidence; ast-grep was independently
  run with 13 self-tests passed and scan exit 0.
- Required bookkeeping only: manifest provenance says 17 workspace version
  changes but the actual count is 16; add the reviewer-run ast-grep gate to the
  report. These edits must not change the source aggregate or binary hash.
- Suite-green remains unestablished: no quiet A/B or clean-v0.9 baseline, and
  recorded shared-load failures remain a CI merge-readiness limitation.
- Independent live validation still gates adoption. The validator has this
  source approval and the separately approved namespace runtime; current pins
  and old approved binaries remain unchanged.
- Both bookkeeping corrections are complete: 16 self-version entries in the
  manifest provenance, and the reviewer-executed ast-grep result in the mutable
  report. New manifest-file SHA256:
  `a1abd17039dd952f54074f5a7b75a5f57d1355dd95bc5836281414d9e6b0cc92`.
  Parent verified this metadata hash and the unchanged approved source aggregate;
  the live validator received the updated provenance reference.

### L2-v0.9 - stock artifact comparison handoff

- Official stock executable:
  `/tmp/opencode/v09-stock-bin/rbitcoin-node-x86_64-linux`, SHA256
  `16f47f80f0f69bd0dfbcb75e0d67a6b77f9e2af62d2df25c3dab0182e69279f7`.
- TFF verified downloaded bytes against the release API digest and published
  checksum, and the tag chain to release commit `9614b27e`. This attests the
  official artifact bytes, not a locally reproduced source build.
- Stock version/help/parser probes confirm version 0.9.0 and refusal of the
  lab retarget flag before creating resources. Provenance is recorded in
  `/tmp/opencode/lab-v09-stock-binary.json`.
- Independent validator is running the frozen profile gate with this stock
  executable in a new run directory/input file. Original incomplete-profile
  artifacts and the eight-file gate under Fable review remain unchanged.

### L2-v0.9 - stock comparison supplement complete

- Frozen validation gate `bb8e8009` produced `release_checks_passed` at
  `/tmp/opencode/LP-v09-live-profile-stock-r1/public/summary.json`, SHA256
  `a96fa067da9428d733a096616ea25bac14df6ba870a7d4364b889a5d4b05ef24`.
- Stock CLI lacks/refuses the retarget flag; stock default RPC has no consensus
  object and its height-1 GBT bits are `207fffff`. Patched default/retarget
  profiles and all seven negative cases passed the same frozen gate.
- Parent verified summary, new input, provenance and structured comparison
  hashes. The former incomplete profile artifact, previous inputs and all
  source/report files remain unchanged; this is a later supplemental run.
- Supplemental evidence is with Fable. All four functional stages now have
  passing TFF results; final independent review and adoption are still pending.

### L2-v0.9 - live evidence review outcome

- [Fable validation review](reviews/2026-11-06-lab-v09-validation-fable-review.md):
  REQUEST_CHANGES on gate `bb8e8009`, confined to two fail-open predicates.
- All four recorded live stages independently verified: baseline share/window/
  payout/settlement/scanner evidence, 73 retarget headers, stock profile comparison,
  and a real same-ancestor namespace fork followed by a seven-block reorg.
- G1: require divergent hashes at a common height and phase-scoped branch
  displacement; a lagging node on the same chain must not pass as a fork.
- G2: each APPROVE pass must bind one unambiguous full snapshot hash consistent
  with its verdict; incidental historical hashes cannot also become approved.
- Same TFF validator is correcting these with negative fixtures and raw-artifact
  replay, plus small chronology/fenced-verdict/stock-refusal improvements. No
  re-mining is required and existing live artifacts remain preserved.
- Final integration author received the fork-semantics handoff for typed phase
  evidence. Source/port approval remains valid; reusable live-gate approval and
  adoption remain pending.

### L2-v0.9 - revised live gate

- Snapshot: `5473dbc851b01b731f05810af792d810e002e5a004a760a342ae070dcf3f9da0`,
  ten files; parent verified all source hashes, aggregate and manifest-file hash.
- G1 now derives node-specific parent-linked chains and phase-scoped
  displacement, with a redacted structural fixture and explicit lag/early-reorg
  negatives. G2 requires one unambiguous full approval hash per review pass.
- Implementer reports 15 Nix self-tests, contract/static/input checks, raw
  namespace replay and a fresh non-mining stock/profile supplement passed.
  Original mining runs and approved production inputs remain unchanged.
- Same Fable reviewer resumed to check G1/G2, negative fixtures and the replay
  against previously verified four-stage evidence. No re-mining was performed;
  reusable gate approval remains pending.

### L2-v0.9 - final bounded validation approval

- [Fable validation review](reviews/2026-11-06-lab-v09-validation-fable-review.md):
  APPROVE for gate snapshot
  `5473dbc851b01b731f05810af792d810e002e5a004a760a342ae070dcf3f9da0`.
- Reviewer independently reran 15 self-tests, exact approval/input checks,
  negative fork/approval fixtures and byte-identical parent-linked namespace
  replay. G1/G2 and supporting nits are resolved; earlier raw four-stage
  verification remains valid.
- Validated combination: node source
  `0e037d358c057d382adcca8d6c094bcf4a46eb1e02758777d20f5d70eb4ddcda`
  on release `9614b27e`, node binary `bcb6d5ee`, and the explicitly recorded
  approved pool/miner/namespace-runtime inputs. Source/port/node approval and
  independent functional validation are both complete.
- This is bounded lab validation, not production deployment, broad node-suite
  green, quiet A/B stability or general interoperability/convergence approval.
  No pin/adoption change has been made. The N1-N7 final runtime merge review is
  separate and remains pending.

### LP-network-runtime - complete candidate, first combined review

- Full snapshot: `553cd4e8bb40d34be514a4ebd55d466904e1357f174b2b184b5dcf8724aa5692`;
  tested code: `2d6c752d24d4f4cc9ea30f6d5ced8b507dfd860acb8721ded85c758992705098`.
  Manifest `/tmp/opencode/LP-network-runtime-manifest.json`; parent verified
  all 364 file hashes/modes, full and code aggregates, and manifest hash.
- Candidate implements host-controller/per-actor user+net namespace launch,
  private host PostgreSQL sockets, authenticated per-pool Redis namespaces,
  bounded control/RPC, parent-death cleanup, T1-T9 fixes and integrated docs/F1.
- Implementer reports all six default Nix checks passed, 13 host namespace tests
  passed and 11 runtime unit tests passed. Network tools remain host tools;
  pinned Nix network-tool realization remains an external blocker.
- Genuine Nix-built two-node/two-pool/native-miner proof reports 37 accounted
  records per front, 45 dropped packets, 276 no-bypass checks with no violations,
  and matching paused canonical tips at height 45. Separate controller SIGKILL
  proof reports no actor/holder/PostgreSQL survivors before reap.
- Evidence: `/tmp/opencode/LP-network-kimi-final-live-v2-evidence.json` and
  `/tmp/opencode/LP-network-kimi-final-crash-v1-evidence.json`; binary provenance
  `/tmp/opencode/LP-network-kimi-nix-binary-provenance.json`.
- Fable reviewer `ses_e512efa05ffeZbiKXB9sLD6n3A` is reviewing the combined
  candidate at `docs/reviews/2026-11-06-lab-network-runtime-fable-review.md`.
  These reported results are not yet approval of the changed runtime.

### LP-network-runtime - final bounded approval

- [Fable review](reviews/2026-11-06-lab-network-runtime-fable-review.md):
  APPROVE for code `2d6c752d24d4f4cc9ea30f6d5ced8b507dfd860acb8721ded85c758992705098`
  and full `553cd4e8bb40d34be514a4ebd55d466904e1357f174b2b184b5dcf8724aa5692`.
- Reviewer independently reproduced runtime-driven native mining on both pools,
  a same-height fork, sustained packet drops, no-bypass checks, heal and confirmed
  branch displacement, with all four assertions passed and 78/78 consistent work
  records. No RPC generation or mock time was used.
- Independent SIGKILL proof included broker, holders, actors and 20 PostgreSQL
  processes: all gone in 511 ms, then successful lock acquisition by reap.
- Approval retains explicit scope: old L2 node, consensus-link fault, nearly all
  shares block-qualified, host network tools, and network isolation rather than
  a filesystem/security sandbox. Existing LP-live covers non-block work.
- Non-blocking merge notes N1-N7 are assigned to the same Kimi author in new
  `lab-final-integration`: structured error/current-state handling, helper auth
  and input parsing, typed topology evidence, explicit convergence phase, and
  accurate boundary/docs provenance. The approved namespace tree stays frozen.
- The v0.9 validator has this approved runtime handoff and continues on its exact
  frozen binary pair, independently of final merge cleanup.

### LP-docs - first review

- Snapshot: `ff6909cd9a99e6bc57377e18394a84887a7f514e6a5b39d7eadb4c62fd885570`,
  30 documentation files, manifest `/tmp/opencode/lab-docs-review-manifest.json`.
  Parent verified all file hashes and the canonical `base`/`files` aggregate.
- Candidate consolidates approved versioned work/window/scrub/wire/type-32 and
  Noise behavior into SPEC/WIRE/OPERATIONS, with a host-runtime operator guide.
  Tool-only changelog records remain outside normative SPEC frontmatter.
- Implementer reports valid frontmatter, relative-link checks with three explicit
  main-only inputs, punctuation checks and CLI examples checked against approved
  help/API. No code was edited or binaries rebuilt.
- Fable reviewer: `ses_e51699a1bffeNWVQks9CcWT7Db`; report destination
  `docs/reviews/2026-11-06-lab-docs-fable-review.md`.
- Namespace operation remains a pending guide handoff. Network-runtime author
  has the candidate paths and must preserve the reviewed doc delta at final
  integration rather than overwrite it with older host copies.

### LP-docs - first review outcome

- [Fable review](reviews/2026-11-06-lab-docs-fable-review.md): REQUEST_CHANGES,
  confined to D1-D3. Core normative consolidation, operator CLI, provenance and
  declared capability limits were verified against approved code and decisions.
- D1: storage bootstrap can export identity-free v2 blobs; the v2 front's refusal
  to populate/serve type-32 does not imply an empty bootstrap blob list.
- D2: disclose the malformed-hash consistency-check limitation in the approved
  host, with F1 accepted separately but still pending integration.
- D3: add the required superseded-in-part note to the historical share-target
  decision, preserving its v1 semantics and original body.
- Same author is applying these and small wording/date corrections in the
  docs-only worktree; no implementation behavior or frozen input is changed.

### LP-docs - second candidate

- Snapshot: `1d40b2b705dc59afd2f3496011acc6c4c948c33aa6f2e6d4dab8be334d40d9c0`,
  32 documentation files; parent verified every hash and aggregate.
- Candidate corrects bootstrap producer/consumer wording, explicitly scopes the
  approved host's malformed-hash limitation, and adds the historical decision's
  superseded-in-part banner without changing its original body.
- Supporting date/permissions/pow-limit wording and a persistent baseline/merge
  hash record are included. No implementation or build file changed.
- Same Fable reviewer resumed for the confined second pass. Namespace operator
  instructions remain pending the actual working integration.

### LP-docs - final approval

- Fable APPROVE, documentation integration packet, snapshot
  `1d40b2b705dc59afd2f3496011acc6c4c948c33aa6f2e6d4dab8be334d40d9c0`.
- D1-D3, wording/date nits and baseline hash record verified; approved normative
  semantics and historical v1 decision body remain unchanged. Link exceptions
  are exactly the three declared main-only inputs.
- Approved owned-document delta handed to the network-runtime implementer for
  final integration, preserving frozen source docs and recording input hashes.
- Namespace-specific operator instructions and merged F1 status must follow the
  actual implementation and its combined review. Existing LP-live canonical-v2
  evidence remains approved; the new namespaced runtime needs its own proof.

### LP-runtime - combined host candidate

- Snapshot: `c9dfa986c2f6ad0daca44912906a0a2ec94129c25d465eee246f19a43ccd543d`,
  172 files, manifest `/tmp/opencode/lab-runtime-review-manifest.json`.
  Parent verified every file and aggregate; recipe includes the full annotated
  file records under `base`/`files`.
- Candidate renders real private service configs, provisions persistent
  identities, runs isolated pool storage, collects live evidence and supplies
  browser add-miner/switch/budget controls through durable commands.
- Pool changes isolate the blocking Redis reader and deliver new jobs during
  pending share outcomes without changing the deadline or credited work.
- Implementer reports 604 workspace tests, real Redis and SV2 regressions,
  UI/oracle tests and combined Nix checks passed. Provisional two-pool live8
  reached height 159; observed tip-to-job p50/p95 was 13/16 ms and 55 of 663
  submissions were stale. This is not a causal budget-response experiment.
- Fable runtime reviewer: `ses_e5159da4effeMVHqYQgWkl1Z9u`, report
  `docs/reviews/2026-11-06-lab-runtime-fable-review.md`.
- Fable pool-delta reviewer: `ses_e5211f6f5ffecy91VOFutC6Nur`, report
  `docs/reviews/2026-11-06-lab-runtime-pool-fable-review.md`.
- Candidate remains frozen while reviewed. LP-topology works independently in
  a new tree; pending runtime fixes must be reconciled before combined approval.

### LP-runtime - pool delta approval

- [Fable pool review](reviews/2026-11-06-lab-runtime-pool-fable-review.md):
  APPROVE for the six-file pool/store delta against LP-1 `c16032a7`, tied to
  combined snapshot `c9dfa986c2f6ad0daca44912906a0a2ec94129c25d465eee246f19a43ccd543d`.
- Independent real-Redis tests confirm a separate blocking-reader connection,
  prompt publication and preserved pending/ACK semantics. SV2 tests confirm
  new-job delivery before an old outcome and exactly one original-credit reply.
- Delivery changes preserve policy and durability ordering; no new normative
  decision is needed. This approval excludes runtime/config/UI/collectors/Nix.
- Remaining review notes include correcting unsupported fail-first wording,
  considering production-reader and deadline-under-updates regressions, and
  explicitly retaining pre-existing unknown-outcome evidence gaps.
- The current on-disk pool binary differs from live8's recorded binary hash;
  provenance correspondence is assigned to the host/runtime reviewer. Historical
  recorded hashes are not to be replaced with a later build's hash.
- Candidate stays frozen pending the host/runtime verdict and coordinated rework.

### LP-runtime - host review outcome

- [Fable host review](reviews/2026-11-06-lab-runtime-fable-review.md):
  REQUEST_CHANGES, focused R1-R4. Pool/store delta approval stands.
- Reviewer independently ran two real pools, exercised add/retry/conflict,
  switch, pause/resume/budget and clean stop, and verified distinct ledgers,
  rendered config/privacy, build evidence and foundation lifecycle behavior.
- R1: relabel the limited target self-consistency check and join accounted work
  to its front/job/generation; do not claim full header/credit verification.
- R2: replace fixture-grade scenario assertions with real work/evidence checks,
  including work on each required front and the scoped consistency result.
- R3: rerun the two-pool proof with Nix-built binaries and explicit store/source
  provenance; correct stale hashes/status tables, preserve clock-labelled
  latency analysis and remove unsupported fail-first claims.
- R4: distinguish tip sampling, tip height and actual canonical coverage; do not
  retain an unqualified current-canonical claim after a tip is displaced.
- Runtime owner is revising these and bounded capability/identity/observer/lock
  fixes. Topology author has the affected-interface handoff; topic trees remain
  separate and final combined approval requires reconciling the runtime changes.

### LP-runtime - second host candidate

- Full snapshot: `e891f34c69c70c2a6748be1ca2a77a71aba4a63377eda55a43afeb8613b68043`,
  174 files. Parent verified all file hashes and the full annotated aggregate.
- Tested code snapshot:
  `d75ab391b0ca1c8ab5b03af27dd5cf45a8a384ef8ac056a1ceeb2b2f456d8684`,
  frozen before build/run in `/tmp/opencode/lab-runtime-final-src`.
- R1 adds scoped announced-job/target/credit consistency with explicit limits;
  R2 asserts real evidence and accounted work on each required front; R3 records
  Nix output/store/derivation provenance and qualified clock-based analysis;
  R4 labels tip sampling and rechecks eight observed heights for displacement.
- Supporting fixes cover capability claims, identity validation, blocking metrics
  requests, observer shutdown, scanner retry coverage and SQL identifiers.
- Implementer reports 609 workspace tests and targeted/UI/oracle/Nix checks
  passed. New two-pool run `/tmp/opencode/lab-runtime-runs/packet2/` used Nix-built
  runner/pool/miner/scanner, with the approved external L2 node explicitly
  identified. Both fronts' assertions passed and all ten process records ended
  terminal; the reported consistency check covered 810/810 records without gaps.
- Same Fable host reviewer resumed. The pool/store delta is unchanged; its
  bounded approval stands. Topology author received the new interface handoff
  and must reconcile these changes before combined approval.

### LP-runtime - final bounded host approval

- Fable APPROVE for full snapshot
  `e891f34c69c70c2a6748be1ca2a77a71aba4a63377eda55a43afeb8613b68043`
  and tested code
  `d75ab391b0ca1c8ab5b03af27dd5cf45a8a384ef8ac056a1ceeb2b2f456d8684`.
- Reviewer independently validated the code-subset recipe, source copy, Nix
  derivation/output paths and binary hashes, packet2 evidence and R1-R4 fixes.
  The earlier bounded pool/store approval remains unchanged.
- Required merge fix F1: malformed non-hex or non-32-byte `share_hash` must not
  bypass the hash comparison and count as consistent. Assigned to the topology
  integration copy with negative tests; the approved host tree remains frozen.
- Namespace/control transport, causal hash-budget response, full header/window
  payout verification and fee/witness/SV1/Ark coverage remain outside this
  approval. Final combined integration and documentation checks still apply.
- Runtime author now handles final documentation in `lab-docs`, separately from
  topology changes, for a Fable documentation review before the final merge.

### LP-topology - runtime integration dispatch

- Implement actual namespace placement, published addresses, controller RPC and
  miner duplex control while preserving verified executable/process ownership.
- Required real proof: multiple nodes and independent pools, genuine native SV2
  work, sustained partition/reconnect/heal, and source-scoped canonical views.
- Parent-death-only backend lifetime, PostgreSQL user-namespace restrictions,
  IPv6 routing and unavailable pinned Nix tools remain explicit constraints.
- No edits to the runtime candidate or approved primitives are authorized from
  this topic tree. Fable review and final serial integration remain required.

### LP-topology - partial delivery and completion handoff

- Frozen helper snapshot:
  `374241a3edd5ccf13c02b37fb9fa626baf58831744bc200362ebfa77c9adf61d`.
  Parent verified 359 file hashes/modes and aggregate (`base` plus full `files`).
  The inventory includes a `.git` worktree pointer, which must be excluded from
  future source manifests and never copied into another worktree.
- Delivered: broker/placement/observer APIs, namespace supervisor exec handshake,
  netns backend, and malformed-share-hash F1 fix with negative tests. A real
  two-node partition/heal probe reports split/rejoin observations and cleanup.
- Pool/front/miner/storage placement and runtime RPC/stdio wiring are absent.
  This is a precursor, not completion of the assigned native mining topology.
- Fable reviewer `ses_e512efa05ffeZbiKXB9sLD6n3A` reviews the frozen delivered
  delta and evidence in `docs/reviews/2026-11-06-lab-topology-fable-review.md`.
- Same TFF author continues the missing runtime wiring in `lab-network-runtime`
  using approved host `e891f34c` and the candidate helpers, excluding Git metadata.
  Required proof remains two pools/miners doing real SV2 work on partitioned
  nodes, then heal, with node-scoped canonical and accounted-work evidence.
- Helper review findings must be incorporated into that new integration tree;
  no approval is inherited by the unfinished runtime or its new binaries.

### LP-topology - helper review outcome

- [Fable review](reviews/2026-11-06-lab-topology-fable-review.md):
  REQUEST_CHANGES on `374241a3`. F1 malformed-hash guard is independently
  accepted; approved host and LP-4a broker behavior are not reopened.
- Reproduced defects: scenario link IDs renamed, stale Rust broker startup
  returning success, stale/error-collapsed link state, symlink/exit/reaping
  failures in exec handshake, and false displacement from non-atomic tip reads.
- Default Nix workspace check is red because new namespace tests unconditionally
  depend on host tools. Explicit capability-gated proofs and a green core check
  are required. Missing host-address enumeration must fail, not pass vacuously.
- Storage connectivity remains unresolved: another namespace's loopback is not
  reachable, and current Redis code has no Unix-socket support. The integration
  must implement and test its chosen transport rather than describe a future one.
- The node-only proof used RPC `generate` and shell `nsenter`, not native SV2 or
  the Rust exec handshake. Packet-drop counters do not count dropped blocks.
- T1-T9 fixes and report corrections are assigned to `lab-network-runtime`
  alongside the already-required full runtime wiring and real mining proof.

### LP-live - first review

- Source snapshot: `afb48ddc4127757866a25093a48d77fd61dcae94016f42295e610d8631309599`,
  five deliverables, manifest `/tmp/opencode/lab-live-review-manifest.json`.
  Generated Python bytecode was removed/excluded, not treated as source.
- Candidate provides reusable real-node v2 and retarget drivers plus an independent
  stdlib oracle. Baseline reports nine assertions passed, Q65/work68, confirmed
  selected block and two recovered outputs. Retarget reports all eleven passed,
  real mining through height 72 and independently checked boundaries 24/48/72.
- No mock time or generated setup blocks are claimed in the retarget run;
  requested versus achieved hash rates and pre-ladder payout scope are explicit.
- Artifacts: `/tmp/opencode/LP-live-baseline-verified` and
  `/tmp/opencode/LP-live-retarget-run6`.
- Fable reviewer: `ses_e5057f85affebQdtmo6uWTJfRc`; record
  `docs/reviews/2026-11-06-lab-live-fable-review.md` when returned.
- Review begins source-only while the controlled L2 stability check runs; heavy
  artifact validation/live reruns wait for quiet-window release.
- Quiet window has now been released; Fable resumed full evidence/oracle review.

### LP-live - first review outcome

- [Fable review](reviews/2026-11-06-lab-live-fable-review.md): REQUEST_CHANGES,
  bounded, snapshot `afb48ddc4127757866a25093a48d77fd61dcae94016f42295e610d8631309599`.
- Independently verified all 83 settled blocks, window/high-water selection,
  finding-slice exclusion, every sealed credit's accounted evidence, payouts,
  scanner proofs and all 73 retarget headers. No premature settlement was found;
  minimum observed settlement depth was six. Independent baseline rerun passed.
- B1: approval checks accept rejected historical hashes when an unrelated later
  APPROVE appears in the same report. Exact verdict/snapshot binding is required.
- B2: node retarget arithmetic is verified, but the data does not establish that
  hash-budget changes caused the observed block-time changes. About 73% of
  submissions were stale and about 97% of block-qualified solutions did not
  become canonical blocks. Phase measurements and qualified claims are required.
- Same implementer is revising B1/B2 plus bounded provenance, metadata, oracle
  and job-ID fixes. Frozen live artifacts remain valid and will be preserved;
  derived analyses must identify their source artifacts explicitly.
- A separate read-only diagnostic investigates the stale-work path before any
  performance change is assigned to the runtime integration. No specific delay
  mechanism or successful hash-budget response is assumed from these counts.

### LP-live - second candidate

- Snapshot: `a3e2117be3fb26487242a0f0e6b14f79c271c5d39b0565d84ce528f996dcb79d`;
  parent verified all five source hashes, aggregate and manifest file hash.
- Candidate binds approval checks to review-pass snapshots, removes unsupported
  causal framing, and fixes provenance/metadata, compact encoding and job-ID
  parsing. Original live artifacts remain unchanged.
- New historical analyses in `/tmp/opencode/LP-live-r2-derived-final/` record
  input hashes, phase duty cycles and retarget/payout replay results. They are
  derived evidence, not new live runs from the revised driver.
- Implementer reports 15 Nix self-tests, both scenario validators, frozen-source
  preflight, historical replay, pyflakes and documentation checks passed.
- Same Fable reviewer resumed to verify B1/B2 and helper changes against the
  existing independently verified live evidence. Approval remains pending.

### LP-live - second review outcome

- Fable verified B1, B2 and supporting items 3-8 resolved, including the actual
  approval records, synthetic negative cases and byte-identical derived replay.
- Remaining C1: canonical phase counts are hard-coded and confuse block counts
  with retarget timestamp intervals. Reviewer-reference counts also remain
  embedded in the harness/report instead of being derived from run records.
- Same implementer is replacing those literals with node-record/canonical-header
  joins and profile-derived interval counts, updating the report and producing
  new derived artifacts while retaining the original and second-pass evidence.
- No new live run is required; a confined third Fable pass will check the count
  derivation, regression tests and updated artifact hashes.

### LP-live - third candidate

- Snapshot: `aa3b37b67960071ec6bd4f09c7cd0ac83e1b526404231994c42dc331dc44ded2`;
  parent verified all five source hashes, aggregate, manifest and both derived
  artifact hashes.
- Canonical counts now join submission headers to observed node tips and parent
  chains, with explicit duplicate/rejected/unknown classifications. Height-based
  block counts (24/24/24 plus one tail block) are separate from each full period's
  23 timestamp intervals and the wall-time miner phase counts.
- New analyses: `/tmp/opencode/LP-live-r3-derived-final-v2/`. Original live and
  second-pass artifacts remain preserved; reviewer-reference literals removed.
- Implementer reports 17 Nix self-tests, both validators, retarget preflight,
  derived replay/payout checks and static checks passed. Prefix consistency and
  supporting streaming/profile fixes are included.
- Same Fable reviewer resumed for the confined third pass; approval pending.

### LP-live - final approval

- Fable APPROVE, bounded LP-live, snapshot
  `aa3b37b67960071ec6bd4f09c7cd0ac83e1b526404231994c42dc331dc44ded2`.
- C1 is resolved through canonical-header joins and profile-derived periods.
  Reviewer reran 17 self-tests, both scenario validators and the historical
  derivation, obtaining byte-identical outputs with original artifacts unchanged.
- Earlier independent verification stands: all 83 settled blocks, 73 retarget
  headers, exact payouts, settlement depth, scanner proofs and a baseline rerun.
- Approved scope is the reusable regtest harness/oracle on the named approved
  inputs and the recorded real-work evidence. It does not establish causal
  budget-to-block-pace response, live fee-term coverage, ladder/witness/Ark,
  type-32 or general interoperability.
- Runtime owner receives the frozen script/oracle for integration. Its changed
  pool paths and stale-work fixes need new tests and Fable review; historical
  approvals cannot be inherited by materially changed binaries.
- Merge notes: remove the cosmetic dead return only in the integration copy;
  consider a checked-in redacted fixture for the run6-dependent self-test.

### LP-stale - diagnostic and runtime handoff

- [Diagnostic](reviews/2026-11-06-lab-stale-work-diagnostic.md) identifies shared
  Redis-client serialization: blocking `XREADGROUP` and inline `XADD` publication
  use one physical connection. A fresh Redis probe reproduced a 641 ms publish
  delay, compared with 0.58 ms on an independent publisher connection.
- Recorded canonical-tip-to-first-activation delay was 3.591 s median and
  11.011 s p95; exact job-issue-to-activation was 4 ms median. The separate
  per-channel share-outcome wait also prevents timely job delivery.
- All 2,529 block-qualified submissions were locally active when sent: 73 became
  canonical, 1,846 received stale verdicts and 610 remained transport-unknown.
  These findings do not establish a validated hash-budget response.
- Runtime owner is assigned a dedicated Redis reader connection and job delivery
  during pending outcomes, preserving stream durability, per-job credit and the
  original timeout deadline. Actual Redis and independent SV2 wire regressions
  plus a bounded real-runtime replay are required before performance claims.
- Approved component trees and original live artifacts remain frozen. The new
  pool changes require Fable review as part of runtime integration.

### LP-4a - isolation primitive first review

- Snapshot: `d004a6df21dbd7dea12713f0148dfa26b577e3f20f7f4d33eca8e54af27dc5c6`,
  ten files, manifest `/tmp/opencode/lab-lp4a-review-manifest.json`. This manifest
  includes executable mode as well as path/content hashes.
- Candidate provides unprivileged namespace capability checks, PID-handle/veth/
  nftables/netem primitives, scoped command API and teardown, plus TCP fixtures.
- Implementer reports repeated 12/12 namespace suites, sustained reconnect
  failures during faults, unreachable non-link/host addresses, unrelated-link
  survival, successful heal and cleanup. Relay fallback is explicitly weaker;
  its test demonstrates direct bypass and does not claim isolation.
- Fable reviewer: `ses_e512efa05ffeZbiKXB9sLD6n3A`.
- Review record: `docs/reviews/2026-11-05-lab-lp4a-fable-review.md` when returned.
- Full node P2P/Ark wiring, mining/reorg scenarios and runner address integration
  remain LP-4 work; primitive TCP proofs are not substituted for them.

### LP-4a - first review outcome

- [Fable review](reviews/2026-11-05-lab-lp4a-fable-review.md): REQUEST_CHANGES.
- Verified scoped namespace operations, cleanup (including manager death),
  established TCP interruption and honest weaker relay guarantees.
- Reproduced blockers: IPv6 bypasses IPv4-only partition rules; duplicate `up`
  damages an existing backend; repeated partition has a transient fail-open
  window and heal can falsely succeed; preflight mixes backend-specific claims.
- MiniMax is fixing those and bounded control-plane, spawn, error-reporting,
  lifetime-limit and input/output hygiene findings before re-review.

### LP-4a - revised candidate

- Recipe hash: `a981352732f2312967866a5d7a5469167f0131e84606785a11a165b7bb8c4ef4`,
  12 files. Manifest uses normalized Git executable modes (100644/100755);
  parent verified content hashes, modes and aggregate.
- Candidate adds dual-family interface drops/forwarding checks, exclusive
  ownership before startup, atomic nft updates, checked cleanup/evidence,
  bounded concurrent control and a persistent spawner thread, plus limits and
  scoped capability reporting.
- Implementer reports 22 namespace tests and separate relay tests passed,
  including the direct-bypass negative proof for the weaker relay mode.
- Same Fable session resumed for live counterexample rechecks and code review.
  Pinned Nix tool-environment validation remains a declared integration gate.

### LP-4a - second review outcome

- Fable verified all original blockers fixed, including IPv6 partitioning,
  duplicate-live-owner refusal, atomic reapply and control responsiveness.
- Remaining bounded issues: stale-directory `up` can read old success state;
  `leave-for-adoption` advertises behavior not implemented under parent-death
  cleanup; an unexpected IPv6 route escapes the route audit.
- MiniMax is revising those and related bounded error/reporting cleanup for a
  third pass. Unsupported survival/adoption is to be rejected or clearly absent.

### LP-4a - third candidate (2026-11-06)

- Recipe hash: `bce931514318110699a5aeb3ae0bdebd8d9204cc7e07b429b5961b9e9977036e`;
  parent verified all 12 hashes, executable modes and aggregate.
- Candidate refuses stale-directory CLI reuse, requires a live new-token
  handshake, rejects unsupported actor-survival/adoption, and detects unowned
  IPv6 routes. It also tightens output/error handling and control admission.
- Implementer reports 28 namespace tests and separate relay negative tests passed.
- Same Fable session resumed for a confined third review.
- Locked nixpkgs resolves offline, but required tool realization needs unreachable
  mirrors/bootstrap sources. Host-tool results are not relabeled as Nix results;
  the network-tool Nix integration gate remains externally blocked.

### LP-4a - final approval

- Fable APPROVE, bounded precursor, recipe hash
  `bce931514318110699a5aeb3ae0bdebd8d9204cc7e07b429b5961b9e9977036e`.
- Live/stale ownership, unsupported-adoption refusal, dual-family route checks,
  atomic reapply, concurrent control and cleanup independently rechecked.
- Published bounds remain explicit: parent-death cleanup only, no survival of
  backend death, tested IP guarantees (not ARP), no full node/Ark/mining proof.
- Runtime address placement and namespace integration must be implemented before
  full topology scenarios claim those guarantees. Nix tool realization remains
  an external integration gate; approved evidence is host-tool evidence.

### LP-3 - runner foundation first review

- Snapshot: `0f2a49c4c2becb9941aebf004977c4e3662d560854abf91cac2273226ace517a`,
  53 files, manifest `/tmp/opencode/lab-lp3-review-manifest.json`.
- Candidate includes strict scenario types, durable commands, evidence/oracle,
  supervision/adoption, isolated resources/keys, adapters, HTTP/SSE and thin UI.
- Implementer reports 52 tests plus two explicit real-process proofs, workspace
  tests, fmt/clippy, Nix package/check builds passed. The real-process proofs
  cover baseline node startup/profile/shutdown and control-only miner operation,
  not a full mining/payout/retarget scenario.
- Fable reviewer: `ses_e5159da4effeMVHqYQgWkl1Z9u`.
- Review record: `docs/reviews/2026-11-05-lab-lp3-fable-review.md` when returned.
- LP-4a starts independently on capability/network primitives; it does not adopt
  unapproved runner code or claim integrated node partitions. It must distinguish
  OS-enforced no-bypass isolation from any weaker declared relay fallback.

### LP-3 - first review outcome and coordinated rework

- [Fable review](reviews/2026-11-05-lab-lp3-fable-review.md): REQUEST_CHANGES.
- High-priority failures reproduced: owned exited children mislabeled unverified,
  stop commands succeeding while processes remain alive, and interrupted resume
  commands falsely reconciled from default memory. Manifest synchronization,
  persistent entity state and serialized command effects also need correction.
- Core owner handles lifecycle/journal/locking, snapshot/evidence semantics,
  private-file permissions, missing-capability checks and related cleanup.
- HTTP coauthor handles B8/B10 in disjoint paths: bounded header reads,
  Last-Event-ID/reconnect behavior, generic event handling and correct miner/worker
  control IDs. Shared Cargo/flake and core API changes stay with the core owner.
- Final combined checks and Fable re-review wait for both coauthors to freeze
  their files. No partial rework is marked approved.

### LP-3 - HTTP helper handoff

- B8/B10 helper is complete: bounded pre-allocation header reads, cursor validation,
  Last-Event-ID, generic ordinary SSE `message` events, browser reconnect/dedupe,
  and correct miner/worker control payloads.
- Report: `docs/reviews/2026-11-05-lab-lp3-http-implementation.md` in `lab-runner`.
- Helper reports ten Rust HTTP tests and ten Node helper tests passed; core
  in-flight compile/lint results were kept separate.
- Core owner now owns final integration/formatting and combined gates. It was
  asked to add the Node tests to the supported Nix path and preserve the explicit
  SSE wire change in the final report.
- Driver `dropped_before` is cumulative: runner loss accounting must use per-
  incarnation deltas, not mark every subsequent event as another loss.

### LP-3 - combined second candidate (2026-11-06)

- Snapshot: `9bec018d34273f0a7e783cfc36e1e8646b9c032ea915cce32464168888842f85`,
  60 files, manifest `/tmp/opencode/lab-lp3-r2-review-manifest.json`.
- Parent verified all file hashes and canonical aggregate.
- Candidate addresses lifecycle/reconciliation, serialized effects, record sync,
  snapshot barriers, private permissions, scoped evidence loss, kernel locks,
  readiness ownership and canonical vocabulary, plus the frozen HTTP/browser work.
- Implementer reports 71 library/integration tests plus two explicit real-process
  proofs, 500 workspace tests, ten Node tests, Nix package/checks and run-once
  durable start/stop passed. These remain foundation/process proofs, not a full
  v2 mining or retarget scenario through this runner.
- Same Fable reviewer resumed with the original failure probes and checks on
  locking, snapshot semantics, port/listener ownership and unchanged integration
  boundaries. Real pool config rendering and namespace placement remain follow-up.

### LP-3 - second review outcome

- Fable verified B1-B18 fixed, tests/build evidence consistent, original false-
  success/recovery/header/permissions counterexamples no longer reproducing.
- New D1: shutdown publishes evidence while a supervisor mutex guard is retained
  by a loop expression; snapshots take evidence then supervisor locks. The
  resulting AB/BA deadlock reproduced in four of eight probe runs.
- DeepSeek is dropping guards before publication and adding concurrent snapshot/
  stop coverage, with bounded journal-order/subscriber-resync cleanup. No broader
  runtime integration is mixed into this approval revision.

### LP-3 - third candidate

- Snapshot: `6a97dff090e9cea1b37258fc3e9c944cd5d9df1afd0f9689a896bfc769fe3290`,
  63 files, manifest `/tmp/opencode/lab-lp3-r3-review-manifest.json`.
- Parent verified every file hash and aggregate.
- D1 guard lifetime fixed; new concurrent snapshot/stop test, sequence-based
  replay, explicit subscriber resync and interruptible observation wait added.
- Implementer reports 75 core tests, 504 workspace tests, ten Node tests, Nix
  package/checks and run-once/real-process proofs passed.
- Same Fable session resumed for the confined third review. Real runtime and
  namespace integration are still the following work, not claimed by this packet.

### LP-3 - final foundation approval

- Fable APPROVE, bounded foundation, snapshot
  `6a97dff090e9cea1b37258fc3e9c944cd5d9df1afd0f9689a896bfc769fe3290`.
- Original deadlock probe now passed ten of ten runs with terminal processes;
  sequence replay, subscriber resync, interruptible wait, locks and Nix evidence
  independently checked. Original approved worktree remains frozen.
- LP-runtime now combines the approved components and supplies the real pool
  configurations, live collectors and browser add-miner workflow. Heavy checks
  are deferred until the L2 quiet-window gate completes.

### LP-1 - full integration first review

- Snapshot: `81fa17d27ee80641167aaeb53e97f627ba8daa4688e56a227a1ef44c9776b329`,
  56 files (28 unchanged approved inputs, 10 modified, 18 added).
- Manifest: `/tmp/opencode/lab-lp1-review-manifest.json`; hash uses compact
  sorted-key JSON of `base` and path/sha256 pairs, excluding change annotations.
  Parent verified all 56 hashes and aggregate.
- Candidate includes checked v2 arithmetic adoption, strict regtest profile,
  immutable job/generation credits, Q-based settlement, SQL migration 0005,
  definite wire/profile metadata, restore checks and evidence/outcome handling.
- Implementer reports 483 workspace tests, native main-process m=2 non-block/
  candidate share checks, four conformance tests, 20+1 real Postgres tests,
  legacy SV1 node smoke, fmt/clippy/doc checks passed; no checks still running.
- The main-process v2 test uses a mock template source. It proves real SV2/PoW/
  booked-credit plumbing, not v2 full-node canonical acceptance or settlement.
- Fable reviewer: `ses_e51699a1bffeNWVQks9CcWT7Db`.
- Review record: `docs/reviews/2026-11-05-lab-lp1-fable-review.md` when returned.
- LP-2/LP-3 received the candidate's exact config/evidence/API handoff to prepare
  adapters; adoption and full-node claims remain gated on approval/integration.

### LP-1 - first review outcome

- [Fable review](reviews/2026-11-05-lab-lp1-fable-review.md): REQUEST_CHANGES.
- Verified correct through source inspection and independent probes: core v2
  work/window arithmetic, immutable generation credits, frozen settlement,
  migration/wire/restore paths, legacy behavior and SPc/Ark evidence separation.
- Blocking F1: type-32 remains served at ingress despite v2 capability omission;
  wire requests must be refused and v2 slices excluded from the type-32 store.
- Blocking F2: a failed refresh can leave an old job live after slice/height
  mutation; fail atomically or invalidate jobs before more work can be credited.
- Same Kimi session resumed for F1/F2 and bounded remaining cleanup: actual
  pow-limit validation, checked payout-side restoration, complete byte tables,
  fingerprint-shape checks, stronger evidence fields and exact metric tests.
- Cross-packet follow-up: the base front maps `retain_windows` to a fixed slice
  count (default 16). A minority pool's N-network-block-work window may need many
  more slices. LP-1 was asked to verify this path, add a regression and define
  v2 retention/cap behavior or record a concrete integration blocker; persistent
  retention-induced underfill must not be called full N-block coverage.

### LP-1 - revised candidate

- Snapshot: `69ac72b6ed7ab514f1870cfc55832d8c8d202576a9c01bc27aaeb72d2a20da6d`;
  parent verified all 57 file hashes and aggregate.
- Candidate resolves F1/F2 and F3-F9 cleanup. It also adds bounded, work-aware
  v2 retention (4096 slices and 4096 network heights of durable credits), with
  40-slice/Q65 roll and bootstrap regressions; cap exhaustion remains underfill.
- Implementer reports 489 workspace tests, 226 targeted tests, 20+1 PostgreSQL
  tests, legacy node smoke, fmt/clippy/docs and all five Nix checks passed.
- Same Fable session resumed, explicitly reviewing retention/scrub disclosure,
  version scoping and real evidence alongside the original guard fixes.
- Full-node v2 canonical acceptance and TDP/ladder integration remain open.

### LP-1 - second review outcome

- Fable verified all previous F1-F10 fixes, including fail-closed refresh and
  the initial minority-pool retention regression. Two amendment issues remain.
- R2-1: scrub SQL must use each slice row's work version with separate v1/v2
  horizons; current payout mode must neither erase v2 early nor extend v1 privacy
  retention. A mixed-row PostgreSQL test is required.
- R2-2: retaining only current-Q work discards history before a harder retarget.
  Kimi is changing retention to preserve eligible headroom under the explicit
  cap/age horizon and adding retarget/roll/bootstrap boundary regressions.
- Same implementer is revising for a confined third Fable pass.

### LP-1 - third candidate

- Snapshot: `c16032a78902e88b0157efed6780bc8b424e942b1c3bf46893d8fb09ae4708ee`;
  parent verified all 57 hashes and aggregate.
- Scrub now uses each row's version with separate strict horizons. V2 live
  history is Q-independent within its 4096-slice/age bounds, preserving headroom
  for harder retargets. Decision/changelog disclose the v2-only retention change.
- Implementer reports 494 workspace tests, 230 targeted tests, 20+2 PostgreSQL
  tests, legacy smoke, fmt/clippy/docs and all five Nix checks passed.
- Same Fable session resumed for confined R2-1/R2-2 review; LP-live remains
  gated on this approval for its final real-node run.

### LP-1 - final approval

- Fable APPROVE, bounded LP-1, snapshot
  `c16032a78902e88b0157efed6780bc8b424e942b1c3bf46893d8fb09ae4708ee`.
- Mixed-row SQL horizons and retarget headroom verified through independent
  probes and test evidence. RAM's age cutoff is one block stricter than SQL,
  so it cannot use already-pruned credit rows; exact wording is a doc nit.
- Approved sources/config/evidence handed to LP-live and LP-3. Full-node v2,
  multi-pool/retarget, TDP/ladder and combined source integration remain gates.

### LP-2 - first review

- Snapshot: `af5313704b81ee20868d634dc19127c97687a8d89b541e00c9d8d105a120e635`,
  27 files, manifest `/tmp/opencode/lab-lp2-review-manifest.json`.
- New `spc-cpu-miner` implements native SV2, pinned Noise, real hashing/budgets,
  per-job targets, N connections/workers, JSONL commands/events and fixtures.
- Implementer reports 24 targeted tests, 453 workspace tests, fmt/clippy passed.
- Actual approved L1 pool `main.rs` + Bitcoin Core smoke: 62 real submissions,
  correlated ACK, height 101 -> 102, node tip matched a local candidate, independent
  Python SHA256d/target verification, clean miner exit. This is legacy-policy
  native SV2 proof, not non-block-share/v2-credit proof.
- Evidence directory: `/tmp/opencode/LP2-main-integration-xptd5vo2/`.
- Full Nix package build remains unresolved because the new package is untracked
  in Git-filtered flake source; the report does not claim that build passed.
- Fable reviewer: `ses_e519daeceffeqeYF6DE2BPE64Q`.
- Review record: `docs/reviews/2026-11-05-lab-lp2-fable-review.md` when returned.
- LP-3 received the exact `PROTOCOL.md`/fixtures handoff, including process-local
  cursors/idempotency and EOF shutdown semantics relevant to runner recovery.

### LP-2 - first review outcome

- [Fable review](reviews/2026-11-05-lab-lp2-fable-review.md): REQUEST_CHANGES.
- Four independently reproduced blockers: historical pacing debt plus
  uninterruptible sleep after lowering rate; rejection of legitimate same-tip
  job timestamps; incomplete batched-ACK resolution and unbounded pending map;
  duplicate activation events/worker notifications on non-activation messages.
- Same Kimi session resumed for bounded fixes and regression tests, followed by
  Fable re-review of a new snapshot.
- Fable independently validated the native L1/Core smoke evidence and built the
  Nix package successfully from a filtered path source. The earlier missing-
  untracked-source error is not a packet defect. Final combined tracked-source
  build/vendor validation remains required at integration.

### LP-2 - revised candidate

- Snapshot: `bcb9b34280fb8d21b0f5d10d07af1a18a4e4de8cc8e148bc6c519d21d24849bb`;
  parent verified all 27 file hashes and aggregate.
- B1-B4 addressed in code and regressions; protocol adds explicit
  `submissions_evicted` evidence and clarifies pacing/ACK/counter semantics.
- Implementer reports 28 targeted tests, 457 workspace tests, fmt/clippy and a
  fresh native L1/Core smoke passed. New artifacts:
  `/tmp/opencode/LP2-main-integration-j1mitlwz/`.
- Same Fable session resumed to verify fixes, including ambiguity/backpressure
  behavior; candidate remains frozen pending verdict.

### LP-2 - second review outcome

- Fable verified pacing/stop, same-tip time handling and activation fixes; revised
  Nix path build and native legacy-policy smoke also passed independent checks.
- Remaining B3b: a batch count that does not uniquely identify pending shares
  still guesses an accepted subset. Presentation-history trimming also must not
  report still-tracked requests as transport-unknown.
- Same Kimi session is applying a bounded ambiguity/history fix and regressions
  for a third Fable pass; other resolved areas remain stable.

### LP-2 - third candidate

- Snapshot: `6e2eb2aa6537187bc78fb9ba822f0a564f895c4a3e8be26d85fc12ed3b146676`;
  all 27 file hashes and aggregate verified by the parent.
- Inconsistent batch membership no longer guesses ACKs; presentation trimming
  is distinct from unresolved transport state; bounded late verdicts are explicit.
- Implementer reports 28 miner/457 workspace tests, fmt/clippy and another native
  L1/Core smoke passed; artifacts `/tmp/opencode/LP2-main-integration-8qrnqxj6/`.
- Same Fable session resumed. It was also asked to check concurrent event-ID
  publication against the source-cursor contract before assuming ID gaps mean loss.

### LP-2 - third review outcome

- Fable verified B3b, late verdicts and history handling resolved; prior pacing,
  timestamp and activation fixes remain good. Tests, Nix path build and smoke
  evidence were independently checked again.
- New B5 is reproduced: eight concurrent publishers delivered 80,000 events with
  zero drops but 22,209 adjacent ID inversions. Atomic ID allocation alone did
  not serialize publication, so apparent cursor gaps could be false losses.
- Kimi is adding shared bounded publication ordering and concurrent no-drop/drop
  regressions. Cumulative drop counters and non-blocking consumer behavior remain
  part of the protocol contract; another confined Fable pass follows.

### LP-2 - fourth candidate

- Snapshot: `5e27e9180e765e8049cbcccbcadc38b24c85d402505108f8f20c320f73961250`;
  parent verified all 27 file hashes and aggregate.
- Publication ordering now serializes ID/drop accounting and queue admission;
  no consumer I/O is performed under the producer lock. Queue-full remains a drop.
- Implementer reports concurrent ordering/drop regressions, 30 miner tests,
  459 workspace tests, fmt and clippy passed; protocol fields are unchanged.
- Same Fable session resumed for a confined B5 fourth review.

### LP-2 - final approval

- Fable APPROVE, bounded LP-2, snapshot
  `5e27e9180e765e8049cbcccbcadc38b24c85d402505108f8f20c320f73961250`.
- Reviewer reran 30 tests, fmt/clippy and the independent concurrent probe:
  zero inversions, exact correspondence between 303 real drops and ID gaps,
  monotone cumulative drop counts and no duplicate IDs.
- All prior runtime fixes remain verified; hashing/protocol/tracker source is
  unchanged from the verified native legacy-policy node smoke.
- Approved protocol/binary handed to LP-3 and LP-1. LP-live will add the actual
  v2 node/payout proof once LP-1 is approved, with honest policy/capability labels.
- Full multi-pool/retarget/partition and combined tracked-source package gates
  remain separate from this bounded miner approval.

### L1 - first review

- Candidate: 26 files, snapshot
  `c3bad8d05a224398c20871980a5b3a9943b6661167c07af5bfc0ecbea0c22a1d`.
- Manifest: `/tmp/opencode/lab-l1-review-manifest.json`.
- Implementer reports 442 passing workspace tests, three independent upstream
  SV2 conformance tests, two agent client tests, formatting/clippy/check and a
  real bitcoind/SV1 regtest smoke pass. Native SV2/rbitcoin/non-block-share proof
  remains a later integration gate; the smoke run is not substituted for it.
- Code covers wire/job/target/Merkle fixes, negotiated extranonce constraints,
  front header revalidation, stable miner-facing Noise keys and correlated SV2
  outcomes. Latest-job-only staleness and SV1 ingress-only ACKs are explicit.
- Fable reviewer: `ses_e5211f6f5ffecy91VOFutC6Nur`, checking both normative and
  key/privacy correctness against the exact diff and test evidence.
- Review record: `docs/reviews/2026-11-05-lab-l1-fable-review.md` when returned.
- No L1 integration or full LP-1 policy wiring until approval.

### L1 - first review outcome

- [Fable review](reviews/2026-11-05-lab-l1-fable-review.md): REQUEST_CHANGES.
- Evidence checked and focused tests independently rerun successfully by Fable.
- R1: remove ordinary `curtime` ticks from job identity/publication to avoid
  one-second latest-only stale-job churn.
- R2: same-prevhash `nbits` changes require future-job plus SetNewPrevHash
  activation; a nonfuture job alone cannot convey updated network bits.
- R3: SV1 `clean_jobs` must stay true when all older jobs are rejected.
- Same Kimi session is revising code, independent conformance cases, decision
  and fragment; no share-unit/window scope added.

### L1 - revised candidate

- Snapshot: `fa532607fd20ce8f9dabcae1f552973e3c45c12b99ab7be1eb2f148e36056cd2`;
  parent verified all 26 files and aggregate.
- R1-R3 implemented, including preserving the complete issued job on ordinary
  timestamp-only refresh, `(prev_hash, nbits)` activation, and SV1 clean flags.
- Also added secret Debug redaction and a two-transaction upstream-channel wire
  test validating an actual share.
- Implementer reports 444 workspace tests passed, four conformance tests, all
  agent tests, fmt/clippy and the SV1 regtest smoke passed.
- Same Fable session resumed for second review; code frozen pending verdict.

### L1 - final approval and LP-1 handoff

- Fable APPROVE, bounded L1, snapshot
  `fa532607fd20ce8f9dabcae1f552973e3c45c12b99ab7be1eb2f148e36056cd2`.
- R1-R3 and N1/N10 verified in actual code/tests; reviewer independently reran
  focused pool/agent/conformance tests, formatting and clippy successfully.
- LP-1 continues in a new worktree using byte-verified approved L1, LP-1a and L3
  inputs. It owns v2 wiring, checked accounting adoption, job evidence, schema/
  wire/restore handling and relevant acknowledgement integration gaps.
- LP-2 and LP-3 received the approved pool's protocol/configuration handoff;
  native SV2 node-main and non-block-share proofs remain integration gates.

### LP-1a - pure arithmetic review

- Snapshot: `45df0eb346787828e4b58a2fec229e1fce82ded72d1730ce011e82463696379c`.
- Manifest: `/tmp/opencode/lab-lp1a-review-manifest.json`, five files including
  regression seeds; only existing tracked source change is the minimal re-export.
- Implementer reports 59 accounting tests passed (24 new, 35 legacy), fmt/clippy
  passed. Full v2 aggregation/persistence adoption is explicitly not claimed.
- Fable reviewer: `ses_e52005d39ffe1cTotCquiwCgcl`, reviewing arithmetic, public
  API validation, independent oracle/property tests and Nix-path verification.
- Review record: `docs/reviews/2026-11-05-lab-lp1a-fable-review.md` when returned.

### LP-1a - final approval

- [Fable review](reviews/2026-11-05-lab-lp1a-fable-review.md): APPROVE, bounded
  pure module, snapshot `45df0eb346787828e4b58a2fec229e1fce82ded72d1730ce011e82463696379c`.
- Reviewer reran Nix accounting tests (59 passed), formatting and clippy.
- Independent Python/public-API scratch checks covered 110,903 arithmetic vectors
  plus 20,000 target ordering/bit-length cases, in debug and release, with no
  mismatches. Candidate remained unchanged.
- Full LP-1 integration still must adopt checked arithmetic in every v2 path,
  capture job/generation evidence, persist units/rule/Q and reject incompatible
  restores. Finding-slice exclusion and immutable settlement inputs remain gates.
- Module is ready for integration after L1's remaining SV2 fixes are approved.

### L2 - first review

- Candidate introduces `--retarget-regtest` / `retarget_regtest=1`: 10-second
  spacing, 24-block adjustment interval, minimum-difficulty exceptions off,
  explicit first-period initialization and wrap-safe retarget target ceiling.
- `getblockchaininfo.consensus` exposes effective parameters for harness preflight.
- Implementer reports targeted CLI, consensus and RPC tests plus a real-node TCP
  RPC retarget journey passed. That journey uses node generation/mock time; it is
  not the eventual native SV2 CPU-mining end-to-end proof.
- Full gate outcomes are pending and must be recorded before approval.
- Source snapshot: `646656a84cbb5001f96559d98511499eaba35501d93038dc9560cd787bc764ff`.
- Manifest: `/tmp/opencode/lab-l2-source-review-manifest.json` (17 code/test/docs
  files; mutable implementation report and generated bytecode excluded).
- Fable reviewer: `ses_e5223ce14ffeYm7aoPv0MNREcB`.
- Review record: `docs/reviews/2026-11-05-lab-l2-fable-review.md` when returned.
- Status: source review and existing broad test suites ongoing; no integration.
- Gate update: formatting, ratchets, affected-crate clippy and consensus/node/RPC
  suites passed. Initial net suite failed a timing-sensitive case (isolated retry
  passed); initial integration suite had six failures. Corrected supported-layout
  broad runs are pending. Failure causes are not considered resolved solely from
  absent source diffs or isolated passes; Fable has this evidence distinction.

### L2 - first review outcome

- [Fable review](reviews/2026-11-05-lab-l2-fable-review.md): REQUEST_CHANGES.
- Code blocker: remove a redundant post-first-period overlay guard whose only
  reachable incompatible-history path can feed the unsafe large target into
  retarget arithmetic.
- Required regression: accepted slow periods through boundaries 48 and 72 must
  exercise the easier direction and the actual retarget-floor clamp; prove the
  test detects removing the floor assignment.
- Broad gates remain unresolved, including initial suite failures and an external
  registry-query failure for cargo-deny. Exact final exit codes and corrected-
  layout suite passes or baseline reproductions are required before approval.
- Parent reproduced the original aggregate hash and all 17 file hashes. The
  recipe is SHA256 of UTF-8 compact, sorted-key JSON containing exactly `base`
  and `files`; this recipe was sent to the implementer for the revised manifest.
- Same GLM session resumed with the bounded fixes, documentation/call-order nits,
  and requirements to tie tests to the revised source snapshot.

### L2 - revised code, final gate still pending

- Implementer reports guard removed and regression added through slow boundaries
  48/72. Compact rounding yields `0x1e7ffffc` at 48 and the floor `0x1e7fffff`
  at 72; deleting the floor assignment makes the 72 assertion fail, then restored
  code passes. Documentation and call-order suggestions were addressed.
- Reported revised passes: fmt; consensus 318, node 116, RPC 212, real-node
  journey 1; ratchets; full clippy; ast-grep; cargo-deny; NixOS module evaluation.
- Correct-layout full workspace test is still running, followed by the bench
  library gate and final source manifest. Fable re-review waits for that result.

### L2 - completed evidence and second review

- Final code snapshot: `d2f37b9113e8855245866289ed393043aed6e6153e8101375072cbb23624d499`,
  18 files, parent verified all hashes and aggregate.
- Code fixes, targeted suites, real-node journey, bench, clippy, ratchets,
  ast-grep, cargo-deny and NixOS evaluation completed with passing exit codes.
- Full suites remain red with documented baseline failures and a candidate/base
  split for `sv2_tp_bootstrap`; these are not relabeled as all-green.
- Fable APPROVE for the profile code after independently examining the split
  and code paths. The timing/layout attribution remains a hypothesis.
- **Pre-integration stability gate:** in a quiet test window, run
  `cargo test -p rbitcoin-test --test suite` three times each on candidate and
  clean base, same documented silo/environment. Candidate failures >=2/3 with
  base 0/3 require root-cause resolution before integration. Keep actual failure
  sites and logs. Do not substitute this code approval for that check.
- Fresh-datadir use is a documented profile contract, not an implemented
  open-time rejection or supported migration of default-regtest history.

### L2 - controlled stability check dispatched

- Same GLM session is executing the six planned package-suite trials, candidate
  and base three times each with identical documented environments and no retry-
  until-green. Sources stay frozen; no implementation changes are authorized.
- Other work is source-only during the reserved window. The result will record
  every failure site and apply Fable's threshold without relabeling red suites.
- Report destination: `docs/reviews/2026-11-06-lab-l2-stability.md`; machine
  results: `/tmp/opencode/lab-l2-stability-results.json`.

### L2 - stability results

- All six planned serial trials completed; no source drift. The named
  `sv2_tp_bootstrap` case passed all six, so the specified bootstrap trigger was
  not met.
- Candidate suites still exited 101 in all three runs: one `end_of_ibd_work_fork`
  failure and two `mocktime_generate_keeps_ponging_peer` failures. Base suites
  passed all three. These raw outcomes are retained, not called all-green.
- Both tests had prior base failures under full-workspace load; the current
  split's cause remains unproven. Fable is adjudicating whether a specific fix
  is needed before integration.
- [Stability report](reviews/2026-11-06-lab-l2-stability.md) and machine-readable
  results contain commands, source hashes, environment and all failure sites.
- Quiet window released: runtime validation and live-evidence review may proceed.

### L2 - final integration clearance

- Fable judgment: **CLEARED_WITH_RECORDED_BASELINE_LIMITATIONS**, snapshot
  `d2f37b9113e8855245866289ed393043aed6e6153e8101375072cbb23624d499`.
- The specified bootstrap trigger was not met: candidate and base each passed
  `sv2_tp_bootstrap` 3/3. That concern is closed.
- Candidate suites still had one other timing failure each while base suites
  passed. Fable examined the relevant paths and found no specific profile-code
  mechanism requiring a hold; these results remain limitations, not all-green.
- The additional three journey-skipping trials are recommended diagnostics,
  not required for lab use. No further mandatory node check is outstanding.
- Decision and rationale: [L2 review](reviews/2026-11-05-lab-l2-fable-review.md),
  final adjudication; [stability report](reviews/2026-11-06-lab-l2-stability.md).
- Runtime integration and live-evidence verification may consume this cleared
  profile. Final combined runtime and namespace approvals remain separate gates.

### L3 - first review

- Candidate: seven new files in `lab-contracts`; implementation contract, two
  proposed decisions, two changelog fragments, implementation report and doc checker.
- Proposed decisions: exact reference-target work units and a separately versioned
  network-block-work window. These are not adopted pending Fable review.
- Implementer validation: doc checker and whitespace checks passed; no Rust changes.
- Review snapshot: `6a2a5967113417e6ed8c348f11a6498c98e24685b21f2548f0d34b62b0f935f0`.
- Manifest: `/tmp/opencode/lab-l3-review-manifest.json`.
- Fable reviewer: `ses_e524e6989ffelCFmkVUllSKJla`.
- Review record: [Fable L3 review](reviews/2026-11-05-lab-l3-fable-review.md).
- Verdict: REQUEST_CHANGES; no candidate files integrated.
- Required changes: reference family covers the node pow limit and defines
  multiplier step-down; accounting bounds/checks include fee-weighted sums and
  payout products; compact-target decode matches consensus; job metadata supplies
  independent per-share credit verification.
- Same TFF session resumed for fixes and should-fix clarifications. Fable will
  re-review a newly hashed snapshot. L1 also received the verified upstream
  SetTarget future-job/active-job semantics to align its implementation.

### L3 - revised candidate

- Revised snapshot: `a53e9a29611df3d79aac01d12272b853a18998441f943ba764c7c014bbeeff2f`.
- Parent independently verified all seven file hashes and the manifest hash.
- Implementer addressed B1-B4 and items 5-13; doc checker now includes independent
  integer/Fraction, compact-target and fingerprint vectors.
- Same Fable session resumed for second-pass APPROVE or REQUEST_CHANGES.
- No candidate files integrated while the review is pending.

### L3 - second review outcome

- Fable verified all previous blockers and supporting comments resolved.
- Remaining R1: the arbitrary-reference divisor search has no bounded runtime.
- Revision requested: power-of-two reference family and requested multipliers,
  bit-length-based feasible selection, negative/boundary vectors, and two naming/
  hard-cap consistency nits. Same TFF session is revising for a third Fable pass.

### L3 - third candidate

- Snapshot: `3edcd4e744149667b184f44bdf3833ba465e210671eb90912fb1db9b9b11ba25`.
- Parent independently verified all file hashes and the manifest.
- Power-of-two family and bounded bit-length selection replace arbitrary divisor
  search; new rejection/boundary vectors pass the implementer's checker.
- Same Fable session is checking the confined R1/N1/N2 changes for approval.

### L3 - final approval

- Fable APPROVE on `3edcd4e744149667b184f44bdf3833ba465e210671eb90912fb1db9b9b11ba25`.
- All four first-pass blockers, supporting comments and second-pass bounded-
  selection issue are resolved. Reviewer independently checked 2000 randomized
  selector cases, vectors, manifest, doc checker and whitespace.
- No remaining blockers. Candidate is retained intact in `lab-contracts` for
  downstream work and later serial documentation/SPEC integration.
- New implementation sessions consume this exact approved contract by absolute
  path; they may not silently reinterpret the accepted work/window semantics.
- Write-up status refreshed on 2026-11-09: `docs/lab-build-report.html` remains
  an interim draft, now distinguishing integrated components, tested candidate
  work, and unfunded versus funded acceptance. Latest owner reports show four
  AP-3b wallets opened/registered in an owned unfunded Nix fixture, and T1/J1
  fresh node/probe/driver positives plus mismatch negatives passed. T2 is
  explicitly resumed against that final tested T1 source for executor tests
  and real native funding. Full components/acceptance are not complete yet.
