> Copied from the research wiki (`bitcoin-mining-payout-schemas/output/plan-silent-pool-spec-2026-10-09.md`) on 2026-10-12. Cited wiki articles and raw sources are snapshotted under `docs/research/` (same relative layout); links were rewritten to point there.

---
title: "Spec: silent-pool — SPc pool technical specification"
type: plan
format: spec
generated: 2026-10-09
updated: 2026-11-09
status: current
supersedes: none
changelog:
  - "2026-11-12: native wallet startup and CPU authorization bind the exact opaque Supervisor run marker by a required controller-generated private run_token_hash, not a run-ID prefix. Persisted config/recorded markers and stale lifecycle bindings fail closed on restart/adopt. Public codec, SourceFeed v2 and native evidence policy are unchanged. See docs/decisions/2026-11-12-native-opaque-run-token.md."
  - "2026-11-11: full lab audited native CPU profiles require source-owned durable retention of actual public CandidateRequest/Approved bytes before activation. Versioned spc.native-evidence/1 envelopes and native_public_evidence_durable_v1 capability bind private controller-derived capture paths, process/config/manifest provenance and content hashes. Storage failures, budget exhaustion, deadline expiry and cancellation remain inactive; native wire/signature preimages and N1/N2 bounds are unchanged. See docs/decisions/2026-11-11-native-evidence-retention.md and docs/NATIVE-EVIDENCE.md."
  - "2026-11-11: full-rail decision observations retain actual settlement delivery reads, public front derivation inputs/results, bracketed grant-time canonical reads and immutable signed authorization history. Opaque lease-consumption persistence precedes cosign without changing the ten-second lease or payout rules. See docs/decisions/2026-11-11-fullrail-decision-observations.md and docs/FULLRAIL-OBSERVATIONS.md."
  - "2026-11-10: net-pool pending correction: K=T-sum(raw) is gross bookkeeping/JD weight, not an independent pool-template fee floor. Actual full ladder payments must fit T; pay_P=K+sum(ark)+sum(forfeit)+sum(E-P). Empty known windows use the same frozen-P routing and settlement checks. This corrects the Nov-3 funding shorthand and the failed native r4 candidate, without changing JD/O11 or trimming entitlement. See docs/decisions/2026-11-10-net-pool-pending-correction.md."
  - "2026-11-10: typed pre-request Ark admission: capacity deadlines and actual wallet input/proof/fee/policy shortages retain the same order as unpinned Held before any nonce/request. A durable fair PEL cursor retries Held after readiness/restart. Post-request ambiguity remains pinned. See docs/decisions/2026-11-10-pre-request-ark-readiness.md."
  - "2026-11-10: audited warmup clarification: complete independently sealed eligible history may be empty/underfilled; retain frozen Q, compute raw against actual selected work, and preserve pending/coinbase/ownership/witness checks. Missing/pruned history stays UNKNOWN. Source-feed/2 signs derived window facts; the loaded-key wallet API returns public committed evidence with request-owned key cleanup. Full measured acceptance still requires full Q. See docs/decisions/2026-11-10-audited-warmup.md."
  - "2026-11-10: explicit loss-held recanonicalization restores the complete originally validated ledger atomically only after fresh canonical confirmation and exact retained-row/Ark-provenance checks. Retained Sent is not rebooked or reissued; remaining rows restore additive deltas under Rule-B reservation checks, and confirmed recanonical Sent members clear only their corresponding pool loss. Unknown provenance or shallow ancestry holds. See docs/decisions/2026-11-10-loss-held-recanonicalization.md."
  - "2026-11-10: source-feed v1 supplies signed pool trace seals and independent collector checkpoints over bounded private Unix frames; translator commits a job/generation/digest-correlated consumption ACK before activation and invalidates superseded work during audit. Wallet capacity is acquired before fresh authorization, with same-grant fences under the wallet permit and at registration/cosign. Migration 0011 records the previously unjournaled initial economic row; only proven fresh-empty seeds permit replay. See docs/decisions/2026-11-10-source-feed-and-grant-fences.md and docs/decisions/2026-11-10-economic-initial-checkpoint.md."
  - "2026-11-10: bounded private frozen-order API: owner-authenticated GET /api/sp/me/ark-aggregates/<order_id>/order returns exact immutable signed ArkSendOrder bytes, maximum 348 bytes, after canonical/digest/signature checks. All uncertainty states remain watch-preserving. See docs/decisions/2026-11-10-frozen-order-owner-api.md."
  - "2026-11-10: aggregate review A1-A5 boundary correction: independent payout-owned authorization scheduling, exact VTXO policy plus frozen full Ark server key, transactional legacy-receipt exclusion, per-identity orphan reversal, and durable fair receipt quarantine/retry. EconomicChanged is a reload hint; Rule-B SQL/bootstrap authority and SPCB/3 suffix are specified. See docs/decisions/2026-11-10-aggregate-receipt-boundaries.md."
  - "2026-11-10: Rule B provisional-pending clarification: reserve only max(0,-delta_b), with actual mined delta_b computed from frozen job P_b; B/R/A and per-block lifecycle/ancestry are persisted and bootstrapped coherently. Fresh raw is never reduced by full unbooked payouts. Book applies additive delta and releases c atomically; unbooked orphan never reverses an unapplied delta. SPCB/3 freezes economic job provenance; missing views are unknown and descendant publication is chain/economic gated. See docs/decisions/2026-11-10-provisional-pending-net-debits.md."
  - "2026-11-10: real private source-evidence v2 and committed accounting-history v1 adapters, source-bound actual-SV2-job verifier hooks, and refuse-mode source-feed requirement. Pool ingress attribution is explicitly attested; raw PoW/Merkle/prevout fees/window arithmetic and payout ownership are checked independently. Complete external source/history seals are required, not pool owed scalars. Journal strips destination/VTXO-proof fields; no source mapping enters Ark. Hard coinbase budget ceiling, not autoscaler floor, bounds the full no-trim set. See docs/decisions/2026-11-10-source-audit-runtime-binding.md."
  - "2026-11-09: aggregate anchor/fresh-send-authorization policy authorized: freeze oldest covered canonical row's stored destination and bounded complete membership; fresh nonce-correlated all-member chain check before durable signed send authorization; potentially authorized sets never regroup on timeout; atomic Sent/NoEffect resolution with orphan loss or blocked_unknown absent proof. Anchor watches persist through coinbase discovery/retry horizons; status requires fresh B_scan API ownership proof. Supersedes §11.3 timeout-to-pending for potentially sent aggregates. See docs/decisions/2026-11-09-aggregate-anchor-and-send-authorization.md."
  - "2026-11-09: bounded ladder-entry-v2 amendment fixes WIRE's single-key contradiction with §5.1: SPCB/2 block events, one logical identity row with k-ordered key array (2048 representation cap), mandatory full owed_sats and versioned SQL/JSON entries, exact per-k settlement and no frozen-window retry. JobEntriesV2 and complete-event snapshot IDs bind remainder/destination/window changes. Recovery returns all outputs; policy audit is explicitly source-version-bound. Aggregate destination and over-revenue error-held funding remain unresolved, not silently chosen. See docs/decisions/2026-11-09-ladder-entry-wire-v2.md."
  - "2026-11-10: full lab native SV2 candidates require source-feed/2 committed wallet audit evidence before JobActivated/worker publication. The public spc.native-job-gate/1 codec binds actual ReceivedJob, SP identity, session/candidate epoch, fresh nonce, wallet lifecycle instance and controller peer/config generation; CPU holds public material only. Signed underfilled facts remain explicit, unknown/expired/replayed proofs grant no work, and funding/registration barriers remain. See docs/decisions/2026-11-10-native-audit-gate.md and docs/NATIVE-JOB-GATE.md."
  - "2026-11-09: T1-DEST lab-only output admission permits exactly standard regtest P2PKH/P2WPKH/P2TR destinations through controller, checked address, request, builder and declared-raw admission. Address APIs require_network(Regtest); unsupported templates/versions/lengths and falsely owned witness outputs reject before payment-journal mutation. All traffic inputs, owned funding and receive/change derivation remain P2PKH; actual output lengths govern fee selection and final signed bytes govern exact fee/vbytes. Control v1/history 3/derivation v2, unknown/reorg/maturity/budgets and SPEC §11.3 privacy/fees are preserved. This enables admission of published stock Bark funding addresses, not funded node/Ark acceptance. See docs/decisions/2026-11-09-traffic-destination-extension.md."
  - "2026-11-09: contextual native difficulty consumer amendment: required bound min-difficulty, spacing, interval and overlay facts; shared genesis-anchored raw-header verifier applies the node's exact gap/backtracking/boundary/inheritance/overlay order to GBT and observer/completion. No-periodic-retarget does not imply constant bits. Node policy and 40 H/s remain unchanged; periodic retargeting stays unsupported. See docs/decisions/2026-11-09-contextual-native-difficulty.md."
  - "2026-11-09: T2-L private native-bootstrap evidence rechecks owned non-symlink 0700 roots, has an explicit 256 MiB bound, and records source/executable/artifact provenance through the existing private channel. Compiled tests pass; fresh actual-node GBT 207fffff versus bound fixed 200fffff is refused before hashing, not treated as funded acceptance. See docs/decisions/2026-11-08-native-bootstrap-executor.md and the implementation report."
  - "2026-11-08: lab T2-L native bootstrap executor uses controller-reserved bounded CPU hash batches, strict legacy templates and BIP34 coinbases, bound no-retarget/subsidy policy, canonical raw-block proof before actor funding observation, and same-signer allocation distribution before independent completion. Raw work evidence is private and non-authoritative; J1/J2 and fresh funded acceptance remain separate. See docs/decisions/2026-11-08-native-bootstrap-executor.md."
  - "2026-11-08: the lab Ark deployment is self-hosted and Nix-built from stock Bark c419159fa54085572f8ca72d2ba1716556a9d3eb. The run owns one provider Postgres cluster and separate per-pool Ark wallets/processes, discovers and binds real ArkInfo before front startup, and records initialized_unfunded separately from funded validation. Hosted-provider fallback is forbidden in the lab. Production independent-server and identity-stream boundaries are unchanged. See docs/decisions/2026-11-08-self-hosted-lab-ark.md."
  - "2026-11-08: traffic-runtime amendment (e) adds TrafficJournal::observer_snapshot, an owned deterministic committed tip/sequence/outpoint-status copy under the existing journal mutex, without file I/O or mutable state authority. Snapshot freshness is explicit and the observer still rescans inclusively. See docs/decisions/2026-11-08-traffic-observer-snapshot.md."
  - "2026-11-08: traffic-runtime amendment (f) activates history 3 / spc-lab-tx-journal/3 coherently in core and T0, preserving control v1 and key derivation v2; one versioned raw-header proof atomically admits lower canonical tips, every send rechecks retained coinbase and own-change ancestry maturity, and opaque queued-intent revalidation requires controller-local serialization. Reopen refuses history 2 and invalid proofs before mutation; existing bindings precede repair/session append. See docs/decisions/2026-11-08-traffic-rollback-history3.md."
  - "2026-11-08: durable `0x5350` v1 registration, acknowledgment and Ark destination propagation. Versioned request/response frames add per-`sp_id` monotonic `request_id` correlation, the new `unsupported-mailbox-mode`, and 5 s retryable `registration-commit-unknown`. Fable-facing amendment: `request_id` is signed in `hash_SPc/ArkIdentity` so an old signature cannot replay with a larger counter. Migration 0006 atomically journals `sp_id`, request counter, immutable signed-fields digest, `ark_delivery` and payout revision under a row lock. The scoped Redis ack has TTL 3600 and is cache only. Same-request replays resolve durably; mismatch/stale/superseded never roll back newer delivery. Full registration state propagates and bootstraps, fresh `dest_id` is derived per distribution generation, and committed-but-underived routes pending, never forfeit. Advertisement exists only with the real handler; Ark order/receipt privacy is unchanged. See docs/decisions/2026-11-08-ark-registration-ack.md and docs/reviews/2026-11-07-ark-registration-implementation.md."
  - "2026-11-08 AP-2 rework: internal per-handle attempt nonce/ack keys force every registration attempt through durable Superseded resolution rather than trusting stale cached Applied, with concurrent reply isolation and no client-wire/operation-ID change. Store upsert binds epoch_fallback correctly; validly signed semantic rejection journals and returns terminal errors without poisoning PEL. SPc/EpochMailboxes domain separates epoch signatures; hold outside epoch_registered rejects. Registration-sensitive rail/destination digest rolls the job even with same coinbase fingerprint, with stale old work and new dest frozen. Front-required public ArkInfo server/mailbox pubkeys and bootstrap upgrade semantics are documented. No approval or primary integration is implied until the same Fable re-reviews."
  - "2026-11-08 R2.1 narrow resolution clarification: apply_registration now replays the full journal under the existing identity lock before is_new, preventing a rejected-ID PK collision without effect or PEL poison. Existing different digest returns registration-mismatch; identical digest preserves rejected terminal errors, current already_applied and old superseded outcomes. No counter advancement is needed to mask a conflict; missing/pruned rows use existing stale policy. No protocol/Ark-server change. Required revalidation commands and receipt are recorded in docs/reviews/2026-11-07-ark-registration-implementation.md when the build reservation opens."
  - "2026-11-08: host operator runtime wiring: off-by-default --operator-ui flag (host-only, api-role-gated) plans one leased loopback front.<id>.operator listener per front, renders the approved [operator] section, publishes the URL only after an HTTP 200 route proof recorded as evidence, and links it as a typed operator_ui interface in the hub inventory and JS guard. See docs/decisions/2026-11-08-operator-runtime-wiring.md."
  - "2026-11-08: add the lab expanded-access foundation for the spc-lab controller (scenario field `expanded_access`): per-principal run-local claim files under `<run>/private/access/` (0700/0600 atomic, CSPRNG 32-byte hex tokens, operator claim generated with the run, identity-miner-worker-pool bindings and the reserved principal/filename namespace validated at prepare before any filesystem effect, deletion- and rewrite-based revocation observed on the very next request through an unconditional fully validated claim-directory reload - hygiene-checked owner/mode/type, duplicate principal or token rejected, a failed load denying everyone including the operator until repair); Bearer authentication for every expanded route with exactly one header, constant-time compare and a closed unauthenticated static bootstrap list (same-origin POST check, CSP `default-src 'self'`, no CORS anywhere, whole-run artifacts and catalog operator-only); principal-bound command scope (users: pause/resume/budget naming an explicit non-empty subset of the claim's own workers, `send_user_payment` for self; operator: everything else) checked before any journal effect, with closed generic rejections that never reveal another principal's stored outcome; the command-log replay hash extended for expanded runs only with a persisted `hash_scheme` discriminator (`kind_only_v1`/`expanded_v1` - sha256 over scheme tag, principal, run id and the full kind payload), so idempotence is keyed per principal/run/semantics, legacy journal lines stay byte-identical and replay unchanged, an old runner never adopts an expanded entry, and an unknown or missing scheme fails closed; and the server-side user read projection - filtered snapshot/SSE/replay/resync through the claim's own worker whitelist (own fields plus allowlisted aggregates, explicit gaps and watermarks, every stream including the operator's re-authenticating at every event and heartbeat with revocation or rotation closing it at the next boundary) plus the `spc-user-projection/v1` miner-poll DTO that serves real worker rows or null-with-reason, never zeros, while the operator keeps the whole-run view. A GET-only role relay over declared topology listeners with a closed asset map backs the same-origin workbench bootstrap (test-only address mapping; no production relay reach claimed). No new dependencies; no live Ark or wallet fact claimed. See docs/decisions/2026-11-08-lab-expanded-access-foundation.md. Merge handoff (R4): in expanded runs the hub inventory route is operator-only (user claim 403 before any inventory build or I/O, tokenless 401 at the Bearer gate) and the /lab bootstrap pages join the closed pre-auth static list byte-exact to the Hub-approved assets (verified to carry no inline private fact, status, address or token); legacy runs keep the Hub's public inventory route unchanged."
  - "2026-11-08: the private traffic history journal contract is version 2, aligned to the Fable-approved T1a core header before the first runtime journal write. `TrafficHistoryHeader` is now the full 14-field record mirror (record tag, seq 0, format magic, journal protocol `spc-lab-tx-journal/2`, regtest network, lab deterministic key mode, derivation version 2, run and driver ids, run-seed fingerprint, allocation digest, retry caps, creation time) - byte-identical to the core's first line, strict (deny_unknown_fields, no defaults), with the scenario, hello-ack and header all refusing the never-written v1 shape explicitly and never migrating it. The control protocol stays v1 and separate. A conformance suite proves the contract against a real journal written by the core (byte roundtrip, typed facts, digests, golden line, fail-closed mutations). No gate, capability, pool, payout, SV2, SPc or Ark semantics change; traffic runs remain safely unavailable. See docs/decisions/2026-11-08-traffic-history-v2.md."
  - "2026-11-08: read-only live lab hub: /lab pages plus GET /api/lab/v1/runs/<id>/inventory serving the closed spc.lab.inventory.v1 allowlisted projection (evidence-fed health, loopback-only links, recorded/read-only serve); lab-tool scope, no SPEC body/payout/Ark semantics. See docs/decisions/2026-11-08-lab-live-hub.md."
  - "2026-11-08: read-only loopback operator page: optional off-by-default [operator] listen (loopback-only, api-role-gated) serving four GET routes and the spc-pool-observation/v1 DTO from one Store::liability_snapshot() call with closed-code nullable errors. See docs/decisions/2026-11-08-pool-operator-page.md."
  - "2026-11-08: lab-only opt-in rbitcoin bark-v31 RPC compat profile adopted for isolated Ark development use; default node, pins and v0.9 adoption unchanged, no SPEC sections change. See docs/decisions/2026-11-08-lab-bark-v31-profile.md."
  - "2026-11-08: lab traffic wallet core rework (T1a, `spc-lab-txd`) per the Fable review dispositions. The traffic journal header is version 2 (`format_magic`, `history_version: 2`, `protocol: spc-lab-tx-journal/2`, network, key mode, derivation version, run id, driver id, run-seed fingerprint, allocation digest, persisted `JournalCaps`, creation time); the T0-approved v1 header shape cannot carry these fields, so T0 adopts the matching v2 type and allocation-digest semantics before the first T1 journal write, and v1-shaped journals are refused without migration. Core semantics: an ever-unknown, ever-RPC-accepted or reorg-resubmitted payment never releases its inputs on rejection (durable counters decide, crash replay agrees); reorg displacement cascades invalidation through own-transaction funding and descendants with generation-by-generation recovery via proven-unspent, capped resubmission, and a canonical re-inclusion of a reorged transaction in the new chain is recordable from `Reorged` as block evidence (never naming the displaced block on stale evidence: the journal derives the displacement seq and the last committed `ChainObserved` seq from the record stream, and a newer chain observation - a reorg back - makes the same-hash confirmation admissible, the original displacement identity preserved through the cascade and its generational restore), restoring the cascade one generation at a time; retry caps bound sends (N retries + the original), never evidence recording, and are persisted and revalidated on reopen; an exclusive kernel flock makes one journal one owner; a restart is journaled uncertainty - reopen is a writer operation that appends a durable `SessionStarted` record under the exclusive lock and moves in-flight broadcasts to unknown without consuming retry budgets or inflating attempt counters; funding observations carry provenance and a derivation locus but no script (the wallet derives it), own-transaction backing requires the journaled confirmed creator with an exact vout match, and coinbase funding is spendable only at 100 confirmations of the observed tip; the command digest covers the payer role and category; spending resolves keys by direct derivation at the recorded index; multi-actor allocations are validated (disjoint ranges, exactly one bootstrap host, unique driver ids, equal caps) before provisioning - production provision and reopen consume an opaque `ValidatedAllocation` token whose canonical digest is persisted in the header and verified on reopen before any mutation; failed journal writes - injected or real - share one rollback-or-poison recovery; late lifecycle observations after a reorg cycle are journaled with only the finish guarded (`note_rejected` always records the rejection and guards only the finish; an 'already known / already in block chain' node answer is a confirmation observation T1b maps to `note_confirmed`, never a rejection, and the core never parses the code string); traffic keys derive as HMAC-SHA256 over run seed and run id (derivation v2, key mode `lab_deterministic_run_seed_v1`). See docs/decisions/2026-11-08-traffic-wallet-core-rework.md."
  - "2026-11-08: Reworked the T0 traffic-contract packet per the Fable review: the traffic capability gate now runs in `RunEngine::prepare` before any filesystem, lock or authority effect and on the adopt path before the owner lock (`traffic_capability_unrecorded` when no record exists, even if the node config declares the capability name); the AP-1a projection gained the frozen 17-row expected-check table with exact id-method binding, strict report provenance/target/profile validation and full per-check rows the gate recomputes both verdict stages from; the single `satisfied` verdict became the two-stage `recognized` (preflight gate) vs `behavior_proven` (false by construction for read-only reports; T3 consumes its own exercised evidence); the Fable-approved r2 contract `f4fc3e04...` is bound as the sole approved lineage (hash + table + decision in one change; the r1 candidate stays recognized-unapproved diagnostics; unknown hashes fail closed); `send_user_payment` gained the explicit fail-closed `destination_address` field (regtest p2pkh, resolving the plan 6.6/12 ambiguity); actor/driver ids are typed `ActorId`s; the history journal is confined to `traffic/`; the protocol string is exact (no trim) and the planned-schedule seed domain is frozen. Traffic scenarios still cannot start until T1 records a capability - the safe-unavailable state. Decisions: `docs/decisions/2026-11-08-lab-traffic-contract-t0.md` and `docs/decisions/2026-11-08-traffic-ap1a-lineage-r2-approved.md`."
  - "2026-11-08: agent wallet foundation (AP-3a) made normative. §11.5 gains the exact BIP 85 child-wallet derivation (APP-39, m/83696968'/39'/0'/12'/0', English 12 words, index 0, empty passphrase - defined, not 'BIP85-style', vectors pinned in-tree), the offline provisioning shape (branch account xprvs m/352'/0'/0'/1' and 2' plus the 64-byte child seed and sp_address only; exact child-index, network-magic, shared-parent-fingerprint, B_scan and B_ark != B_spend validation at load, fail closed, nothing regenerated), the persistence layout (<datadir>/state.sqlite schema v2 for the SP-aware keystore/VTXO ledger/registration journal/scan cursor, bound to one identity, foreign or damaged files refused; <datadir>/bark/db.sqlite as the stock bark persister initialized via stock init_wallet with WalletProperties(network, fingerprint, none, none), 0700/0600 before open, reopen re-validated, foundation only - full Wallet::open is AP-3b), the registration client journal and terminal policy (durable request-id reservation before send, fresh id per changed payload, effective binding = most recent acknowledged applied/already_applied state so a historical row is not liveness and superseded clears the binding it names, hard terminals and counter exhaustion fatal until an explicit operator restart, wedge kinds mismatch/stale/superseded get exactly one fresh same-payload retry, all non-success in the 1/2/4/8/16/30 s backoff with registration_unavailable surfaced after six failures while retries continue), and status honesty (registered and connected are distinct; no transport means dormant, never a invented connection; loopback enforced at bind; unknown is null; monotonic-only scan cursor with journaled reorg rewind an explicit AP-3b dependency). §11.10 spc-agent [ark] gains seed_child, scan_xprv, ark_xprv, datadir, status_listen, network and ark_id (paths/ids only, key material never inline). Changelog fragment: its serial position relative to the concurrent AP-2 (lab-ark-registration) §11.4/§11.9 fragment is pending splice at merge; both fragments edit disjoint sections. See docs/decisions/2026-11-08-agent-wallet-foundation.md."
  - "2026-11-08: pool-template jobs support segwit transaction sets, fail closed. Every template transaction is strict-decoded (full-tx deserialize plus byte-exact roundtrip; a non-canonical encoding or a zero-input transaction fails the refresh) and analyzed into txids, wtxids and a witness count. A set carrying witness data gains exactly one zero-value BIP 141 commitment output appended last (pool output stays index 0); BIP 141 leaves the reserved value unconstrained, and the pool draws it once per witness root - a share-stability choice, not a protocol obligation - so unchanged-set republications keep issued shares valid, and the commitment joins the job-change comparison (including the main-level publication identity) so a same-txid witness malleation republishes the job on both ingresses. Block assembly injects the coinbase witness (txid unchanged), re-verifies the commitment against the actual transaction set, and rejects wrong-reserved, missing, duplicate, malformed, valued or mismatched commitments without fallback, logging a bounded error when assembly fails after a share was credited; witness-free templates are byte-for-byte unchanged. Payout mode detection no longer reads a `6a24` commitment as the JD `OP_RETURN` carrier (`6a23` SPc), and settlement's tier crosscheck mirrors the 224 WU commitment overhead detected from the mined coinbase. Sections 2.4.2, 3.3 and 4.8 are amended. See docs/decisions/2026-11-08-pool-witness-commitment.md."
  - "2026-11-05: the v2 accounting window is measured in network-block work, not N share-difficulties. With `N = 8`, reference target `R = 2^255 - 1`, network target `T` and `E(T) = 2^256/(T + 1)`, the persisted threshold is `Q(T) = ceil(N * (R + 1) / (T + 1))` reference units. On `0x200fffff`, `Q = 65` (floor 64); on the `0x207fffff` pow limit, `Q = 9` (floor 8). The window is at least N blocks of expected work; rounding error is below `E(R)` and whole-slice overshoot is reported. `underfilled`, `actual_sum` and `overshoot` are derived from `high_water` and `window_target`, not stored columns. A persisted `window_rule` uses code 0 `share_work_legacy` (`N * d`, v1) or code 1 `network_block_work` (v2), also carried as `U8 window_rule` on `spc:blocks`. Settlement uses the snapshot's rule and target and never recomputes from current bits; `max_window_target_units` is a hard cap `<= 2^64`, and `Q > max_window_target_units` fails with `window_target_too_large`. Sections 4.1, 4.3, 5.1, 5.3 and 7.4 and WIRE-FORMATS 3.1 are amended. See docs/decisions/2026-11-05-window-network-block-work.md."
  - "2026-11-05: native SV2 conformance and authenticated miner sessions. A prevhash or nbits change publishes a future job before its single SetNewPrevHash; other same-(prevhash, nbits) template changes publish a nonfuture job, while ordinary curtime-only refreshes preserve the immutable issued job. Only the newest job is accepted (older ids return stale-share), and every SV1 republication sets clean_jobs=true. Targets use U256 little-endian and SetTarget precedes effective-target changes, with each job capturing its target at activation rather than retroactively changing active work. Coinbase Merkle paths are deepest-first across SV1/SV2/front/agent. One connection has one extended channel/identity, deterministic front-namespaced prefixes and exact negotiated extranonce validation. Bits/version changes republish jobs and the front rebuilds the coinbase/header hash before credit. Fronts require a provisioned matching Noise authority key pair and agents pin it. Native SV2 share replies correlate with front slice acceptance and carry the job generation's credited d; success does not assert node acceptance, settlement or payout. No share-work-unit, window, payout-tier or Ark-boundary policy changes. See docs/decisions/2026-11-05-sv2-native-conformance.md."
  - "2026-11-05: lab work units v2 add an opt-in exact reference-target share-work profile restricted to the power-of-two family. `R = 2^k - 1` with `1 <= k <= 255`, `m = 2^j` with `0 <= j <= k - 1`, and `S = 2^(k-j) - 1` make `E(S)/E(R) = 2^j` exactly, where `E(T) = 2^256/(T + 1)`. The profile requires `R >= effective pow limit` (`reference_below_pow_limit`), a power-of-two `R + 1` (`reference_not_power_of_two`), and a power-of-two `m_cfg = 2^j_cfg` (`multiplier_not_in_family`). Issuance is O(1): `j_issue = min(j_cfg, k - ceil_log2(T + 1))` with exact-power-boundary handling; it records `multiplier_stepdown`/`multiplier_stepup` at a new generation, is not a clamp, and never scans for divisors. Hard caps are `m_cfg <= max_work_multiplier <= 2^32` and `Q <= max_window_target_units <= 2^64`. v2 checks slice `sum(d)`, `d*f`, `diff_fee_sum`, window `D_win`, SPEC 4.4 products and final conversions, failing with named errors rather than saturating, while v1 keeps its saturating behavior. Compact decode follows consensus `SetCompact` (including valid exponents 33/34). Work-unit version/fingerprint and `window_rule` are persisted and carried in definite `spc:slices`/`spc:blocks` fields; restore rejects mixed/mismatched profiles and the DB enforces the version/fingerprint relation. The oracle reads `job_issued` (`job_id -> share_target, m_issue`); v1 type-32 behavior is unchanged and v2 type-32 is not claimed. Sections 3.4, 4.1, 4.2, 4.4, 5.1, 5.3, 7.4 and WIRE-FORMATS 3.1/3.6 are amended. See docs/decisions/2026-11-05-lab-share-work-units.md."
  - "2026-11-05: integrate the approved LP-1 reference-work and network-block-window decisions. v2 is opt-in regtest-only, rejects legacy `share_difficulty`, SV1 and v2 type-32, calculates exact targets with stepdown/up per immutable job generation, and settles from frozen version/fingerprint/rule/Q. V1 rows and arithmetic remain legacy. v2 slice, fee aggregate, window and payout products use checked exact arithmetic and restore rejects mixed profiles. `spc:slices` carries version/fingerprint, `spc:blocks` carries version/fingerprint/window_rule, and migration 0005 adds definite enums/CHECKs plus legacy defaults. Job/accounted evidence carries generation/target/credit, with disjoint SV1/SV2 correlation and `share_outcome_unknown` instead of false definitive timeout rejection. Native main-process SV2 non-block and block-qualified m=2 evidence is implemented; v2 type-32 and TDP/ladder production scenarios remain unclaimed. See docs/decisions/2026-11-05-lab-lp1-integration.md. Refresh failures invalidate front and ingress work. v2 refuses type-32 at SV2 ingress and never populates its store, regtest pow_limit_bits is pinned to consensus, payout v2 restoration validates stored sums with checked arithmetic, WIRE documents definite metadata fields, SQL pins nonzero 32-byte fingerprints, and evidence includes raw share hashes/full extranonce plus byte-order metadata. R2 retention correction: durable scrub chooses legacy-configured or 4096-height deletion strictly by each stored row work_unit_version, so payout configuration cannot delete v2 early or extend v1 identity retention; the v2-only 4096-height SPc attribution extension is explicit. Live/bootstrap v2 keeps all age-eligible slices up to 4096 independently of current/previous Q (same tip-horizon as SQL), preserving headroom for later harder windows. Real cap/age exhaustion remains explicit underfill/fail-closed; v1 retention and approved work-unit arithmetic are unchanged."
  - "2026-11-04: Tor scope narrowed to mining connections. Inbound `[reach]` is unchanged; outbound `[net]` is `spc-agent` only and covers upstream SV2 and JDP. The `ark` role's Ark-server dial and the agent's Ark/mailbox traffic are direct (no proxy, no .onion gate); the accepted residual is that the Ark server and mailbox endpoint see the network address. §3.8 outbound rewritten, §6.3 pool `[net]` removed, Ark egress clauses dropped. See docs/decisions/2026-11-04-tor-scope-mining-only.md."
  - "2026-11-03: settlement window target is persisted, not estimated. §5.1 `distribution_snapshot` gains `window_target NUMERIC(40,0)` (nullable; the exact `N x d` the front selected the window with, computed in u128 because `N x d` can exceed BIGINT, encoded exactly like `slice.difficulty`); the front writes it when it builds the distribution and carries it in the §3.3 `spc:blocks` event (WIRE-FORMATS §3.1 `U128 window_target`). §7.4 settlement reconstructs the window with the persisted target; only a pre-migration row (NULL) falls back to the slice estimate, with a warning naming the block, and a stored value that is not a u128 fails closed. See docs/decisions/2026-11-03-settlement-window-target.md."
  - "2026-11-03: denomination-ladder coinbase payouts with Ark remainders (operator product decision; tradeoffs in the record, not buried). FINAL ladder: constant rungs {500 000; 1 000 000; 3 000 000; 5 000 000} sats, greedy largest-first decomposition of `owed`, remainder < 500 000 sats (and full sub-500k balances) to the Ark rail at `H + confirmation_depth`, aggregated per `sp_id` below `[ark] min_overage_sats`. NO era scaling and no halving re-parameterization, forever: the band function is trivially pure over the amount (no dependence on height, subsidy, fees or the coinbase value), which removes the front/settlement divergence risk entirely. Output count is value-bounded by `T/500k` (~619 in era 1); `max_payout_slots = 1024` is a policy cap, not a weight limit (2 048 documented as fee-spike headroom; the 1 000 000-WU ceiling, coinbase <= 25 % of block, stands). Owner constraints applied: maximize on-chain share; NO deferral for capacity or trimming (trim removed); anything not payable in the coinbase goes over Ark; a small error rail (failed sends, unknown keys) is retained. Accepted costs: sub-500k identities get no coinbase output (~86 % of a 1 000-identity pool in the record's model), remainders trustodial until refresh, Ark server sees exact remainders, and each halving doubles the rungs' relative size with no scheduled change. Pool-template mode only in v1; JD weights stay exact-`owed` (O20). Sections: §0, §2.3, §3.6, §4.1, §4.4, §4.6, §4.7, §4.8, §5.1, §6.1, §6.3, §10 (O8 amended, O20 amended), §11.1, §11.2, §11.3, §11.6, §11.7, §11.9, §11.10. See docs/decisions/2026-11-03-banded-amounts-with-ark-overage.md."
  - "2026-11-03: JD fallback semantics defined. §3.5 gains 'Declaration answers' (the JDS MUST answer every parseable declaration; decline codes `invalid-job` / `no-payout-distribution` / `stale-payout-distribution` / `invalid-payout-distribution`, plus upstream `internal-error`) and 'JDC fallback' (declare at most once per (distribution_id, template) key; wait `[jd] declare_timeout_secs` (default 10, minimum 1) for the answer; on a hard decline, silence, or a refuse-mode own-payout failure the JDC latches the fallback - stops declaring, alarms once per episode, retries once per changed declaration key; JDP carries no pool-template jobs, so an agent-level fallback is the co-resident translator role, never a job the §3.6 check did not pass; a template move abandons the outstanding declaration with a reason and rebuilds the whole template, coinbase and transaction list). §3.6 and §6.3 amended accordingly. See docs/decisions/2026-11-03-jd-fallback.md."
  - "2026-11-02 (f): Tor is a supported transport in both directions, with no protocol change. §3.8 added: `[reach]` publishes onion endpoints and forces every onion-published listener onto loopback (fail closed); `[net] proxy` routes `spc-agent`'s upstream/JDP/Ark traffic and the `ark` role's gRPC through a remote-resolution SOCKS5 proxy with per-connection stream isolation; a `.onion` target with no proxy is a startup error; downstream SV1 stays plain TCP. §6.1 `/api/pool` publishes `endpoints`; §6.3 `[reach]` and `[net]`; §9 one onion address per front (or OnionBalance) with the onion key backed up; §11.3/§11.7/§11.10 Ark-role and mailbox egress, one connection per epoch read; `/api/pool` and abuse controls no longer assume a peer IP. See docs/decisions/2026-11-02-tor-transport.md."
  - "2026-11-02 (e): front/settlement/JDS integration. §4.3 `high_water` is the newest *sealed* slice the window consumed (`0` when the window holds none), never the still-open finding slice, so the settlement retry heuristic is gone. §5.1 `slice_id = front_id << 56 | local_seq`, `local_seq = max(last + 1, unix_ms)` masked to 56 bits (same composition as the §3.5 distribution id), seeded from the bootstrap tail so restarts never reuse or collide. §5.3 the bootstrap snapshot's `window_tail` rebuilds the live window (credits parsed, newest first, truncated to the configured `[accounting] retain_windows`; rows without credits are skipped, never zero-filled) and the high-water mark/sequence come from the newest restored id; a restored balance whose miner keys are unknown is held on the `pending` rail rather than paid to a guessed output. §4.7/§11.2 JD weights come from the same routing as the coinbase distribution (`build_weights`) and are published as `distribution_from_weights(weights, p_x_by_id, pool_script, a_send_x, dust_limit)`; the sats-as-weights conversion is removed. §2.2/§3.5 the front refreshes the template on `[template] refresh_interval_ms` (default 1000, MUST be > 0) and republishes jobs plus the JDS distribution only when `(height, prev_hash, fingerprint)` changed, so an external new block cannot leave them stale. See docs/decisions/2026-11-02-share-target-policy.md."
  - "2026-11-02 (d): share target/difficulty policy added. `[accounting] share_difficulty` (default 1, MUST be > 0) defines the pool share difficulty; every ingress validates a share against `effective_target = max(target_for_difficulty(share_difficulty), network_target)` (never harder than the network target) and the front credits `d = min(share_difficulty, max(1, round(network_difficulty)))`, which is never 0. The same `d` is used for the `ShareRec`, the §4.3 window target (`N x d`, was `N x network difficulty`) and the slice aggregates; block detection still requires the consensus target. §3.4 is amended: SV1 sends `mining.set_difficulty` and rejects a submission below its target (`low difficulty share`), SV2 announces `effective_target` in `OpenExtendedMiningChannelSuccess.target` and rejects with `SubmitSharesError{difficulty-too-low}`; the front re-checks before accounting. Share `job_fees` are used when non-zero (JD mode), falling back to the template fees in pool-template mode. `front` now applies the bootstrap snapshot (pending balances, Ark registrations) and `spc:front:events` (`LedgerApplied` deltas, `ArkRegApplied`) and carries trimmed/held balances into the next distribution. See docs/decisions/2026-11-02-share-target-policy.md."
  - "2026-11-02 (c): O13 resolved - the agent's SP-aware wallet is a thin in-tree wallet over `ark-lib` + `bark-server-rpc` (no upstream bark patch or bark-wallet fork for v0.1), exactly as `spc_ark_client` validated against a stock server. Recorded gap: in-round refresh and unilateral exit of `u_i`-keyed VTXOs ride stock bark key lookups, so v0.1 Ark-tier VTXOs cannot be refreshed in a round or exited; durability paths are arkoor spend, cooperative offboard and the §11.5 shutdown warning. Pool-side transport is a stock `bark-wallet` send-only adapter plus a separate board/maturity loop. bark pinned at `ark-bitcoin/bark @ c419159fa54085572f8ca72d2ba1716556a9d3eb` (UPSTREAM_DEPS.md; pool uses stock `bark-wallet` + `ark-lib`, agent uses `ark-lib` + `bark-server-rpc`). See docs/decisions/2026-11-02-bark-adapter.md."
  - "2026-11-02 (b): API challenge signature defined as BIP340 by `b_scan` over `hash_SPc/Api(nonce bytes)` (was the informal `schnorr_sig(b_scan, \"SPc/api\" ‖ nonce)`); `spc:slices` payload added to WIRE-FORMATS §3.6."
  - "2026-11-02: native SV1 in `front` (same §2.1 fail-closed identity parse from the SV1 username, pool-template only, one share/accounting path) and Nix-flake-only build/CI/deploy. Protocol baseline is `stratum-core` 0.6.0; `0x0003` and `0x5350` codecs live in `spc-ext`. `§1`, `§3.1`, `§3.4`, `§6.3`, `§7.1`, `§9` amended. See docs/decisions/2026-11-02-native-sv1-nix-and-protocol-baseline.md."
  - "2026-10-15: four design decisions applied. D1: MULTI-REGION §10 deltas 3, 6, 7, 8 applied - `high_water` vector (§4.3 `select_window`, §4.7, §5.1 `distribution_snapshot.high_water`, §3.3/§5.2 `spc:blocks` and `BlockFound` payloads, §7.4 settlement reconstructs the window from it); §11.1 tier gains a committed-but-not-propagated registration -> `pending` branch and §4.6 (f) reads 'no `ark_delivery` row in Postgres at settlement'; §11.3 `ark` tracks pool outputs by `(txid, vout)` and re-checks the chain at maturity (+ §11.9 same-height test); §3.7/§5.3 type-32 served from bootstrap-RPC blobs + live `spc:slices` tail. D2: `provisional_paids` announced on `BlockFound`, not `LedgerApplied` (§5.2, §5.3). D3: `ArkSendOrder.sig` = BIP340 over `hash_SPc/ArkSendOrder(...)`, no chain prefix (§11.3 order verification, §11.10). D4: `block_found.fingerprint` nullable with `CHECK (status <> 'confirmed' OR fingerprint IS NOT NULL)` (§5.1, §7.4, §11.1)."
  - "2026-10-14 (c): multi-region amendments adopted (docs/decisions/2026-10-14-multi-region-amendments-adopted.md). §2.2 repeat alarm scoped per front; §3.3 the `spc:blocks` event carries `entries` (Redis snapshot and `snapshot_ttl_secs` deleted); §3.5 `distribution_id = front_id << 56 | local_seq`, `local_seq = max(last_local_seq + 1, unix_time_ms)`, and the grace rule rewritten as 'previous id from the same front on the same connection' (no `id − 1` arithmetic); §5.1 `slice_id` per front; §5.2 streams (+`spc:arkreg`, +`spc:front:events`, −`spc:snapshot:*`, `spc:dist:current` struck as intra-front) and trim discipline (XTRIM MINID behind the slowest live reader — payout group ack and front cursor heartbeats, TTL 120 s — 600 s floor, 1M-entry backstop alarm, re-bootstrap); §5.3 front RAM; §6.3 `front_id`, `outbox_max`, `tip_divergence_secs`, `front_cursor_*` (replacing `snapshot_ttl_secs`); §7.4 missing-event settlement failure model; §9 topology ('RAM-only state — no durable state of its own'), `front_id` lease, monitoring. MULTI-REGION §10 deltas 3, 6, 7, 8 were applied afterwards (2026-10-15 entry)."
  - "2026-10-14 (b): O11 resolved — JD-mode claim formula is revenue-scaled: `claim(id) = floor(owed(id) · (T_actual − F) / W)`; `pending'(id) = claim(id) − paid(id)` (docs/decisions/2026-10-14-o11-claim-formula.md). O14 resolved — default mailbox mode is `epoch_derived` (variant B); no server-side registration exists; caveats on related keys and network linkability recorded (docs/decisions/2026-10-14-mailbox-default-epoch.md). M3 resolved — order-signing key is secp256k1 BIP340 Schnorr, CSPRNG-generated, stored at `/run/secrets/spc/ark-order-key` with 0o600/0o700 perms; pubkey via `[ark] order_pubkey` config; operator-driven rotation (docs/research/output/ark-order-key-spec-recommendation.md)."
  - "2026-10-14 (a): §11.3 'Reorg and float edge cases' added (6-block rule normative; deeper reorgs and float surplus/sizing are operator policy with defaults); §11.4 states that no server-side registration endpoint exists; docs/WIRE-FORMATS.md added (byte-level formats of the new streams, `spc:arkreg` and the bootstrap RPC) and linked from §5.2."
  - "2026-10-13 (d): spc-jds is a module of the spc-pool binary's front role (matching the SRI pool's bundled JDS), not a separate binary; JD is always on and [jd] enabled is removed. a_send sharing becomes in-process; spc:dist:current becomes intra-front state. See docs/decisions/2026-10-13-jds-in-pool-binary-jd-always-on.md."
  - "2026-10-13 (c): payout flow per the operator decisions. One hot pool output per block, fresh P2TR from `pool_payout_descriptor` (xpub under the `ark` seed) indexed by block height; Ark-tier miners paid from the float at `confirmation_depth` = 6 (was 3; was coinbase maturity); at maturity `ark` boards only what the float needs, the rest is operator surplus with no prescribed handling. Treasury signer, watch-only `pool_payout_script` and `float_max_sats` removed. Unregistered miners below `C` are forfeited to the pool (published policy; `spc-agent` registers automatically and will not mine unregistered); `pending` now arises only from trimming and failed sends. Coinbase-as-board recorded as rejected (O17)."
  - "2026-10-13 (b): pool-side key separation (§11.3). The pool's bark wallet is a hot key in a new `ark` role on its own host, generated independently of every SPc-side key, holding a float capped at `float_max_sats` and topped up by an off-host treasury signer in round amounts decoupled from blocks. `ark` sees only signed, capped send orders carrying what the Ark server sees anyway — no `sp_id`, `P_i` or block data — so compromising it risks the float but never SPc receiver privacy, and compromising the SPc side risks attribution data but not the float beyond `max_daily_sats`. `pool_payout_script` returns to watch-only (§9)."
  - "2026-10-13: reconciled to DESIGN.md, which is authoritative for payouts wherever the two disagree. §2.3/§2.5/§11.8 adopt DESIGN's derivation (`SPc/Inputs`, `SPc/TierSpc`, `SPc/TierArk`; the tag is the domain separator; hash-to-scalar fails rather than reduces), superseding the `BIP0352/*` tag-reuse interview decision. §11.1 tier split is now coinbase-threshold (default = dust) → coinbase, below → Ark (replaces the 500 000-sat band). Ark sends happen at the found block's coinbase maturity, from a bounded float held by a separate `ark` role (see the next entry). §11.4 separates identity from delivery capabilities: `SetArkIdentity` requires a separate `B_ark`, carries a mailbox mode, and the literal-address `SetArkPayout` fallback is removed in favour of DESIGN's empty-delivery + out-of-band fetch. Pool now retains per-block Ark destinations until the Ark send (a_send is erased). Fixed: custody wording (§0), threshold/revenue `T` name clash, `k` usage, `spc-agent` in the workspace."
  - "2026-10-12 (d): license changed to MIT OR Apache-2.0; §7.6 rewritten — Blitzpool patterns re-implemented from the spec, no AGPL code ported, clean-room rule added."
  - "2026-10-12 (c): §11.8 reversed — Ark-tier key derivation ADOPTED per DESIGN.md (separate hot B_ark, domain-separated tweaks); §11.4 gains SetArkIdentity {B_ark, M, D}, SetArkPayout becomes the fallback; §11.5 agent holds b_ark and an external-key table. Code-read corrections recorded as implementation notes (mailbox needs M+D; stock bark drops derived keys)."
  - "2026-10-12 (b): PR #203 impact pass (workflow, 110 agents): pins by SHA (47dbedd; bitcoin_core_sv2 block_reserved_weight fix), §3.5 publication bullet re-timed to ledger apply, §4.6 settlement-reads-mined-coinbase rule + O11, §11.9 conservation identity corrected, §2.4.2 OP_RETURN 46 B/184 WU, §8.4 full-mempool step 9, §11.2 weight_P composition note."
  - "2026-10-12: added §11 Sub-threshold Ark rail (both template modes, SPc extension for the Ark address, spc-agent collapsing translator+JDC+payout agent, single-seed recovery, SP-derived-key appendix); resolved §10 O6; §1 note on spc-agent."
companion: output/plan-spc-pool-pplns-jd-silent-payments-2026-10-09.md
goal: "Technical specification for silent-pool: a Stratum-V2-only mining pool with BIP 352 silent-payment addresses as miner identity, PPLNS-JD/SLICE accounting, coinbase-direct payouts to one-time P2TR outputs (SPc), both pool-template and Job-Declaration modes, an SPc-enforcing Translator and JDC, and the share-accounting extension (type 32) served in v1."
decisions_from_interview:
  - "Both template modes in v1"
  - "Blitzpool-style role split (front/payout/api) over Redis streams + Postgres"
  - "SV2-only; SPc Translator + JDC shipped from this repo"
  - "Share-accounting extension (type 32) in v1, pool-defined where the spec is unfinished"
  - "Hash tags: reuse BIP0352/Inputs and BIP0352/SharedSecret — SUPERSEDED 2026-10-13 by DESIGN.md (`SPc/*` tags, §2.3); compatibility with a plain-SPc scanner is open item O12"
  - "A_send: x-only 32 B on the wire, even-Y normalisation"
  - "License: MIT OR Apache-2.0 (changed 2026-10-12 from AGPL-3.0-or-later — Blitzpool math is re-implemented from §4.6/§4.8, not ported; matches SRI, bark, extensions_sv2 and the spc_ark prototype)"
  - "a_send cadence: per block height in both modes"
authority: "DESIGN.md (repo root) is authoritative for the payout derivation, tier policy and Ark delivery. Where this spec disagrees with it, DESIGN.md wins and this spec is the bug."
sources:
  - output/plan-spc-pool-pplns-jd-silent-payments-2026-10-09.md
  - inventory/candidates/coinbase-native-stealth-payout.md
  - raw/notes/2026-08-14-ll-coinbase-silent-payments-ecdh-nonce.md
  - raw/notes/2026-08-16-ll-sv2-pool-tag-asend-carrier.md
  - raw/notes/2026-08-24-ll-jd-coinbase-control-amount-linkability.md
  - raw/articles/2026-10-09-delving-silent-payments-coinbase-thread.md
  - wiki/concepts/pplns-jd.md
  - wiki/concepts/sv2-share-accounting-ext.md
  - wiki/concepts/xpub-payout-identity.md
  - wiki/concepts/coinbase-address-rotation.md
  - wiki/concepts/coinbase-amount-linkability.md
  - wiki/concepts/payout-attribution-privacy.md
  - wiki/concepts/block-withholding.md
  - raw/repos/2026-07-27-blitzpool-server-rust-github.md
  - raw/repos/2026-07-27-blitzpool-finder-bonus-code-read.md
  - raw/repos/2026-07-29-sv2-apps-xpub-coinbase-rotation-code-read.md
  - raw/repos/2026-07-14-demand-share-accounting-ext-github.md
  - ../mining-pool-architecture/wiki/concepts/share-accounting-and-durability.md
external_pins:
  - "bitcoin/bips bip-0352.mediawiki v1.1.1 @ 60f5b33"
  - "stratum-mining/sv2-spec @ d8bf5b8; PR #203 extensions/0x0003-coinbase-transaction-payouts.md @ 47dbedd (single commit 2026-09-10; unchanged as of 2026-10-12)"
  - "demand-open-source/share-accounting-ext extension.md @ c661e93 (crate v0.0.13)"
  - "lorbax/pplns-with-job-declaration.tex (master)"
  - "warioishere/blitzpool-server-rust @ 7815884 (AGPL-3.0-or-later)"
  - "ark-bitcoin/bark @ c419159fa54085572f8ca72d2ba1716556a9d3eb (ark-lib, bark-server-rpc, bark-wallet; pool adapter is stock bark-wallet, agent is an in-tree thin wallet - see UPSTREAM_DEPS.md and docs/decisions/2026-11-02-bark-adapter.md)"
---

# silent-pool — Technical Specification (v0.1 draft)

> Generated from [bitcoin-mining-payout-schemas](research/README.md). Companion to the [roadmap](ROADMAP.md), whose eight decisions this document makes concrete. Normative words (MUST/SHOULD/MAY) are RFC 2119.

## 0. Scope, terminology, threat model

**silent-pool** pays each miner of a found block on one of two tiers, both derived from the same per-block ECDH against the miner's BIP 352 silent-payment address (`DESIGN.md`): a **fresh, unlinkable P2TR output in the coinbase** (the **SPc** construction, coinbase-scoped silent payments) when the miner's balance clears the coinbase threshold, and otherwise a **fresh Ark VTXO key** paid out-of-round once the block's proceeds mature (§11). Accounting is **PPLNS-JD / SLICE**.

**Custody, stated once.** The coinbase tier is non-custodial: those sats are the miner's the moment the block is mined. With denomination payouts (§4.4, decision 2026-11-03), the coinbase carries whole rungs only; the remainder below the base rung (500 000 sats) is custodial for the 6-block confirmation window and then trustodial until refresh, exactly like the Ark tier it rides. Everything else is held by the pool for a bounded time: sub-rung balances during the 6-block confirmation window before the Ark send (§11.3), error-held balances (§4.6), and failed Ark sends awaiting retry. Earnings of a miner with no Ark delivery registered that cannot be coinbase-paid are forfeited to the pool by published policy (§11.1). After the Ark send, Ark payouts are trustodial (sender-or-server honest) until the miner refreshes (§11.7). The total is reported at `/api/pool/liability`.

| Term | Meaning |
|---|---|
| `B_scan`, `B_spend` | miner's BIP 352 scan/spend public keys, parsed from the `sp1q…`/`tsp1q…` address in `user_identity` |
| **identity** | the SP address, and nothing else. It is the only thing that keys the miner's account |
| `sp_id` | `tagged_hash("SPc/Id", ser_P(B_scan) ‖ ser_P(B_spend))`, 32 B — the pool's internal ledger key. **Not** a consensus or wire value; never leaves the pool |
| **delivery capability** | optional, private, per-`sp_id` registration data the pool needs to *deliver* to a tier, never to identify the miner: today only the Ark registration `{B_ark, M, D, mailbox mode}` (§11.4). Never published; losing it changes where a miner can be paid, not who they are |
| `a_send`, `A_send` | pool's per-height ephemeral ECDH keypair; `A_send` is published in the coinbase, `a_send` is never persisted |
| `H` | block height being mined (the `SetNewPrevHash` height + 1 relationship is fixed by the template) |
| `ecdh_i` | the per-`(miner, H)` shared secret, `ser_P(input_hash · a_send · B_scan,i)` (§2.3); the single root of every per-block destination |
| `t_onchain`, `t_ark` | domain-separated tier tweaks from `ecdh_i` (§2.3) |
| `P_i` | miner `i`'s coinbase output key for height `H`, `B_spend + t_onchain·G` |
| `B_ark`, `U_i` | miner's separate Ark base key (`m/352'/0'/0'/2'/0`), and its per-block Ark VTXO key `B_ark + t_ark·G` (§11.8) |
| `C` | `coinbase_threshold_sats`: the balance a miner must reach to get a coinbase output (default = `dust_limit_sats`, §11.1). Not to be confused with `T`, the block revenue |
| carrier | where `A_send` sits in the coinbase: **scriptSig** (pool-template mode) or **`OP_RETURN`** output (JD mode) |
| slice | a run of shares on one prev-hash whose job fees lie within `[f_ref, f_ref + δ)` |
| window | the slices whose summed difficulty reaches `N × D` walking back from the block-finding share |

**Threat model (stated, not overclaimed).** SPc hides *which output pays which miner* from **third-party chain observers**. It does **not** hide anything from the pool, which validated the shares ([Payout Attribution Privacy](research/wiki/concepts/payout-attribution-privacy.md)). Coinbase **amounts** are denomination-quantized (§4.4): every coinbase payout in a block is a greedy decomposition over the constant public rung set `{500 000; 1 000 000; 3 000 000; 5 000 000}` sats, so a chain observer learns the per-rung output counts, the block total, and timing — not an exact amount per miner; the sub-base remainder rides the Ark rail, so exact remainders for every paid miner are visible to the Ark server, which is the cross-layer observer the design already assumes (§11.7). Residual channels (rung-class composition, per-block counts, timing, whale top classes, redemption-time clustering) are listed in §11.7 and quantified in the decision record. Forward secrecy is bounded per height: compromise of one `a_send` reveals one block's payee set and nothing else; `a_send` authorises **no funds**.

Out of scope for v0.1: BIP 352 labels; a Lightning/BOLT12 rail for pending credits; finder bonuses; Job Declaration over SV1 (SV1 is pool-template only); mainnet.

## 1. System architecture

```
   SV1 ASICs ──SV1──▶ spc-translator ─┐                              ┌─ bitcoin-core (IPC / sv2-tp) ◀── TDP ──┐
                        (sp_address)  │  SV2 Mining Protocol          │                                        │
   SV2 miners ───────────────────────▶├──────────▶  spc-pool :: front ◀┘   (pool-template mode)                 │
                                      │             ├─ channel→sp_id map                                        │
   miner's own template               │             ├─ a_send[H]  (RAM only, zeroised on advance)               │
   ┌──────────────┐  JDP (0x0003)     │             ├─ open slice, seen_shares                                   │
   │   spc-jdc    │◀────────────────▶ front's spc-jds ├─ distribution builder → coinbase outputs                 │
   │ (sp_address) │──SV2 MP────────▶  module          └─ Redis streams: spc:slices, spc:blocks, spc:stats        │
   └──────┬───────┘                                                 │                                            │
          └── TDP ─▶ miner's bitcoin-core (JD mode)                 ▼                                            │
                                                     spc-pool :: payout ──▶ Postgres (slice, snapshot, ledger)  │
                                                     spc-pool :: api    ──▶ HTTP (public + b_scan-authenticated) │
   miner wallet ◀── spc-scan (header + merkle branch + coinbase ≈ 500 B/block) ◀── any block source ─────────────┘
```

**Components**

| Binary / crate | Role | Forked from |
|---|---|---|
| `spc-pool` | one binary, `--roles front,payout,api,ark` (Blitzpool pattern). `front`: SV2 listeners **and a native SV1 listener**, template stream, job building, share validation, slice aggregation, block submit. SV1 is pool-template only: its username is parsed with §2.1 and its shares enter the same pipeline (§3.1, §3.4). `payout`: slice persistence, confirmation-gated ledger apply, Ark send orders, scrub. `api`: read-only HTTP. `ark`: the pool's bark wallet and Ark float; executes signed send orders and nothing else (§11.3) | `sv2-apps/pool-apps/pool` (SV2 structure); `stratum-core` for codecs |
| `spc-jds` (**module of `spc-pool`'s `front` role**, not a separate binary) | Job Declarator Server implementing sv2-spec PR #203 (`0x0003`), publishing `SetPayoutDistribution` per height; JD is always on (decision 2026-10-13, matching the SRI pool's bundled JDS); PR #203 §10's "Pool and JDS are assumed to share state" is satisfied in-process | `sv2-apps` pool (JDS included) |
| `spc-agent` | the miner side, one binary: SV1/SV2 downstream listener and SV1→SV2 translation (the **translator role**, forked from `sv2-apps/miner-apps/translator`); optional Job Declarator Client with `0x0003` that builds outputs per PR #203 §4 (the **JDC role**, forked per roadmap Phase 4); own-payout verifier (§3.6); scanner and SP-aware Ark keystore (§11.5). `sp_address` config becomes the upstream `user_identity` | `sv2-apps/miner-apps/{translator,jd-client}` |
| `spc-crypto` | BIP 352 address parsing, `sp_id`, the SPc ECDH, the tier KDF (`t_onchain`, `t_ark`, `t_mb`), carriers, mailbox blinding, zeroising key types — every derivation in DESIGN.md / MAILBOX.md, used by pool and agent alike | new; absorbs the `spc_ark` and `spc_ark_mailbox` prototypes |
| `spc-scan` | library + CLI: find and export SPc payouts from blocks or header+branch+coinbase triples | new |
| `spc-accounting` | pure, I/O-free: slices, window, SLICE scores, pending ledger, denomination solver, autoscaler, output builder | new; ledger/solver/autoscaler re-implemented from §4.6/§4.8 (Blitzpool patterns, no code) |
| `spc-share-ext` | extension type 32 messages on current `stratum-core` codec; server + sampling verifier, v1 only | `demand-open-source/share-accounting-ext` |

**Miner-side consolidation (2026-10-12, made canonical 2026-10-13).** SRI ships the translator and JDC as separate binaries; this project ships them as **roles of one binary, `spc-agent`**: one process, one config, one seed. Where this spec says `spc-translator` or `spc-jdc` it means `spc-agent` running in that role.

**Native SV1 (2026-11-02).** The pool itself accepts SV1 connections on a separate listener (`[sv1]`, §6.3). This is a reach decision, not a second protocol implementation in the accounting sense: the SV1 username is parsed by the same §2.1 function and rejected fail-closed, SV1 is always pool-template mode (`delta = 0`, §4.1), and its shares use the same replay set, slice store and distribution builder as SV2. The `spc-agent` translator role remains for miners that want local fan-out, own-payout verification (§3.6) or a single upstream connection for several devices; it is not required to reach the pool.

**Process boundaries that matter.** Only `front` ever holds `a_send`; the `spc-jds` module is part of `front` (decision 2026-10-13), so JD-mode use of the key is an in-process share. Only `payout` writes Postgres. `api` reads Postgres and Redis. Only `ark` holds the `ark` seed (bark wallet and pool output keys), and `ark` sees no attribution data — only signed send orders carrying what the Ark server sees anyway (§11.3). SPc receiver privacy and Ark-tier funds are protected by disjoint secrets on disjoint hosts.

## 2. SPc cryptographic protocol (normative)

All hashes are BIP 340 tagged hashes. `ser_P(X)` is 33-byte compressed; `ser_x(X)` is the 32-byte x-coordinate; `ser32(n)` is 4-byte big-endian; `G`, `n` are secp256k1's. The derivation (§2.3) is DESIGN.md's, and its frozen vector is `spc_ark/src/lib.rs::test_vector`; the regenerated `sp-coinbase-vectors` cover carriers and negative controls.

### 2.1 Miner identity

```
addr          = user_identity with an optional ".worker" suffix stripped at the first '.'
(hrp, ver, B_scan ‖ B_spend) = bech32m_decode(addr)
REQUIRE hrp == "sp" (mainnet) | "tsp" (testnet4/signet/regtest), per pool network
REQUIRE ver == 0  (character 'q'); payload == 66 bytes; both keys valid compressed points
sp_id         = tagged_hash("SPc/Id", ser_P(B_scan) ‖ ser_P(B_spend))
```

Any failure → channel rejected (§3.1). A v0 address is 116 (`sp`) / 117 (`tsp`) characters, inside `STR0_255`.

### 2.2 Sending key schedule (pool side)

```
on SetNewPrevHash(prev_hash, H):
    a_raw  ← CSPRNG 32 bytes, reduce mod n, reject 0
    A_raw  = a_raw·G
    if has_odd_y(A_raw): a_send = n − a_raw else a_send = a_raw      # even-Y normalisation
    A_send = a_send·G                                                # ser_x(A_send) goes on the wire
    zeroise(a_send[H − 1])                                           # previous height's key is destroyed
```

- `a_send` MUST be generated from an OS CSPRNG. It MUST NOT be derived from any seed, xprv, descriptor, or earlier `a_send` — additive-tweak ladders (BIP 32 unhardened, `A_0 + H(·)·G`) let one secret regenerate every key and make erasure impossible ([08-16 L2](research/raw/notes/2026-08-16-ll-sv2-pool-tag-asend-carrier.md); vectors case 9).
- `a_send` MUST exist only in process memory (`Zeroizing<[u8; 32]>`); it MUST NOT be written to disk, logs, Redis, or Postgres. `ser_x(A_send)` MAY be stored (it is public on-chain anyway).
- One `a_send` per height per `front` process: `spc-jds` is a module of `front` (decision 2026-10-13), so the key is shared by reference and never crosses a process or machine boundary (never Redis). With one `front` per region (§9), each region has its own `a_send[H]`; that is safe — miners scan `A_send` from the coinbase carrier — and the key is RAM-only, so it adds no durable state (MULTI-REGION §3).
- A restart mid-height generates a new `a_send` for the same `H`. This is safe: outputs differ, both are scannable (§2.5). Reuse of one `a_send` across two heights is **not** a correctness failure (the height is in `input_hash`) but widens forward-secrecy exposure; `front` MUST alarm if `ser_x(A_send)` repeats across heights within its own sequence (cross-front repeats are detected retrospectively from the persisted `a_send_x`, MULTI-REGION §3).
- **Erasure vs deferred Ark payouts.** Ark sends happen `confirmation_depth` (6) blocks after the find, long after `a_send[H]` is erased at the next `SetNewPrevHash` (§11.3), so `front` MUST derive each Ark-capable id's Ark destination (`U_i` and its blinded mailbox id, §11.8) while `a_send[H]` is live, and put it in the distribution entries (§4.7), which travel in the `spc:blocks` event (§3.3). Only those public points are retained — never `ecdh_i`, `t_onchain` or `t_ark`: a stored tweak plus one leaked per-output secret (`u_i` or `p_i`) reveals the miner's base key (`b_ark` or `b_spend`).

### 2.3 Output derivation (pool → miner `i`, height `H`)

Normative per DESIGN.md "Derivation scheme". One shared secret per `(miner, H)`; every per-block destination on every tier is derived from it, domain-separated by hash tag alone (no extra tier bytes are hashed):

```
input_hash = hash_SPc/Inputs(ser32(H) ‖ ser_P(A_send))                 # ser_P(A_send) = 0x02 ‖ ser_x (even Y, §2.2)
ecdh_i     = ser_P(input_hash · a_send · B_scan,i)                      # pool side
           = ser_P(input_hash · b_scan,i · A_send)                      # miner side
t_onchain  = hash_SPc/TierSpc(ecdh_i ‖ ser32(k))      → P_i = B_spend,i + t_onchain·G    (coinbase tier)
t_ark      = hash_SPc/TierArk(ecdh_i ‖ ser32(k))      → U_i = B_ark,i   + t_ark·G        (Ark tier, §11.8)
output_i   = P2TR(ser_x(P_i)), amount ∈ ladder (§4.4)                                    # scriptPubKey: OP_1 <32 B>
```

With denomination payouts (§4.4), a miner `i` may receive several outputs per block; they are derived by incrementing `k` (`k = 0 … q_i−1`, same `ecdh_i`):

```
P_{i,k}    = B_spend,i + hash_SPc/TierSpc(ecdh_i ‖ ser32(k))·G      # the id's k-th output at H
```

- **Hash-to-scalar fails; it never reduces.** If `input_hash`, `t_onchain` or `t_ark` is 0 or `≥ n`, or any point result is infinity, derivation fails (BIP 352 semantics). The probability is negligible. Handling: `input_hash` is per-height, so on its failure `front` discards `a_send[H]` and draws a new one (§2.2); a tier tweak or point failure is per-id, so that id gets nothing on that tier at `H` and its `owed` is booked as pending. Never substitute a value.
- **`SPc/SharedSecret` is reserved** for a 32-byte digest of `ecdh_i` (`SharedSecret::digest`) and is **not** a tier-tweak input. `SPc/Mailbox` is the third tier tag (§11.8, MAILBOX.md variant B); `SPc/ArkIdentity` is the message tag for the `b_scan` signature on Ark registrations (§11.4).
- **Never reuse a scalar across tiers.** If `P = B_spend + t·G` and `U = B_ark + t·G` ever coexisted, `P − U` would expose `B_ark − B_spend`. The tags prevent it; do not shortcut them.

Design notes, each with its source:
- **Nonce = height.** Replaces `outpoint_L`, which is constant across all coinbases. Shared, non-circular, consensus-unique by BIP 34, and no gap limit applies because the receiver computes one candidate for the height in front of it rather than scanning forward ([08-14 L1–L2](research/raw/notes/2026-08-14-ll-coinbase-silent-payments-ecdh-nonce.md)).
- **`input_hash` commits to `A_send`.** This, not the nonce, is what defeats the `why_include_A` grinding attack (BIP 352 footnote; [08-24 L2](research/raw/notes/2026-08-24-ll-jd-coinbase-control-amount-linkability.md)). The pool chooses `a_send` freely every height, so it has exactly the freedom the attack needs — the commitment removes it.
- **Fresh `SPc/*` tags (DESIGN.md; supersedes the `BIP0352/*` reuse decision).** Domain separation is by tag, so no SPc value can collide with a BIP 352 value computed over the same bytes. The cost is that an unmodified BIP 352 library cannot compute `t_onchain`; whether `t_onchain` should instead use the upstream SPc/BIP 352 tag so a single-tier SPc scanner finds coinbase outputs is open item O12, and depends on the SPc BIP.
- **`k` enumerates an identity's outputs in a block.** Each miner has a distinct `B_scan`, so each is a BIP 352 group of size one; the accounting puts an id on exactly one tier per block (§11.1) and, with denomination payouts (§4.4), pays it `q_i` outputs at `k = 0 … q_i−1` (`q_i` can be large; `ser32(k)` in the preimage was reserved for exactly this). `K_max` bounds a block's outputs only through the §4.8 budget. The BIP 352 `:319` dropped-output footgun applies per `k`: miners scan `k = 0, 1, …` until the first miss ([08-14 L5, L7](research/raw/notes/2026-08-14-ll-coinbase-silent-payments-ecdh-nonce.md)).
- **No labels in v0.1.** A miner wanting several identities uses several addresses.

### 2.4 Carriers

**2.4.1 scriptSig (pool-template mode).** The coinbase input script MUST be exactly:

```
<BIP34 height push>  OP_PUSHBYTES_32 <ser_x(A_send)>  <extranonce push>
```

- Placed in `coinbase_tx_prefix` (SV2 §5.4.1.6: prefix ‖ extranonce_prefix ‖ extranonce ‖ suffix), so the key is **structurally unrollable** — the wire format, not discipline, keeps it out of the rolled bytes ([08-16 L5](research/raw/notes/2026-08-16-ll-sv2-pool-tag-asend-carrier.md)).
- Budget: `100 − 4 (height, 5 past 16,777,215) − 33 (push32) = 63 B` for the full extranonce; default allocation 16 B leaves ~4× headroom. No ASCII pool tag in v0.1 — `A_send` **is** the tag ("tag == entry H of pool X's published key" is unforgeable attribution). The 100 B / 400 WU are reserved by the Template Provider unconditionally, so marginal on-chain cost is 0.
- Implementation: `channels-sv2::JobFactory.pool_tag_string: Option<String>` becomes `pool_tag: Option<Vec<u8>>` in the vendored fork (the UTF-8 constraint is an SRI type, not protocol).

**2.4.2 `OP_RETURN` (JD mode).** The pool's `SetPayoutDistribution.additional_outputs[0]` MUST be a 0-value output with:

```
OP_RETURN  OP_PUSHBYTES_35  0x53 0x50 0x63 <ser_x(A_send)>        # "SPc" ‖ 32 B
```

Costs 46 B = 184 WU of real block space per block (8 value + 1 length + 37 script; not pre-paid — stated honestly; a `0x0003` JDC reserves exactly this TxOut size in `CoinbaseOutputConstraints`). Required because Coinbase-only JD carries no scriptSig bytes and Full-Template gives the pool approval but not specification authority over the scriptSig ([08-24 L3](research/raw/notes/2026-08-24-ll-jd-coinbase-control-amount-linkability.md)). Pool-template mode MUST NOT emit the SPc `OP_RETURN` carrier (it would be a protocol fingerprint with no benefit). 2026-11-08: the prohibition targets the SPc carrier script above, not `OP_RETURN` outputs in general - the BIP 141 witness commitment output (§3.3) is a zero-value `OP_RETURN` output and is not an SPc carrier.

### 2.5 Scanning (miner side; `spc-scan`)

```
for each block (header, coinbase, merkle_branch):
    REQUIRE merkle_branch proves coinbase at index 0 under header.merkle_root
    H  = parse_bip34_height(coinbase.vin[0].scriptSig)
    A  = scriptsig_push32_after_height(coinbase)            # carrier 1
         ?? op_return_SPc_payload(coinbase.vout)             # carrier 2
    if A is None: skip block (not SPc)
    for h in {H, H − 1}:                                     # tolerance, §3.5 R4
        input_hash = hash_SPc/Inputs(ser32(h) ‖ 0x02 ‖ A)
        ecdh = ser_P(input_hash · b_scan · lift_x_even(A))
        for k in 0..2048:                                   # ladder-entry-v2 representation bound
            t = hash_SPc/TierSpc(ecdh ‖ ser32(k))
            P = B_spend + t·G
            if OP_1 <ser_x(P)> ∈ coinbase.vout: record (txid, vout, k, amount, t)
        REQUIRE owned k values form one contiguous, duplicate-free sequence from 0
    if Ark tier enabled:                                     # DESIGN.md "Miner can scan both tiers"; h = H only —
        ecdh  = (as above, at h = H)                         #   the pool derives Ark destinations at the mined height
        t_ark = hash_SPc/TierArk(ecdh ‖ ser32(0))
        keystore.insert(U = B_ark + t_ark·G)                 # §11.5; u = b_ark + t_ark when the signer is present
        mailbox.open_epoch(H, ecdh)                          # coinbase discovery does not settle a remainder
```

The Ark tier is scanned for **every** block the pool found, including blocks where this miner got no coinbase output — that is how a sub-threshold miner learns which Ark keys to expect without any notification.

Light-client cost: header 80 B + branch (~12 × 32 B) + coinbase + 32 B ≈ 500 B/block, provable against the header ([08-14 L6](research/raw/notes/2026-08-14-ll-coinbase-silent-payments-ecdh-nonce.md), in-session arithmetic, medium confidence). Spending: `d = b_spend + t (mod n)`, negated if `P` has odd Y, exactly as BIP 352 §Spending. `spc-scan --export-tweaks` emits `(outpoint, t)` pairs for wallets that can import tweaks (BIP 352 §"Backup and Recovery" caveat: outputs found by notification but not by derivation are unsafe — ours **are** derived, so this is safe).

### 2.6 What this section does not do

Hides amounts (no), hides the payee set from the pool (no), hides the coinbase contents from the miner hashing it (impossible: outputs follow the extranonce in serialization order, [08-24 L4](research/raw/notes/2026-08-24-ll-jd-coinbase-control-amount-linkability.md)).

## 3. Stratum V2 surface

### 3.1 Channel open and identity (fail-closed)

On `OpenStandardMiningChannel` / `OpenExtendedMiningChannel`, `front` runs §2.1 on `user_identity` (`STR0_255`).

| Outcome | Response |
|---|---|
| valid SP address for the pool's network | `.Success`; `channel_id → sp_id` recorded; `sp_identity` upserted via `spc:stats` |
| anything else (bare bc1q…, descriptor, `sri/solo/…`, wrong HRP, bad checksum, empty) | `OpenMiningChannel.Error{ error_code: "invalid-user-identity" }` (`STR0_255`), connection kept open so the client can retry |

There is **no default payout mode and no donation fallback.** This is the direct consequence of the sv2-apps rotation branch, where an unrecognised identity became `PayoutMode::FullDonation` — the whole block to the pool, no log line ([xpub spec rev. 5](research/output/plan-xpub-miner-identity-spec-2026-07-29.md)). An integration test asserts the error and asserts no coinbase was ever built for that channel.

One connection = one `sp_id`. Extended channels on the same connection MUST present the same address (differing addresses on one connection → `invalid-user-identity`), because SRI's payout slot is per connection and group jobs never rebuild the coinbase per channel ([xpub spec rev. 5](research/output/plan-xpub-miner-identity-spec-2026-07-29.md)). Worker labels (`.worker`) are stats-only.

**SV1 channel open (native listener, 2026-11-02).** `mining.subscribe` returns the connection's `extranonce1` (default 4 bytes) and `extranonce2_size` (default 4); `mining.authorize` carries the identity in its username. `front` strips an optional `.worker` suffix at the first `.` and runs §2.1 on the remainder; any failure returns JSON-RPC error `invalid-user-identity`, sends no `mining.notify`, and leaves the connection open for a retry. Every `mining.notify` is pool-template mode: `coinb1`/`coinb2` carry the BIP 34 height push, `push32(ser_x(A_send))` and the extranonce split; the §2.4.1 budget MUST be checked at `mining.subscribe` and a connection whose `extranonce1 + extranonce2` does not fit is rejected before any job. `mining.submit` shares are validated, deduplicated and recorded exactly as §3.4, at the connection's current vardiff target. SV1 clients MUST NOT be offered `0x0003`, `0x0020` or `0x5350`, cannot run §3.6 own-payout verification, and are told the §11.1 forfeiture policy in the `mining.subscribe` response extensions.

Channel open establishes **identity only**. Delivery capabilities (§0) are registered separately, over the authenticated session, by the SPc extension (§11.4). A channel with none is valid, but under the pool's published policy its earnings below `C` are **forfeited** (§11.1); only balances at or above `C` are paid, in the coinbase. The policy is advertised in the `0x5350` negotiation response and at `/api/pool`, and `spc-agent` does not start submitting shares until `SetArkIdentity.Success` (§11.5), so the default setup never forfeits.

### 3.2 Extension negotiation

Clients that want JD payouts or auditing send `RequestExtensions{request_id: U16, requested_extensions: SEQ0_64K[U16]}` (ext `0x0001`) immediately after `SetupConnection.Success`. `spc-pool` advertises:

| ID | Extension | Mandatory for |
|---|---|---|
| `0x0001` | extensions negotiation | everyone (`spc-translator`, `spc-jdc`) |
| `0x0003` | Coinbase Transaction Payouts (PR #203) | `spc-jdc`, on **both** its JDP and MP connections |
| `0x0020` (32) | Share Accounting (§3.7) | optional in v1 and requested by `spc-translator`/`spc-jdc` by default; **MUST NOT** be advertised or accepted in v2 |
| `0x5350` | SPc delivery capabilities (§11.4) | optional; `spc-agent` requests it when its Ark tier is enabled |

A client whose `RequestExtensions.Error` lists `0x0003` MUST NOT run JD mode (PR #203 §2).

### 3.3 Pool-template mode: job construction

Per `NewTemplate` (and per `SetNewPrevHash`) `front`:

1. Looks up `a_send[H]` (§2.2).
2. Asks `spc-accounting` for `Distribution(H, T = coinbase_tx_value_remaining, A_send)` (§4.7) → ordered outputs `[pool_output, P2TR(P_i)…]`.
3. Builds `coinbase_tx_prefix = version ‖ vin_count ‖ null_prevout ‖ scriptSig_len ‖ height_push ‖ push32(A_send)` and `coinbase_tx_suffix = sequence ‖ outputs ‖ locktime`, honouring `coinbase_output_max_additional_size` reserved from the TP (`spc-pool` sets it from the current weight budget, §4.8, Blitzpool's per-stream reservation).
4. Sends `NewExtendedMiningJob` / `NewMiningJob`. Standard channels receive the merkle root only; they cannot verify payouts (§3.6).
5. XADDs a `spc:blocks` event `{block_hash, height, front_id, fingerprint, high_water, window_target, entries, work_unit_version, work_unit_fingerprint, window_rule}` where `high_water` is the window's high-water vector (§4.3), `window_target` is the exact `N * d` (v1) or `Q(T)` (v2) the window was selected with (persisted by settlement, §7.4), `work_unit_version`/`work_unit_fingerprint` select the profile, `window_rule` is `0` (`share_work_legacy`) or `1` (`network_block_work`), and `entries` is the winning distribution's full entry list, which the finding front holds in RAM for the jobs it issued. Settlement reads the event payload; no Redis snapshot key. The distribution fingerprint (sha256 of the distribution) serves for deduplication/idempotency; the work-unit fingerprint is the reference-target profile hash, not a distribution hash.

Distribution recomputation is bounded: weights change with every share, but the output **keys** `P_i` are fixed per `(sp_id, H)`, so only amounts move. `front` rebuilds amounts on each template (every ~30 s) and never per share.

**Job lifecycle and stale policy (2026-11-05).** A `(prev_hash, nbits)` change
publishes exactly one future `NewExtendedMiningJob` (empty `min_ntime`) followed
by one `SetNewPrevHash` naming it; a same-prevhash `nbits` change is included
because SV2 carries header bits only in `SetNewPrevHash`. For unchanged
`(prev_hash, nbits)`, fee/distribution/transaction/version changes publish only
a nonfuture job. Ordinary GBT curtime advancement alone does not change the
job or its immutable issued `min_ntime`; version/ntime rolling remains subject
to the existing validation bounds. Only the newest job is accepted: any older
job, including an older same-tip job, is rejected as `stale-share`, and every
SV1 republication sets `clean_jobs = true`. Delivering a new job while a prior
share outcome is pending is a scheduling detail; it does not change
latest-only acceptance, activation order or per-job credit.

**Witness commitments (2026-11-08).** Every template transaction is
strict-decoded - full-transaction deserialization with trailing bytes
rejected, plus a byte-exact re-serialization - and analyzed into txids,
wtxids and a witness count; a non-canonical encoding (a segwit marker/flag
with empty witness, or the zero-input serialization ambiguity) fails the
refresh. There is no best-effort skip and no fallback. When the transaction
set carries witness data, the coinbase gains exactly one additional
zero-value output appended last - the 38-byte BIP 141 commitment
`OP_RETURN 0x24 0xaa21a9ed SHA256d(witness_root || reserved)` - so the pool
payout output stays at index 0 and consumer indices are unchanged. The
witness root is the merkle fold of the wtxids with the coinbase leaf fixed
at zero. BIP 141 leaves the reserved value unconstrained (any 32 bytes
commit); the pool draws it from the OS RNG once per witness root, not per
republication - a pool-side share-stability choice, not a protocol
obligation: a refresh that republishes an unchanged transaction set keeps
the commitment, so shares already issued against that job stay valid.
BIP 141's consensus rule recognizes a commitment output by the 6-byte
`6a24aa21a9ed` prefix with a script of at least 38 bytes and, where
several outputs match, takes the highest output index; the pool's policy
for its own coinbases is deliberately stricter - exactly one well-formed
38-byte zero-value commitment output, appended last, with duplicate or
malformed commitment-shaped outputs rejected outright - and parsing
arbitrary node-accepted blocks under the lenient consensus rule is
crosschecked against a real node in T4b, not assumed here. The commitment
joins the job-change comparison: same-txid witness
malleation moves the wtxid, hence the witness root and the commitment, and
republishes the job even though the non-coinbase txids and the merkle branch
are unchanged. A winning share on a witness job assembles the block by
strict re-verification: every transaction strict-decoded, exactly one
zero-value commitment in the coinbase, and that commitment equal to the one
recomputed from the actual transaction set. A wrong reserved value or a
missing, duplicate, malformed, valued or mismatched commitment rejects the
block, which is never submitted. The injected coinbase witness (marker/flag
after the version, one 32-byte item before the locktime) leaves the coinbase
txid unchanged, so shares hashed over the stripped coinbase remain valid
evidence. Witness-free templates are byte-for-byte unchanged. See
docs/decisions/2026-11-08-pool-witness-commitment.md.

### 3.4 Share handling

Share target validation is versioned. **v1** follows changelog (d) and
`docs/decisions/2026-11-02-share-target-policy.md`: every ingress validates a
share against `effective_target = max(target_for_difficulty(share_difficulty),
network_target)`. SV1 sends `mining.set_difficulty` and rejects a submission
below its target (`low difficulty share`); SV2 announces `effective_target` in
`OpenExtendedMiningChannelSuccess.target` and rejects with
`SubmitSharesError{difficulty-too-low}`. The front re-checks the target before
accounting.

**v2** is the opt-in `network_block_work` profile (§4.1). It uses the exact
reference family `R = 2^k - 1`, configured multiplier `m_cfg = 2^j`, issued
target `S(2^j_issue) = 2^(k-j_issue) - 1`, and credits exactly `d = 2^j_issue`
(no float, no clamp, and no legacy `share_difficulty` key). The issued target
MUST be no harder than the job's network target; a target outside the family
or below the network target fails the generation rather than being rounded.
v2 is restricted to `pool.network = regtest` and rejects SV1 ingress. The front
publishes `work_unit_version = 2`, the nonzero reference fingerprint, the
exact target and `m_issue` with the job; a later template or multiplier update
allocates a new generation and never reprices an already issued job.

Validate as SRI does (target, job, ntime, version mask), then:

```
key = sha256(channel_id ‖ job_id ‖ nonce ‖ ntime ‖ version ‖ extranonce)
REQUIRE key ∉ seen_shares                         # replay; identity-free nullifier (wiki: Nullifier vs Pseudonym)
rec = ShareRec{ sp_id, d: share_difficulty, job_fees, job_id, nonce, ntime, version, extranonce, share_index }
open_slice.push(rec)                              # §4.2 sealing rules may seal first
reply SubmitShares.Success (or ShareOk{ref_job_id, share_index} in v1 when 0x0020 is active; v2 never activates it)
```

`seen_shares` is per process and is lost on restart; `front` MUST reject shares whose `job_id` predates process start (jobs are re-issued on start), which bounds the replay window to zero at the cost of a few lost shares ([share accounting & durability](research/sibling/mining-pool-architecture/wiki/concepts/share-accounting-and-durability.md)).

SV1 shares take this exact path (2026-11-02): the SV1 `mining.submit` is converted to the same `ShareRec` with the connection's `sp_id`, `job_id` derived from `(H << 32) | sv1_job_seq`, `job_fees = ref_fees` (pool-template), and `extranonce = extranonce1 ‖ extranonce2`; the nullifier key is computed over that concatenation. There is no SV1-specific slice, ledger or distribution code path.

### 3.5 JD mode (PR #203, extension `0x0003`)

**Publication.** `spc-jds` sends `SetPayoutDistribution` as the first message after negotiation and again:
- **always** on `SetNewPrevHash` (new `H` ⇒ new `A_send`, new `P_i`), and
- on a timer (`distribution_interval_secs`, default 60) when weights moved by more than `distribution_rebalance_bps` (default 100 = 1 %), and
- on **ledger apply** (booking at `confirmation_depth`, §7.4): force a republish regardless of `distribution_rebalance_bps` so pending credits reach `payouts` immediately, and drop the grace slot — the immediately preceding id FROM THE SAME FRONT on that connection is rejected after this republish (PR #203 §10: the grace window SHOULD NOT span a settlement event; Blitzpool's `invalidate_all_distributions()` + forced publish does exactly this at booking, not at found-time). Found-time invalidation is already satisfied by the `SetNewPrevHash` bullet plus the same-height rule: every id published before the pool's own block belongs to the previous height and is rejected.

```
SetPayoutDistribution {
  distribution_id:    U64   strictly increasing, per front: `id = front_id << 56 | local_seq`, `local_seq = max(last_local_seq + 1, unix_time_ms)`
  pool_payout:        TxOut{ script: P2TR(pool_payout_descriptor @ H), value: weight_P }   # weight, not sats; fresh per height
  payouts:            [ TxOut{ script: OP_1 ser_x(P_i), value: weight_i } ] # one per sp_id kept by §4.8
  dust_limits:        [ 330 … ]                                             # per payouts[i]; P2TR dust
  additional_outputs: [ TxOut{ script: OP_RETURN "SPc" ser_x(A_send), value: 0 } ]
}
```

`weight_P` = pool fee weight **plus** the weight of every identity not represented in `payouts` (below the payout floor, pruned), per PR #203 §3.1; `payout` books those per §11.1 (Ark tier or error-held pending) when the block settles.

**Validation (`spc-jds` and `front`).** On `DeclareMiningJob` (Full-Template) or `SetCustomMiningJob` (Coinbase-only) carrying the `distribution_id` TLV (`0x0003`/`0x01`, 8 B): recompute `pay[i] = floor(weight_i · (T − F) / W)`, prune `< dust_limits[i]`, `pay_P = T − Σ pay[i] − F`, assert output order `pool_payout, payouts…, additional_outputs, JDC 0-value outputs, TP outputs`; mismatch → `invalid-payout-distribution`. Unknown/old id → `stale-payout-distribution`.

**Grace window (R4 in the roadmap).** `spc-jds` MAY accept the immediately preceding id FROM THE SAME FRONT on that connection **only if it was published at the same height `H` by the same front on the same connection**. A declaration built for `H` and accepted at `H + 1` would pay outputs nobody can scan at `H + 1` (`input_hash` differs). Height-straddling declarations MUST be rejected with `stale-payout-distribution`. `spc-scan`'s `{H, H − 1}` tolerance is belt-and-braces for third-party pools, not a license to relax this rule. Ids are not comparable across fronts.

**Shares under JD.** Carry `job_fees` from the declared job's template value into `ShareRec`; this is where `score_f` becomes non-trivial (§4.4).

**Declaration answers (fail closed; 2026-11-03).** `spc-jds` MUST answer every parseable `DeclareMiningJob` (JDP) or `SetCustomMiningJob` (the MP custom-job path the same module serves) with the base protocol's `.Success` or `.Error`; it is silent only on frames that fail Noise/codec decode, where no `request_id` exists to answer. Codes live in the `.Error` `error_code` (`STR0_255`); the first four are project codes under the negotiated `0x0003` extension, in the spirit of PR #203 §7:

| Condition | `error_code` |
|---|---|
| Malformed declaration (undecodable coinbase, missing or duplicate `distribution_id` TLV) | `invalid-job` |
| No distribution published on this connection yet | `no-payout-distribution` |
| Unknown id, id outside the grace slot, or a height-straddling declaration | `stale-payout-distribution` |
| Recomputed outputs mismatch the declaration | `invalid-payout-distribution` |
| Pool-side internal inconsistency | `internal-error` (upstream code) |

A coded decline is a protocol message, so a client can always distinguish it from a transport error.

**JDC fallback (2026-11-03; `docs/decisions/2026-11-03-jd-fallback.md`).** The JDC declares at most once per declaration key `(distribution_id, template)` — "template" is the whole job template: height, `coinbase_tx_value_remaining`, extranonce length and the transaction list (`wtxid_list`). After sending `DeclareMiningJob` it waits up to `[jd] declare_timeout_secs` (§6.3; default 10, fail-closed minimum 1) for the matching `.Success`/`.Error`, retaining any distributions published meanwhile. On a **hard decline** (`invalid-job`, `invalid-payout-distribution`, `internal-error`, or any unknown code) or on **silence** past the window, the JDC latches the fallback: it stops declaring, alarms once per episode (error level), and stays on the connection; on a **transient decline** (`stale-payout-distribution`, `no-payout-distribution`) it does the same at warn level — those are expected at height races. Recovery is event-driven only: a *changed* declaration key (a new `SetPayoutDistribution` — published on every `SetNewPrevHash`, on rebalance, on ledger apply — or a new template) triggers exactly one fresh declaration. There is no retry timer and no per-loop alarm, so a dead JDS latches exactly once and is retried only when an input moves. JDP carries no mining jobs and PR #203 defines no pool-template mode on it, so there is no in-session fallback work source; `spc-agent` operators who want continuity run the translator role, whose MP pool-template stream is unaffected by the JDC latch. A template move (the JDC's own TP delivering a new prev-hash/height) abandons the outstanding declared job with a logged reason (`new-prev-hash` or `distribution-replaced`) and the JDC rebuilds the whole template — coinbase and `wtxid_list` — against the newest key; JDP has no cancel message and none is needed, because the grace rule above makes the abandoned job unacceptable to the pool.

**Leak, stated.** Every JDC sees `payouts` (one-time `P_i` scripts, no identity) and relative weights. Weights are derivable from on-chain amounts anyway; the residue a retaining JDC gains is distributions for blocks nobody won and the sub-dust set (PR #203 §9.2; [08-24 L5](research/raw/notes/2026-08-24-ll-jd-coinbase-control-amount-linkability.md)). `max_payout_slots` (default 256) is the only dial.

### 3.6 Client-side payout verification (`spc-translator`, `spc-jdc`)

**Audited warmup (2026-11-10).** A source-verified pool-template job MAY activate
with an empty or underfilled window only under §4.3's complete-history rule.
This is not an own-output absence bypass: the same contextual fees, coinbase
conservation, Merkle/witness, Rule-B prior and complete owned k-set checks run.
Wallet-owned verification signs derived Q/actual-work/shortfall/underfilled
facts with the job/generation and source/checkpoint digests before collector
commit (WIRE §3.10). Full measured N-block/T3 acceptance still requires full Q.
The native gate consumes public evidence; keys remain in the existing wallet.

Both clients know `(b_scan or B_scan, B_spend)`, `H`, and `A_send` (from the job's `coinbase_tx_prefix` or the distribution's `OP_RETURN`). Before mining a job they MUST derive their own `P_{i,k}` (§2.3, receiver side; scan `k` from 0 until the first miss) and check the outputs carry the denomination shape of §4.4 — the sum over `k` must equal exactly the greedy-decomposition value of `owed` over the constant published rungs (§4.4), with `owed` recomputed from the v1 type-32 window (v2 uses the pool's frozen v2 window; v2 does not serve type-32) plus the prior error-held balance (any ladder ambiguity or absent publication is a hard reject; the rungs are four constants shared by every party, so no height or template data feeds the check). Result handling is `require_own_payout = warn | refuse` (default `warn`, because a sub-base miner legitimately has no coinbase output some blocks). The decomposition value — not `owed` — is what "correctly paid" means on the coinbase leg; the remainder leg is verified per §11.6. This is the "pre-block payout verification via the existing encrypted session" the wiki lists as engineering-solvable ([08-24 L6](research/raw/notes/2026-08-24-ll-jd-coinbase-control-amount-linkability.md)), and it is also how `spc-translator` detects at startup that it is pointed at an SPc pool at all (no `push32` after the height ⇒ abort with a loud error).

The check gates the JD fallback too (§3.5): under `refuse`, a distribution whose coinbase would omit the miner's `P` — or whose missing carrier makes the check impossible — is never declared; the JDC enters the same latched fallback having sent nothing, so no path exists on which the miner silently hashes a job that cannot pay it. Under `warn` the declaration proceeds and the warning stands.

### 3.7 Share Accounting extension (type 32 = `0x0020`)

Served by `front` from the slice store (§5) in **v1 only**, message set and semantics per `extension.md` @ c661e93 with the **documented deltas** below (`docs/share-accounting-ext-deltas.md`). `front` serves type-32 for any pool block from share blobs obtained via the `payout` bootstrap RPC (WIRE-FORMATS §3.5 `type32_blobs`) plus the live `spc:slices` tail; there is no other cross-front share-blob path. In v2 type-32 is unsupported: it MUST NOT be advertised, requests receive `Type32Message::Error{code: "unsupported-extension"}`, and no v2 slice may populate the type-32 store. The bootstrap payload MAY still contain identity-free blobs for v2 `window_tail` rows because the payout export covers every row; the v2 front ignores them.

| Item | upstream | silent-pool |
|---|---|---|
| activation | `Activate`/`Activate.Success` (handshake TODO, sv2-spec #95) | negotiated via `0x0001`; `Activate` accepted as a no-op for compatibility |
| codec | demand fork of `roles_logic_sv2` | current `stratum-core` (`binary-sv2`, `codec-sv2`) |
| type names | `GetTransationsInJob`, `ProvideMissinTransactions` | spelled correctly; wire bytes unchanged |
| `Slice.job_id` | "last 4 bytes MUST be the same as the job's" | `job_id = (H << 32) | mining_job_id`, satisfies the rule |
| window older than `retain_windows` | unspecified | `GetWindow.Busy{retry_in_seconds: 0}` means *gone*; documented |

Server behaviour: `NewBlockFound{block_hash}` broadcast on every pool block; `GetWindow` → slices + phashes for the window (§4.3); `GetShares` → `Share{nonce, ntime, version, extranonce, job_id, reference_job_id, share_index, merkle_path}` from the slice's share blob; `GetTransactionsInJob` → pool's tx set (pool-template) or the JDC's declared set (JD). `GetWindow.Busy{retry_in_seconds}` for windows older than `window_hot_blocks` (default 6).

Client verifier (shipped in both clients and as `spc-scan audit`): sample `s` slices × `m` shares (defaults 4 × 8), check PoW, `merkle_path(share) + share_hash == slice.root`, `Σ d ≤ slice.difficulty`, `share.job_fees ≤ slice.fees + δ`. Statistical, not total — a cheating pool is caught over windows, not per window ([SV2 Share Accounting Extension](research/wiki/concepts/sv2-share-accounting-ext.md)).

### 3.8 Transport and reach (Tor)

Tor is a **transport option, not a protocol mode**. It changes no message, no encoding, no accounting and no key: every listener in §3.1-§3.7 is already TCP, and the SV2 Noise session, the SV1 session and the JDP session are carried unchanged. The two directions are independent and either MAY be used alone.

**Inbound (pool reachability).** The `front` listeners (SV2 `[pool] listen`, native SV1 `[sv1] listen`, JDP `[jd] listen`) and `[api] listen` MAY be published as Tor onion services. The Tor daemon owns the onion key and forwards to the listener; the binary MUST NOT create, sign or manage onion services. `[reach] onion` (§6.3) maps each listener to its advertised `<id>.onion:<port>` and `[reach] advertise` lists the endpoints published verbatim at `GET /api/pool` (§6.1), so a miner can learn the onion address from the clearnet API. **Binding rule (fail closed):** if a listener has a non-empty `[reach] onion` entry, it MUST bind a loopback address and `spc-pool` MUST refuse to start otherwise. An onion-published listener still reachable on a clearnet interface is a silent bypass of the privacy the onion service is supposed to give, so the check is made once at startup rather than trusted to operator discipline.

**Outbound (mining connections only).** `spc-agent`'s upstream SV2 connection and its JDP connection to `spc-jds` MAY be routed through a SOCKS5 proxy (`[net] proxy`, §6.3; agent only - `spc-pool` has no `[net]`). Rules:

- The URL MUST select remote name resolution (`socks5h://…`, or an equivalent mode in the chosen client). Resolving a pool name locally before handing it to the proxy leaks the query and MUST be rejected at startup.
- With `[net] stream_isolation = true` (default whenever `proxy` is set) each logical connection MUST present a distinct SOCKS username/password so the Tor daemon assigns it a separate circuit.
- A `.onion` target with no `proxy` is a configuration error, not a runtime fallback: `spc-agent` MUST refuse to start. There is no direct route, and there is no clearnet fallback by design.
- `[net] proxy` never applies to `[storage]` (Postgres/Redis) or to the template source. Those are on the trusted side of the deployment boundary; proxying them adds failure modes and hides a misconfiguration.
- The downstream SV1 listener (`[downstream] listen`) is unaffected and stays plain TCP on the local network: ASIC firmware does not speak Tor, and the agent is the anonymity boundary for its own ASICs. `spc-agent` MUST NOT silently mix a proxied upstream with a direct one.

**Ark egress is out of scope (2026-11-04).** The `ark` role's dial to the third-party Ark server and the agent's Ark-server/mailbox traffic are **direct**: no proxy, no Tor, and no startup rule that keys a `.onion` Ark endpoint to a proxy. The accepted residual is that the Ark server and the mailbox endpoint see the miner's (or pool's) network address; a miner who needs that hidden uses an operator-level network control. Ark delivery privacy is bounded by the Ark server's view as stated in §11.7, not by Tor.

**What Tor does and does not buy.** It removes the transport peer (source IP/port, ASN, reconnect co-timing) from the pool's view — leak vector 6 of [Payout Attribution Privacy](research/wiki/concepts/payout-attribution-privacy.md). It does **not** hide the miner from the Ark server or the mailbox endpoint: those connections are direct (§3.8 outbound scope), so residual leak (b) of `2026-10-14-mailbox-default-epoch.md` remains. It does **not** change share timing, hashrate signature, the amount channel, or anything the pool learns by validating shares (§0); it MUST NOT be described as anonymity from the pool. Noise is unaffected end to end (the exit node is a byte pipe), and `authority_public_key` means no CA or TLS name is involved on either path.

**Costs, stated to miners.** In pool-template mode job delivery is one-way, so the added RTT costs well under 0.1 % of hashrate; the material cost is circuit churn and its reconnect transient (new `extranonce1`/`extranonce2` and a fresh VarDiff ramp after each reconnect). JD mode is more sensitive because declarations and template distribution are per height; `spc-agent` SHOULD keep the JDC role on a direct connection unless the miner explicitly asks for Tor, and MUST NOT fall back silently.

**Abuse controls.** Onion clients reach `front` from the daemon's loopback address, so every peer-IP-based control is inert: connection concurrency caps, per-`sp_id` share-rate limits and `spc_channel_reject_total{reason}` accounting MUST NOT rely on the source address. §3.1's keep-open-on-`invalid-user-identity` behaviour is bounded by the concurrency cap, not by IP reputation.

**Multi-region.** One onion service terminates on one `front`, so a single advertised address defeats the geo split (§9, MULTI-REGION.md). A multi-front deployment MUST either publish one address per front in `[reach] advertise` (and let `spc-agent` rotate on failure) or front the set with OnionBalance behind one address. The onion key belongs to the front and MUST be backed up with that front's other secrets; losing it changes the pool's public address.

## 4. Accounting (normative; `spc-accounting`, pure functions)

### 4.1 Parameters

| Name | Default | Meaning |
|---|---|---|
| `N` | 8 | v1 window target `= N × d`, `d` = credited share difficulty (changelog (d)); v2 threshold is `Q(T)` network-reference units (§4.3). TIDES and SLICE both use 8, [PPLNS-JD](research/wiki/concepts/pplns-jd.md) |
| `work_unit_version` | 1 | `1` frozen legacy (`share_work_legacy`) or `2` exact reference work (`network_block_work`, opt-in regtest only) |
| `work_reference_target` | none | v2 only: `R = 2^k - 1`, `1 <= k <= 255`; `R + 1` MUST be an exact power of two and `R >= pow_limit` |
| `share_work_multiplier` | none | v2 only: `m_cfg = 2^j_cfg`, `0 <= j_cfg <= k - 1`; the issued `m_issue` is `min(m_cfg, 2^(k - ceil_log2(T+1)))` |
| `max_work_multiplier` | `2^32` | v2 hard cap; must be `>= m_cfg` and `<= 2^32` |
| `max_window_target_units` | `2^64` | v2 hard cap on `Q(T)`, decimal u128 string; larger Q fails `window_target_too_large` |
| `window_rule` | `1` for v2, `0` for v1 | `0` = `share_work_legacy` (`N * d`), `1` = `network_block_work` (`Q(T)`); explicit mismatch is an error |
| `pow_limit_bits` | pinned `0x207fffff` | v2 effective regtest consensus value; any other value is rejected. The build does not dynamically query/override a different consensus value. |
| `δ` (`slice_fee_delta_sats`) | 0 in pool-template mode; 50 000 under JD | new slice when a share's job fees `≥ f_ref + δ` |
| `max_slice_shares` | 65 000 | hard seal (keeps `GetShares` payloads bounded) |
| `fee_bps` | 100 | pool fee, taken from both subsidy and fees |
| `dust_limit_sats` | 330 | P2TR dust |
| `coinbase_threshold_sats` (`C`) | = `dust_limit_sats` | JD-mode `dust_limits[i]` floor and the rung sanity floor (every rung MUST be `≥ C`). MUST be `≥ dust_limit_sats` |
| `ladder_base_rungs_sats` | `[500000, 1000000, 3000000, 5000000]` | rungs for pool-template coinbase decomposition (§4.4, FINAL decision 2026-11-03): **constant, forever** — no era scaling, no re-parameterization; greedy largest-first; remainder < smallest rung to Ark |
| `coinbase_weight_budget` floor / ceiling | 50 000 / 1 000 000 WU | autoscaler bounds; ceiling caps the coinbase at 25 % of block weight |
| `max_payout_slots` | 1024 | **policy cap, not a weight limit** (2026-11-03): era-1 value bound `T_owed/500k = 619` (26.6 KB, 106k WU); 1024 outputs = 44.0 KB / 176k WU; 2048 (88.1 KB / 352k WU) is the documented fee-spike headroom setting |
| `retain_windows` | 16 blocks | v1 slice/type-32 retention horizon; never applied to v2 rows |
| `scrub_after_blocks` | 100 | delete `sp_id → P_i` / `U_i` snapshot entries this many blocks after the block is fully settled — booked **and**, for Ark-tier entries, sent (§11.3). v2 `slice_credit` uses the separate 4096-height cutoff, not this value. |

### 4.2 Slices

```
ShareRec { sp_id, d: u64, job_fees: u64, job_id: u64, nonce, ntime, version, extranonce: [u8;≤32], share_index: u32 }
Slice    { prev_hash, H, ref_job_id, ref_fees, shares: Vec<ShareRec>, difficulty: u128, root: [u8;32],
           work_unit_version: u8, work_unit_fingerprint: [u8;32] }

seal_if_needed(open, incoming):
    if incoming.prev_hash != open.prev_hash        → seal (new H)
    or incoming.job_fees ≥ open.ref_fees + δ       → seal (fee band exhausted; lorbax: MMEF only grows within a round)
    or open.shares.len() == max_slice_shares       → seal
seal(open): open.root = merkle_root([share_leaf(s) for s in shares]); emit to spc:slices
share_leaf(s) = sha256d(ser(nonce ‖ ntime ‖ version ‖ extranonce ‖ job_id ‖ share_index))   # identity-free
```

v2 `Slice::push` and aggregate construction use checked accumulation for
`sum(d)`, `d * f`, and each per-identity/slice `diff_fee_sum`, returning the
named accounting error on overflow; v1 keeps its frozen saturating path.

Leaves contain no `sp_id`, so `GetShares` responses and retained share blobs carry no identity (the extension's types have zero identity fields — [SV2 Share Accounting Extension](research/wiki/concepts/sv2-share-accounting-ext.md)). Identity lives only in the per-slice aggregates (§5.1 `slice_credit`), which are what get scrubbed. v2 slices additionally carry the definite `work_unit_version = 2` and a nonzero reference fingerprint; v1 rows carry version `1` and the 32-byte zero sentinel on the wire (NULL in SQL). A slice whose stored version and fingerprint do not form one valid profile is never reinterpreted or mixed into another profile; a distribution or block additionally carries `window_rule` and must agree with the slice profile.

### 4.3 Window

The window rule is selected by the **stored** `window_rule`; a live window is
never mixed across versions. Both rules exclude the finding slice, walk newest
first, take whole slices until the threshold is reached, include the slice that
crosses it, and compute the same `high_water = {front_id:
last_slice_stream_id}` vector.

**v1 (`share_work_legacy`, frozen).** The target is `N * d`, where
`d = min(share_difficulty, max(1, round(network_difficulty)))`; the same `d` is
used in `ShareRec`, the window target and the slice aggregates (changelog (d),
`docs/decisions/2026-11-02-share-target-policy.md`). Arithmetic remains the
existing saturating path. Existing v1 rows, windows and distributions are never
converted to v2.

**v2 (`network_block_work`).** With v2 reference target `R`, network target
`T` decoded from the finding job's `nbits`, and `N`:

```
Q(T) = ceil( N * (R + 1) / (T + 1) )      # reference units, exact then one ceiling
```

For `N = 8` and `R = 2^255 - 1`, `Q = 65` on the `0x200fffff` regtest profile
and `Q = 9` on the `0x207fffff` pow limit (floors 64 and 8). The ceiling is
deliberate: `floor` could select less than N blocks of work. The rounding error
is strictly below one reference unit, and whole-slice overshoot is reported
separately.

The window accumulates whole slices until the stored `sum(difficulty) >= Q(T)`.
The finding slice is still excluded, and the crossing slice is included.
`Q(T)` is persisted in the distribution snapshot and settlement never
recomputes it from later bits. `Q > max_window_target_units` fails with
`window_target_too_large`; exhausting the bounded history before Q leaves
`window_underfilled = true` and must not be described as N-block coverage.
`underfilled`, `actual_sum` and `overshoot` are derived from `high_water` and
the persisted target, never stored columns:

```
actual_sum = sum(slice.difficulty for slices at/below high_water)
overshoot  = max(0, actual_sum - window_target)
underfill  = max(0, window_target - actual_sum)
```

**Complete-history warmup.** A complete independently verified source/economic
checkpoint may prove all eligible available history is known but sums below Q.
Select ALL those completed slices, excluding the finding/open slice; use actual
selected work and slice count in §4.4. Never lower Q, node bits or N, fabricate
shares/work, trim entitlement, or claim full-Q coverage. Empty history needs
proof of emptiness, including high-water entries for every participating front
(zero for a known empty front). Missing or unproved-pruned history is UNKNOWN.

For proven empty history the existing raw adapter assigns raw=0 per identity
and raw pool_sats=T. Verified available prior pending is still added to owed
and follows existing routing/conservation/error-ledger rules, never erased for
bootstrap. An empty-window on-chain pending payout violating the existing
actual-coinbase bound still rejects when full rung payments exceed T. Gross
pool_sats=T is not a floor when repaying old P. With no prior entitlement the actual coinbase
is pool-only T apart from validated zero-valued carrier/witness outputs.
Legitimate retained sub-base pending remains owed. Derived underfill facts are
proof/ACK annotations, not a new stored accounting rule or enum.

The live v2 RAM tail retains every eligible stored-v2 slice up to the explicit
4096-slice hard cap, using the template height (`tip + 1`) so the RAM cutoff is
one block stricter than the SQL `tip - horizon` cutoff. It never retains a
slice older than the SQL eligibility horizon, and current-Q selection still
uses the whole crossing slice. This bounded headroom exists for a future
harder/larger-Q window; it is not unbounded retention.

Whole-slice granularity makes a v2 window at least `Q(T)` units and may exceed
it by less than one reference unit plus the crossing slice; that overshoot is
reported, not hidden. Block detection remains the consensus target.

### 4.4 Scores and payout (lorbax formula)

For the found block with net subsidy `r_net = r − ⌊r·fee_bps/10⁴⌋` and net fees `F_net = F − ⌊F·fee_bps/10⁴⌋`, over window slices `S₁…S_t`:

```
D_win        = Σ_{s ∈ window} d_s
DF_S         = Σ_{s ∈ S} d_s · f_s                         (per slice)
raw(id)      = ⌊ r_net · Σ_{s∈id} d_s / D_win ⌋
             + Σ_S ⌊ (F_net / t) · Σ_{s∈id, s∈S} d_s·f_s / DF_S ⌋
```

Equivalently `payout(sᵢ) = r·score_d(sᵢ) + F_S·score_f(sᵢ)` with `F_S = F/t`, `score_d = dᵢ/D_win`, `score_f = dᵢfᵢ/DF_S` (lorbax; gap research). All arithmetic in `u128`; floors per term; remainders fall to the pool output. **Pool-template collapse**: with `δ = 0` every share in a slice has `f_s = f_ref`, so `score_f ≡ dᵢ/D_S` and the fee term becomes a difficulty-weighted split of `F_net` — plain PPLNS, same code path.

**v1 arithmetic is frozen** with its existing saturating behavior and
interpretation. **v2 arithmetic is checked**: slice `sum(d)`, `d * f`,
`diff_fee_sum`, `D_win`, the `r_net`/`F_net` products and every final
conversion fail with a named error on overflow or division by zero instead of
saturating or guessing. A v2 restore or distribution build that would mix
versions/fingerprints fails closed; an already frozen distribution settles with
its own persisted rule/target/profile. That explicit `owed` value is the
settlement/audit authority; it is not a production-policy change, and the
denomination ladder remains the separate `DESIGN.md`/2026-11-03 policy (the lab
feature does not amend production payout policy).

**Denomination payouts (FINAL decision 2026-11-03).** `raw(id)` and `owed(id)` stay exact - settlement and miner audits recompute them. Coinbase outputs use the constant rungs `{500000,1000000,3000000,5000000}` forever, without era scaling or halving re-parameterization. The band function depends only on owed. A kept identity receives its complete greedy largest-first decomposition, with repeated rungs at `P_{i,k}` (§2.3); the sub-base remainder follows §11.1, never forfeiture by rounding. Rounding is strictly down because outputs cannot exceed full owed. Gross `pool_sats = T - sum(raw)` is bookkeeping, not a separate reserve below which net `pay_P` cannot fall: signed pending repayment is accounted for by §4.6/§11.9. Full actual ladder payments must fit T, with no trimming or partial fallback. Every output is at least 500000 sats, so output count is bounded by T/500000 and the representation/weight limits in §4.8. This shaping is pool-template-only; JD keeps exact owed weights and O11 scaling (§4.7, §10 O20).

### 4.5 Worked example

`r = 312 500 000`, `F = 20 000 000`, `T = 332 500 000`, `fee_bps = 100` → `r_net = 309 375 000`, `F_net = 19 800 000`. Window: 2 slices, `F_S = 9 900 000`.

| slice | ref fees | Alice (d, f) | Bob (d, f) | Carol (d, f) | `DF_S` |
|---|---|---|---|---|---|
| S₁ | 15 M | 60, 15 M | 40, 15 M | — | 1 500 M |
| S₂ | 18 M | 50, 18 M | 30, 18 M | 20, 19 M (JD, picked richer txs) | 1 820 M |

`D_win = 200`.

| miner | subsidy term | S₁ fee term | S₂ fee term | `raw` |
|---|---|---|---|---|
| Alice | ⌊309 375 000·110/200⌋ = 170 156 250 | ⌊9.9 M·900/1500⌋ = 5 940 000 | ⌊9.9 M·900/1820⌋ = 4 895 604 | **180 991 854** |
| Bob | 108 281 250 | 3 960 000 | ⌊9.9 M·540/1820⌋ = 2 937 362 | **115 178 612** |
| Carol | 30 937 500 | 0 | ⌊9.9 M·380/1820⌋ = 2 067 032 | **33 004 532** |

`Σ raw = 329 174 998`; `pool = T − Σ = 3 325 002` (= 1 % fee 3 325 000 + 2 sats rounding). Carol's S₂ fee share (20.9 %) exceeds her S₂ difficulty share (20 %) because she declared higher-fee transactions — the SLICE property. This table is a fixture in `spc-accounting/tests/worked_example.rs`.

### 4.6 Pending ledger (Blitzpool `bp-pplns` pattern, re-implemented)

**Rule B (2026-11-10).** The pending contribution at build is available
`A = B - sum(c_b)` over the complete canonical validated unbooked set, where B
is booked pending, `delta_b = E_b - P_b` uses that job's frozen prior P_b and
actual mined validation, and `c_b = max(0,-delta_b)`. New owed is fresh raw + A.
Booking applies B += delta_b and releases c_b atomically; positive delta is
unavailable before booking. Unbooked orphan releases c only, never -delta.
See the binding Rule-B decision for lifecycle, JD/O11, reorg and readiness rules.

```
owed(id)  = raw(id) + pending(id)                      # pending is error-held only (2026-11-03: no capacity deferral)
kept      = { id : owed(id) ≥ 500_000 }                # coinbase-payable; below the base rung goes per §11.1
for id in kept:    greedy largest-first decomposition over {500k; 1M; 3M; 5M} → outputs at P_{id,k}
                   pay(id)  = owed(id) − (owed(id) mod 500_000)
                   rem(id)  = owed(id) mod 500_000      # routed per §11.1: ark_due' if deliverable, else error pending'; never forfeited
for id not kept:   pay(id) = 0;        owed(id) routed by §11.1 to ark_due', error pending' or forfeited
pay_P     = T − Σ pay(id)                              # NET: K + Σ ark_due' + Σ forfeited + Σ (pending' - P)
```

**Gross versus net pool value (2026-11-10 correction).** Let
`K = T - sum(raw)`, `P` be this job's frozen available prior, `E` its resulting
error pending, and `delta = E-P`. Per row,
`paid + ark + forfeit + E = raw + P`; therefore
`pay_P = K + sum(ark) + sum(forfeit) + sum(delta)`.
Delta is signed. Gross fees were already deducted from fresh raw. Paying an
old liability can make the current pool UTXO smaller than K or even smaller
than the gross fee; that reduction is offset by liability reduction and prior
holdings, not fee waiver or donation. Pool-template validation MUST NOT impose
either extra floor. It retains `T.checked_sub(sum_actual_ladder_paid)` and all
per-row/full-rung checks. Total owed may exceed T when legitimate error/Ark
holds mean actual coinbase payments still fit. `raw.pool_sats` and
`Routing.pool_fee` remain gross K for bookkeeping and JD weights; this does
not alter legacy JD routing, O11 claims, Ark float, send caps or maturity.

Known complete empty windows have raw=0 and gross K=T, but may repay frozen P
through the same ordinary ladder/rail/conservation path. Missing ancestry,
missing nonzero high-water slices, missing frozen v2 Q or incomplete history
still fails closed. No special all-T pool-output requirement applies when P
is being repaid.

`held' = pending' + ark_due'`: everything the pool owes a miner after this block that is not in the coinbase. `forfeited` is not owed to anyone; it is pool revenue (§11.1). §11.1 decides where each non-kept id lands; this section only decides who gets a coinbase output.

**Custody statement.** Sats behind `held'` land in `pay_P` and are owed by the pool until a later coinbase (error-held balances) or the Ark send six blocks later (ark_due: sub-base-rung balances and remainders) pays them (PR #203 §3.1/§9.3 says the same of `weight_P`). This is where silent-pool is custodial (§0), bounded by roughly `Ark-tier and remainder sats of the blocks inside the confirmation window + error-held + failed sends`; decision 2026-11-03 made remainders part of that window for every kept miner — they are `ark_due'` sats of pool blocks inside the confirmation window, paid from the float like the sub-rung tier. It is reported at `/api/pool/liability`.

**Invariants (proptested).** (a) `Σ pay + pay_P = T`. (b) one deterministic decomposition per `(sp_id, H)` — outputs are keyed by `(sp_id, H, k)`, `k = 0 … q_i−1`; Blitzpool's missing duplicate-address merge ([finder-bonus code read](research/raw/repos/2026-07-27-blitzpool-finder-bonus-code-read.md) §8) still cannot recur because the decomposition loop runs over an `sp_id`-keyed set. (c) `Σ held' + Σ forfeited − Σ pending = Σ_{not kept} raw − Σ_{kept} pending + Σ_{kept} rem`. (d) idempotent apply: re-applying a settled block changes nothing (§5.1 `ledger_history` PK). (e) an id is on exactly one rail per block (`kept`, `ark_due'`, `error pending'`, `forfeited` are disjoint): one ledger row per `(block_hash, sp_id)`; a kept id's row is rail `coinbase` and carries its remainder as the row's `ark_due_sats`/`pending_delta` component — the rail gate is `owed ≥ 500_000` on un-decomposed balances and is never re-derived from shaped amounts, so §4.4 shaping cannot move an id between rails. (f) `forfeited(id) > 0` only if `id` has no `ark_delivery` row in Postgres at settlement and `owed(id) < 500_000`.

**Settlement reads the mined coinbase, never the snapshot's sats.** `payout` books from the block's actual coinbase: `T_actual = Σ coinbase outputs`; `paid(id)` = value of the output whose script is `OP_1 ser_x(P_id)` from `entries` (0 if absent). Pool-template mode: `T_actual = T` and `paid(id) = pay(id) = owed(id) − (owed(id) mod 500_000)` by construction — settlement re-runs the same accounting against the same four constant rungs and fails closed on `Σ_k value(P_{id,k}) ≠ pay(id)`, accumulating an id's outputs across `k` before the check (decision 2026-11-03: the ladder is a pure function of the amount; nothing else feeds it). JD mode: the JDC paid `floor(owed(id) · (T_actual − F) / W)` at **its** template revenue (PR #203 §4) and may have dust-pruned an id the snapshot kept, so the mined sats differ from `entries[].sats` on every JD block whose `T_actual ≠ T`; `pending'(id)` MUST be derived from `paid(id)`, never copied from the snapshot. The claim is `claim(id) = floor(owed(id) · (T_actual − F) / W)` (revenue-scaled; resolved 2026-10-14, `docs/decisions/2026-10-14-o11-claim-formula.md`), so `pending'(id) = claim(id) − paid(id)` keeps the ledger unsigned and matches pool-template mode's scaling. Blitzpool books `claim(T_actual) − paid` per address from the mined coinbase (`bp-pplns-engine` `on_block_found`, `ActualCoinbase`) for the same reason; the §11.6 miner audit closes to zero only under this rule. JD weights are exact `owed` in v1 — the §4.4 shaping is pool-template-only (§10 O20).

### 4.7 Distribution builder

**Entry binding (2026-11-09).** Each logical row additionally freezes
`owed_sats = raw + prior_pending`; its `p_x` array is in k order, while
transaction outputs are globally script-sorted. WIRE-FORMATS §3.1 defines
the mandatory version-2 binary/JSON encoding. Settlement MUST compare the
full reconstructed entitlement, canonical count, every key-associated amount
and sum, rather than accepting another owed value in the same band. Frozen
high-water/window metadata MUST NOT be retried with another slice set.

```
Distribution(H, T, a_send):
    (raw, pending) ← §4.4–4.6 on the current window-in-progress   # the window ending at "now"
    rungs = (500_000, 1_000_000, 3_000_000, 5_000_000)          # §4.4; constants, shared by build/settlement/audit
    for id in window:  ecdh_id = ecdh(a_send, H, B_scan,id)                 # §2.3; cached per (id, H), §7.3
    for id in kept:    for each rung piece k of the greedy decomposition:  P_id,k = B_spend,id + t_onchain(ecdh_id, k)·G   # §2.3
    for id in ark_capable:                                                 # registered Ark delivery, §11.4
                       U_id = B_ark,id + t_ark(ecdh_id)·G; dest_id = ark_destination(U_id, ecdh_id)   # §11.8
    outputs = [ P2TR(pool_payout_descriptor @ H, pay_P) ] ++ [ P2TR(P_id,k, rung_j) per decomposition, sorted by script ] ++ carrier (JD only)
    fingerprint = sha256(ser32(H) ‖ ser_x(A_send) ‖ Σ ser(outputs))
    high_water = {front_id: last_slice_stream_id for each front whose slices are in the window}
    entries = [(sp_id, rail, {ser_x(P_id,k)} | dest_id, pay(id) | owed(id))]    # snapshot payload; scrubbed later
```

`dest_id` (the `U_i` policy key plus its delivery: blinded mailbox id, or empty) is computed here because this is the last moment `a_send[H]` exists (§2.2). Fingerprint covers the coinbase outputs only; Ark destinations ride along in `entries`. `dest_id` is computed for every Ark-capable id, kept or not, because which rail an id lands on is decided at settlement from the mined coinbase (§4.6) — under JD the JDC may dust-prune an id the snapshot kept. Denomination payouts (§4.4) make this load-bearing in both modes: a kept id's remainder rides `dest_id`, so a kept id with no propagated destination pays its remainder into the error rail instead (§11.1). Outputs are sorted by script bytes (deterministic, pseudorandom on-chain; settlement matches by script, §4.6, so order carries no information).

In JD mode the same function emits `weights` instead of sats (`weight_id = owed(id)`, `weight_P = fee_weight + Σ_{not kept} owed`) and lets the JDC compute amounts per PR #203 §4 — the two modes share everything except the final scaling. JD weights are **not** denomination-shaped in v1 (§4.4): the JDC computes exact sats from exact weights, so JD coinbases remain amount-exact until the O20 convention exists.

### 4.8 Weight budget and autoscaler (ported from Blitzpool)

P2TR output = 43 B = 172 WU. Since 2026-11-03 the budget does not trim miners; denomination payouts (§4.4) bound the output count by value: every output is `≥ 500 000` sats, so `outputs ≤ T/500 000` (~619 in era 1; a fee-doubled block can reach ~1 230). The fixed slot policy `max_payout_slots = 1024` (§4.1; 44.0 KB / 176k WU) is the recommended cap — belt-and-braces, honestly unreachable in era 1, deliberately approach-able only under fee spikes, for which **2 048 (88.1 KB / 352k WU) is the headroom setting**; hitting the cap at build fails closed and alarms, never silently defers. An under-sized weight budget costs **coverage elsewhere**, not validity and never delay (A2: no miner waits a block for capacity; the `trim_to_budget` loop and its trimmed-to-`pending` rail are removed). Autoscaler per template: `utilisation = used/budget`; step `×1.15` when `≥ 0.85` for `up_debounce` (3) consecutive templates; step `÷1.15` when `≤ 0.50` for `down_debounce` (20); `cooldown_secs` 300; clamp to floor/ceiling (50 000 / 1 000 000 WU, §4.1); on change send new `CoinbaseOutputConstraints` to the TP (pool-template) or a new `SetPayoutDistribution` (JD). The capacity alert fires if `T/500 000` would exceed the slot cap (a fee event), not before trimming — there is no trimming.

**Witness overhead (2026-11-08).** A job whose transaction set carries
witness data has a fixed additional 224 WU - 188 WU for the zero-value
commitment output (47 vbytes) and 36 WU for the coinbase witness - counted
in the tier weight budget only when a commitment is present. Witness-free
templates carry no overhead. Settlement's tier crosscheck mirrors the same
224 WU, detected from the mined coinbase, so a boundary-exact front routing
reconstructs without spurious divergence. The constant does not cover the
coinbase output-count CompactSize growing from one byte (<= 252 outputs) to
three (`0xfd` + 2 bytes, >= 253 outputs) when the commitment output crosses
that boundary (+2 vbytes = +8 WU): the whole-block weight model remains the
preexisting tier-budget approximation, and the ladder keeps governing
pool-controlled payout outputs - a documented model limit, not a hard block
weight limit. See
docs/decisions/2026-11-08-pool-witness-commitment.md.

## 5. Data model

### 5.1 Postgres (written only by `payout`; migrations via `sqlx`, advisory-locked at boot)

```sql
CREATE TABLE sp_identity (
  sp_id       BYTEA PRIMARY KEY,                 -- 32 B, §2.1
  b_scan      BYTEA NOT NULL CHECK (octet_length(b_scan) = 33),
  b_spend     BYTEA NOT NULL CHECK (octet_length(b_spend) = 33),
  network     SMALLINT NOT NULL,                 -- 0 mainnet, 1 testnet4, 2 signet, 3 regtest
  first_seen  TIMESTAMPTZ NOT NULL DEFAULT now(),
  last_seen   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE ark_delivery (                      -- a delivery capability (§0), NOT identity; at most one per sp_id
  sp_id        BYTEA PRIMARY KEY REFERENCES sp_identity,
  ark_id       BYTEA NOT NULL,                   -- the configured server it was registered against
  b_ark        BYTEA NOT NULL CHECK (octet_length(b_ark) = 33),        -- MUST differ from sp_identity.b_spend
  mailbox_mode TEXT NOT NULL CHECK (mailbox_mode IN ('none','static','epoch_derived','epoch_registered')),
  mailbox_m    BYTEA,                            -- M; NULL iff mailbox_mode = 'none'
  mailbox_d    BYTEA,                            -- D = b_ark·S_mb; NULL iff mailbox_mode = 'none'
  epoch_fallback SMALLINT NOT NULL DEFAULT 0 CHECK (epoch_fallback IN (0,1)),
  updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
  -- CHECK ((mailbox_mode = 'none') = (mailbox_m IS NULL AND mailbox_d IS NULL))
);

CREATE TABLE ark_epoch_mailbox (                 -- MAILBOX.md variant A only
  sp_id      BYTEA REFERENCES ark_delivery ON DELETE CASCADE,
  height     INTEGER NOT NULL,                   -- epoch i
  mailbox_mi BYTEA NOT NULL,                     -- M_i
  PRIMARY KEY (sp_id, height)
);

CREATE TABLE ark_registration_state (         -- migration 0006, current request state per identity
  sp_id                 BYTEA PRIMARY KEY REFERENCES sp_identity,
  last_request_id       BIGINT NOT NULL CHECK (last_request_id BETWEEN 1 AND 4294967295),
  signed_fields_digest  BYTEA NOT NULL CHECK (octet_length(signed_fields_digest) = 32),
  registration_rev      BIGINT NOT NULL,
  updated_at            TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE ark_registration_request (       -- durable applied-request journal, NOT a Redis cache
  sp_id                 BYTEA NOT NULL REFERENCES sp_identity,
  request_id            BIGINT NOT NULL CHECK (request_id BETWEEN 1 AND 4294967295),
  signed_fields_digest  BYTEA NOT NULL CHECK (octet_length(signed_fields_digest) = 32),
  outcome               TEXT NOT NULL CHECK (outcome IN ('applied','superseded','mismatch','stale')),
  registration_rev      BIGINT NOT NULL,
  error_code            TEXT,
  created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (sp_id, request_id)
);

CREATE TABLE slice (
  slice_id    BIGINT PRIMARY KEY,                -- `front_id << 56 | local_seq` (as distribution_id, §3.5): monotone per front, unique globally
  height      INTEGER NOT NULL,
  prev_hash   BYTEA NOT NULL,
  ref_job_id  BIGINT NOT NULL,
  ref_fees    BIGINT NOT NULL,
  n_shares    INTEGER NOT NULL,
  difficulty  NUMERIC(40,0) NOT NULL,            -- u128
  root        BYTEA NOT NULL,
  shares      BYTEA NOT NULL,                    -- identity-free compact blob for GetShares (§4.2)
  work_unit_version SMALLINT NOT NULL DEFAULT 1 CHECK (work_unit_version IN (1,2)),
  work_unit_fingerprint BYTEA CHECK (work_unit_fingerprint IS NULL OR (octet_length(work_unit_fingerprint) = 32 AND work_unit_fingerprint <> decode(repeat('00', 32), 'hex'))),
  sealed_at   TIMESTAMPTZ NOT NULL,
  CHECK ((work_unit_version = 1) = (work_unit_fingerprint IS NULL))
);
CREATE INDEX slice_height_idx ON slice (height DESC, slice_id DESC);

CREATE TABLE slice_credit (                      -- the only share→identity link; scrub target
  slice_id    BIGINT REFERENCES slice ON DELETE CASCADE,
  sp_id       BYTEA REFERENCES sp_identity,
  diff_sum    NUMERIC(40,0) NOT NULL,            -- Σ d
  diff_fee_sum NUMERIC(60,0) NOT NULL,           -- Σ d·f
  PRIMARY KEY (slice_id, sp_id)
);

CREATE TABLE distribution_snapshot (
  fingerprint     BYTEA PRIMARY KEY,
  height          INTEGER NOT NULL,
  a_send_x        BYTEA NOT NULL,                -- public; never the secret
  distribution_id BIGINT,                        -- JD only
  entries         JSONB NOT NULL,                -- [{format_version:2, sp_id, rail, p_x:[], dest_id, sats, owed_sats}] (§4.7); scrubbed to []
  high_water      JSONB NOT NULL,                -- {front_id: last_slice_stream_id} (§4.3)
  window_target   NUMERIC(40,0),                 -- v1 exact N*d, v2 exact Q(T) (§4.3); NULL only for pre-migration rows, which settlement estimates with a warning (§7.4)
  work_unit_version SMALLINT NOT NULL DEFAULT 1 CHECK (work_unit_version IN (1,2)),
  work_unit_fingerprint BYTEA CHECK (work_unit_fingerprint IS NULL OR (octet_length(work_unit_fingerprint) = 32 AND work_unit_fingerprint <> decode(repeat('00', 32), 'hex'))),
  window_rule     SMALLINT NOT NULL DEFAULT 0 CHECK (window_rule IN (0,1)),
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  CHECK ((work_unit_version = 1 AND work_unit_fingerprint IS NULL AND window_rule = 0)
      OR (work_unit_version = 2 AND work_unit_fingerprint IS NOT NULL AND window_rule = 1))
);

CREATE TABLE block_found (
  block_hash   BYTEA PRIMARY KEY,
  height       INTEGER NOT NULL,
  fingerprint  BYTEA REFERENCES distribution_snapshot, -- NULL only while the found-block event is lost (§7.4)
  status       TEXT NOT NULL CHECK (status IN ('pending','confirmed','orphaned')),
  found_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  settled_at   TIMESTAMPTZ,
  CHECK (status <> 'confirmed' OR fingerprint IS NOT NULL)
);

CREATE TABLE ledger_history (                    -- idempotency + accounting; no scripts
  block_hash   BYTEA REFERENCES block_found,
  sp_id        BYTEA REFERENCES sp_identity,
  rail         TEXT NOT NULL CHECK (rail IN ('coinbase','ark','pending','forfeit')),   -- exactly one per block, §4.6 (e)
  paid_sats    BIGINT NOT NULL,                  -- coinbase sats mined to P_id; 0 on the other rails
  ark_due_sats BIGINT NOT NULL DEFAULT 0,        -- scheduled for the Ark send at H_f + confirmation_depth, §11.3
  forfeited_sats BIGINT NOT NULL DEFAULT 0,      -- unregistered and below C, §11.1; pool revenue
  pending_delta BIGINT NOT NULL,
  PRIMARY KEY (block_hash, sp_id)
);

CREATE TABLE ark_payout (                        -- one per (Ark-rail ledger row, or kept-row remainder component, §4.6); drives the send at H_f + confirmation_depth
  block_hash    BYTEA,
  sp_id         BYTEA,
  sats          BIGINT NOT NULL,
  due_height    INTEGER NOT NULL,                -- found height + confirmation_depth (6)
  status        TEXT NOT NULL CHECK (status IN ('scheduled','sent','failed_to_pending')),
  vtxo_id       BYTEA,                           -- set on 'sent'; scrub target
  expiry_height INTEGER,
  PRIMARY KEY (block_hash, sp_id),
  FOREIGN KEY (block_hash, sp_id) REFERENCES ledger_history
);

CREATE TABLE pending_balance (
  sp_id      BYTEA PRIMARY KEY REFERENCES sp_identity,
  sats       BIGINT NOT NULL CHECK (sats >= 0),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

**Rule-B durable state (migration 0009).** `economic_state` has singleton `id`,
`revision`, `anchor_height`, nullable `anchor_hash`, `complete`, and `reason`.
`economic_block` is keyed by `block_hash` and retains `parent_hash`, `height`,
`payload_digest`, scrub-sensitive nullable `raw_event`, immutable `frozen_job`,
`validated_ledger`, `consumption`, `lifecycle` (`unbooked`, `booked`, `cancelled`,
`loss_held`), `transition`, and `revision`. `economic_chain` records each
verified height's `block_hash`, `parent_hash`, `pool_block`, `coinbase_digest`,
and `revision`. The booked balance remains `pending_balance.sats`; U is the
complete canonical `unbooked` set. These tables and their committed history
MUST move atomically with monetary transitions. An existing balance/history
without an economic checkpoint MUST NOT be initialized as empty history.
Raw events follow the snapshot attribution horizon; history excludes their
raw bytes and retains frozen monetary inputs/lifecycle evidence. Lost raw
attribution MUST NOT be replaced by guessed recanonicalization inputs.

Migration 0011 adds `economic_checkpoint` (WIRE §3.11). It records the exact
actual initial-row bytes, bounded monetary snapshot bytes/digest, prior schema
checksums, economic revision, preceding history watermark and SQL transaction
snapshot boundary. The checkpoint itself is journaled as a real checkpoint
INSERT. Only the exact untouched initial state plus the existing verified
empty `history_start` qualifies as `fresh_empty`. Nonempty or inconsistent
pre-migration data remains `unverified_existing`, preserving all amounts and
holding readiness. Missing/altered/unverified seeds MUST fail closed; source
history's append-only checkpoint copy binds the readback. No prior migration
or history record is rewritten and no balance is credited by this migration.

**Aggregate receipt state (migration 0010).** Frozen `ark_aggregate.server_pubkey`
is the immutable full compressed 33-byte key from the configured server's
verified ArkInfo. A historical NULL does not authorize a proof. SPc-side
`ark_receipt_disposition` is keyed by `(stream,stream_id)` and records `status`
(`terminal_invalid`, `retry_unknown`, `applied`), `reason`, bounded `payload`,
full `payload_sha256`, `original_len`, and `attempts`. Payload retention is at
most 4 MiB per item, with full hash/length even for oversized invalid items.
`ark_receipt_cursor(stream,last_id)` persists fair pending-entry traversal.
These records MUST NOT discharge or refund uncertain obligations.

Nothing here is a per-share row ([share accounting & durability](research/sibling/mining-pool-architecture/wiki/concepts/share-accounting-and-durability.md)). Attribution includes `slice_credit`, `distribution_snapshot.entries`, raw economic events and Ark delivery proofs/IDs; the **scrub job** selects deletion horizons **per stored row version**, never from the payout process's configured version:

- v1 `slice_credit` follows the unchanged legacy horizon derived from `retain_windows`, confirmation depth and `N`; v1 identity retention is never extended by a v2 configuration.
- v2 `slice_credit` uses the explicit **4096 network-height** cutoff, `height < tip - 4096` (a row at `tip - 4096` is retained). Unknown versions are undeleted (fail closed).
- live/bootstrap v2 RAM retains age-eligible slices only, up to the explicit 4096-slice cap and the same 4096-height horizon. The RAM cutoff uses the template height `tip + 1` (`height >= tip + 1 - 4096`) and is therefore one block stricter than the SQL cutoff; it never serves credits already expired under SQL. This bounded headroom preserves history for a future harder/larger-Q window.
- `entries` are emptied (and `vtxo_id` nulled) only for blocks fully settled - booked, and every `ark_payout` row out of `scheduled` - more than `scrub_after_blocks` ago. The v2 4096-height extension is an SPc-side attribution-retention extension only; it adds no identity field to the Ark boundary and is not applied to v1. Ark-tier entries live until their send completes (normally `H_f + 6`, longer on retries): the Ark destination cannot be re-derived once `a_send` is gone (§2.2). `ark_delivery` is not scrubbed; it is live configuration, deleted when the miner withdraws it. `ledger_history` keeps `(block_hash, sp_id, rail, sats)` for idempotency and pending reconciliation but never a script or key, so a DB dump after scrub links identities to **amounts and tiers**, not to **outputs or VTXOs** — the residual leak is the amount channel, which exists on-chain anyway (§0).

### 5.2 Redis

| Key / stream | Producer → consumer | Content |
|---|---|---|
| `spc:slices` (stream) | front → payout | sealed `Slice` + its per-identity aggregates, including definite work-unit version/fingerprint |
| `spc:blocks` (stream) | front -> payout | SPCB/3 `{block_hash, height, front_id, fingerprint, high_water, window_target, entries, work_unit_version, work_unit_fingerprint, window_rule, economic}` on `SubmitSolution` accepted |
| `spc:stats` (stream) | front → payout/api | per-minute `{sp_id, accepted_d, rejected, hashrate_est}` — never per share |
| `spc:arkreg` (stream) | front → payout | `{sp_id, b_scan, b_spend, ark_id, b_ark, mailbox_mode, M, D, epoch_fallback, sig}` (registration) |
| `spc:front:events` (stream) | payout -> all fronts | `ArkRegApplied`, `LedgerApplied`, historical `BlockFound`, `TipHeartbeat`, `EconomicChanged{rev,economic_revision,anchor_hash}`; WIRE §3.2 |
| ~~`spc:dist:current`~~ | intra-`front` state since 2026-10-13 | current `distribution_id`, `H` (JD validation); was a stream when the JDS was a separate process |
| `spc:ark:orders` (stream) | payout → ark | signed `ArkSendOrder` (§11.3) - no identity fields; Ark also writes opaque fresh authorization requests and reads nonce-specific grants (WIRE §3.8) |
| `spc:ark:receipts` (stream) | ark -> payout | `ProvedReceipt` (SPRP/1), bound to the durable grant; legacy `ArkSendReceipt` only for explicitly unreserved legacy obligations |

Consumer groups give at-least-once delivery; `ledger_history`'s PK makes apply idempotent, so redelivery is harmless (Blitzpool's exactly-once-by-idempotency).

**Delivery semantics.** `payout` consumes `spc:slices`, `spc:blocks`, `spc:arkreg` via consumer group (at-least-once; idempotent apply via `ledger_history` PK). Fronts live-tail `spc:slices` and `spc:front:events` via plain XREAD with RAM-only cursors re-primed at start. Trim discipline: `XTRIM MINID` behind the slowest LIVE reader — the payout consumer group's ack position AND every front's advertised cursor (each front heartbeats its cursor per stream to `spc:cursor:{stream}:{front_id}:{instance}` every `front_cursor_heartbeat_secs`, TTL `front_cursor_ttl_secs` = 120 s; a front whose key has expired is no longer live and stops holding the trim back); never `MAXLEN`. Absolute floor: never trim entries younger than 600 s. Backstop: if XLEN exceeds 1M entries, alarm. A front whose cursor fell behind the trim horizon MUST re-bootstrap via the payout bootstrap RPC.

Byte-level formats of the stream payloads above, the `spc:arkreg` entry and the front bootstrap RPC are defined in [`WIRE-FORMATS.md`](WIRE-FORMATS.md); multi-region rationale in [`MULTI-REGION.md`](MULTI-REGION.md) §6.

### 5.3 `front` process memory

Front RAM holds channels/identity keys, registration and epoch-mailbox caches,
the height's zeroizing send secret/ECDH cache, open/sealed slices, replay set,
v1-only type-32 blobs, output budget and bounded outbox. Its pending cache is
available A from a complete Rule-B economic view, with frozen per-job B/R/A
and selected slice IDs. Historical `provisional_paids` is not monetary
authority. Readiness requires bootstrap plus an economic anchor matching the
actual parent hash. `EconomicChanged` and `LedgerApplied` trigger coherent
reload; dropped/reordered hints cannot authorize work from an incomplete view.

**Restore rejection.** A bootstrap/restore that would mix v1 and v2 slices,
unknown versions, mismatched fingerprints, or a distribution `window_rule`
that disagrees with its stored version fails closed. v1 rows are replayed under
the v1 rule only; no conversion or zero-fill is performed.

**v2 bounded live tail.** For v2 the front keeps every eligible stored-v2 slice up to the 4096-slice hard cap. It applies the same 4096-height horizon as SQL before current-Q selection, but computes it from the template height (`tip + 1`): `slice.height >= tip + 1 - 4096`. That is one block stricter than the SQL row cutoff (`height >= tip - 4096`), so the RAM set can never contain a credit SQL has expired. Bootstrap fetches no more than the cap; subsequent rolls use the new tip before selection. Unbounded retention and live use of expired credits are forbidden. If the bounded history cannot reach Q, `window_underfilled` is true and the run is warmup/inconclusive, never N-block coverage.

Loss on crash: the open slice (≤ `max_slice_shares` shares, typically seconds of work) and `seen_shares` (mitigated §3.4). `ponytail:` accepted for v0.1; upgrade path is a periodic open-slice checkpoint to Redis.

## 6. Interfaces

### 6.1 HTTP API (`api` role, read-only, JSON)

Public endpoints expose **pool-wide** data only; anything per-identity requires proof of `b_scan`, otherwise the API is an attribution oracle for anyone holding a miner's public address.

| Method / path | Auth | Returns |
|---|---|---|
| `GET /api/pool` | — | hashrate estimate, connected channels, `N`, `D`, current `H`, `A_send_x`, the ladder policy (`ladder_base_rungs_sats`, constant; §4.1, §4.4 — a miner audits any block against four constants; ambiguity → hard reject, §3.6), configured Ark `ark_id`, the forfeiture policy (§11.1: unregistered below the base rung is forfeited), mode flags, extensions advertised, advertised `endpoints` (§3.8: clearnet and onion, verbatim from `[reach] advertise`) |
| `GET /api/pool/window` | — | current window: `t`, `Σd`, `Σd·f` per slice (no identities) |
| `GET /api/pool/blocks?limit=` | — | `block_found` rows (hash, height, status, output count, `pay_P`) |
| `GET /api/pool/liability` | — | `Σ pending_balance.sats + Σ ark_payout.sats WHERE status = 'scheduled'` — everything the pool holds for miners (§0, §4.6) |
| `GET /api/pool/capacity` | — | weight budget, utilisation, slots used/max, next autoscale direction |
| `POST /api/sp/challenge` | — | `{nonce}`; 60 s TTL |
| `GET /api/sp/me` | `Authorization: SPc <addr> <nonce> <schnorr_sig(b_scan, hash_SPc/Api(nonce bytes))>` | pending balance, scheduled and sent Ark payouts `{block_hash, sats, due_height, status, vtxo_id}` (`vtxo_id` is the delivery path in `none` mailbox mode, §11.8), accepted difficulty last 24 h, last 50 `{block_hash, height, rail, paid_sats, forfeited_sats}` (no scripts or keys — the miner derives those with `spc-scan`) |
| `GET /api/sp/me/audit/:block_hash` | same | the type-32 window for that block as JSON (same data the extension serves) |
| `GET /healthz`, `GET /metrics` | — | liveness; Prometheus |

Signing with `b_scan` (not `b_spend`) keeps the spend key cold, mirroring BIP 352's own key split.

### 6.2 `spc-scan` CLI

```
spc-scan find    --address tsp1q… --scan-key <hex b_scan> --from <height> [--to <height>] --rpc <url>|--blocks <dir>
spc-scan light   --address … --scan-key … --proofs <file of header‖branch‖coinbase>      # ≈500 B/block input
spc-scan export-tweaks --format {sparrow,json}                                           # (outpoint, t) pairs
spc-scan audit   --pool <host:port> --address … --block <hash> [--slices 4 --shares 8]   # type-32 verifier
spc-scan vectors                                                                          # run the SPc vectors
```

### 6.3 Configuration (TOML; one file per binary)

```toml
# spc-pool.toml
[pool]
network = "testnet4"            # fixes HRP ("tsp") and TP params
roles = ["front", "payout", "api"]
listen = "0.0.0.0:34254"
authority_public_key = "…"      # Noise static key (SRI convention); public pin
authority_secret_key = "…"      # front only; base58check private key, mode 0600, never logged or published
pool_payout_descriptor = "tr(tpub…/0/*)"   # xpub of the ark seed's account m/86'/coin'/1' (§11.3); index = block height
fee_bps = 100
front_id = 0                    # 0..127, unique per front (§3.5, §9); single-site = 0
outbox_max = 4096               # front: bounded in-RAM outbox of unpublished Redis events (~10 min of slices + block events); full ⇒ fail closed (MULTI-REGION §8a)
tip_divergence_secs = 30        # front: alarm when its tip differs from the central TipHeartbeat for this long

[reach]                          # §3.8. The Tor daemon (not this binary) owns onion keys and forwards to loopback.
advertise = ["sv2://pool.example:34254", "sv1://pool.example:3333"]   # published verbatim at GET /api/pool `endpoints`
onion = { sv2 = "", sv1 = "", jd = "", api = "" }   # "<id>.onion:<port>" per listener; non-empty ⇒ that `listen` MUST be loopback

[template]
source = "ipc"                   # "ipc" (bitcoin-core Cap'n Proto) | "sv2-tp"
socket_path = "/run/bitcoin/node.sock"
extranonce_bytes = 16            # scriptSig budget: 100 − 4 − 33 − 16 = 47 B spare (§2.4.1)

[sv1]                              # native SV1 listener (2026-11-02); pool-template only
enabled = true                     # MUST be false when work_unit_version = 2
listen = "0.0.0.0:3333"
extranonce1_bytes = 4            # per connection; extranonce1 + extranonce2 must fit §2.4.1
extranonce2_bytes = 4            # miner-chosen
vardiff = true

[accounting]
window_n = 8
slice_fee_delta_sats = 0
max_slice_shares = 65000
dust_limit_sats = 330
coinbase_threshold_sats = 330    # C: JD dust_limits floor and rung sanity floor, §4.1; MUST be ≥ dust_limit_sats
ladder_base_rungs_sats = [500000, 1000000, 3000000, 5000000]   # §4.4; constant, forever — no era scaling (FINAL 2026-11-03)
max_payout_slots = 1024          # policy cap, not a weight limit (era-1 value bound 619; 2048 = fee-spike headroom; §4.1/§4.8)
retain_windows = 16
scrub_after_blocks = 100
confirmation_depth = 6
# v2 is opt-in and regtest-only; all keys below are v2-only. v1 keeps the
# legacy share_difficulty policy frozen and MUST NOT carry these keys.
# work_unit_version = 2
# work_reference_target = "7fff...ffff"   # 32-byte big-endian, R = 2^k - 1
# share_work_multiplier = 2               # power of two, m = 2^j
# max_work_multiplier = 4294967296        # hard cap <= 2^32
# max_window_target_units = "18446744073709551616"  # hard cap <= 2^64
# pow_limit_bits = 0x207fffff             # pinned regtest value; any other is rejected
# window_rule = "network_block_work"      # required for v2; 0/legacy is rejected

[budget]
floor_wu = 50000
ceiling_wu = 1000000             # coinbase <= 25% of block weight (decision 2026-11-03)
up_threshold = 0.85
down_threshold = 0.50
step_factor = 1.15
up_debounce = 3
down_debounce = 20
cooldown_secs = 300

[jd]                                # JD is always on (decision 2026-10-13); no enabled flag
distribution_interval_secs = 60
distribution_rebalance_bps = 100
grace_same_height_only = true    # MUST stay true (§3.5)

[extensions]
share_accounting = true          # 0x0020; MUST be false in v2 (type-32 unsupported)
window_hot_blocks = 6

[storage]
postgres = "postgres://…"
redis = "redis://…"                # TLS + per-front ACLs when fronts are remote (MULTI-REGION §6)
front_cursor_heartbeat_secs = 30
front_cursor_ttl_secs = 120
```

```toml
# spc-agent.toml                           # JDC role adds [jd] and [tp] for the miner's own node
[upstream]
address = "pool.example:34254"
authority_public_key = "…"
sp_address = "tsp1q…"                      # becomes user_identity; validated at startup
require_own_payout = "warn"                # "warn" | "refuse"   (§3.6)
request_share_accounting = true    # v1 only; ignored/refused in v2

[downstream]
listen = "0.0.0.0:34255"                   # SV1 ASICs point here; their usernames are labels only

[net]                                      # §3.8. Mining connections only: upstream SV2 and JDP. Not used for Ark/mailbox traffic or [downstream].
proxy = ""                                 # e.g. "socks5h://127.0.0.1:9050"; MUST use remote name resolution
stream_isolation = true                    # per-connection SOCKS credentials; a .onion `address` with no proxy is a startup error

[jd]                                       # JDC role (§3.2, §3.5, §3.6); the miner's own TP supplies templates
jds_address = "pool.example:34264"
declare_timeout_secs = 10                  # bounded wait for the declaration answer (§3.5); 0 is a startup error, minimum 1
```

`spc-pool` MUST refuse to start unless `pool_payout_descriptor` is a single-key `tr(xpub/…/*)` with exactly one unhardened wildcard, no hardened step after the xpub, and no multipath (the xpub spec's three assertions, §3.1 of `docs/research/output/plan-xpub-miner-identity-spec-2026-07-29.md`), and it contains no private key. The reference scanner and the output-order rule assume a 34-byte P2TR script, and a non-P2TR pool output would be the only fingerprint distinguishing pool from miner outputs.

## 7. Implementation details

### 7.1 Workspace

```
silent-pool/                      MIT OR Apache-2.0
  Cargo.toml                      workspace; rust-toolchain pinned to sv2-apps'
  crates/spc-crypto               secp256k1, bitcoin, bech32, zeroize; SPc ECDH, tier KDF, mailbox blinding (no I/O)
                                  — promoted from spc_ark/ + spc_ark_mailbox/, frozen vectors carried over
  crates/spc-scan                 lib + bin                                       (bitcoin RPC optional)
  crates/spc-accounting           pure; proptest dev-dep                         (no I/O)
  crates/spc-share-ext            binary-sv2 / codec-sv2 message structs + server/verifier
  crates/spc-ext                  0x0003 (PR #203) and 0x5350 delivery-capability messages (§3.5, §11.4)
  crates/spc-store                Postgres models/migrations, Redis stream codecs, bootstrap RPC (WIRE-FORMATS)
  bins/spc-pool                   fork of sv2-apps pool (JDS included) + roles front/payout/api/ark;
                                  the spc-jds module (in front) implements PR #203 0x0003 — JD always on;
                                  the native SV1 listener shares the §3.4 share path
  bins/spc-agent                  translator role (fork of sv2-apps translator) + JDC role (fork per roadmap
                                  Phase 4, 0x0003) + verifier + SP-aware bark keystore (§11.5)
  vendor/channels-sv2             patched: pool_tag: Option<Vec<u8>>
  testing/regtest-harness         bitcoind + pool + agent + CPU miners
  flake.nix                       devShell, packages, checks, NixOS modules (only build/CI path)
  docs/spc-spec.md                §2 of this document, standalone, for BIP review
  docs/share-accounting-ext-deltas.md
  UPSTREAM_DEPS.md                pinned commits: stratum-core, sv2-apps, sv2-spec PR #203, share-accounting-ext, bips
```

**`spc-pool` pins (added 2026-11-02).** The codec baseline is `stratum-core` 0.6.0 (crates.io; `binary_sv2`, `codec_sv2`, `framing_sv2`, `noise_sv2`, `mining_sv2`, `channels_sv2`, `sv1_api`). Project-defined `0x0003` (PR #203 text at `47dbedd`) and `0x5350` message structs live in `spc-ext`, so no fork of `extensions_sv2` is required. The `sv2-spec` and warioishere pins below remain behavioural references; deviations are absorbed in-tree per `docs/decisions/2026-10-14-upstream-own-suite.md`. Pin SHAs, never branches. `sv2-spec` PR #203 @ `47dbedd` — its single commit (2026-09-10) and the commit warioishere/stratum#1 and warioishere/sv2-apps#3 implement. `bitcoin_core_sv2` MUST be pinned at a commit containing the shared `block_reserved_weight(size) = max(1168 + 4·size, 2000)` helper in `unix_capnp/mod.rs` (it patches both `v30x` and `v31x` `bootstrap_template_ipc_client_from_coinbase_output_constraints`; warioishere/sv2-apps#2 @ `ec35ac8`, carried as `4e526de` in #3's branch, or its upstream merge) in **both** `spc-pool` (`[template] source = "ipc"`) and `spc-agent`/`spc-jdc` (`[tp]`). Upstream `sv2-apps` `main` (as of 2026-10-12) and tag `v0.8.0` still reserve `max(4·size, 2000)` — none of the 1168 WU of header, tx count, coinbase fixed fields, 100 B scriptSig and witness commitment once `coinbase_output_max_additional_size > 208 B` — and are excluded; a `0x0003` JDC that reserves the exact output size got `bad-blk-weight` on mainnet with 42 outputs and a full mempool. `source = "sv2-tp"` needs no pin (it reserves `1168 + 400 + 4·size`). If `spc-jdc` is forked from warioishere/sv2-apps#3 (roadmap Phase 4), pin `warioishere/sv2-apps @ 90b3e3d` and `warioishere/stratum @ 9be4023` (`extensions_sv2` 0.3.1, MIT OR Apache-2.0); #3's `stratum-apps/Cargo.toml` pins `stratum-core` by branch, so re-pin by SHA. `warioishere/blitzpool-server-rust @ b48998f` (release 2.4.1) is the reference `0x0003` JDS for reading, not porting; the frontmatter pin `7815884` stays as the provenance of the `bp-pplns` patterns §7.6 re-implements.

### 7.2 Key types and zeroisation

`struct SendKey { h: u32, a: Zeroizing<[u8; 32]>, a_x: [u8; 32] }` with `Drop` zeroising; constructed only in `front::template::on_set_new_prev_hash`. `derive()` takes `&SendKey` and never copies `a` out. `tracing` fields are allow-listed; `SendKey` implements `Debug` as `SendKey{h, A_x}` only. CI greps for `a_send` in any `serde` derive or SQL string.

### 7.3 Hot path cost

Per template: `n_kept` scalar multiplications (`ecdh_i` — `B_scan,i` is fixed, `input_hash·a_send` is one scalar per height, so `ecdh_i = s_H · B_scan,i` is **one** point multiplication per miner per height, cached until `H` changes) plus one `P_i = B_spend,i + t_onchain·G`, and for Ark-capable ids one `U_i = B_ark,i + t_ark·G` and one `t_ark·S_mb` for the blinded mailbox id (§11.8). At 256 slots that is ≤ ~1 000 EC ops per height, not per template — the keys don't change within a height; only amounts do. Share path adds one `HashSet` insert and one `Vec::push`; slice sealing adds one merkle root over ≤ 65 000 leaves (~2 ms) off the share path.

### 7.4 Idempotency and exactly-once

Block settlement runs in one Postgres transaction. `payout` first reconstructs the paid window from `distribution_snapshot.high_water` (§4.3, MULTI-REGION §1): exactly the slices at or below the vector, never a re-derivation from the current contents of Postgres. The window target is the snapshot's persisted `window_target` - v1 exact `N * d`, v2 exact `Q(T)` - selected by the snapshot's persisted `work_unit_version`, `work_unit_fingerprint` and `window_rule`. A difficulty retarget or later template change cannot alter the window or target settlement rebuilds. Only a pre-migration row (NULL `window_target`) is estimated from the stored slices, with a warning naming the block, and a stored value that is not a u128, a mixed profile, or a v2 row without a nonzero fingerprint is a corrupt row that fails closed rather than a silent guess or conversion. The finding job's `nbits`/generation and the snapshot's profile are immutable settlement inputs. Then: `INSERT block_found … ON CONFLICT DO NOTHING` → if inserted, `INSERT ledger_history` rows (PK rejects replays) → `INSERT ark_payout` rows (`status='scheduled'`, `due_height = height + confirmation_depth`, so normally due at once) for the Ark-rail ids → `UPDATE pending_balance` → `UPDATE block_found.status`. The Ark send is a second, separately idempotent step keyed by the same `(block_hash, sp_id)` and driven by the `ark` role's receipts (§11.3). Orphan check (`confirmation_depth`) runs before the transaction; an orphaned block writes `status='orphaned'` and **no** ledger rows. If a front cannot deliver a found block's event (crashed before XADD, outbox overflow), `payout`'s chain reconciliation sees the pool coinbase on-chain with no `block_found` row (`spc_orphaned_settlement_total{reason=missing_block_event}`, MULTI-REGION §8a), books it `status='pending'` with `fingerprint` NULL (§5.1: the column is nullable while the event is lost; `CHECK` forbids `confirmed` with NULL) and alarms — a build MUST never fail over a lost event (Blitzpool's vouching invariant), and the operator reconstructs from the on-chain coinbase + `slice_credit` if needed; that reconstruction sets `fingerprint`, after which the block may be confirmed and booked.

### 7.5 Pool-tag patch

`channels-sv2/src/server/jobs/factory.rs`: `pool_tag_string: Option<String>` → `pool_tag: Option<Vec<u8>>`; the `Sv2/<pool>/<miner>/` ASCII framing is dropped when `pool_tag.len() == 32` and the push is emitted as `OP_PUSHBYTES_32`. `fits_script_sig_budget` keeps its conservative 8-byte-height check; `ScriptSigSizeTooLarge` still fails at channel setup, not at block submission ([08-16 L5](research/raw/notes/2026-08-16-ll-sv2-pool-tag-asend-carrier.md)). Upstreamable as a standalone change ("binary pool tags"), independent of SPc ([08-24 L7](research/raw/notes/2026-08-24-ll-jd-coinbase-control-amount-linkability.md) — pitch the change so it stands without the proposal).

### 7.6 Blitzpool-derived design (no code ported)

The pending-credit ledger, budget/autoscaler constants, payout-list fingerprint, confirmation-gated idempotent apply and the vouching invariant are **patterns and parameters taken from Blitzpool's README and this wiki's code reads**, re-implemented here from §4.6 / §4.8 / §7.4 — not copied from `bp-pplns`, which is AGPL-3.0-or-later. (Blitzpool's keep-largest trim informed the budget design but trimming itself was removed 2026-11-03 in favor of adaptive denominations, §4.8.) Clean-room rule for implementers: write from the spec text and its proptests, do not read `crates/bp-pplns/src/distribution.rs` while doing so. Differences from Blitzpool are deliberate: the ledger key is `sp_id` not `address`; there is no finder bonus and therefore no debit side (`pending` is unsigned); withheld value lands in `pay_P` (`WithheldValue::ToPool` shape, §11.2). Blitzpool is cited as provenance (`7815884`) and as the `0x0003` JDS reading reference (`b48998f`, §7.1); no `NOTICE` obligation arises.

## 8. Testing strategy

### 8.1 Unit — `spc-crypto` (vectors regenerated for the chosen tags)

The derivation vectors are DESIGN.md's: port `spc_ark/src/lib.rs::test_vector` (`input_hash`, `ecdh`, `t_onchain`, `t_ark`, `P`, `U`, `D`, blinded mailbox id, `ark1` address) and `spc_ark_mailbox/src/lib.rs::tests::test_vector` (`m_i`, `M_i`, variant-B `M_i`, authorization bytes) unchanged, plus `pool_blinding_matches_bark` and `pool_blinding_unblinds_to_epoch_mailbox`. Regenerate `~/repos/sp-coinbase-vectors` with the `SPc/*` tags and `ser_P(A_send) = 0x02‖x` in `input_hash`, then port all cases as `#[test]`s:
- positive: ordinary-tx split sanity; coinbase native (null prevout, height nonce); `N = 5` fan-out with a dropped output not affecting others; nonce distinctness across heights and across `A_send`.
- negative controls (each MUST fail where stated): vanilla BIP 352 eligibility gate rejects the coinbase; constant-null-outpoint nonce collides across blocks; `input_hash` without `A_send` admits the `:92` grind (and binding `A_send` kills it); odd-Y `A_send` without negation misses; group-linear key list `A_H = A_0 + H(A_0‖H)·G` yields every epoch from `a_0`.
- carriers: scriptSig layout round-trips byte-exact; `OP_RETURN "SPc"` round-trips; the rejected 1-of-2 multisig carrier stays as a **documentation test** that asserts `a_send` can sign the output (why it's rejected).
- tiers: `t_onchain ≠ t_ark` for the same `(ecdh, k)`; a digest of 0 or `≥ n` fails rather than reducing (forced via a test-only hash hook); `B_ark = B_spend` is rejected at registration (§11.4).

### 8.2 Property tests — `spc-accounting`

`proptest` over random windows (1–40 slices, 1–300 identities, random `d`, `f`, `T`, pending, budget): invariants §4.6 (a)–(e); `δ = 0` pool-template result equals an independent plain-PPLNS oracle to the sat; the greedy decomposition is canonical (largest-first), every coinbase output's value is one of the four constant rungs, every kept id's decomposition sums to `owed − (owed mod 500_000)`, and total outputs never exceed `T/500_000`; §4.5 worked example as a fixed fixture.

### 8.3 Integration — SV2 surface (in-process, no bitcoind)

- channel open with `sp`, `tsp`, bare `bc1q`, descriptor, `sri/solo/…`, empty, 63-char-truncated, two addresses on one connection → exactly the §3.1 table; **assert no coinbase was built for a rejected channel**.
- `RequestExtensions` matrix: `{0x0001}`, `{0x0001, 0x0003}`, `{0x0001, 0x0020}`, unknown id → correct `.Success`/`.Error` lists; JDC with `0x0003` on only one connection refuses JD mode. In v2 `0x0020` is not advertised and a request receives `unsupported-extension`.
- v2 profile vectors: non-power-of-two reference/multiplier, reference below pow limit, `Q > max_window_target_units`, multiplier stepdown at a new generation, checked overflow/division failure, mixed-profile restore refusal, and a frozen settlement target after a later retarget.
- share replay (same share twice, same share on two channels), stale job after restart → rejected.
- slice sealing on prev-hash change, fee band, `max_slice_shares`; merkle root matches an independent implementation.
- `SetPayoutDistribution` published on height change; validation rejects mutated amounts, reordered outputs, a missing `OP_RETURN`, and a **height-straddling** declaration under the grace window; accepts the previous id from the same front on the same connection at the same height.
- type-32 (v1 only): `GetWindow` for a found block returns a window whose `Σ difficulty ≥ N × D`; `GetShares` merkle paths verify; a pool build compiled with `--features cheat-slice-difficulty` is caught by the verifier within 3 windows at `4 × 8` sampling (statistical test, seed-pinned).

### 8.4 End-to-end — regtest harness (`testing/regtest-harness`, CI)

bitcoin-core (IPC) + `spc-pool` (JDS module included) + 2 × `spc-translator` + 1 × `spc-jdc` + CPU miners:
1. **Pool-template happy path**: three `tsp` identities mine; block found; coinbase has pool P2TR + 3 P2TR + no `OP_RETURN`; each `spc-scan find` recovers exactly its own output; each spends it with `b_spend + t`.
2. **JD happy path**: `spc-jdc` declares; block found; coinbase has `OP_RETURN "SPc"`; JDC's own `P` present; `spc-scan` recovers via carrier 2; type-32 `GetTransactionsInJob` returns the JDC's set.
3. **Threshold and delivery**: one Ark-registered identity below the base rung → `ark_payout` scheduled and paid at `H_f + 6` from the float (§11.9 covers the Ark leg); one identity below the base rung with no Ark registration → `rail='forfeit'`, its sats in `pay_P`, nothing pending; `/api/pool/liability` returns to 0 after the send.
4. **Orphan**: invalidate the found block → `status='orphaned'`, no ledger rows, pending unchanged.
5. **Restart mid-height**: kill `front`; new `A_send` for the same `H`; shares resume; next block still scannable.
6. **Fail-closed**: SV1 miner with a bare address → `spc-translator` refuses to start; a hand-rolled SV2 client with a bare address → `invalid-user-identity`, nothing mined for it.
7. **Light client**: `spc-scan light` on exported `header‖branch‖coinbase` triples finds the same outputs as `find`.
8. **Scrub**: after `scrub_after_blocks`, `distribution_snapshot.entries = []` and `slice_credit` rows beyond retention are gone; a block with a still-`scheduled` Ark payout keeps its entries until the send; `ledger_history` still reconciles `pending_balance`.
9. **Full mempool**: fill the regtest mempool to ≈ 990 000 vB (Blitzpool `regtest_budget_autoscale.rs` `MEMPOOL_TARGET_VBYTES`; Core packs to `block_max_weight − block_reserved_weight`, so a part-full mempool never binds) before (a) the pool mines its own template with `max_payout_slots` outputs and (b) `spc-jdc` declares a block with `max_payout_slots` outputs; both blocks must pass the JDS node's `checkBlock` and be accepted. Fails with `bad-blk-weight` on `bitcoin_core_sv2` from upstream `sv2-apps` `main`/`v0.8.0` once `coinbase_output_max_additional_size > 208 B` (it reserves `max(4·size, 2000)` instead of TDP's `1168 + 4·size`); passes with the `block_reserved_weight` fix pinned in §7.1 or with `source = "sv2-tp"`. Steps 1–8 cannot surface this — it is the one class of bug the 2026-10-11 mainnet run found.

### 8.5 Adversarial (manual + CI where deterministic)

Grinding `a_send` to collide with a known earlier payment (must be a fixed-point search — test asserts the naive solve fails); JDC distribution shopping across the grace id and the current id at the same height (accepted by design, bounded by `distribution_rebalance_bps`); 10 k channels with distinct valid addresses (memory, autoscaler, decomposition cost at population scale).

## 9. Deployment

**Topology.** One `spc-pool` binary; dev/testnet runs `--roles front,payout,api` in one process, and MAY add `ark` to it on regtest/signet only; production splits `front` (one per region; RAM-only state — no durable state of its own) from a single `payout` (sole Postgres writer), N `api` replicas, and a single `ark` on its own host. Each `front` listens for SV2 (Noise) and native SV1 (plain TCP, §3.1) on separate ports; both listeners feed the same share and distribution path. Any listener (SV2, SV1, JDP, `api`) MAY be published as a Tor onion service, in which case it binds loopback and the address is advertised via `[reach]` (§3.8); a multi-front deployment publishes one address per front or fronts them with OnionBalance, and the onion key is backed up with that front's other secrets. Each `front` claims its `front_id` at start with a Redis `SET NX` lease (heartbeat-refreshed, holding an instance UUID) and refuses to start if the lease is held — a duplicate `front_id` would silently collide `distribution_id`s and `slice_id`s (MULTI-REGION §10). The `spc-jds` module is part of `front` — JD is always on, and `a_send` is shared in-process (§2.2). Redis and Postgres are the only durable state of the SPc side, and **no spending key lives on it**: it holds only the xpub of `pool_payout_descriptor`. The pool has exactly one hot seed, the `ark` seed (bark wallet + pool output keychain), held only by the `ark` role on its own host with its own secret store and no access to the attribution data (§11.3). Its blast radius is the float plus whatever pool revenue the operator leaves under it; moving surplus elsewhere is operator policy and out of scope.

**Migrations.** `sqlx migrate` at boot under `pg_advisory_lock`; every process may run them (Blitzpool pattern). Schema changes are additive until v1.0. Migration `0005_lab_work_units.sql` adds the v2 profile columns/checks with legacy v1 defaults; v1 rows remain readable and are never converted.

**v2 is a lab profile, not a production rollout.** The `network_block_work`
profile is opt-in and restricted to regtest in this revision. Production
testnet4/mainnet rollout remains v1 until a separate decision and evidence
packet; the lab profile does not amend `DESIGN.md` payout policy.

The expanded lab MUST build its own stock pinned Ark server and tooling through
Nix, and own its storage, wallets, private configuration and process lifetime.
It MUST NOT fall back to a hosted Ark provider. The lab's provider database MUST
be independent of every pool database, and each pool MUST retain a separate
Ark-role wallet/process and the §11.3 identity boundary. Startup MUST bind
actual ArkInfo server/mailbox keys and policy before rendering front delivery
metadata. RPC health and initialized wallets MUST NOT imply funded readiness:
native funding, boarding, receive, refresh and offboard require their own real
transaction evidence. An infrastructure-only run MAY remain initialized and
unfunded while the native funding and registered-agent runtime are integrated.

**Rollout.** regtest CI → testnet4 (`tsp`) soak ≥ 24 h with ≥ 1 found block independently recovered by a third party's `spc-scan` → publish `docs/spc-spec.md` to the Delving thread and the BIP mailing list → mainnet is out of scope for this document.

**Rollback.** Stopping `front` stops mining. Coinbase-tier payouts are already on-chain and not at risk; pending balances and scheduled Ark payouts survive in Postgres and are paid when the pool resumes (the Ark send is idempotent per `(block_hash, sp_id)`, §7.4). Downgrading `spc-pool` across a schema version is unsupported; restore from the pre-migration dump.

**Monitoring (Prometheus).** `spc_a_send_age_seconds` (alarm > 1 h — a stuck height), `spc_a_send_repeat_total{front_id}` (alarm > 0), `spc_open_slice_shares`, `spc_slices_sealed_total{reason}`, `spc_distribution_slots{kept,ark,error}`, `spc_distribution_outputs_total` (approaching `max_payout_slots` = alarm; an honest build cannot approach it since `outputs ≤ T/rungs[0]`), `spc_budget_wu`, `spc_budget_utilisation`, `spc_blocks_total{status}`, `spc_pending_liability_sats`, `spc_ark_due_sats`, `spc_ark_payouts_total{status}`, `spc_ark_overdue_total` (scheduled past `due_height + ark_retry_blocks` = page), `spc_ark_float_sats` (below `float_target_sats` = refill needed; `ark` boards at the next matured pool output), `spc_ark_surplus_sats` (matured pool revenue not boarded, for operator tooling), `spc_forfeited_sats_total`, `spc_ark_orders_rejected_total{reason}` (any `bad-signature` = page), `spc_channel_reject_total{reason}`, `spc_jd_declaration_reject_total{code}`, `spc_ext32_requests_total{msg}`, `spc_outbox_depth`, `spc_outbox_age_seconds`, `spc_stream_lag_seconds{stream}`, `spc_tip_divergence_seconds`, `spc_registration_lag_revs`, `spc_orphaned_settlement_total{reason}`.

**Operational privacy.** Logs never contain `user_identity`, `sp_id`, `P_i`, `U_i`, mailbox ids or VTXO ids at `INFO`; `DEBUG` redacts to 8-hex-char prefixes. Postgres backups inherit the scrub policy; a backup older than `scrub_after_blocks` is itself attribution data and SHOULD be rotated on the same schedule.

## 10. Open items (carried from the roadmap, narrowed by this spec)

| # | Item | Status |
|---|---|---|
| O1 | BIP posture — amendment to 352 vs new informational BIP | **Open.** With fresh `SPc/*` tags (DESIGN.md) the construction no longer shares BIP 352's hash domain, and it adds a second tier BIP 352 has no concept of; that argues for a new informational BIP referencing 352 (the roadmap's original recommendation). O12 can move this back toward an amendment for the coinbase tier alone. |
| O2 | `A_send` serialization inside `input_hash`: `ser_P` (this spec) vs `ser_x` | Decided `ser_P` for maximal reuse of BIP 352's `input_hash` shape; vectors regenerate accordingly. |
| O3 | `{H, H−1}` scan tolerance | Decided for `spc-scan`; pool-side grace is same-height-only. Revisit if third-party SPc pools appear. |
| O4 | Labels (BIP 352 `B_m`) | Deferred to v0.2; one identity per address in v0.1. |
| O5 | Solo-pool profile (one payee) | Same carrier and derivation; a `[pool] mode = "solo"` flag is a ~20-line addition; not in v0.1. |
| O6 | Rail for pending credits below the coinbase threshold | **Resolved 2026-10-12 → §11 (Ark via a third-party bark server).** Lightning/BOLT12 stays a v0.2 alternative for miners without an Ark wallet. |
| O7 | Empirical mainnet scriptSig occupancy alongside AuxPoW tags | Unmeasured; needed before any mainnet claim about the 47 B spare. |
| O8 | Amount-linkability mitigation (randomised slot count, deferred payouts) | **Amended 2026-11-03**: coinbase payouts are denomination-shaped per §4.1/§4.4 (constant rungs `{500k; 1M; 3M; 5M}`, remainders over Ark; decision `docs/decisions/2026-11-03-banded-amounts-with-ark-overage.md`). Within a block the observer sees only per-rung counts (era-1: 500k rung ≈ 59-103 outputs at 1 000 identities); residual channels are class composition, counts, timing, whale top classes and redemption clustering. Deferred-payout mitigations were rejected by policy (no capacity deferral); randomised slot count remains a v2 candidate (ROADMAP R1). |
| O9 | `demand-share-accounting-ext` porting cost on current `stratum-core` | Unknown until Phase 6; message set is small. |
| O10 | PR #203 drift | Pinned at `47dbedd` (the PR's single commit, 2026-09-10; no spec movement as of 2026-10-12 — warioishere's 2026-10-11 comment pins warioishere/stratum#1 and sv2-apps#3 to the same commit). Residual text risk: TheBlueMatt's unanswered 2026-09-10 question on all-static payouts with only a subset paid (would relax the §7 'no partial payment of `additional_outputs`' MUST at L168) and warioishere's requested §9.3 sentence (prose only). `spc-jds`/`spc-jdc` isolate `0x0003` in one module each. |
| O11 | JD-mode booking when the JDC's template revenue ≠ the pool's (§4.6) | **Resolved 2026-10-14** (`docs/decisions/2026-10-14-o11-claim-formula.md`): revenue-scaled — `claim(id) = floor(owed(id) · (T_actual − F) / W)`; `pending'(id) = claim(id) − paid(id)`. Unsigned ledger preserved; §11.3 Ark sends pay `ark_due'` as booked from the mined coinbase. |
| O12 | Plain-SPc compatibility for the coinbase tier (DESIGN.md open question) | **Open.** Should `t_onchain` use the upstream SPc / BIP 352 tag instead of `SPc/TierSpc`, so a single-tier SPc scanner also finds coinbase outputs? Depends on the SPc BIP, which isn't final. `t_ark` and `t_mb` keep `SPc/*` tags either way. |
| O13 | Bark integration for derived keys (DESIGN.md) | **Resolved 2026-11-02** (`docs/decisions/2026-11-02-bark-adapter.md`): neither an upstream patch nor a bark-wallet fork for v0.1 - the agent's wallet is built in-tree over `ark-lib` + `bark-server-rpc` (`SpArkKeystore` `U_i -> u_i` extra-keys table, `claim_vtxos`, `read_arkoor_mailbox`, `query_vtxo_status`, `spend_tweaked_vtxo`), exactly as `spc_ark_client` validated against a stock server. **Recorded gap:** in-round refresh and unilateral exit of `u_i`-keyed VTXOs are unavailable in v0.1; durability paths are arkoor spend, cooperative offboard and the §11.5 shutdown warning. The ~150-300-line upstream extra-keys patch (RESULTS.md) stays on the table as the known shape of upstreaming, to be revisited after v0.1 or immediately if offboard-from-`u_i` fails validation. |
| O14 | Default mailbox mode (DESIGN.md per-block vs static; MAILBOX.md variants) | **Resolved 2026-10-14** (`docs/decisions/2026-10-14-mailbox-default-epoch.md`): default = `epoch_derived` (variant B). No per-epoch registration window; no fallback-to-static. Variant A (`epoch_registered`) remains an option for independent per-epoch keys. |
| O15 | Wallet UX across tiers (DESIGN.md) | **Open.** One balance spanning pending, scheduled Ark payouts, immature coinbase outputs and VTXOs; `GET /api/sp/me` (§6.1) supplies the pool-side half. |
| O16 | Light-client scanning of both tiers (DESIGN.md) | **Open.** The ≈500 B/block proof (§2.5) covers the coinbase tier and the pool-block list; the Ark tier then needs only mailbox reads, but a light client still needs a trusted source of pool-block heights. |
| O17 | Coinbase output as the board itself (`board_funding_address` in the coinbase) | **Rejected (2026-10-13).** The pool does not run the Ark server, so the design MUST work with a stock third-party `captaind`, and stock mainnet `captaind` forces `require_board_funding_tx`, whose cosign check rejects a coinbase's null-prevout input. Recipients would also have no unilateral exit until coinbase maturity. The pool output is paid normally and boarded when mature (§11.3). |
| O18 | Prototype timing | **Resolved 2026-10-13.** `spc_ark_mailbox` now schedules reads from `PAYOUT_DELAY = 6` (was `COINBASE_MATURITY = 100`); tests updated and passing. |
| O19 | Covenants (CTV / BIP 119, or an equivalent such as `OP_TEMPLATEHASH`) | **Future; needs a soft fork.** Would greatly improve the Ark story by **removing interactivity**: today every board, arkoor and refresh needs the third-party Ark server online to cosign (bark is covenant-less Ark), which is why sub-threshold payouts come from a hot float and why the coinbase can't be the board (O17). With a covenant, the coinbase could commit the sub-threshold payouts to a pre-defined tree whose leaves are each miner's `U_i`, giving every recipient a unilateral exit with no cosigning party online and no pool float. More generally, **all** pool payouts could be compressed into one coinbase output committing to a payout tree: the §4.8 weight budget, trimming, `max_payout_slots` and the coinbase dust floor stop binding, and per-miner amounts stay off-chain until a leaf is expanded (helping R1 amount linkability). Costs to weigh then: coinbase maturity still applies to the tree root, and expanding a leaf costs one on-chain transaction per tree level. Track; no v0.1 work. |
| O20 | JDC-side denomination convention | **Open (v1 shapes pool-template only, §4.4; decision 2026-11-03).** A denomination-shaped JD mode needs the ladder rules shipped to and enforced by every JDC (a PR #203-side convention beyond §3.5 validation) and an O11-class claim formula composed with the decomposition; shaping and revenue scaling do not commute (`decompose(floor(owed·x)) ≠ floor-scaled decompose(owed)`), so the rule must be nailed before any JD miner relies on it. |

## 11. Sub-threshold Ark tier (added 2026-10-12; reconciled to DESIGN.md 2026-10-13)

> Balances that do not clear the coinbase threshold are paid over **Ark (bark)** to a **per-block key derived from the same ECDH as the coinbase output** (DESIGN.md), from the pool's Ark float on a **third-party** Ark server, once the found block has 6 confirmations; the block's own pool output refills the float when it matures. Grounding: `DESIGN.md` (authoritative), `MAILBOX.md`, `RESULTS.md`; [Ark (bark) as a Sub-Threshold Payout Rail](research/wiki/concepts/ark-sub-threshold-payout-rail.md), [Payout Threshold Economics](research/wiki/concepts/payout-threshold-economics.md), and the two bark code reads ([send side](research/raw/repos/2026-10-10-bark-barkd-openapi-source-boat001.md), [key side](research/raw/repos/2026-10-12-bark-key-management-mailbox-blinding-sp-derived-vtxo.md)). The tier changes **how** `owed(id)` is paid, never how §4 computes it — SLICE scoring, JD or pool-template, is untouched.

### 11.1 Tier policy (completes §4.6)

DESIGN.md "The proposal": the tier is decided per block by whether the miner's balance clears the coinbase threshold.

```
owed(id) = raw(id) + pending(id)                       # pending = error-held only
if   owed ≥ 500_000, keys known (§4.7)              → coinbase: greedy decomposition over constant
                                                      rungs {500k; 1M; 3M; 5M}, outputs at P_{id,k}   rail = coinbase
                                                      rem = owed mod 500_000: ark_due' if deliverable (§11.4),
                                                      error pending' otherwise; never forfeited (§4.6 (e): same ledger row)
elif owed ≥ 500_000 (delivery keys unknown at build)   → error pending'(id) = owed  rail = pending (error rail)
elif ark_delivery(id) registered, dest known (§11.4)   → ark_due'(id) = owed  (sent at H_f + 6, §11.3)  rail = ark
elif ark_delivery(id) committed in Postgres, dest not yet propagated to this front → error pending'(id) = owed  rail = pending (error rail)
else                                      → forfeited(id) = owed (kept by the pool)       rail = forfeit
```

- **Denominations (decision 2026-11-03), policy constraints.** The on-chain share is maximized every block subject to the decided rungs; nothing is deferred for capacity — output count is value-bounded (`≤ T/500k`) and cannot exceed the slot policy cap (§4.8); whatever cannot be paid in the coinbase (`rem`, or the whole balance below 500 000 sats) goes over Ark. The rail gate is `owed ≥ 500 000` on un-decomposed balances and every rung is `≥ C ≥ dust`, so every coinbase output is a whole denomination; cross-block movement in `owed` changes the decomposition, never the rail rule. The ladder is a constant: four rungs, no height, subsidy, fee or coinbase-value dependence — build, settlement and audit apply the same pure function of the amount. This is the accepted product tradeoff restated: every miner gains an Ark footprint, remainders are trustodial until refresh (§11.7), and the Ark server sees exact per-miner remainder amounts.

- `C` = `coinbase_threshold_sats` (§4.1), default **`dust_limit_sats`** (330). Since 2026-11-03 it is no longer the coinbase gate — the pool-template gate is the constant base rung (500 000 sats; §4.4). `C` remains the JD-mode `dust_limits[i]` value and the sanity floor every rung must satisfy. Published at `/api/pool`.
- Under JD the branch is taken from the **mined** coinbase (§4.6 settlement rule): an id the JDC dust-pruned is treated as not coinbase-paid for this block whatever the snapshot said.
- **No capacity deferral (decision 2026-11-03).** The rungs are fixed per era and the output count is value-bounded (`≤ T/rungs[0]`), so nothing ever waits for capacity, and it never diverts a large balance to Ark whole: large balances are exactly what should not sit under arkoor trust, and the decomposition pays them on-chain at any rung scale.
- **Unregistered below 500 000 sats is forfeited to the pool.** Ark delivery is part of the instructions for mining here; `spc-agent` registers automatically on connect (§11.5), so only a miner running other software, or misconfiguring it, reaches this branch. Their sub-rung earnings for that block go to `pay_P` and are booked as `forfeited` — not carried as pending, so the pool holds no liability for operators who don't follow the instructions. This is a published policy, not a silent fallback: it is stated at `/api/pool`, reported per block at `/api/sp/me`, and signalled at channel open (§3.1). It is distinct from the sv2-apps `FullDonation` defect (§3.1), which silently took 100 % of the block for *every* miner on an unparsed identity; here identity is still validated fail-closed, and only the earnings of an unregistered miner below the base rung are affected. At or above it an unregistered miner is paid in the coinbase as usual (its remainder, undeliverable to Ark without registration, is error-held, never forfeited).
- `pending` is now the **error rail** only: failed Ark sends (§11.3), delivery keys unknown at build time (`hold_undeliverable`, §4.7), and a registration committed in Postgres whose `dest_id` had not reached the building front (MULTI-REGION §6). It is never a capacity decision (trimming is removed). None of these is ever forfeited; forfeit applies only when no `ark_delivery` row exists in Postgres at settlement.
- Booking requires `block_found.fingerprint IS NOT NULL` (§5.1, §7.4); a block with NULL fingerprint contributes nothing to `owed()` until resolved.
- Cadence: **one Ark send per id per found block**, never per share or per template (Second: per-minute payments "build long exit chains … prohibitively expensive to exit on small balances"). Kept-id remainders below `min_overage_sats` are the single exception: they aggregate per `sp_id` across blocks and emit one send for the accumulated sum (§11.3 send loop), so a micro remainder never becomes a micro-VTXO.

### 11.2 Both template modes

| | Pool-template | Job Declaration (PR #203) |
|---|---|---|
| Where Ark-tier and forfeited sats land | in `pay_P` (§4.6) — the single hot pool output | in `weight_P` — §3.1 of #203 explicitly allows "value credited on behalf of miners that are not represented in `payouts`"; `pool_payout` is the same per-height hot output |
| Carrier for `A_send` | scriptSig (§2.4.1) | `OP_RETURN` (§2.4.2) |
| Ark destination | derived at distribution build, stored in the snapshot (§4.7) | identical |
| What the pool does after booking (`H_f + 6`) | `payout` emits a send order per `ark_due` id; the `ark` role pays it from the float (§11.3) | identical |

The coinbase leg is unchanged in JD mode and denomination-shaped in pool-template mode (§4.4: copies of the four constant rung values, never exact `owed`) — the Ark leg is purely pool-side settlement either way, and a kept id's remainder is booked from the mined coinbase exactly like a sub-rung `ark_due` row. The JD column is the `WithheldValue::ToPool` weight composition from Blitzpool `bp-pplns` (`weights.rs`, origin/main), deliberately not its PPLNS `ToOtherMiners` composition — the one warioishere described on PR #203 (2026-09-11): `weight_P` is the fee only, withheld weight is spread over the published miners, who are then booked matching debits. Silent-pool cannot use `ToOtherMiners`: the pool output is what refills the Ark float (§11.3), so the withheld sats MUST land in it, and §4.6's `pending` is unsigned. Unlike Blitzpool's Team Mining (which pairs `ToPool` with no ledger), the withheld value is booked per §11.1 (`ark_due`, `pending` or `forfeited`). The JDC computes `pay_P = T − F − Σ pay[i]` without inspecting how `weight_P` was composed, so the two shapes are wire-identical.

### 11.3 Pool side: the `ark` role

**Aggregate authorization amendment (2026-11-09).** The binding decision
`docs/decisions/2026-11-09-aggregate-anchor-and-send-authorization.md` selects
the oldest covered canonical row's stored destination and requires frozen
whole-set membership, fresh attempt-correlated all-member chain checks,
durable authorization before grant, and atomic definitive outcome application.
Its potentially-sent uncertainty rule supersedes the older timeout-to-pending
send-loop clause below: uncertain membership remains pinned as `blocked_unknown`.
Ark messages carry only opaque delivery/authorization data, never member mapping.
The private SPc owner API also exposes the exact frozen signed order via
`GET /api/sp/me/ark-aggregates/<order_id_hex>/order` (WIRE §3.8). Authentication
uses a fresh single-use B_scan challenge. Lookup MUST constrain authenticated
sp_id and order_id before reading bytes. The stored order MUST roundtrip
canonically, bind its ID/digest and any durable grant digest, and verify with
the configured order public key before return. Return stored bytes including
signature, never reconstruct from a current destination. Unknown/corrupt state
does not prove NoEffect; the wallet keeps its watch.

**Review boundary correction (2026-11-10, A1-A5).** The fresh-authorization
consumer MUST run independently of unrelated blocking stream reads, within
the payout role and using its Store. Quiet streams MUST NOT consume the Ark
worker's authorization deadline. After an authorization request may have
escaped, deadline expiry/overload retains uncertainty and never permits a
replacement grant or send. Proven pre-request capacity/readiness shortages are
instead Held as specified below; they are not ambiguous send attempts.

The stock sender acquires that wallet's shared operation permit BEFORE
requesting its fresh grant. Manual/automatic board, maintenance, recovery and
send selection share the same mutex. Guarded execution MUST reject a foreign
permit and MUST NOT reacquire the mutex. It validates the existing immutable
grant after acquisition and immediately before input registration and cosign
after intervening awaits. The local cosign escape is single-use; durable grant
and sender pin precede it. After possible escape, completion/reconciliation
keeps the same action and reservations and does not require a replacement
fresh grant. The payout dispatcher never takes wallet capacity.

**Known pre-request readiness (A1-RP1).** Before creating any nonce/request,
the worker MUST check the actual wallet's input state, eligibility, complete
proof/anchor availability, signing-key availability and supported address/
provider fee policy under the same operation permit. Zero/short, locked,
expired or depth-ineligible float cannot count as spendable; actual gross fee
coverage is checked, not a configured balance target. The pinned fee API's
zero arkoor fee is verified as the supported case, never generalized to waive
a nonzero/unsupported quote. These bounded read/planning operations perform no
input registration, locking or cosign. Their closed typed pre-request shortage
outcomes return Held without a new ambiguity pin, refund or terminal receipt.
Existing pins are checked first and never cleared. Later funding/confirmation
can make the SAME frozen order eligible for its first grant/send. The existing
PEL retains Held orders, with a persisted bounded cursor for fair same-consumer
restart retries. Failures after a request/grant remain on the unknown path.
This send-readiness gate does not gate mining needed to fund/confirm the float.

Receipt proof validation MUST compare the entire VTXO policy with the frozen
address policy and the VTXO's full server key with the frozen verified ArkInfo
key. Same-user HTLC or other-server proofs do not discharge a Pubkey order.
Canonical proof encoding, full signature/genesis/anchor verification, unique
VTXO IDs, exact total and durable grant binding remain mandatory. Aggregate
mode rejects legacy receipts, and Store MUST serialize legacy Sent against
member reservation/freeze to prevent partial settlement.

Orphan handling MUST preserve reserved/possibly sent or sent identities while
reversing unrelated reversible identities in the same block exactly once.
The block's orphan flag is not a reason to skip later member reconciliation.
No-authorization cancellation uses that same per-identity reversal boundary;
all monetary and Rule-B lifecycle updates commit together. Sent orphan value
remains recorded as pool loss, never refunded to pending.
An explicit recanonical transition of a `loss_held` block MAY restore the
complete original booked ledger only after fresh canonical confirmation at
the configured depth. Existing retained rows must exactly match frozen
effects and retain valid reserved/Sent Ark provenance; other original rows
are restored additively in the same transaction. Complete coverage and B>=R
are checked before commit. Retained Sent is never applied or sent twice.
Only that confirmed transition clears the recanonical member's orphan flag
and recomputes its aggregate's remaining pool loss. Missing raw provenance,
inconsistent retained effects, inadequate depth or reservation conflict holds
UNKNOWN rather than applying a partial ledger.

Receipt processing isolates invalid items. Terminal-invalid evidence is
durably quarantined before ACK. Missing anchors, canonicality or other unknown
evidence remains retryable and reserved. Pending-entry traversal is bounded
and fair across more than one batch and survives restart. One invalid or
unknown prefix MUST NOT permanently starve a later valid proof. Pool Sent
only wakes client retrieval; a wallet watch closes after wallet-verified owned
VTXO/proof or explicit terminal no-delivery reconciliation.

**Flow per found block.**

```
H_f            coinbase = [ pool_output = P2TR(pool_payout_descriptor @ index H_f) , P2TR(P_{i,k}, rung_j) per §4.4 … , carrier (JD) ]
                 pool_output = fee + rounding + Σ ark_due + Σ forfeited + Σ error-pending changes   (one hot output, §11.2;
                               Σ ark_due now includes kept miners' remainders, §4.6)
H_f + 6        booked (confirmation_depth); `ark` pays every ark_due id from its float (arkoor to U_i)
H_f + 100      pool_output matures; `ark` boards only what the float needs; the rest is operator surplus
```

**Separation principle.** The pool has two secrets with two different jobs, and compromise of either MUST NOT compromise the other:

| | SPc side (`front`, `spc-jds`, `payout`, Postgres) | Ark side (`ark` role) |
|---|---|---|
| Secret | `a_send[H]` (RAM, per height); the attribution data (`slice_credit`, snapshot `entries`, `ark_delivery`) | the `ark` seed: the pool's bark wallet **and** the `pool_payout_descriptor` private key (hot, by necessity: it cosigns arkoors, refreshes and boards unattended) |
| Protects | **privacy of SPc receivers**: which coinbase output pays which miner | **pool funds**: the float and the pool outputs |
| Compromise costs | one height's payee set, or the attribution DB until scrub; **no funds** (the SPc side holds only the descriptor's xpub) | the float plus whatever pool revenue the operator has not moved out; **no SPc receiver privacy** |

Concretely:

- **Independent key.** The `ark` seed MUST be generated from its own CSPRNG draw. It MUST NOT share a seed, xprv or derivation tree with the Noise `authority_public_key` or anything on the SPc side, and it MUST NOT be stored in the same secret store as Postgres/Redis credentials. Under it, bark's own wallet uses its usual paths, and the pool output keychain is a **separate account** (`m/86'/coin'/1'`, so it never collides with bark's BIP86 account 0): `pool_payout_descriptor = tr(<account xpub>/0/*)`.
- **Fresh P2TR pool output per height, indexed by the height.** `front` and `spc-jds` hold only the account **xpub** and derive `pool_output(H) = descriptor.at_derivation_index(H)` — a pure function, no coordination with `ark`, no state, and coinbase building never waits on `ark` or the Ark server. Every height gets a fresh key-path P2TR that looks like the SPc outputs beside it. The descriptor is a pool constant, so the xpub spec's three assertions (wildcard, no hardened step, not multipath) are checked once at startup. A compromised SPc side can link the pool's own outputs to each other; it learns nothing about miners and can spend nothing.
- **`ark` finds its outputs without a gap limit.** Indices are sparse (only heights the pool wins are ever paid), so `ark` MUST NOT rely on a gap-limit wallet scan. `ark` records matured pool outputs by `(txid, vout)` outpoint, never by height alone, and re-checks the active chain at maturity (H_f + 100) before boarding — two pool blocks at the same height share the script but not the txid; boarding is idempotent per outpoint. For each new block it derives the script for that height and checks the coinbase for it — one derivation per block.
- **Separate process, least knowledge.** The Ark wallet lives only in the `ark` role (`spc-pool --roles ark`, its own host in production). `ark` has **no** credentials for the pool's Postgres or the identity-bearing Redis keys. Its entire input is a send-order stream, and each order carries only what the **Ark server will see anyway**:
  ```
  ArkSendOrder   { order_id: [u8;16] (random, opaque), ark_address (policy U_i, delivery per §11.8),
                   sats, not_before_height, sig }                  # spc:ark:orders, payout → ark
  ArkSendReceipt { order_id, status: sent | failed, vtxo_id, expiry_height }   # spc:ark:receipts, ark → payout
  ```
  No `sp_id`, `B_scan`, `B_spend`, `P_i`, block hash or snapshot data crosses into `ark`, so a compromised `ark` host learns exactly the Ark server's view (destination keys, amounts, timing) and nothing about who was paid in any coinbase. `payout` alone maps `order_id → (block_hash, sp_id)`. The v2 4096-height retention extension (§5.1) is SPc-side only; it adds no identity field or longer retention to the Ark boundary.
- **Egress.** `ark` reaches the Ark server directly; Tor is not used for Ark egress (decision 2026-11-04). The server therefore sees the pool host's network address, which is operator-owned infrastructure rather than a miner's. What matters is the visibility below: the server already sees the pool's wallet, destinations, amounts and timing (§11.7), and no network control hides the wallet.
- **Orders are authenticated and capped.** `payout` signs each order with a dedicated order-signing key — secp256k1 BIP340 Schnorr, CSPRNG-generated (independent of the `ark` seed and all SPc keys; resolved 2026-10-14, `docs/research/output/ark-order-key-spec-recommendation.md`), stored hex-encoded 32-byte secret at `order_key_path` (default `/run/secrets/spc/ark-order-key`) with `0o600` file / `0o700` dir perms (bark `fs_perms` convention), x-only pubkey distributed to the `ark` role via config (`[ark] order_pubkey`), validated at startup; the signed message is defined in `WIRE-FORMATS.md` §3.4. Rotation is operator-driven: generate new keypair on `payout` host, update `order_pubkey` on `ark` host, restart `ark`; in-flight orders on rotation follow the existing retry/`failed_to_pending` logic (orders are short-lived, max `retry_blocks` = 6). **Order verification (`ark`)**: (1) parse per WIRE-FORMATS §3.4; (2) reject `sats == 0` or `sats > max_order_sats`; (3) hold (send nothing) while `not_before_height > tip`, per the send rule below; (4) reject if `ark_id` parsed from `ark_address` != configured `ark_id`; (5) reject duplicate `order_id` within the retry window; (6) verify BIP340 `sig` over `hash_SPc/ArkSendOrder(...)` against configured `order_pubkey`; any rejection increments `spc_ark_orders_rejected_total{reason}` and sends nothing. Rationale: `ark_address` binds the server (and thus network); `order_id` + `not_before_height` prevent replay; no identity or block-hash field is bound (privacy, §11.3).

**Server choice.** `[ark] server = "https://…"` MUST be a server **not operated by the pool**. Arkoor security is "at least one party (sender or Ark server) honest" (Second docs); a pool that is both removes the assumption ([critiques](research/raw/articles/2026-10-10-ark-oor-trust-payer-equals-asp-critiques.md) §1). Default: Second's mainnet server (`ark_id` pinned in config). A self-hosted `captaind` MAY be used **only on regtest/signet for tests**. RESULTS.md: a stock server and a stock pool-side bark wallet pay derived destinations unmodified.

**Paying (`H_f + 6`).** Ark-tier payouts for a block found at `H_f` are sent at `due_height = H_f + confirmation_depth` (6), from the float — the pool's own working capital, so miners do not wait for coinbase maturity. Pay only from VTXOs with `expiry_height − tip ≥ min_remaining_blocks` (default **3 024 ≈ 21 d**) so recipients inherit a near-full window; keep `arkoor_depth` under the server's `max_vtxo_exit_depth` by refreshing on depth, not only on expiry. A reorg deeper than `confirmation_depth` after the send is the pool's loss: arkoors are not reversible, which is the reason to wait 6 blocks rather than pay at find time.

**Reorg and float edge cases.**

_Normative (MUST):_
- `ArkSendOrder.not_before_height = H_f + confirmation_depth` (6), and `ark` MUST NOT send before its own chain tip reaches it.
- `payout` MUST re-verify that `H_f` is still on the active chain immediately before emitting each order (the send loop below; `ark` never sees the block hash).
- A reorg that removes `H_f` before depth 6 cancels the `ark_payout` row and reverses its ledger entries (send loop); the block no longer exists, so nothing is owed, pending or forfeited.

_Operator policy (defaults given; deployments MAY change them):_
- Reorgs deeper than 6 that touch already-sent orders are the pool's loss (above). Default: alarm on any invalidation deeper than `confirmation_depth` that affects an `ark_payout` row with `status='sent'`, then manual review.
- Where surplus above `float_target_sats` goes (cold storage, another setup, left in place) is operator-configured; the spec prescribes nothing beyond "never board more than the float needs" (Refilling, below).
- `float_target_sats` sizing: **mean = (per-find Ark flow including remainders) x finds-per-refill-lag (94 network blocks) x 1.5 margin; worst-case flow is bounded by `T`** (all identities below the base rung), so the absolute worst case is `float ≤ 94 × f × T × 1.5` and the 2026-11-03 record's measured means apply: era 1, decided rungs — m=100 identities ≈ 2.4e7 sats per find, m=1 000 ≈ 1.4e8, m=10 000 ≈ 2.2-2.8e8. (The "100x flow" phrasing once used is superseded; the 94-block lag `H+6 → H+100` is the correct window.) An operator MUST NOT size the float from sub-`C` balances alone, and MUST alarm on population drift: a growing pool moves toward the worst case.

**Refilling (`H_f + 100`).** When a pool output matures, `ark` boards **only what the float needs**: it builds a PSBT spending the pool output (key path) to `board_funding_address(..)` for `float_target_sats − float_balance` (if positive), with the remainder to an on-chain address `ark` controls, and registers it with `board_psbt` (`bark/src/board.rs:153-240`: the board output may sit at any index alongside other outputs). Everything not boarded is **operator surplus**. This spec does not define what the operator does with it — sweep to cold storage, pay it out over Ark to another setup, or leave it — only that `ark` never boards more than the float needs. Size `float_target_sats` to cover Ark-tier payouts **plus kept-miner overages** (§4.6, decision 2026-11-03) for the blocks the pool expects to find in one maturity period (≈ 1 block's worth for a 1 % pool, ≈ 10 for a 10 % pool), plus margin.

**Send loop.** `payout`, per `ark_payout` row reaching `due_height` (idempotent per `(block_hash, sp_id)`, §7.4): check block `H_f` is still on the active chain (if reorged out, cancel the row and reverse its ledger entries); build the address from the stored destination (§4.7, §11.8); emit a signed `ArkSendOrder`. **Remainder aggregation (decision 2026-11-03):** remainder rows below `min_overage_sats` do not emit individually; when the sum of an `sp_id`'s unsent rows reaches `min_overage_sats` (default 10 000), or the oldest unsent row is `overage_max_delay_blocks` (default 144) old, `payout` emits **one** order for the accumulated sum with `order_id` derived deterministically from `(sp_id ‖ block_hash of the oldest covered row)` — restart- and replay-stable, so `ark`'s dedupe keeps exactly-once, and every remainder sat is ledger-booked at settlement (it is part of `/api/pool/liability` until sent), so "eventually paid over Ark" is a ledger property, not a timing promise. `ark` sends and returns a receipt. On `sent`, `payout` sets `status='sent'` (for every covered row), records `vtxo_id`, `expiry_height`, and emits `ArkPayoutNotice` (§11.4); on `failed` or no receipt (Ark server down, float short), re-order next block (same `order_id`, so `ark` dedupes); after `retry_blocks` (default 6) set `status='failed_to_pending'`, credit `pending_balance`, and alarm. A failed send is the pool's failure, so it becomes pending, never forfeit.

**Why not make the coinbase output the board itself.** The pool does not run the Ark server, so everything here MUST work against a stock third-party `captaind`, and a stock mainnet server rejects a coinbase-funded board: mainnet forces `require_board_funding_tx = true` (`server/src/config.rs:587`), and the cosign path then requires every funding input's txid to be a known transaction (`server/src/lib.rs` board cosign, "invalid funding tx: unknown input tx"), which a coinbase's null prevout never is. Even with a patched server, the VTXO's exit transaction cannot confirm before coinbase maturity, so every arkoor recipient would have no unilateral exit for ~100 blocks, and the board's expiry clock would start before the funds are spendable. Recorded as O17. A covenant (O19) is what would actually remove this interactivity.

**Fees.** 0 % on sends/receives at Second today; the pool absorbs refresh and boarding costs out of its own fee. Nothing is deducted from miners for the tier.

### 11.4 Delivery capabilities - the SPc extension

**Identity vs delivery capability (section 0).** `user_identity` is the SP
address and nothing else; it alone keys the account. The optional delivery
capability is private, per-`sp_id`, per-Ark-server data and is never published.
There is no Ark-server-side registration endpoint; the Ark server only
unblinds. Per-block destinations are always derived (sections 2.3, 11.8); a
literal-address registration does not exist.

Extension identifier `0x5350` is negotiated through `0x0001`. Every v1
message payload starts `U8 version = 1`; old unversioned bytes and trailing
bytes are invalid. Message IDs are `0x01 SetArkIdentity`,
`0x02 SetArkIdentity.Success`, `0x03 SetArkIdentity.Error`,
`0x04 RegisterEpochMailboxes`, `0x05 RegisterEpochMailboxes.Success`,
`0x06 RegisterEpochMailboxes.Error`, `0x07 ArkPayoutNotice`. Exact fields are
in WIRE-FORMATS section 3.7.

`SetArkIdentity` carries `ark_id`, `b_ark`, `mailbox_mode`, `mailbox_pubkey`,
`mailbox_dh`, `epoch_fallback`, `sig`, and finally `request_id`. Mailbox modes
are 0 none, 1 static, 2 epoch_derived, 3 epoch_registered; fallback is 0
static or 1 hold. M and D are empty iff `none` and are valid compressed points
otherwise. `B_ark` MUST be valid and MUST differ from `B_spend`; `ark_id` MUST
match `ArkId::from_server_pubkey(server_pubkey)`. `hold` is accepted only in
`epoch_registered` mode; every other mode with `hold` is `bad-mailbox-fields`.
Invalid fallback byte/field combinations fail closed.

**Signed correlation amendment (Fable review required).** BIP340 by `b_scan`
authenticates the following digest, with the counter included even though it
follows `sig` on the wire:

```
hash_SPc/ArkIdentity(
  sp_id || version || ark_id || b_ark || mailbox_mode ||
  B0_33(mailbox_pubkey) || B0_33(mailbox_dh) || epoch_fallback || request_id)
```

The immutable `signed_fields_digest` is this digest. Unlike the earlier draft,
an old signed payload cannot be replayed with a larger unsigned counter to
roll back new delivery. `RegisterEpochMailboxes` signs the corresponding
`sp_id || version || count || entries || request_id` preimage under the
separate tag `SPc/EpochMailboxes`. The AP-3 client's local payload-reuse digest
MAY exclude `request_id` to identify one attempt; that comparator is distinct
from the signed wire digest and never substitutes for it.

The request is scoped `(sp_id, request_id)`. IDs start at 1, increase strictly
per identity, and never wrap; zero/exhaustion is `request-id-exhausted`. Two
users may each use ID 1. Retries of one attempt keep its ID; changing signed
fields requires a new ID. Same ID/different fields is
`registration-mismatch`. A journal-matching older request returns
`superseded` but has no effect; a different-digest older request is mismatch.
`stale-request` is a lower request ID that was never applied (including a
concurrent reorder loser or an old/pruned journal row), not a claim that the
pool has pruned a record.

`Success` carries `request_id`, payout-assigned `registration_rev`, `ark_id`,
and state 0 applied, 1 already_applied, or 2 superseded. `Error` carries
`request_id`, retryable, `error_code`, and detail. Terminal codes are
`bad-signature`, `invalid-point`, `ark-key-is-spend-key`, `wrong-ark-server`,
`bad-mailbox-fields`, `unsupported-mailbox-mode` (raw mode > 3, new),
`registration-mismatch`, `stale-request`, and `request-id-exhausted`.
`registration-commit-unknown` and `store-unavailable` are retryable;
`registration-busy` and `rate-limited` are reserved v1 error names and no
current path emits them. A structurally bad frame is `invalid-message`.

**Durable atomic resolution (migration 0006).** Only payout writes. The
`sp_identity` row is locked `FOR UPDATE`, then a strictly newer request
upserts identity, `ark_delivery` or epoch mailboxes,
`ark_registration_state`, the applied journal row and commit revision in one
transaction. State stores `last_request_id`, the digest and revision; the
journal is keyed `(sp_id, request_id)` and stores digest, outcome and
revision. No duplicate, mismatch, stale or superseded resolution applies an
older effect. The digest and revision never come from Redis or the client.
Before `is_new`, that same locked transaction first replays any existing
`(sp_id, request_id)` journal outcome. Existing different-digest rows resolve
`registration-mismatch` without INSERT or any effect. Equal-digest rows replay
their stored resolution: `rejected` keeps the original terminal error,
currently-applied rows are `already_applied`, and an older applied row remains
`superseded` after newer current state. A missing row alone may proceed through
the existing counter-check; a lower never-applied/pruned ID remains stale.
A validly parsed/signed request that fails semantic validation has outcome
`rejected` and its exact terminal code (`bad-mailbox-fields`, `invalid-point`,
`invalid-message`, `bad-signature`, `wrong-ark-server`, or another v1 terminal
code) plus a payout revision, not a delivery effect or current-counter update.
When an identity row exists this rejection is also journaled atomically, so
the stream entry resolves and its PEL entry can be acknowledged. Without an
identity row the FK-safe `invalid-message` rejection has no journal row but is
still terminal and replays deterministically. Actual database and Redis
failures leave entries unacknowledged as unknown/retryable; no semantic reject
is silently dropped as unknown commit.

The `spc:arkreg` stream has a new trailing request ID. `ArkRegApplied` carries
full current identity/delivery/fallback and changed epoch mailboxes together
with `request_id`, `registration_rev` and state. Superseded replay never
restores old fields. Bootstrap carries the same registrations and identity
keys plus `ark_epoch_mailbox` state. A committed but not propagated or
explicitly held destination routes pending (section 11.1), never forfeit.

Redis `spc:arkreg:ack` is a **3600 second notification cache, never durable
authority**. Each handle generates a fresh random `B016 attempt` only on the
internal `spc:arkreg` stream (after `request_id`), not on client wire and not
signed/digested. The ack key `reg:<sp_id_hex>:<request_id>:<attempt_hex>`
includes this nonce and its JSON echoes `attempt`. Acknowledgment therefore
requires identity, request ID, immutable digest, nonce and `expires_at` to
match. Every handle triggers the durable Postgres resolver even if an older
cached field is live: an old cached `Applied` can never bypass `superseded`.
Distinct concurrent handles cannot overwrite/steal each other's ack; attempt is
not a durable operation ID or an additional registration effect. On a miss
the same stream entry is published and resolved from Postgres.

Front waits at most five seconds for that exact acknowledgment. Timeout emits
retryable `registration-commit-unknown`, never an unregistered verdict. The
agent retries the same ID with 1, 2, 4, 8, 16, 30 second backoff (cap 30),
surfaces local `registration_unavailable` after six attempts while continuing
at 30 seconds, and does not mine until success/current guidance. A late commit
is valid; the next identical retry resolves from cache or Postgres. No success
is sent before successful durable commit.

`SetArkIdentity` has a real handler and advertisement only together: when the
handler or `[ark]` is disabled, API/RequestExtensions omit `0x5350`, and a
negotiated direct frame receives `unsupported-extension`, never synthetic
success. Registration carries no identity fields into the `ark` role; its
order/receipt contract in WIRE section 3.4 is unchanged.

`D` is not independently provable by the pool (that needs a DLEQ proof); a
wrong D dead-letters only the registering miner's payouts and remains signed
by that miner. For `epoch_derived`, D is still required for
`M_i + D + t_ark*S_mb`; an AP-3 semantic helper MUST NOT substitute `None`.
`RegisterEpochMailboxes` remains variant-A only and uses the same durable
request resolution. A validly signed registration for a non-`epoch_registered`
identity returns terminal `bad-mailbox-fields` and resolves the stream entry;
it does not retry forever as commit-unknown. A registration event without
valid identity keys is logged and skipped rather than aborting front.
`SPc/JobEntriesV1(sp_id || rail || dest_id length || dest_id)` is tracked
beside the coinbase fingerprint: a destination or rail-only change rolls the
job ID even when coinbase outputs are identical. Work from the previous job is
stale; snapshot `ON CONFLICT DO NOTHING` cannot conflate those generations.

`ArkPayoutNotice` retains block hash/sats/vtxo id/expiry
with a leading version byte.

### 11.5 Miner side (`spc-agent`)

**Ladder recovery clarification (2026-11-09).** A coinbase output does not
close the block's Ark mailbox: a kept miner can still be owed a remainder.
Recovery returns every owned `(k, vout, amount, tweak)`. Amount audit requires
an explicit source-format binding; historical exact-k0/JD blocks are never
claimed to follow current ladder policy merely because recovery found them.

One binary (§1). Holds **`b_scan`** (to verify its own `P` in every job/distribution, §3.6, to scan both tiers, §2.5, and to sign registrations, §11.4), the mailbox key(s) for its mode, and **`b_ark`** unless running watch-only; it MUST NOT hold `b_spend` or the master seed. Compromise of the agent therefore exposes privacy and unrefreshed Ark-tier VTXOs only — never a coinbase output.

**Key derivation, exactly (AP-3a, normative).** The agent's material is provisioned offline (the provisioner owns the master seed; `spc_crypto::provision` is its pure core and exists for fixtures):

- the **BIP 85 child wallet** is defined, not "BIP85-style": `child_mnemonic = BIP85(master, m/83696968'/39'/0'/12'/0')` (application 39, English, 12 words, index 0), `child_seed = BIP39(child_mnemonic).to_seed("")` — 64 bytes. The BIP 85 test vectors are pinned in-tree;
- the agent receives only the **branch account xprvs** `m/352'/0'/0'/1'` (scan) and `m/352'/0'/0'/2'` (ark), the 64-byte child-seed file, and the `sp_address` — never a master, never the spend branch. It validates: exact branch child index (not merely depth-4 hardened), matching network magic, the shared **parent fingerprint** between the two branch files (a required provisioning-metadata consistency check — the 4-byte fingerprint does not cryptographically prove the derivation path), `B_scan` against the `sp_address`, `B_ark != B_spend`, and the child seed's stock bark fingerprint against the in-tree derivation. Any failure is fatal at load; nothing is regenerated.

**Persistence layout (AP-3a, normative).** One wallet datadir holds two databases, fail closed on every ambiguity:

- `<datadir>/state.sqlite` — the agent's own store (schema version 2): the SP-aware `U_i → u_i` keystore (§11.8 item 2), owned VTXO records with expiry provenance (never a finding height), the per-identity monotonic registration counter/journal, and the chain scan cursor. Bound to one `(wallet_fingerprint, sp_id, network)` at creation; a reopen with different material, a schema-version mismatch, or a foreign non-empty sqlite file is refused — state is never wiped or recreated to make a mismatch go away;
- `<datadir>/bark/db.sqlite` — the **stock** bark persister (pinned `SqliteClient`, migrations included), initialized through the stock `BarkPersister::init_wallet` with `WalletProperties(network, fingerprint, server_pubkey = none, server_mailbox_pubkey = none)` and re-validated on reopen (same child-seed fingerprint and network, bytes untouched on refusal). Directories are 0700 and files 0600, created private **before** sqlite opens them. A missing database is the only genuinely-fresh case; a claimed-but-broken initialization is damage, never re-initialized. This is the persister **foundation**: opening the full stock bark `Wallet` (live chain source), balances, refresh and offboard are AP-3b.

**Registration client journal and terminal policy (AP-3a, normative).** The request id is reserved durably **before** any send; retries of the same attempt (same payload digest) reuse it, a changed payload allocates a fresh id (no self-wedge), and `u32::MAX` is refused without a wrap. Registration liveness is the **effective binding**: the payload is registered only while it matches the single most recent acknowledged `applied`/`already_applied` state — a historical journal row is not proof (applied A → B → A does not re-register A), and a `superseded` verdict clears the binding it names. Terminal handling: hard terminal refusals (`bad-signature`, `ark-key-is-spend-key`, `wrong-ark-server`, `bad-mailbox-fields`, `unsupported-mailbox-mode`, unknown codes) and counter exhaustion are **fatal immediately** — attempts cease, the exact code is surfaced, and recovery requires an explicit operator restart; the wedge kinds (`mismatch`/`stale`/`superseded`) get at most **one** fresh same-payload retry, then fatal. Every non-success outcome participates in the backoff schedule (1, 2, 4, 8, 16, 30 s, cap 30 s); after six failed attempts local `registration_unavailable` is surfaced while the 30 s retries continue.

**Status honesty (AP-3a, normative).** The loopback status JSON (§4.7; loopback enforced at bind, not only in config) reports `registered` (the effective-binding answer) and the role state as **distinct questions**: without a registration transport the role is **`dormant`** — a persistent registration ack is durable journal state, not a live connection, and a connection is never invented. Unknown aggregates are `null`, never zero; no key material, `sp_id`, `U` or VTXO id crosses the status surface. The scan cursor is monotonic-only in AP-3a: a chain reorg requires a journaled rewind (record the fork point, replay forward, then advance) before live scanning — an explicit AP-3b dependency, not implemented here.

- **Keys (DESIGN.md / MAILBOX.md paths, one master seed).** `b_spend` `m/352'/0'/0'/0'/0` (cold), `b_scan` `m/352'/0'/0'/1'/0`, `b_ark` `m/352'/0'/0'/2'/0`, per-epoch `m_i` `m/352'/0'/0'/3'/i'` (`epoch_registered`; the agent may hold only the `…/3'` account xpriv). The agent's bark wallet — whose `m/350'/1'` key is the static mailbox `M` — runs on the BIP 85 **child seed** above, not on the master itself, so that holding the bark wallet does not mean holding `b_spend`. One backup (the master) recovers both tiers. The `spc_ark_client` prototype derives everything from one seed for test convenience; production MUST split as above.
- **Hot/cold posture (DESIGN.md).** Scanning and recognising `U_i` watch-only need only `b_scan` and `B_ark`; signing for a VTXO (status attestation, refresh, spend, exit) needs `u_i = b_ark + t_ark`. `b_ark` MAY be held hot by the agent (default: refresh is unattended) or by an external signer, which MUST then come online inside the refresh window. `b_ark` is never `b_spend`, and `b_spend` is never used on the Ark tier.
- **Register (mandatory).** On connect, once per Ark server: `SetArkIdentity` (§11.4) with `D = b_ark·S_mb` computed by whoever holds `b_ark`. In `epoch_registered` mode, also `RegisterEpochMailboxes` per MAILBOX.md — with sends at `H_f + 6`, after-the-find registration has only ~6 blocks, so pre-register a height window instead. (Default mode is `epoch_derived`, which needs no per-epoch registration; resolved 2026-10-14.) The agent MUST NOT submit shares until `SetArkIdentity.Success`: unregistered earnings below `C` are forfeited (§11.1), so a registration failure is a hard stop, not a warning.
- **Receive.** For each pool block found while scanning (§2.5): insert `U_i → u_i` (or `U_i` watch-only) into the SP-aware keystore (§11.8 item 2), then fetch per mailbox mode — `static`: bark's normal sync of `M`; `epoch_*`: MAILBOX.md §3 on-demand reads of `M_i` at `H_f + 6 + [0, 6, 36, 144, 1008]`, closing the epoch when the VTXO for `U_i` arrives or an SPc output for that block is found; `none`: `ArkPayoutNotice` / API `vtxo_id` → `GetVtxosByIds` → `import_vtxos`. Validate each VTXO (ARK #2) and record it against the notice.
- **Refresh.** Auto-refresh at `vtxo_refresh_expiry_threshold` (bark default 144 blocks before expiry, inside the free window). Optional `refresh_on_receive_above_sats`: refresh immediately (`refresh/counterparty`) for payouts above a size the miner cares about, shedding the sender+server trust early at 0.2–0.5 %.
- **Dormancy.** The agent runs while the miner mines; a miner that stops for > 28 days with unrefreshed VTXOs is swept (ToS: "no obligation to return"). The agent MUST warn at shutdown if unrefreshed VTXOs exist or Ark payouts are still scheduled, and SHOULD offer `--offboard-on-exit <address>` (cooperative on-chain payment of the whole Ark balance, one tx) for anyone going dark.

**Live adapter invariants (AP-3b).** A wallet process MUST hold an exclusive
writer lock for its datadir. A new connection MUST obtain a fresh durable
registration resolution before reporting `connected`; a stored acknowledgment
alone is insufficient. Revalidation of an unchanged acknowledged payload uses
its original signed request id, provided no newer request is pending. An
acknowledgment MUST match both the reserved payload digest and wallet identity;
`registration-mismatch` invalidates the effective binding it names.

The full stock `Wallet::open` MUST use the existing validated Bark persister
and provisioned BIP85 child seed, with `create_if_not_exists = false`. Actual
ArkInfo server and mailbox keys MUST match configured pins before first-use
server-key persistence; fees MUST pass stock validation. Opening an unfunded
wallet does not establish a received balance. Real VTXO identifiers are the
stock 36-byte outpoints, never the AP-3a synthetic 32-byte test identifiers.
Synthetic persisted identifiers MUST be refused rather than truncated, padded
or promoted to live receipts. Raw receipt proofs and mailbox checkpoints live
in the existing agent SQLite database; checkpoints MUST advance only after
validated receipts and the sole SP balance ledger commit. Canonical block and
anchor proof, rederived owner key, mailbox key and server spend state are
required before a live receive becomes spendable. An unavailable proof remains
unknown, never a zero balance or guessed expiry.

Implementation evidence and outstanding runtime/maintenance work are recorded
in `reviews/2026-11-09-ark-wallet-bridge-implementation.md`; these invariants do
not assert that funded receive, refresh or offboard has been exercised.

**Runtime and maintenance (AP-3b).** Lab `wallet_agents` declarations are
independent of miner, worker, identity and hash-budget counts. Every Ark mining
connection MUST have one configured wallet writer in its pool before runtime
effects. Initial miner launch, dynamic miner addition and dynamic connection
addition MUST use the same current-registration/owned-process predicate. A
wallet startup hook MUST NOT activate miners before other startup prerequisites
(including native funding) complete. Unknown balance or float shortage does
not invalidate a current registration.

The wallet MUST journal canonical scan hash history and a reorg rewind before
replay; displaced receipt freshness MUST remain unknown until revalidated.
SP-owned maintenance signatures remain in the client adapter. Delegated round
outputs MUST match the requested policy, amount and unlock hash and pass
canonical funding and exitability checks before input forfeits are sent.
Cooperative offboard MUST validate the stock offboard/forfeit context and actual
fees. Requests and input reservations MUST persist before mutating RPCs;
ambiguous replies MUST retain uncertainty and MUST NOT trigger blind duplicate
spends. An initial delegated-round reply lost before learning its unlock hash
has no standard client-id lookup and remains unknown. Owner commands are
strictly scoped to the wallet's private command queue; secrets MUST NOT appear
in argv, public JSON, hub projections or Ark-role streams.

### 11.6 Audit across both tiers

At booking (`confirmation_depth` = 6), the agent asserts
`owed(id) [from the type-32 window] − paid_coinbase [Σ over k of own P_{i,k} outputs found in the coinbase] − ark_due [pool-reported] − forfeited [pool-reported; MUST be 0 for a registered agent] − Δpending [pool-reported] == 0`;
with denomination payouts (§4.4) this identity is unchanged in shape but two-part in expectation: `paid_coinbase` MUST equal exactly `owed − (owed mod 500_000)` on the constant ladder (any other composition at `OP_1 ser_x(P_{i,k})` is a misbehaving pool, fail closed), and `ark_due` MAY be > 0 for a kept id — it is the remainder `owed mod 500_000`. When the Ark send lands (at `H_f + 6`, or later under the §11.3 aggregation rule — the agent tolerates the delay and tracks the accumulated sum) it asserts the VTXO(s) received for `U_i` carry the accumulated `ark_due`. It alarms otherwise. Type-32 covers "what was I owed"; the coinbase output and the VTXO cover "what was I paid"; `/api/sp/me` covers "what is still in flight". ArkPool's hash-chained receipts are the post-hoc half of this only.

### 11.7 Trust and privacy statement (normative text for README)

Following DESIGN.md "Privacy analysis, by observer":

- **Custody.** Ark-tier balances are custodial for the 6-block confirmation window, then **trustodial for up to 28 days**: safe if either the pool (sender) or the Ark server is honest. The pool deliberately uses a server it does not control so that this remains a 1-of-2 assumption. Swept VTXOs are unrecoverable. Earnings below the coinbase threshold of a miner who has not registered Ark delivery are forfeited to the pool.
- **Pool compromise is split.** The pool's hot Ark seed is a separate key on a separate host that never sees who was paid in a coinbase; compromising it risks the pool's funds (float and unswept pool revenue), never SPc receiver privacy. Compromising the mining side risks attribution data and nothing spendable (it holds only an xpub, §11.3).
- **The pool** sees everything on both tiers, as it must to do the accounting; one identity makes this no worse. Mitigation is operational (scrub, §5.1).
- **On-chain observers** see coinbase-tier outputs only; the Ark tier is invisible on-chain, and derived Ark keys leak nothing. Coinbase amounts are **denomination-shaped** (§4.4): every payout output is one of the four public constant rung values, so the observer learns the per-rung counts, the block total and timing — not an exact amount per miner; the sub-base remainders leave the chain on the Ark rail. **Never publish a bundle** of SP address and Ark material (e.g. a BIP 321 `?sp=…&ark=…` URI): it would be the correlation point the design avoids.
- **The Ark server** sees every Ark-tier destination key, amount and timing, and can parse public coinbases. Bursts of pool-originated arkoors ~6 blocks after each pool block are fingerprintable, and amounts map to the pool's known payout distribution; the pool's boards, ~100 blocks after its blocks, link its coinbase outputs to its Ark wallet (the pool's own outputs, not miners'). **With denominations, coverage becomes total (accepted tradeoff, decision 2026-11-03): the server receives the exact remainder of every paid miner every block, and exact full balances of sub-base-rung miners.** Amount privacy from the chain is thus exchanged for exact-remainder exposure to the Ark-server trust assumption this design already makes. In `static` mailbox mode it links all of a miner's Ark payouts through `M`; the `epoch_*` modes reduce that to one block. The network-level residue MAILBOX.md describes is **not** mitigated by this pool: mailbox reads are direct (decision 2026-11-04), so the mailbox endpoint sees the reader's network address. Jitter and a fresh connection per epoch read remain the only counters, and a miner who needs network separation uses an operator-level control. If a coinbase-tier miner later boards that output into Ark, the server links it to their Ark activity. Second publishes no privacy policy.
- **Irreducible costs**: amount fingerprinting on the coinbase tier is **bounded, not removed** — denomination classes still compose (a whale's top class is near-singleton), per-block counts and timing are untouched, redemption-time consolidation re-links a miner's outputs, and with §4.4 the residual amount signal lives entirely on the Ark rail (above). Tier transitions remain observable to anyone watching both layers. Soft mitigations only: timing jitter, boarding coinbase outputs via paths that do not advertise "fresh coinbase payout", wallet discipline against consolidation.

### 11.8 Ark-tier derivation and delivery (DESIGN.md, authoritative)

The Ark destination comes from the **same per-block ECDH** as the coinbase output, domain-separated by tag (§2.3 is normative; prototype `spc_ark/`):

```
t_ark = hash_SPc/TierArk(ecdh_i ‖ ser32(k))   →  U_i = B_ark + t_ark·G        (the VTXO Pubkey policy key; k = 0)
t_mb  = hash_SPc/Mailbox(ecdh_i)               →  epoch_derived mailbox tweak  (MAILBOX.md variant B)
```

Both fail on a digest of 0 or `≥ n` (§2.3). The miner recomputes `ecdh_i` for every block the pool found, so it finds Ark payouts for blocks where it was below threshold with no notification.

**Delivery, by mailbox mode.** bark blinds a mailbox id as `blinded = M + u·S_mb`; the server unblinds `blinded − s_mb·U = M`. The pool lacks `u = b_ark + t_ark` but, by linearity, needs only `D = b_ark·S_mb` (DESIGN.md "Recommended approach" 1; tested in `pool_blinding_matches_bark`):

| `mailbox_mode` | Pool computes per payout | Wallet reads | Server can link a miner's payouts? |
|---|---|---|---|
| `static` (DESIGN.md) | `blinded = M + D + t_ark·S_mb` | `M` (bark's normal sync) | yes, all of them, via `M` |
| `epoch_derived` (v0.1 default; resolved 2026-10-14, MAILBOX.md variant B) | `M_i = M + t_mb·G`; `blinded = M_i + D + t_ark·S_mb` | `M_i` with `m_i = m + t_mb`, on demand | no (only within one block); `m_i` are related keys — privacy-only exposure (see caveat below) |
| `epoch_registered` (MAILBOX.md variant A) | registered `M_i`; `blinded = M_i + D + t_ark·S_mb`; if `M_i` missing at send: `static` or hold, per `epoch_fallback` | `M_i` from `m/352'/0'/0'/3'/i'`, on demand | no; independent per-epoch keys |
| `none` (DESIGN.md "Fallback") | address with an **empty delivery list** (valid per `check_delivery`) | nothing; fetch by `vtxo_id` and `import_vtxos` | not by mailbox |

The address is `ark::Address::builder().server_pubkey(..).pubkey_policy(U_i).delivery(ServerMailbox { blinded })` (`spc_ark::build_bark_address`); `BlindedMailboxIdentifier::from_pubkey(U)` alone is a dead letter (DESIGN.md (b)). Default is `epoch_derived` (resolved 2026-10-14, `docs/decisions/2026-10-14-mailbox-default-epoch.md`). **Caveats on `epoch_derived`:** (a) mailbox keys are additive tweaks of static `m` — anyone holding BOTH static `m` and `b_scan` can derive all `m_i`; this is privacy-only exposure (VTXO spending needs `u_i = b_ark + t_ark_i`, which stays independent); (b) reads of `M_i`, `M_j` from one IP/connection are linkable at the network level — use a new gRPC connection per epoch, Tor stream isolation where possible (MAILBOX.md §4).

**The receiving wallet needs an SP-aware keystore** (DESIGN.md "Recommended approach" 2). Stock bark resolves keys only via `m/350'/0'/idx` and drops a VTXO to `U_i` as "not owned by this wallet" (`find_signable_clause`, and `import_vtxos` → `KeyNotFound`) — confirmed against a stock server in RESULTS.md. `spc-agent`'s embedded wallet consults an extra-keys source holding `U_i → u_i` for each scanned pool block; signing is already abstracted over a plain `Keypair`. Patch vs fork is O13.

### 11.9 Tests (extend §8)

- Unit: `SetArkIdentity` matrix — bad `b_scan` signature, signature over another `sp_id`, `B_ark = B_spend`, invalid points, wrong `ark_id`, unknown mode, `none` with `M`/`D` present, `static` without them → exact error codes; nothing derived for a rejected registration. DESIGN/MAILBOX frozen vectors per §8.1.
- Property: §4.6 (a)–(f) hold with the tier enabled (Ark-tier ids have no coinbase output); separately, per settled block `pay_P = fee_P + rounding + Σ ark_due' + Σ forfeited + Δpending`, where `fee_P` is the pool's own fee share (§4.5 `pool` minus rounding), `Σ ark_due'` includes kept ids' remainders (§4.6), and `Δpending = Σ pending' − Σ pending` (error rail only). Ladder-specific: every coinbase output's value ∈ `{500_000, 1_000_000, 3_000_000, 5_000_000}` (constant); the greedy decomposition is canonical (largest-first, repeated rungs allowed); per kept id `Σ_k paid == owed − (owed mod 500_000)` and `rem < 500_000`; total outputs `≤ T/500_000 < max_payout_slots`; a fee-heavy block (`T ≫ subsidy`) leaves the ladder unchanged — it depends on the amount only — and costs output count, bounded by the same value rule.
- Integration (regtest, `captaind` as the *third-party* role, `confirmation_depth = 6`): five agents; block found at `H_f`; the above-`C` miner gets a coinbase output; three registered below-`C` miners in `static`, `epoch_derived` and `none` modes get **nothing before `H_f + 6`**, then exactly one VTXO each from the float — in `M`, in `M_i` (static `M` stays empty), and via notice + `import_vtxos` respectively — with a full expiry window; an unregistered below-`C` miner (hand-rolled client) is booked `forfeit`, its sats in `pay_P`, nothing pending; agents reconcile §11.6 to zero; a `SetArkIdentity` for another miner's `sp_id` without its `b_scan` signature is rejected; `spc-agent` against a pool that rejects its registration submits no shares; kill the Ark server → sends fail → retried → `failed_to_pending` after `retry_blocks` (never forfeit); reorg of `H_f` before `due_height` cancels its Ark payouts; two competing same-height pool blocks → ark boards exactly one outpoint; at `H_f + 100` the pool output is found by its height-indexed script and only `float_target_sats − float_balance` is boarded, the remainder left on-chain under `ark`; with the float already at target nothing is boarded; `--offboard-on-exit` produces one on-chain tx.
- Registration v1: exact message IDs/layouts/version/trailing-byte rejection; bad signature/other identity, replaying an old signature with a bumped request ID, invalid points, wrong server, unknown mode, `none` with M/D, and non-`none` missing M/D return exact errors. Two identities each apply ID 1. A fresh per-handle internal attempt nonce prevents cached `Applied`/`already_applied` from bypassing durable `superseded`, even with no expiry or intervening mismatch; concurrent identical requests cannot steal acks; same ID/different digest is mismatch. An older request after a newer one is superseded or stale and never restores delivery; counter wrap/zero is refused. A five-second miss is retryable commit-unknown, and a late durable commit followed by an identical retry succeeds. Concurrent same/higher requests serialize on the identity row lock, preserve one journal row per applied ID, and leave the greatest counter current. Redis cache expiry and front/payout/Postgres restarts recover from Postgres, not global/unscoped cache state. A signed static-mode epoch request returns terminal bad-mailbox-fields with its journal entry and clears PEL rather than looping forever; subsequent records still apply. Destination/rail-only changes roll job ID and make old work stale while coinbase output/fingerprint stays unchanged. Success is observed only after durable apply.

- Registration publication: actual raw Noise/SV2 negotiation and response, API advertisement matching a constructible handler, and disabled negotiation/request never succeeding. Real events plus bootstrap restore full identity/delivery/epoch state. All four destination modes bind the registered U_i, Ark server public metadata and current job generation; frozen block events retain `dest_id`. `epoch_registered + hold` without its height key and registration committed but underived route pending, not forfeit. Captured Ark order/receipt payloads contain no `sp_id`, `B_scan`, `B_spend`, `P_i` or block hash; these AP-2 fields never cross the `ark` role boundary. Wire/durability/derivation tests use synthetic public server keys only and make no actual Ark-service or refresh/offboard claim.

- Negative: pool configured with its own `captaind` on mainnet → refuses to start (`ark.server_is_self = true` requires `network ∈ {regtest, signet}`); `pool_payout_descriptor` with an xprv, a hardened step, multipath, no wildcard, or a non-`tr()` wrapper → refuses to start.
- Separation (§11.3): `ark` with only its own config and Redis ACL can execute orders but cannot read any Postgres table or identity-bearing Redis key (connection attempts fail); a captured `spc:ark:orders` stream contains no `sp_id`, `B_scan`, `B_spend`, `P_i` or block hash (schema test); unsigned, wrong-`ark_id` and over-`max_order_sats` orders are rejected and nothing is sent; the SPc-side processes hold no private key for `pool_payout_descriptor`; `ark` refuses to start if its seed file is readable from the SPc-side secret store path; mainnet config with `ark` in the same process as `front`/`payout` refuses to start.

### 11.10 Configuration additions

```toml
# spc-pool.toml additions (coinbase_threshold_sats lives in [accounting], §6.3)
[ark]
enabled = true
server = "https://ark.second.tech"       # third-party; pinned ark_id below
ark_id = "…"                              # first 4 bytes of SHA256(server_pubkey), cross-checked on startup
server_pubkey = "…"                      # ArkInfo server pubkey (66 hex chars); REQUIRED for selected front role
mailbox_pubkey = "…"                     # ArkInfo mailbox pubkey (66 hex chars); REQUIRED for selected front role
float_target_sats = 0                     # REQUIRED: board up to this at each matured pool output; size = ark_flow_per_find x finds per 94-block refill lag x >=1.5 margin (§11.3; m=1000 identities at u=10k ≈ 5.0e6 sats/find — see the 2026-11-03 record)
max_order_sats = 0                        # 0 = 5_000_000 sats circuit-breaker default: any single remainder is < 500k and an aggregated sum rarely exceeds 5M; over-cap orders alarm and wait, they are never dropped (§11.3)
min_overage_sats = 10000                  # remainders below this aggregate per sp_id across blocks (§11.3); 0 = send every row
overage_max_delay_blocks = 144            # force-emit an aggregate when its oldest unsent row is this old
max_daily_sats = 0                        # optional extra cap; 0 = off
order_pubkey = "…"                        # ark side: BIP340 x-only pubkey (64 hex chars) of the order-signing key (CSPRNG, independent of ark seed and all SPc keys), verifies payout's order signatures; REQUIRED
order_key_path = "/run/secrets/spc/ark-order-key"   # payout side: order-signing key (secp256k1 BIP340, CSPRNG-generated,
                                          # independent of ark seed and all SPc keys); file 0o600, dir 0o700 (bark fs_perms)
seed_path = "/run/secrets/ark/seed"       # ark's own secret store; MUST NOT be shared with SPc-side secrets
pool_output_account = "m/86'/1'/1'"       # pool output keychain under the ark seed (coin type per network); its xpub is [pool] pool_payout_descriptor
min_remaining_blocks = 3024               # pay only from VTXOs with ≥ 21 d left
retry_blocks = 6
server_is_self = false                    # true only on regtest/signet

# spc-agent.toml additions
[ark]
enabled = true
mailbox_mode = "epoch_derived"            # "epoch_derived" (default) | "static" | "epoch_registered" | "none"
epoch_fallback = "static"                 # epoch_registered only: "static" | "hold"
signer = "local"                          # "local" (b_ark hot) | "external" (watch-only U, §11.5)
refresh_on_receive_above_sats = 0         # 0 = never; else refresh/counterparty immediately
offboard_on_exit = ""                     # optional on-chain address
# Wallet role (§11.5; AP-3a). Paths and ids only - key material lives in
# the referenced 0600 files, never inline:
seed_child = "/run/secrets/spc/child.seed" # 64-byte BIP 85 APP-39 child seed (§11.5 derivation)
scan_xprv = "/run/secrets/spc/scan.xprv"  # m/352'/0'/0'/1' branch account xprv (0600)
ark_xprv = "/run/secrets/spc/ark.xprv"    # m/352'/0'/0'/2' branch account xprv (0600)
datadir = "/var/lib/spc-agent/wallet"     # state.sqlite + bark/ (dirs 0700, files 0600)
status_listen = "127.0.0.1:34256"         # loopback-only status JSON (default; enforced at bind)
network = "regtest"                       # when the sp_address HRP cannot tell (tsp is ambiguous)
ark_id = "…"                              # target Ark server id (8 hex chars, registration)
```

`spc-agent` carries its upstream SV2 and JDP traffic through `[net] proxy` when it is set (§3.8). Ark traffic is direct for both binaries (decision 2026-11-04). `[net]` is defined once in §6.3 and is not repeated here.

`ark` refuses to start while a REQUIRED value is 0, or if `pool_output_account`'s xpub does not match `[pool] pool_payout_descriptor`. With `max_daily_sats` set, an order over it is not dropped: it waits for the next day's allowance and alarms, so a legitimate burst costs latency, not funds.

## Sources

- [Roadmap: SPc pool](ROADMAP.md) — the eight decisions this spec implements
- [Ark (bark) as a Sub-Threshold Payout Rail](research/wiki/concepts/ark-sub-threshold-payout-rail.md), [Payout Threshold Economics](research/wiki/concepts/payout-threshold-economics.md) — §11 grounding
- [bark send-side read](research/raw/repos/2026-10-10-bark-barkd-openapi-source-boat001.md), [bark key-side read](research/raw/repos/2026-10-12-bark-key-management-mailbox-blinding-sp-derived-vtxo.md) — §11.3–11.5, Appendix 11.8
- [Inventory: coinbase-native stealth payout](research/inventory/candidates/coinbase-native-stealth-payout.md) — construction, carrier table, SV2 budget, close-out condition 1 (this spec's §2 is the "spec diff")
- [LL 2026-08-14](research/raw/notes/2026-08-14-ll-coinbase-silent-payments-ecdh-nonce.md) — height nonce, `k = 0`, light client, sender-side split
- [LL 2026-08-16](research/raw/notes/2026-08-16-ll-sv2-pool-tag-asend-carrier.md) — no BIP 32 for `a_send`; `String` tag type; budget; rotation as a knob; wire-format-enforced invariants
- [LL 2026-08-24](research/raw/notes/2026-08-24-ll-jd-coinbase-control-amount-linkability.md) — JD carriers, `OP_RETURN`, unhidable suffix, amount ceiling, standalone pitch for the tag patch
- [delvingbitcoin t/2833](research/raw/articles/2026-10-09-delving-silent-payments-coinbase-thread.md) — Seldenthuis's budget correction; Anzus's scanning-profile question; optout's BIP-posture question
- [PPLNS-JD / SLICE](research/wiki/concepts/pplns-jd.md), [SV2 Share Accounting Extension](research/wiki/concepts/sv2-share-accounting-ext.md) — `N = 8 × D`, window definition, type-32 messages, identity-free types
- [xpub Payout Identity](research/wiki/concepts/xpub-payout-identity.md), [Coinbase Address Rotation](research/wiki/concepts/coinbase-address-rotation.md), [Spec rev. 5](research/output/plan-xpub-miner-identity-spec-2026-07-29.md) — ledger-key split, fail-closed, per-connection payout slot
- [Coinbase Amount Linkability](research/wiki/concepts/coinbase-amount-linkability.md), [Payout Attribution Privacy](research/wiki/concepts/payout-attribution-privacy.md), [Block Withholding](research/wiki/concepts/block-withholding.md) — threat model boundaries
- [Blitzpool raw](research/raw/repos/2026-07-27-blitzpool-server-rust-github.md), [finder-bonus code read](research/raw/repos/2026-07-27-blitzpool-finder-bonus-code-read.md) — ledger, trim, autoscaler, roles, snapshot fingerprint, vouching invariant, duplicate-merge landmine
- [share accounting & durability](research/sibling/mining-pool-architecture/wiki/concepts/share-accounting-and-durability.md) — no per-share rows; replay set non-persistence
- External pins: `bip-0352.mediawiki` v1.1.1 (§Versions, §Spending, `:88–:99`, `:299`, `:319`); `sv2-spec` @ d8bf5b8 (`05-Mining-Protocol.md` `user_identity`/`error_code` = `STR0_255`; `extensions/0x0001-extensions-negotiation.md`); PR #203 `0x0003-coinbase-transaction-payouts.md` @ 47dbedd (unchanged since 2026-09-10, verified 2026-10-12); `share-accounting-ext/extension.md` @ c661e93; `lorbax/pplns-with-job-declaration.tex`; `bip-0323.mediawiki` (context only)
