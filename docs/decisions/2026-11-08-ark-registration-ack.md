# 2026-11-08 - Durable Ark registration v1, signed correlation and destination propagation

Baseline: `b0ab4ca34fda07c61a6335d05f598eaacc69678c` plus approved final
runtime manifest `/tmp/opencode/LP-final-runtime-manifest.json`
(`full9ffff6da`, `code30fedaf`). This decision owns SPEC section 11.4/11.9
and WIRE-FORMATS registration fragments only. It does not change tier policy,
work-unit accounting, witness handling, or AP-3's sections 11.5/11.10.

## Decisions

1. **Versioned `0x5350` only.** Message IDs 0x01 through 0x07 carry the v1
   SetArkIdentity/response, RegisterEpochMailboxes/response and
   ArkPayoutNotice shapes in WIRE-FORMATS section 3.7. Every payload begins
   with `U8 version = 1`; old unversioned bytes and trailing bytes are invalid.
   Raw mailbox modes greater than 3 return the new terminal
   `unsupported-mailbox-mode`; a known mode with bad fields remains
   `bad-mailbox-fields`, and structural failures are `invalid-message`.
2. **Correlation is authorized, not just administrative.** The `request_id`
   remains after `sig` on the wire, but it is included in the BIP340 message
   digest:

   ```
   hash_SPc/ArkIdentity(
       sp_id || version || ark_id || b_ark || mailbox_mode ||
       B0_33(mailbox_pubkey) || B0_33(mailbox_dh) || epoch_fallback ||
       request_id)
   ```

   This is an explicit amendment to reviewed plan section 4.1.1, which left
   the counter unsigned. An old signed registration replayed with a greater
   unsigned counter could otherwise roll back newer delivery without a new
   signature. The immutable `signed_fields_digest` is that message digest and
   therefore binds the counter as well as all fields. AP-3/Fable must review
   this amendment explicitly; AP-2 did not silently preserve the unsigned
   interpretation.
3. **Requests are per-identity monotonic and wrap-refused.** Identity is
   `(sp_id, request_id)`, IDs start at 1, two users may independently choose
   ID 1, and zero/wrap is `request-id-exhausted`. Same ID/digest replays the
   stored outcome; same ID/different digest is `registration-mismatch`.
   Journal-matching older requests return `superseded` without effect;
   different-digest older requests return `registration-mismatch`. A lower
   never-applied request returns `stale-request`; this includes a concurrent
   reorder loser or pruned history, not a promise of scheduled pruning. No
   stale or superseded request changes current delivery. Valid
   RegisterEpochMailboxes signatures use the separate
   `SPc/EpochMailboxes` domain, so cross-kind interpretation cannot arise.
4. **Postgres alone is authoritative.** Migration
   `0006_ark_registration_state.sql` adds `ark_registration_state` and
   `ark_registration_request` and persists `epoch_fallback` on `ark_delivery`.
   In one payout transaction, the identity row is locked
`FOR UPDATE`; then the full `(sp_id, request_id)` journal row is also
   selected for replay before `is_new`. Only no existing journal entry and a
   counter-authorized new identity advances to the effect path. Existing
   different-digest rows resolve `registration-mismatch` with no insert.
  Equal-digest `rejected` rows retain the original terminal error; currently
   applied rows return `already_applied`; older applied rows return
   `superseded`, never historical Applied. A strictly newer effect updates
   identity, delivery or epoch mailboxes, journal/state and `registration_rev`
   atomically. Duplicate, mismatch, stale and superseded resolutions have no
   registration effect. Revision is payout-assigned, never a client value.
   Rejected-ID insert collisions cannot become pending-stream poison.
5. **Redis is only attempt-scoped acknowledgment cache.** Every front handle
   creates fresh random `B016 attempt` on the internal `spc:arkreg` entry and
   ack field `reg:<sp_id_hex>:<request_id>:<attempt_hex>`; JSON echoes it. The
   client wire and signed digest are unchanged: attempt is not an operation ID,
   event effect or signed field. Ack matches require identity/request/digest/
   attempt/expiry. Different handles cannot steal replies and **every handle
   triggers the durable Postgres resolution**, even while the old cache is
   live. Old `Applied` or `already_applied` can never bypass durable
   `superseded`. Timeout remains retryable `registration-commit-unknown`.
6. **Success follows commit; propagation affects routing, not forfeit.** Payout
   writes Redis acknowledgment only after durable commit, then publishes a new
   front-event revision. Full current identity, delivery, fallback and changed
   epoch mailboxes propagate in `ArkRegApplied`, along with `request_id`,
   `registration_rev` and state. `superseded` never reapplies old fields.
   Bootstrap restores the same full state. A committed registration whose
   destination is absent or held by its registered fallback routes pending;
   payout re-decides against `ark_delivery` at settlement as before.
7. **Destinations are derived per distribution generation.** Front computes
   `U_i` and mailbox delivery with the current `a_send`, registered keys and
   the configured public server/mailbox pubkeys, and freezes `dest_id` in the
   distribution snapshot. Static/none/epoch_derived/epoch_registered modes all
   fail closed. `epoch_registered + hold` without its exact height key yields
   no destination and pending; `static` falls back to static M; `none` uses an
   explicitly empty delivery list. Configured `ark_id` must equal
   `ArkId::from_server_pubkey(server_pubkey)`.
8. **Advertisement and router are one capability.** `0x5350` is advertised by
   API and RequestExtensions only when a real front handler is constructible
   and `[ark]` is enabled. Otherwise negotiated success omits it and a direct
   request receives `unsupported-extension` (or `invalid-channel` before open),
   never success into a void. Front requires Redis when Ark delivery is
   enabled. `[ark] server_pubkey` and `mailbox_pubkey` are required for the
   selected front role, not ark-only role. Configured `ark_id` must match
   `ArkId::from_server_pubkey(server_pubkey)`. No SPc identity crosses into
   the ark role; Ark order/receipt types are unchanged and remain identity-free.
   `hold` is `bad-mailbox-fields` outside `epoch_registered`; absent server
   config fails closed instead of fallback. Valid semantic rejection is
   `rejected` with its exact code and pending-entry resolution, while DB/Redis
   failure stays unknown/retryable. Entry-rail/dest fingerprinting rolls the
   job even when coinbase outputs/fingerprint are unchanged.

## AP-3 semantic adapter boundary

AP-2 owns the only `0x5350` codec. The AP-3
`RegistrationPayload -> SetArkIdentity` adapter must additionally pass the
reserved request ID to the canonical `sign` method (the counter is signed but
is not an independent user-editable semantic field). `SetArkIdentitySuccess`
and `SetArkIdentityError` map directly to applied/already_applied/superseded,
`registration-mismatch`, `stale-request` and retryable
`registration-commit-unknown` respectively. The AP-3 helper
`epoch_derived(... mailbox_dh: None)` and its unsigned-counter comments are not
the wire contract: `D = b_ark * S_mb` is required for epoch-derived mailbox
blinding and the signature includes the request ID. The adapter must supply D
and use the canonical signer, or reject locally; pool-side
`bad-mailbox-fields` is not to be reinterpreted as a new D-optional policy.
Stock-persister recovery is AP-3's separate gate, not claimed here.

## Scope and acceptance

This packet proves codec, database durability/concurrency, cache/restart,
stream propagation, raw SV2 negotiation/response, full bootstrap state,
per-generation address derivation and Ark-role schema privacy against
throwaway PostgreSQL/Redis and public synthetic keys. It makes no rbitcoin,
witness, named-Bark-profile, captaind, funded-float, VTXO receive/refresh or
offboard claim. Those remain AP-1/AP-3/AP-4/AP-5 and T4b dependencies.

Evidence and exact baseline/delta are recorded in
`docs/reviews/2026-11-07-ark-registration-implementation.md`.
