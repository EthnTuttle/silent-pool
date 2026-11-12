# 2026-11-10 - Retry known pre-request capacity and wallet shortages

## Authority

Same-auditor 419b5f recheck A1-RP1 and parent inspection of the zero/short-float
startup path. A2-A5 are closed in that reviewed packet. Capacity timeout before
any request was incorrectly converted to ambiguous/pinned work; short float
was not checked until after a grant. Both can strand otherwise valid orders.

## Decision and local API

The local transport contract now has closed CapacityError outcomes and a
read-only check_send_readiness(address, amount, permit) stage. ArkService
checks any existing pin first, verifies the order, acquires the wallet's sole
permit, then checks actual readiness before constructing a nonce/request.
Known pre-request deadline/input/proof/policy shortages return Processed::Held.
HANDLE leaves the same stream item unacknowledged, frozen membership reserved,
and adds no ambiguity pin. It emits a bounded stage/reason, no terminal receipt,
refund, cancellation, replacement order or NoEffect claim.

Only closed errors produced in these pre-request stages are retryable this
way. Generic transport errors, lost request/grant replies, grant expiry and
possible cosign escape retain the existing unknown/pin behavior. Existing pins
are never cleared. The same permit spans readiness, request, guarded send and
the pre-registration/pre-cosign fences; no nested acquisition is introduced.

The stock implementation reads the actual wallet's spendable VTXO records,
tip, immutable full proofs/anchors and signing-key availability, validates the
configured address against current ArkInfo policy/fee bounds, and uses the
supported estimate_arkoor_payment_fee result. Selection checks state, expiry,
exit depth and gross amount including fee with checked arithmetic. Largest-first
selection stays shared with guarded send. Locked/expired/deep/short inputs do
not count. The fee API at the pin explicitly reports zero arkoor fee; it is not
a guessed/configured fee. A nonzero quote is never waived: insufficient gross
coverage holds, and an otherwise-funded unsupported fee policy also holds.
Pure package construction checks the intended destination/change/input shape
without input registration, locking, nonce generation or cosign. Readiness is
bounded at 30 seconds. A later failure after a request/grant remains unknown.

Held work is retried from its existing PEL. A durable bounded cursor traverses
more than one pending batch and wraps on exhaustion; same-group/consumer
restart resumes it. No monetary ledger or wire/server RPC/status changes.
The payout authorization dispatcher never takes wallet capacity. Wallet
shortages do not become a mining gate.

## Checks and merge boundary

Authored HANDLE/consumer tests exercise the shared production deadline helper,
zero requests/grants/cosigns/no new pin, retained membership, a pinned prefix,
restart and retry of the same order after release. Actual stock WalletVtxo/
FeeEstimate fixtures exercise zero, locked, expired, depth/fee-inadequate float
and subsequent readiness for the first single grant/escape. Post-request lost
reply/pin tests remain. These are component fixtures, not funded-server claims.

R4 integration must forward both typed capacity and readiness methods and use
the same operation_lock. Preserve R4's newer wallet-control/IO fields and hooks;
do not apply an old whole-file replacement. New source/hunks and tests await
the next heavy ticket; the 419b5f evidence is preserved unchanged.
