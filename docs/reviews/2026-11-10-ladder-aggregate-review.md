# Ladder aggregate/send-authorization source audit

Date: 2026-11-10
Verdict: **REQUEST_CHANGES** for the five concrete findings below.
Scope: frozen aggregate selection, send authorization, receipt application,
owner API and privacy boundaries. This is not a full-lab approval gate.

## Frozen source and evidence

- Source snapshot: `415d21fa88ad2124712cc1fe9a8400bc7fbdd951bd005f223f92f879068f2602`.
- Delta SHA256: `cce40543aaa5fa8d59e52a5ab2477ffb8420a4a0c0233bc48da7fdb16f3e731b`.
- Binding: `/tmp/opencode/ladder-completion-r2-binding.json`.
- Owner API: `/tmp/opencode/ladder-completion-owner-api-r1.json`.
- Source root: `/home/ethan/.local/share/opencode/worktree/ad8886/lab-ladder-completion`.

Recomputed the source recipe, SHA256 of compact sorted-key JSON `{base,files}`,
and the delta patch hash. At initial inspection all 697 source rows matched.
During review the live accounting implementation and SPEC began changing for
Rule B. Recovered the frozen rows from
`/tmp/opencode/ladder-completion-rule-b-predecessor.tar.gz`, whose SHA256 is
`6e88fd6adaad879d87fb5a9a6faff1ad79733f090ea7a78370cbfac740606ef4`.
All 50 archived delta files match the frozen manifest hashes and git modes.
Combining them with the verified unchanged rows reconstructs all 697 files,
11,339,926 bytes, with zero hash/mode mismatches. No archive was extracted to
disk. All citations below name the frozen files and their frozen line numbers;
this verdict does not apply to later live edits.

Read current AGENTS, the already-loaded hash-identical DESIGN, frozen SPEC §11
and frontmatter, the dated aggregate-anchor/send-authorization decision, WIRE
§3.8, candidate report and exact owner API contract. Verified every receipt
hash listed in the binding. Logs record 260 pool tests, three real PostgreSQL
aggregate tests and passing targeted clippy. The native receipt covers all 990
outputs in its stock-Bitcoin component case, not funded Ark execution.

The three PostgreSQL tests exercise real transactions, duplicate receipt
serialization and orphan/receipt races, but supply synthetic destination and
proof data directly to store methods. They do not exercise the live
authorization timing or `apply_proved_receipt` cryptographic boundary. No build,
test, service, funded action, subagent, commit or candidate write was performed
by this auditor. Reproductions below are source-derived cases to add to the
focused checks, not claims of newly executed funded tests.

## Findings

### A1 - P1: normal idle-loop timing permanently strands newly published orders

**Source:** `bins/spc-pool/src/payout.rs:1263-1313,2056-2066,2252-2289`;
`crates/spc-store/src/redis.rs:55-77`;
`bins/spc-pool/src/ark/mod.rs:399-402,478-510`.

The worker allows only two seconds for its authorization response. Payout
services authorizations once at the top of its serial loop, publishes orders
later, then performs separate blocking reads of slices, blocks and
registrations. Each empty read uses `XREADGROUP BLOCK 1000`.

**Reproduction:** with those three streams quiet, let payout publish a due
aggregate and let the Ark worker promptly request authorization. Payout then
spends roughly three seconds in the three reads before servicing the request.
The worker times out at two seconds and permanently pins the order. Payout can
subsequently commit a valid grant, but the worker has abandoned its response;
the durable one-grant rule prevents replacement. A healthy idle deployment can
therefore strand an unsent batch without a network or database failure.

**Minimal fix:** service the request/response path independently of unrelated
blocking stream reads, preserving the nonce, expiry and durable one-send
invariants. Test the actual quiet-stream scheduling path with a newly published
order; increasing a timeout alone does not bound the other sequential work.

### A2 - P1: a valid proof for the same pubkey can discharge a different payout policy/server

**Source:** `bins/spc-pool/src/payout.rs:2395-2467`, especially `2420-2435`;
`crates/spc-ext/src/authorization.rs:10-16,112-127`.

SPRP validation compares only `vtxo.user_pubkey()` with the address policy's
user pubkey. It does not require equality of the full VTXO policy, or bind the
VTXO server key to the order's intended Ark server. `vtxo.validate(anchor)`
validates the supplied VTXO under its own policy/server; it cannot establish
that these are the policy/server the frozen order requested. The genuine
signed grant authorizes the order, but does not sign the subsequently supplied
VTXO list.

**Reproduction:** obtain a genuine grant, then supply an otherwise valid full
VTXO proof of the exact amount and U, but with an HTLC policy whose preimage
the recipient lacks. Its signature/ancestry checks can pass and all members
become Sent, although the requested plain Pubkey payment was not delivered.
A proof under a different Ark server is also not rejected by this comparison.
This is not hypothetical pubkey behavior: pinned Bark
`lib/src/vtxo/policy/mod.rs:982-1006,1065-1073` exposes HTLC constructors and
returns the same `user_pubkey` field for Pubkey and HTLC policies.

**Minimal fix:** verify full policy equality and the expected server identity
against the frozen order/configured server metadata before committing any
receipt. Keep existing full-proof, unique-ID, amount and grant/journal checks.
Add same-U/wrong-policy and same-U/wrong-server proof cases with a genuine grant;
neither may change aggregate or member state.

### A3 - P1: legacy Sent receipts bypass the aggregate proof and whole-set boundary

**Source:** `bins/spc-pool/src/payout.rs:2474-2536`;
`crates/spc-store/src/postgres.rs:1415-1445,1459-1464`.

Even in aggregate mode, any receipt without the SPRP prefix falls through to
the legacy decoder. `apply_receipt` searches all scheduled emissions using the
old deterministic per-row ID, then calls `mark_ark_sent`. That store method
does not reject reserved aggregate members, authenticate a grant, validate a
VTXO proof or require all members. The adjacent Failed-to-pending method does
have the reservation guard; the Sent method does not.

**Reproduction:** freeze a two-member aggregate, then deliver a delayed or
injected legacy Sent receipt with `ark_order_id(member_hash, sp_id)` and an
opaque ID. One member changes to Sent with no SPRP proof. The aggregate stays
published/authorized and its later genuine complete receipt fails the
`status='scheduled'` all-member update. Thus an invalid partial receipt both
discharges one row and prevents normal whole-set resolution.

**Minimal fix:** refuse legacy receipts for aggregate-mode obligations and add
the reserved-member exclusion to `mark_ark_sent` itself, matching the Failed
guard. Preserve any intentional legacy compatibility only for explicitly
unreserved legacy obligations. Test delayed legacy Sent against a frozen set
and assert no member changes before a valid full receipt resolves it.

### A4 - P1: aggregate reorg handling skips reversal of unrelated booked identities

**Source:** `crates/spc-store/src/postgres.rs:1146-1177,1183-1188,1201-1238`;
`crates/spc-store/src/aggregate_store.rs:294-343`.

When any reserved aggregate touches a block, `Store::orphan` marks aggregate
members and the block orphaned, then returns before the normal ledger/pending
reversal. This skips every other identity's unreserved ledger row in that
block, not just the potentially sent covered set. Subsequent no-authorization
cancellation deletes/reverses only that aggregate's own members. The block is
already orphaned, so the normal orphan path cannot later repair the remainder.

**Reproduction:** book block H with A's Ark payout reserved in an aggregate and
B's independent error-held `pending_delta=+100`. Reorg H. A's reservation must
remain conservative, but B's unapplied-to-Ark, now-orphaned earning must be
reversed. The aggregate early return leaves B's 100 sats spendable as pending.
Cancel A using valid no-authorization evidence: B's row and balance still
remain. This also affects unrelated unreserved Ark rows. It is a booked-state
reorg defect, distinct from the explicitly excluded provisional-pending Rule B
work.

**Minimal fix:** preserve potentially sent aggregate members while atomically
reversing the block's other reversible ledger effects exactly once. Ensure
later aggregate cancellation cannot double-reverse them. Extend the real-PG
race coverage to two identities in one block, including an unreserved pending
row and a reserved authorized or never-authorized aggregate row.

### A5 - P2: one bad proved receipt prevents later pending receipts from resolving

**Source:** `bins/spc-pool/src/payout.rs:2474-2492`;
`crates/spc-store/src/redis.rs:86-108`.

The SPRP branch propagates `apply_proved_receipt` failure with `?`, aborting
the entire batch. XREADGROUP has already assigned all returned entries to the
PEL. Retries always read the oldest 64 pending entries from ID 0. An invalid
SPRP receipt, or one whose anchor is unavailable, therefore blocks the valid
entries following it on every retry.

**Reproduction:** return two new receipts in one read: malformed SPRP first,
valid authorized receipt second. The first fails; both remain pending. With
no further new entries, every pass hits the same first failure and never
applies the second. More than 64 unresolvable entries can hide the rest even
after replacing the early return with a per-item continue.

**Minimal fix:** isolate per-receipt failures and give pending entries bounded
fair service. Terminal invalid data must not monopolize retry work; temporary
proof unavailability must retain uncertainty without blocking unrelated orders.
Test malformed-before-valid and an unavailable prefix longer than the pending
read limit. Do not turn either failure into a refund or a fabricated receipt.

## Boundaries that held in this source review

- Selection sorts stored height/hash/identity, rejects mixed identities and
  duplicate members, checks sums and caps, and uses the oldest selected row's
  stored destination. Freeze rechecks durable amounts/status/destination under
  locks. The order/member rows commit before publication intent and XADD.
- Migration 0007 makes frozen fields immutable, enforces one reservation per
  member and a unique 16-byte order ID. The batch-sequence-derived ID is backed
  by a primary-key INSERT, so a collision aborts rather than overwrites another
  payment. Stored order bytes are checked against their full digest.
- Fresh nonce/digest/revision grants are genuinely signed. Payout checks all
  covered hashes at a stable observed tip and journals potential-send state
  before publishing the grant. The worker verifies correlation and expiry
  twice, pins before transport and never obtains a second grant after a lost
  outcome. A1 concerns scheduling-induced loss of that otherwise sound grant.
- The dedicated full-receipt store path applies members and receipt state
  atomically and idempotently, checks total and authorization journal, and
  records sent orphan value as pool loss. A3 is the alternate legacy bypass;
  A4 is the mixed-identity reversal omitted by the early-return branch.
- Local NoEffect cancellation requires proof that no authorization row exists,
  tombstones the old order and releases only its membership. Authorized/unknown
  orders retain reservations; no timeout or generic transport error authorizes
  regrouping. No custom Ark-server method was introduced.
- Ark order/authorization codecs carry opaque order/digest/nonce/revision data,
  not identity or member hashes. Ark-only runtime configuration omits Postgres
  and the order signing secret; the Redis ACL adds only the opaque request and
  grant keys. Source-accounting history stays SPc-side and strips destination,
  order/grant bytes and VTXO proof/ID fields.
- Owner status and receipt routes require the existing fresh single-use
  B_scan challenge signature and query by authenticated identity. Registration
  replay is not read authentication. Sent is explicitly not wallet receipt;
  unavailable responses instruct the client to retain watches.

## Remaining acceptance scope

The known provisional-pending Rule B implementation, AP3 aggregate-watch
consumer and separate full source-audit collector integration are not findings
against this pool packet and were not approved here. The valid nonpending
990-output native component evidence remains valid within its stated scope.
Actual stock-captaind delivery, funded proof validation, send/receipt crash
recovery and wallet refresh/offboard acceptance remain outstanding. Resolve
the concrete affected-path findings before funded use; an additional ceremonial
whole-lab gate is not requested.
