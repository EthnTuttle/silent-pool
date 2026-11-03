---
title: "Multi-Identifier Payouts"
type: analysis
format: markdown
date: 2026-11-02
status: closed
supersedes: none
summary: "v0.1 stays single-identity; multiple payout rails are a separate delivery/account layer, not a username parser."
---

# Multi-Identifier Payouts

## Question

Could a miner use an SPc address, a BTC address, a Lightning address, or a Nostr pubkey as their pool username, and what would it take to support that?

## Short answer

No. v0.1 is single-identity by construction, not by an accident of parsing. The SP address supplies all three things at once:

- **account key:** `sp_id = tagged_hash("SPc/Id", ser_P(B_scan) || ser_P(B_spend))` (SPEC §2.1) keys `sp_identity`, `slice_credit`, `ledger_history`, `pending_balance`, `ark_delivery`, `spc:stats` and the channel map;
- **authentication key:** `b_scan` signs the API challenge (SPEC §6.1) and `SetArkIdentity` (SPEC §11.4);
- **payout root:** `P_i = B_spend + t_onchain * G` and `U_i = B_ark + t_ark * G`, both from the per-height ECDH (SPEC §2.3, §11.8; `spc_ark/src/lib.rs::pool_side`, `tier_tweaks`, `spc_output_pubkey`, `ark_user_pubkey`).

Only **Ark delivery** is separated from identity, and it is one capability per `sp_id`: `ark_delivery` has `sp_id` as its primary key (SPEC §5.1), and `SetArkIdentity` is signed by `b_scan` with no literal-address fallback (SPEC §11.4). Channel open is fail-closed: a bare `bc1q...` is rejected and there is no default or donation fallback (SPEC §3.1).

| String | Account key? | Payout target? | Per-height derivation? |
|---|---|---|---|
| `sp1...` / `tsp1...` | Yes (`B_scan`, `B_spend`) | Yes (`P_i`, `U_i`) | Yes, via ECDH |
| `bc1...` | Only with a new account layer | Yes, static script | No scan/spend keypair |
| LN address | Only as a binding | A lookup returning an invoice | No ECDH |
| `npub...` | Only as an auth binding | No, needs a wallet convention | No payment rail |

## Three distinct ideas

1. **Multiple payout rails under one SP identity.** One `sp_id`, several delivery capabilities with a preference order. This is aligned with the existing identity/delivery split (SPEC §0, §11.4) and is the smallest change.
2. **Multiple account identities.** A BTC/LN/Nostr string becomes its own account key. This needs a canonical `account_id`, ownership proofs, a linking policy, per-rail settlement, and new auth. It is a subsystem.
3. **Multiple addresses for one identity.** The BIP 352 answer is labels (`B_m`), currently deferred to v0.2 (SPEC §10 O4). It keeps one scan key and one account and is more aligned than polymorphic usernames.

Only (1) preserves the current accounting, privacy, and audit properties.

## What multi-rail would change

- **SPEC §0, §2.1, §3.1:** identity is no longer "the SP address and nothing else"; `user_identity` becomes a discriminated union with per-kind parsing and ownership proof, or identity stays SP and the other strings move to a delivery extension.
- **SPEC §4.6, §4.7, §4.8:** the distribution builder emits one output per `(sp_id, H)` from `P_id`; a general `PayoutTarget` is needed for arbitrary scripts, plus per-rail dust, weight (the 172 WU P2TR assumption), and slots.
- **SPEC §5.1:** `sp_identity` has `b_scan`/`b_spend NOT NULL`; `ark_delivery` is one row per `sp_id`; `ledger_history.rail` is `coinbase | ark | pending | forfeit`; `pending_balance` is keyed by `sp_id`. Multi-rail needs `account_id`, `identity_binding`, `payout_target`, and a generalized `payout_attempt` (or `delivery_capability` if identity stays SP).
- **SPEC §6.1:** the `SPc` challenge is `b_scan`-only. BTC needs BIP 322, LN needs LNURL-auth or an invoice preimage, Nostr needs NIP-42. Each needs domain separation and replay protection.
- **SPEC §11.1, §11.4, §11.8:** tier routing is balance-based, not chosen; `SetArkIdentity` is one Ark registration signed by `b_scan`; the Ark destination is derived from the SP ECDH. A non-SP identity has no ECDH, so it cannot get the privacy-preserving Ark destination without falling back to a static Ark address, which the design rejects.
- **DESIGN.md:** "one identifier, one DH, two rails" and the per-observer privacy analysis assume the SP address is the only identifier. A linked BTC/LN/Nostr binding is the published bundle the design warns against.
- **Code paths:** `spc_ark/src/lib.rs` and `spc_ark_mailbox/src/lib.rs` remain SP-only; planned `crates/spc-crypto` and `crates/spc-accounting` assume SP derivation; `bins/spc-pool` settlement matches `OP_1 ser_x(P_id)`; `bins/spc-agent` has an `sp_address` config and refuses non-SP input (SPEC §6.3). A second rail is a new role and ledger state, not a parser.

## Sharp edges

**Per-height `P_i` derivation.** Every miner output is derived from the per-height ECDH, and `a_send` is erased on height advance (SPEC §2.2). A static BTC script or LN invoice is not derived; it must be stored and matched, losing one-time unlinkability. For Ark, a non-SP identity loses `U_i` derivation because there is no `ecdh_i`.

**Ark delivery registration.** `SetArkIdentity` is per `sp_id`, one row, one Ark server, signed by `b_scan` (SPEC §11.4). Several delivery options require a preference list, per-target validation, and a rule for blocks when the preferred rail fails. A failed Ark send becomes `pending`, never forfeit (SPEC §11.3); every new rail needs the same policy.

**Type-32 and audit identity.** Type-32 leaves are identity-free and the client verifier reconstructs the window (SPEC §3.7). The miner-side audit in SPEC §11.6 derives the expected `P_i`/`U_i` and checks the coinbase/VTXO. A BTC or LN recipient cannot independently derive the payout, so their audit degrades to trusting the pool's report or an explorer. That is a real trust regression.

**Tier policy.** Today `owed >= C` goes to the coinbase, below `C` to Ark or forfeit (SPEC §11.1). Letting users choose moves value from non-custodial coinbase outputs to custodial LN or treasury flows. Per-rail minimums differ (P2TR dust 330 sats, on-chain dust and fees, LN routing minimums), so one global `C` no longer works.

**Privacy of linking BTC/LN to an SP address.** The pool already sees both tiers; the new harm is a persistent link. A binding table survives scrub, `ark_delivery` is deliberately not scrubbed (SPEC §5.1), and a Nostr pubkey is often a public social identity. Linking a static BTC or LN alias to an SP address gives up the property the pool exists to provide; a database or log leak turns it into a public bundle.

**Auth and canonicalization.** A BTC address is not always signable (exchange, multisig, script addresses). An LN address is a lookup, not a key. `npub` is a public key but not money. The current `.worker` suffix is stripped at the first `.` (SPEC §2.1), which would mangle `user@sub.domain`; use an explicit scheme or separate field, not auto-detection, and fail closed.

## Recommendation

Close v0.1 as **no change**. The single-identity design is load-bearing for the derivation, the Ark tier, the audit story, and the privacy claim.

If demand is for **more payout options**, implement option (1) first: keep `sp1...` as the identity, add a signed delivery-capability/preference list (reusing the `0x5350` pattern), make BTC a fallback/sweep target, and treat LN as a v0.2 sub-threshold rail (SPEC §10 O6) with its own role, liquidity, settlement, and liability accounting. Rough size: small-to-medium, one phase; the parser is days, the new rail and its operations are weeks. Nostr is a notification/auth binding, not a payout rail.

If arbitrary usernames are a hard requirement, design option (2) before Phase 2/3: `account_id`, `identity_binding`, `payout_target`, `payout_attempt`, per-kind auth, a linking policy, per-rail confirmation and failure rules, plus SPEC/DESIGN updates and a dated decision record. Rough size: a phase-sized subsystem (weeks), not a config flag, with a privacy review and ongoing operational cost per rail.

Do not infer the identity type from a bare string. Use an explicit scheme or a separate field, canonicalize, and reject ambiguity.
