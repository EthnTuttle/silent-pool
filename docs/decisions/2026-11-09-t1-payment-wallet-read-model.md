# 2026-11-09 - Authenticated T1 test-payment wallet read model

## Scope and authority

The user authorized the small T1 provider join for the live-wallet UI after
the separately tested acceptance adapter layer. This record defines one
versioned read model; it adds no stock wallet RPC, signer or money ledger.

## Decision

- `GET /api/lab/v1/runs/<run>/projection` carries `payment_wallet` per user,
  schema `spc-t1-payment-wallet/v1`. Authentication precedes every private
  read. The route accepts no selectors. A user's target is its stored claim;
  operator projection enumerates declared user claims with explicit mapping.
- The controller keeps the full validated allocation from the actual actor
  config/hello binding. It resolves user -> driver/global UserPayment index
  there, never from a browser body. The private actor read operation validates
  that user/index/digest against its own allocation and driver incarnation.
- All reads use the existing TxDriverControl pipe. A busy command returns
  unavailable instead of opening a second channel. The single actor writer
  supplies coherent core state and a last-successful-observer timestamp.
- Spendable funds use the core's available outpoints (excluding reservations,
  unknown outgoing spends and immature funds), additionally matched to the
  verified canonical index. Sum overflow rejects. The actual receive address
  is derived at the current cursor by the existing non-mutating method.
- Missing/stale observation (>15 seconds, future clock or another boot),
  missing native provider/funding stage, absent allocation, old actor capability
  or lost pipe makes funds unavailable. Unconfirmed and pending amounts remain
  NULL: this provider has no complete mempool ownership proof. Unknown is not
  converted to zero. Every amount reaching the browser is an exact decimal
  string; this account is labeled T1 test-payment funds, separate from SP/Ark.
- The optional operator catalog reports traffic controls only with actual
  hello-negotiated readiness, active verified traffic stage and native provider.
  It does not infer capability from a running process. Core send validation
  and authenticated command authorization remain authoritative independently
  of UI button enablement.

## Versioned contract and verification

`docs/t1-payment-wallet-provider-v1.md` specifies exact fields and the UI-owner
mapping from projection to its existing `/wallets` rows. No UI-owner source is
edited here. Source/API tests cover scope before I/O, wrong driver/index,
unavailable/future/stale observations, amounts above 2^53, unknown reservations,
and repeated polls with identical journal bytes and address cursor. Synthetic
unit/owned-process data is not funded pipeline or stock-Ark acceptance.

Control major 1, history 3, derivation 2, contextual policy, fee budgets and
the source-bound definitive-rejection classifier are unchanged.
