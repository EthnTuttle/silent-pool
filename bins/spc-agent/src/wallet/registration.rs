//! Ark identity registration client state machine (SPEC 11.4, plan 4.1.1).
//!
//! AP-3a owns the **client semantics**: the per-identity monotonic
//! `request_id` policy (reserved durably before any send, reused on retry
//! of the same attempt, never wrapped), the retry/backoff schedule, and
//! the mapping of server outcomes onto agent state.
//!
//! AP-3a deliberately does **not** own the `0x5350` wire codec: the
//! versioned framing, BIP340 signature over
//! `hash_SPc/ArkIdentity(sp_id || version || ark_id || b_ark ||
//! mailbox_mode || mailbox_pubkey || mailbox_dh || epoch_fallback || request_id)` and
//! the message-type assignment are AP-2's (lab-ark-registration
//! worktree). This module works with [`RegistrationPayload`] - the
//! semantic signed-field set - plus a local correlation digest used only
//! to decide "same attempt (reuse the id)" vs "changed signed fields
//! (fresh id, avoid the `registration-mismatch` self-wedge)". It is not
//! the BIP340 message hash and never crosses the wire.
//!
//! Request-ID policy enforced here (plan 4.1.1):
//!
//! - a new registration attempt increments `last_request_id`; retries of
//!   the same attempt reuse it (same digest -> same pending id);
//! - changing any signed field requires a fresh id - reusing an id with
//!   changed fields is terminal `registration-mismatch` on the server;
//! - `u32::MAX` is refused (`request-id-exhausted`); no wrap, ever;
//! - the reservation is committed before `send` is called, so a crash
//!   between reservation and send recovers the same id.
//!
//! Terminal handling (the loop that used to spin forever, fixed):
//!
//! - hard terminal outcomes (`bad-signature`,
//!   `ark-key-is-spend-key`, `wrong-ark-server`, `bad-mailbox-fields`,
//!   `unsupported-mailbox-mode`, `request-id-exhausted`, unknown
//!   codes) and counter exhaustion are **fatal immediately**: the
//!   manager stops attempting, surfaces the error code, and stays
//!   stopped until an explicit operator restart. Retrying a
//!   bad-signature forever is a bug, not a strategy.
//! - the wedge kinds (`mismatch`, `stale`, `superseded`) get a
//!   **bounded unwedge**: at most ONE fresh retry of the same payload
//!   (fresh request id - reusing the id would self-wedge), and a
//!   second consecutive terminal for the same payload is fatal.
//! - every non-success outcome participates in the backoff schedule;
//!   after six failed attempts local `registration_unavailable` is
//!   surfaced while 30 s retries continue.

use std::sync::Arc;

use bitcoin::hashes::{sha256, Hash, HashEngine};
use bitcoin::secp256k1::PublicKey;

use crate::wallet::state::{WalletStateError, WalletStore};

/// Backoff schedule from plan 4.1.1: `1, 2, 4, 8, 16, 30` seconds, cap
/// 30 s. Pure (no sleeps) so tests drive it without timers.
pub const BACKOFF_SCHEDULE_SECS: [u64; 6] = [1, 2, 4, 8, 16, 30];

/// The registration payload's semantic version (wire `U8 version = 1`).
pub const REGISTRATION_VERSION: u8 = 1;

/// After this many failed attempts the agent surfaces local
/// `registration_unavailable` (it keeps retrying at the 30 s cap and
/// never mines).
pub const UNAVAILABLE_AFTER_ATTEMPTS: u32 = 6;

/// Terminal outcome of one registration attempt, as classified by the
/// server response (or locally, for transport/exhaustion failures).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistrationOutcome {
    /// `Success{state=applied}`: the fields are live.
    Applied,
    /// `Success{state=already_applied}`: same `(sp_id, request_id)`
    /// and digest already applied; the registration stands.
    AlreadyApplied,
    /// `Success{state=superseded}`: applied historically, but a newer
    /// registration is live. The agent must re-register with a higher
    /// id; it must not treat its old fields as the live destination.
    Superseded,
    /// Terminal `registration-mismatch`: same `request_id` with
    /// different signed fields. The next attempt allocates a fresh id
    /// (no self-wedge).
    Mismatch,
    /// Terminal `stale-request`.
    Stale,
    /// Any other terminal server error code (`bad-signature`,
    /// `ark-key-is-spend-key`, `wrong-ark-server`,
    /// `bad-mailbox-fields`, `unsupported-mailbox-mode`,
    /// `request-id-exhausted`, ...).
    Terminal { error_code: String },
    /// Retryable server error (`registration-commit-unknown`,
    /// `registration-busy`, `store-unavailable`, `rate-limited`): the
    /// same attempt continues with the same `request_id`.
    Retryable { error_code: String },
    /// Local `registration_unavailable`: the transport failed (or six
    /// attempts elapsed). Not a server verdict; the pending id stays
    /// reserved so the retry continues the same attempt.
    Unavailable,
}

impl RegistrationOutcome {
    /// Stable journal label (client-side evidence table).
    pub fn as_str(&self) -> &'static str {
        match self {
            RegistrationOutcome::Applied => "applied",
            RegistrationOutcome::AlreadyApplied => "already_applied",
            RegistrationOutcome::Superseded => "superseded",
            RegistrationOutcome::Mismatch => "mismatch",
            RegistrationOutcome::Stale => "stale",
            RegistrationOutcome::Terminal { .. } => "terminal",
            RegistrationOutcome::Retryable { .. } => "retryable",
            RegistrationOutcome::Unavailable => "unavailable",
        }
    }

    /// The server (or local) error code, when there is one.
    pub fn error_code(&self) -> Option<String> {
        match self {
            RegistrationOutcome::Applied | RegistrationOutcome::AlreadyApplied => None,
            RegistrationOutcome::Superseded => None,
            RegistrationOutcome::Mismatch => Some("registration-mismatch".into()),
            RegistrationOutcome::Stale => Some("stale-request".into()),
            RegistrationOutcome::Terminal { error_code } => Some(error_code.clone()),
            RegistrationOutcome::Retryable { error_code } => Some(error_code.clone()),
            RegistrationOutcome::Unavailable => Some("registration_unavailable".into()),
        }
    }

    /// Whether this outcome means the identity's fields are the live
    /// registration (`applied` now, or confirmed applied before).
    pub fn is_registered(&self) -> bool {
        matches!(
            self,
            RegistrationOutcome::Applied | RegistrationOutcome::AlreadyApplied
        )
    }

    /// Whether the same attempt should continue (pending id reused).
    pub fn is_retryable(&self) -> bool {
        matches!(self, RegistrationOutcome::Retryable { .. })
    }

    /// Whether this is a terminal refusal (server or local).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            RegistrationOutcome::Mismatch
                | RegistrationOutcome::Stale
                | RegistrationOutcome::Terminal { .. }
        )
    }

    /// The wedge kinds: the server says "applied historically, not
    /// live" (`superseded`) or disagrees about the id/fields
    /// (`mismatch`, `stale`). These get exactly one fresh same-payload
    /// retry before the loop stops as fatal - the historical bug was
    /// retrying them forever on the same payload.
    pub fn is_wedge_kind(&self) -> bool {
        matches!(
            self,
            RegistrationOutcome::Mismatch
                | RegistrationOutcome::Stale
                | RegistrationOutcome::Superseded
        )
    }

    /// A terminal refusal that is fatal on first sight: retrying the
    /// same payload can never succeed (`bad-signature`,
    /// `ark-key-is-spend-key`, `wrong-ark-server`,
    /// `bad-mailbox-fields`, `unsupported-mailbox-mode`,
    /// `request-id-exhausted`, and any unknown code - fail closed).
    pub fn is_hard_terminal(&self) -> bool {
        matches!(self, RegistrationOutcome::Terminal { .. })
    }
}

/// Mailbox mode byte (plan 4.1.1 `SetArkIdentity`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MailboxMode {
    None,
    Static,
    EpochDerived,
    EpochRegistered,
}

impl MailboxMode {
    pub fn as_u8(&self) -> u8 {
        match self {
            MailboxMode::None => 0,
            MailboxMode::Static => 1,
            MailboxMode::EpochDerived => 2,
            MailboxMode::EpochRegistered => 3,
        }
    }

    pub fn parse_byte(b: u8) -> Option<Self> {
        match b {
            0 => Some(MailboxMode::None),
            1 => Some(MailboxMode::Static),
            2 => Some(MailboxMode::EpochDerived),
            3 => Some(MailboxMode::EpochRegistered),
            _ => None,
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "none" => Some(MailboxMode::None),
            "static" => Some(MailboxMode::Static),
            "epoch_derived" => Some(MailboxMode::EpochDerived),
            "epoch_registered" => Some(MailboxMode::EpochRegistered),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            MailboxMode::None => "none",
            MailboxMode::Static => "static",
            MailboxMode::EpochDerived => "epoch_derived",
            MailboxMode::EpochRegistered => "epoch_registered",
        }
    }
}

/// Epoch fallback byte (plan 4.1.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EpochFallback {
    Static,
    Hold,
}

impl EpochFallback {
    pub fn as_u8(&self) -> u8 {
        match self {
            EpochFallback::Static => 0,
            EpochFallback::Hold => 1,
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "static" => Some(EpochFallback::Static),
            "hold" => Some(EpochFallback::Hold),
            _ => None,
        }
    }
}

/// The semantic signed-field set of `SetArkIdentity` v1 (plan 4.1.1).
///
/// This is exactly what the BIP340 signature covers - **without** the
/// signature, the `request_id` (signed operation counter) or the wire
/// encoding, all of which AP-2 owns. Changing any field here requires a
/// fresh `request_id`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistrationPayload {
    pub sp_id: [u8; 32],
    pub version: u8,
    /// The Ark server id the registration targets (`B004 ark_id`).
    pub ark_id: [u8; 4],
    /// `B_ark` public key (from the ark branch; never `B_spend`).
    pub b_ark: PublicKey,
    pub mailbox_mode: MailboxMode,
    /// `M` (static mailbox id) for static/epoch modes; absent for
    /// `none`.
    pub mailbox_pubkey: Option<PublicKey>,
    /// Blinding hint `D = b_ark * S_mb`, REQUIRED in every mailbox mode
    /// except `none`; obtained from actual validated ArkInfo before wire send.
    pub mailbox_dh: Option<PublicKey>,
    pub epoch_fallback: EpochFallback,
}

impl RegistrationPayload {
    /// Build the offline `epoch_derived` payload draft. The live adapter MUST
    /// supply `D` from the validated server mailbox key before signing/sending.
    pub fn epoch_derived(
        sp_id: [u8; 32],
        ark_id: [u8; 4],
        b_ark: PublicKey,
        mailbox: PublicKey,
        epoch_fallback: EpochFallback,
    ) -> Self {
        RegistrationPayload {
            sp_id,
            version: REGISTRATION_VERSION,
            ark_id,
            b_ark,
            mailbox_mode: MailboxMode::EpochDerived,
            mailbox_pubkey: Some(mailbox),
            mailbox_dh: None,
            epoch_fallback,
        }
    }

    /// Build a `static`-mode payload with the blinding hint
    /// `D = b_ark * S_mb` (plan 4.4 / SPEC 2.3 variant A).
    pub fn static_mode(
        sp_id: [u8; 32],
        ark_id: [u8; 4],
        b_ark: PublicKey,
        mailbox: PublicKey,
        blinding_hint: PublicKey,
    ) -> Self {
        RegistrationPayload {
            sp_id,
            version: REGISTRATION_VERSION,
            ark_id,
            b_ark,
            mailbox_mode: MailboxMode::Static,
            mailbox_pubkey: Some(mailbox),
            mailbox_dh: Some(blinding_hint),
            epoch_fallback: EpochFallback::Static,
        }
    }

    /// Local correlation digest over the signed-field set. Used only to
    /// decide same-attempt (reuse the pending id) vs changed-fields
    /// (fresh id). **Not** the BIP340 message digest; AP-2's codec
    /// computes `hash_SPc/ArkIdentity(...)` for signing.
    pub fn local_digest(&self) -> [u8; 32] {
        let mut eng = sha256::HashEngine::default();
        eng.input(&self.sp_id);
        eng.input(&[self.version]);
        eng.input(&self.ark_id);
        eng.input(&self.b_ark.serialize());
        eng.input(&[self.mailbox_mode.as_u8()]);
        if let Some(m) = &self.mailbox_pubkey {
            eng.input(&[1]);
            eng.input(&m.serialize());
        } else {
            eng.input(&[0]);
        }
        if let Some(d) = &self.mailbox_dh {
            eng.input(&[1]);
            eng.input(&d.serialize());
        } else {
            eng.input(&[0]);
        }
        eng.input(&[self.epoch_fallback.as_u8()]);
        sha256::Hash::from_engine(eng).to_byte_array()
    }
}

/// The transport that carries a registration to the pool front. AP-2
/// owns the `0x5350` implementation; AP-3a consumes it through this
/// seam so the state machine is testable without wire bytes.
pub trait RegistrationTransport: Send {
    /// Send one `SetArkIdentity` attempt with the reserved
    /// `request_id`. An `Err` is a transport failure (locally
    /// `unavailable`, never a server verdict); a server response maps
    /// onto [`RegistrationOutcome`].
    fn send(
        &mut self,
        payload: &RegistrationPayload,
        request_id: u32,
    ) -> anyhow::Result<RegistrationOutcome>;
}

/// Pure retry backoff: `1, 2, 4, 8, 16, 30` s, cap 30 s. No sleeps;
/// callers apply the delay however they schedule.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Backoff {
    failures: u32,
}

impl Backoff {
    pub fn new() -> Self {
        Backoff { failures: 0 }
    }

    /// The delay to wait **before the next attempt**, given the failures
    /// recorded so far.
    pub fn delay_secs(&self) -> u64 {
        BACKOFF_SCHEDULE_SECS
            .get(self.failures as usize)
            .copied()
            .unwrap_or(*BACKOFF_SCHEDULE_SECS.last().expect("non-empty schedule"))
    }

    /// Record one failed attempt.
    pub fn record_failure(&mut self) {
        self.failures = self.failures.saturating_add(1);
    }

    /// A success resets the schedule.
    pub fn reset(&mut self) {
        self.failures = 0;
    }

    /// Failures recorded since the last success.
    pub fn failures(&self) -> u32 {
        self.failures
    }

    /// Whether local `registration_unavailable` should be surfaced
    /// (six attempts). Retrying continues at the 30 s cap regardless.
    pub fn exhausted(&self) -> bool {
        self.failures >= UNAVAILABLE_AFTER_ATTEMPTS
    }
}

/// One `attempt` result: what the server (or transport) said, the
/// request id that was used, and whether it was journaled locally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attempt {
    pub request_id: u32,
    pub outcome: RegistrationOutcome,
    /// Retryable/unavailable outcomes keep the pending reservation (the
    /// retry continues the same attempt); terminal and applied outcomes
    /// are journaled and clear it.
    pub journaled: bool,
}

/// A fatal stop of the registration loop. Attempts cease until an
/// explicit operator restart (a new process/role over the same state).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistrationFatal {
    /// The terminal error code that caused the stop (server code or
    /// `request-id-exhausted`).
    pub error_code: String,
    /// The request id whose terminal outcome caused the stop.
    pub request_id: u32,
}

/// Errors from driving the registration state machine.
#[derive(Debug, thiserror::Error)]
pub enum RegistrationError {
    #[error(transparent)]
    State(#[from] WalletStateError),
    #[error(
        "registration fatally stopped after request {request_id}: {error_code}; \
         attempts cease until an explicit operator restart"
    )]
    Fatal { request_id: u32, error_code: String },
}

/// The registration client state machine over the persistent store.
pub struct RegistrationManager {
    store: Arc<WalletStore>,
    payload: RegistrationPayload,
    backoff: Backoff,
    last: Option<RegistrationOutcome>,
    /// Consecutive terminal outcomes for `streak_digest`'s payload.
    /// Reset by a success or by a payload change (the one fresh
    /// same-payload retry is per payload).
    terminal_streak: u32,
    streak_digest: Option<[u8; 32]>,
    /// Set when attempts must cease (hard terminal, counter
    /// exhaustion, or a second consecutive terminal for the same
    /// payload). Sticky for the manager's lifetime.
    fatal: Option<RegistrationFatal>,
}

impl RegistrationManager {
    pub fn new(store: Arc<WalletStore>, payload: RegistrationPayload) -> Self {
        RegistrationManager {
            store,
            payload,
            backoff: Backoff::new(),
            last: None,
            terminal_streak: 0,
            streak_digest: None,
            fatal: None,
        }
    }

    pub fn payload(&self) -> &RegistrationPayload {
        &self.payload
    }

    /// Replace the signed-field set (e.g. a new ark server or mailbox
    /// mode). The next attempt detects the changed digest and reserves
    /// a fresh request id.
    pub fn payload_mut(&mut self) -> &mut RegistrationPayload {
        &mut self.payload
    }

    /// The underlying store handle.
    pub fn store(&self) -> &Arc<WalletStore> {
        &self.store
    }

    pub fn backoff(&self) -> &Backoff {
        &self.backoff
    }

    /// The most recent outcome, if an attempt has run.
    pub fn last_outcome(&self) -> Option<&RegistrationOutcome> {
        self.last.as_ref()
    }

    /// The fatal stop, if attempts have ceased. Sticky: once set, every
    /// further `attempt` returns [`RegistrationError::Fatal`] without
    /// sending anything, until an operator restarts the role.
    pub fn fatal(&self) -> Option<&RegistrationFatal> {
        self.fatal.as_ref()
    }

    /// Consecutive terminal outcomes for the current payload.
    pub fn terminal_streak(&self) -> u32 {
        self.terminal_streak
    }

    /// Whether the current payload is the live registration (the
    /// effective binding matches the payload digest).
    pub fn is_registered(&self) -> Result<bool, WalletStateError> {
        self.store.is_registered(&self.payload.local_digest())
    }

    /// Run one attempt through `transport`:
    ///
    /// 1. reserve the request id durably (reusing the pending one for
    ///    an unchanged payload; a changed payload allocates a fresh id
    ///    so a server `registration-mismatch` can never self-wedge);
    /// 2. send; a transport failure keeps the reservation and returns
    ///    [`RegistrationOutcome::Unavailable`];
    /// 3. journal terminal/applied outcomes and clear the reservation.
    ///
    /// Terminal handling: a hard terminal or counter exhaustion stops
    /// the manager fatally (the returned attempt still carries the
    /// outcome and journal row; the *next* `attempt` fails with
    /// [`RegistrationError::Fatal`]). The wedge kinds
    /// (`mismatch`/`stale`/`superseded`) allow exactly one fresh
    /// same-payload retry: a second consecutive terminal for the same
    /// payload is fatal too. All non-success outcomes record a backoff
    /// failure.
    pub fn attempt(
        &mut self,
        transport: &mut dyn RegistrationTransport,
    ) -> Result<Attempt, RegistrationError> {
        if let Some(fatal) = &self.fatal {
            return Err(RegistrationError::Fatal {
                request_id: fatal.request_id,
                error_code: fatal.error_code.clone(),
            });
        }
        let digest = self.payload.local_digest();
        // A payload change restarts the terminal-streak accounting: the
        // single fresh retry is granted per payload.
        if self.streak_digest != Some(digest) {
            self.terminal_streak = 0;
            self.streak_digest = Some(digest);
        }
        let request_id = match self.store.reserve_registration_request_id(&digest) {
            Ok(id) => id,
            Err(WalletStateError::RequestIdExhausted) => {
                // Counter exhaustion is fatal (should-fix 7): no wrap,
                // no arbitrary counter jump, no endless retry. Nothing
                // is sent.
                self.backoff.record_failure();
                self.fatal = Some(RegistrationFatal {
                    error_code: "request-id-exhausted".into(),
                    request_id: u32::MAX,
                });
                let outcome = RegistrationOutcome::Terminal {
                    error_code: "request-id-exhausted".into(),
                };
                self.last = Some(outcome.clone());
                return Ok(Attempt {
                    request_id: u32::MAX,
                    outcome,
                    journaled: false,
                });
            }
            Err(e) => return Err(RegistrationError::State(e)),
        };

        let outcome = match transport.send(&self.payload, request_id) {
            Ok(outcome) => outcome,
            Err(_) => {
                // Transport failure: locally unavailable. The pending
                // reservation stays so the retry continues the same
                // attempt with the same id.
                self.backoff.record_failure();
                let outcome = RegistrationOutcome::Unavailable;
                self.last = Some(outcome.clone());
                return Ok(Attempt {
                    request_id,
                    outcome,
                    journaled: false,
                });
            }
        };

        self.last = Some(outcome.clone());
        if outcome.is_registered() {
            self.backoff.reset();
            self.terminal_streak = 0;
        } else {
            // Every non-success outcome participates in the backoff.
            self.backoff.record_failure();
        }
        match &outcome {
            RegistrationOutcome::Retryable { .. } => {
                // Same attempt continues: keep the pending id, nothing
                // journaled yet.
                Ok(Attempt {
                    request_id,
                    outcome,
                    journaled: false,
                })
            }
            RegistrationOutcome::Unavailable => Ok(Attempt {
                request_id,
                outcome,
                journaled: false,
            }),
            RegistrationOutcome::Applied | RegistrationOutcome::AlreadyApplied => {
                self.store.record_registration_outcome(
                    &self.payload.sp_id,
                    request_id,
                    &digest,
                    &outcome,
                )?;
                Ok(Attempt {
                    request_id,
                    outcome,
                    journaled: true,
                })
            }
            wedge_kind if wedge_kind.is_wedge_kind() => {
                // Journaled, pending cleared: the next attempt
                // allocates a fresh id (no self-wedge). This is the
                // single granted fresh same-payload retry; a second
                // consecutive terminal for this payload is fatal.
                self.store.record_registration_outcome(
                    &self.payload.sp_id,
                    request_id,
                    &digest,
                    &outcome,
                )?;
                self.terminal_streak += 1;
                if self.terminal_streak >= 2 {
                    self.fatal = Some(RegistrationFatal {
                        error_code: outcome
                            .error_code()
                            .unwrap_or_else(|| wedge_kind.as_str().into()),
                        request_id,
                    });
                }
                Ok(Attempt {
                    request_id,
                    outcome,
                    journaled: true,
                })
            }
            // Hard terminal (bad-signature, wrong-ark-server, ...):
            // fatal immediately - retrying the same payload can never
            // succeed.
            terminal => {
                self.store.record_registration_outcome(
                    &self.payload.sp_id,
                    request_id,
                    &digest,
                    terminal,
                )?;
                self.fatal = Some(RegistrationFatal {
                    error_code: terminal
                        .error_code()
                        .unwrap_or_else(|| terminal.as_str().into()),
                    request_id,
                });
                Ok(Attempt {
                    request_id,
                    outcome,
                    journaled: true,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::{Secp256k1, SecretKey};
    use std::path::Path;

    /// A transport script: each queued result is returned in order;
    /// `Err` rows are transport failures.
    struct ScriptedTransport {
        results: Vec<Result<RegistrationOutcome, String>>,
        sent: Vec<(Vec<u8>, u32)>,
    }

    impl ScriptedTransport {
        fn new(results: Vec<Result<RegistrationOutcome, String>>) -> Self {
            ScriptedTransport {
                results,
                sent: Vec::new(),
            }
        }
    }

    impl RegistrationTransport for ScriptedTransport {
        fn send(
            &mut self,
            payload: &RegistrationPayload,
            request_id: u32,
        ) -> anyhow::Result<RegistrationOutcome> {
            self.sent
                .push((payload.local_digest().to_vec(), request_id));
            if self.results.is_empty() {
                return Ok(RegistrationOutcome::Retryable {
                    error_code: "registration-busy".into(),
                });
            }
            self.results.remove(0).map_err(|e| anyhow::anyhow!(e))
        }
    }

    fn fixture_manager(dir: &Path) -> RegistrationManager {
        let secp = Secp256k1::new();
        let scan = SecretKey::from_slice(&[1u8; 32]).unwrap();
        let spend = SecretKey::from_slice(&[2u8; 32]).unwrap();
        let scan_pub = PublicKey::from_secret_key(&secp, &scan);
        let spend_pub = PublicKey::from_secret_key(&secp, &spend);
        let identity = crate::wallet::state::StoreIdentity {
            wallet_fingerprint: bitcoin::bip32::Fingerprint::from([9u8; 4]),
            sp_id: spc_crypto::sp_id(&scan_pub, &spend_pub),
            network: "regtest".into(),
        };
        let store = Arc::new(WalletStore::open(dir, &identity).unwrap());
        let b_ark = PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[3u8; 32]).unwrap());
        let mailbox =
            PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[4u8; 32]).unwrap());
        let payload = RegistrationPayload::epoch_derived(
            identity.sp_id,
            [0xaa; 4],
            b_ark,
            mailbox,
            EpochFallback::Hold,
        );
        RegistrationManager::new(store, payload)
    }

    #[test]
    fn backoff_schedule_is_exact() {
        let mut b = Backoff::new();
        assert_eq!(b.delay_secs(), 1);
        assert!(!b.exhausted());
        for expected in [2u64, 4, 8, 16, 30, 30] {
            b.record_failure();
            assert_eq!(b.delay_secs(), expected);
        }
        assert!(b.exhausted(), "six failed attempts surface unavailable");
        b.record_failure();
        b.record_failure();
        assert_eq!(b.delay_secs(), 30, "capped at 30s, keeps retrying");
        b.reset();
        assert_eq!(b.delay_secs(), 1);
        assert!(!b.exhausted());
    }

    #[test]
    fn outcome_labels_and_codes() {
        assert_eq!(RegistrationOutcome::Applied.as_str(), "applied");
        assert_eq!(
            RegistrationOutcome::AlreadyApplied.as_str(),
            "already_applied"
        );
        assert_eq!(RegistrationOutcome::Superseded.as_str(), "superseded");
        assert_eq!(RegistrationOutcome::Mismatch.as_str(), "mismatch");
        assert_eq!(
            RegistrationOutcome::Mismatch.error_code().as_deref(),
            Some("registration-mismatch")
        );
        assert_eq!(
            RegistrationOutcome::Stale.error_code().as_deref(),
            Some("stale-request")
        );
        assert_eq!(
            RegistrationOutcome::Unavailable.error_code().as_deref(),
            Some("registration_unavailable")
        );
        assert!(RegistrationOutcome::Applied.is_registered());
        assert!(RegistrationOutcome::AlreadyApplied.is_registered());
        assert!(!RegistrationOutcome::Superseded.is_registered());
        assert!(RegistrationOutcome::Mismatch.is_terminal());
        assert!(RegistrationOutcome::Retryable {
            error_code: "rate-limited".into()
        }
        .is_retryable());
    }

    #[test]
    fn transport_failure_reuses_the_pending_id() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = fixture_manager(dir.path());
        let mut t =
            ScriptedTransport::new(vec![Err("no route".into()), Err("still no route".into())]);
        let a1 = mgr.attempt(&mut t).unwrap();
        assert_eq!(a1.request_id, 1);
        assert_eq!(a1.outcome, RegistrationOutcome::Unavailable);
        assert!(!a1.journaled);
        let a2 = mgr.attempt(&mut t).unwrap();
        assert_eq!(a2.request_id, 1, "same attempt, same id");
        assert!(!mgr.is_registered().unwrap());
        assert_eq!(mgr.backoff().failures(), 2);
    }

    #[test]
    fn applied_registers_and_resets_backoff() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = fixture_manager(dir.path());
        let mut t = ScriptedTransport::new(vec![
            Err("x".into()),
            Ok(RegistrationOutcome::Retryable {
                error_code: "registration-commit-unknown".into(),
            }),
            Ok(RegistrationOutcome::Applied),
        ]);
        // Failures first.
        mgr.attempt(&mut t).unwrap();
        mgr.attempt(&mut t).unwrap();
        assert_eq!(mgr.backoff().failures(), 2);
        // Then success.
        let a = mgr.attempt(&mut t).unwrap();
        assert_eq!(a.request_id, 1, "all three were the same attempt");
        assert_eq!(a.outcome, RegistrationOutcome::Applied);
        assert!(a.journaled);
        assert!(mgr.is_registered().unwrap());
        assert_eq!(mgr.backoff().failures(), 0);
    }

    #[test]
    fn terminal_mismatch_allocates_a_fresh_id_next_attempt() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = fixture_manager(dir.path());
        let mut t = ScriptedTransport::new(vec![
            Ok(RegistrationOutcome::Mismatch),
            Ok(RegistrationOutcome::Applied),
        ]);
        let a1 = mgr.attempt(&mut t).unwrap();
        assert_eq!(a1.request_id, 1);
        assert!(a1.journaled);
        assert!(!mgr.is_registered().unwrap(), "mismatch is not registered");
        // The retry after a terminal mismatch must NOT reuse id 1.
        let a2 = mgr.attempt(&mut t).unwrap();
        assert_eq!(a2.request_id, 2, "fresh id avoids the self-wedge");
        assert!(mgr.is_registered().unwrap());
    }

    #[test]
    fn superseded_is_journaled_but_not_registered() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = fixture_manager(dir.path());
        let mut t = ScriptedTransport::new(vec![Ok(RegistrationOutcome::Superseded)]);
        let a = mgr.attempt(&mut t).unwrap();
        assert!(a.journaled);
        assert!(!mgr.is_registered().unwrap());
        assert!(!a.outcome.is_registered());
    }

    #[test]
    fn changed_signed_fields_get_a_fresh_id() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = fixture_manager(dir.path());
        let mut t = ScriptedTransport::new(vec![Err("x".into()), Ok(RegistrationOutcome::Applied)]);
        mgr.attempt(&mut t).unwrap();
        // Change a signed field (the fallback): same id would be a
        // server-side registration-mismatch, so a fresh one is reserved.
        let mut changed = mgr.payload().clone();
        changed.epoch_fallback = EpochFallback::Hold;
        changed.ark_id = [0xbb; 4];
        assert_ne!(changed.local_digest(), mgr.payload().local_digest());
        *mgr.payload_mut() = changed;
        let a = mgr.attempt(&mut t).unwrap();
        assert_eq!(a.request_id, 2);
    }

    #[test]
    fn backoff_counts_every_non_success_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = fixture_manager(dir.path());
        let mut t = ScriptedTransport::new(vec![
            Ok(RegistrationOutcome::Mismatch),
            Err("no route".into()),
        ]);
        mgr.attempt(&mut t).unwrap(); // terminal
        mgr.attempt(&mut t).unwrap(); // transport failure
        assert_eq!(
            mgr.backoff().failures(),
            2,
            "terminal outcomes participate in the backoff too"
        );
        assert_eq!(mgr.backoff().delay_secs(), 4, "1 -> 2 -> 4");
    }

    /// Counter exhaustion is fatal (should-fix 7): one local terminal
    /// outcome, no send, and every further attempt refuses.
    #[test]
    fn request_id_exhaustion_is_fatal_without_a_send() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = fixture_manager(dir.path());
        {
            let store = mgr.store();
            let guard = store.lock_for_tests();
            guard
                .execute(
                    "INSERT OR REPLACE INTO registration_state
                         (id, last_request_id, updated_at) VALUES (1, ?1, 'x')",
                    [u32::MAX as i64],
                )
                .unwrap();
        }
        let mut t = ScriptedTransport::new(vec![]);
        let a = mgr.attempt(&mut t).unwrap();
        assert!(a.outcome.is_terminal());
        assert_eq!(
            a.outcome.error_code().as_deref(),
            Some("request-id-exhausted")
        );
        assert!(t.sent.is_empty(), "no frame is sent on exhaustion");
        // Fatal and sticky: further attempts refuse without a send.
        assert_eq!(mgr.fatal().unwrap().error_code, "request-id-exhausted");
        let mut t2 = ScriptedTransport::new(vec![]);
        let err = mgr.attempt(&mut t2).unwrap_err();
        assert!(
            matches!(
                err,
                crate::wallet::registration::RegistrationError::Fatal { .. }
            ),
            "{err}"
        );
        assert!(t2.sent.is_empty(), "a fatal manager never sends again");
    }

    /// A hard terminal (bad-signature) is fatal immediately: exactly one
    /// send, then the manager refuses further attempts - it never
    /// retries a bad signature forever.
    #[test]
    fn hard_terminal_is_fatal_on_first_sight() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = fixture_manager(dir.path());
        let mut t = ScriptedTransport::new(vec![Ok(RegistrationOutcome::Terminal {
            error_code: "bad-signature".into(),
        })]);
        let a = mgr.attempt(&mut t).unwrap();
        assert_eq!(a.request_id, 1);
        assert!(a.journaled, "the terminal outcome is journaled");
        assert_eq!(t.sent.len(), 1);
        let fatal = mgr.fatal().expect("hard terminal is immediately fatal");
        assert_eq!(fatal.error_code, "bad-signature");
        assert_eq!(fatal.request_id, 1);
        // Sticky: the next attempt fails without another send.
        let mut t2 = ScriptedTransport::new(vec![]);
        let err = mgr.attempt(&mut t2).unwrap_err();
        assert!(err.to_string().contains("bad-signature"), "{err}");
        assert!(t2.sent.is_empty());
    }

    /// The wedge kinds get exactly ONE fresh same-payload retry; a
    /// second consecutive terminal for the same payload is fatal.
    #[test]
    fn wedge_kinds_get_one_fresh_retry_then_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = fixture_manager(dir.path());
        let mut t = ScriptedTransport::new(vec![
            Ok(RegistrationOutcome::Mismatch),
            Ok(RegistrationOutcome::Mismatch),
        ]);
        let a1 = mgr.attempt(&mut t).unwrap();
        assert_eq!(a1.request_id, 1);
        assert!(
            mgr.fatal().is_none(),
            "the first wedge terminal grants one retry"
        );
        assert_eq!(mgr.terminal_streak(), 1);
        // The single fresh retry: same payload, fresh id (2).
        let a2 = mgr.attempt(&mut t).unwrap();
        assert_eq!(a2.request_id, 2, "the unwedge retry uses a fresh id");
        assert_eq!(
            t.sent.iter().map(|(_, id)| *id).collect::<Vec<_>>(),
            vec![1, 2],
            "exactly two sends"
        );
        let fatal = mgr.fatal().expect("second consecutive terminal is fatal");
        assert_eq!(fatal.error_code, "registration-mismatch");
        assert_eq!(fatal.request_id, 2);
        // And it stays stopped.
        let mut t2 = ScriptedTransport::new(vec![]);
        assert!(mgr.attempt(&mut t2).is_err());
        assert!(t2.sent.is_empty());
    }

    /// A success after a wedge terminal clears the streak; a payload
    /// change restarts the per-payload accounting.
    #[test]
    fn wedge_streak_resets_on_success_and_on_payload_change() {
        let dir = tempfile::tempdir().unwrap();
        // Success clears the streak.
        let mut mgr = fixture_manager(dir.path());
        let mut t = ScriptedTransport::new(vec![
            Ok(RegistrationOutcome::Mismatch),
            Ok(RegistrationOutcome::Applied),
            Ok(RegistrationOutcome::Mismatch),
        ]);
        mgr.attempt(&mut t).unwrap();
        mgr.attempt(&mut t).unwrap();
        assert!(mgr.fatal().is_none());
        assert_eq!(mgr.terminal_streak(), 0, "success resets the streak");
        mgr.attempt(&mut t).unwrap();
        assert!(mgr.fatal().is_none(), "a fresh wedge terminal is not fatal");

        // A changed payload restarts the accounting.
        let mut changed = mgr.payload().clone();
        changed.ark_id = [0xcc; 4];
        *mgr.payload_mut() = changed;
        let mut t = ScriptedTransport::new(vec![
            Ok(RegistrationOutcome::Stale),
            Ok(RegistrationOutcome::Stale),
        ]);
        mgr.attempt(&mut t).unwrap();
        assert!(mgr.fatal().is_none(), "first terminal for the new payload");
        mgr.attempt(&mut t).unwrap();
        assert!(
            mgr.fatal().is_some(),
            "second consecutive terminal for the same payload"
        );
    }

    /// Retryable/unknown outcomes keep the pending id and never flip
    /// the manager fatal.
    #[test]
    fn retryable_unknown_preserves_the_pending_id() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = fixture_manager(dir.path());
        let mut t = ScriptedTransport::new(vec![
            Ok(RegistrationOutcome::Retryable {
                error_code: "registration-commit-unknown".into(),
            }),
            Ok(RegistrationOutcome::Retryable {
                error_code: "store-unavailable".into(),
            }),
        ]);
        let a1 = mgr.attempt(&mut t).unwrap();
        assert_eq!(a1.request_id, 1);
        assert!(!a1.journaled);
        let a2 = mgr.attempt(&mut t).unwrap();
        assert_eq!(a2.request_id, 1, "same attempt, same id");
        assert_eq!(
            mgr.store().pending_registration_request_id().unwrap(),
            Some(1)
        );
        assert!(mgr.fatal().is_none());
        assert_eq!(mgr.backoff().failures(), 2);
    }

    #[test]
    fn payload_digest_distinguishes_every_signed_field() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = fixture_manager(dir.path());
        let base = mgr.payload().clone();
        let secp = Secp256k1::new();
        let other = PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[5u8; 32]).unwrap());

        let mut p = base.clone();
        p.b_ark = other;
        assert_ne!(p.local_digest(), base.local_digest());

        let mut p = base.clone();
        p.mailbox_mode = MailboxMode::EpochRegistered;
        assert_ne!(p.local_digest(), base.local_digest());

        let mut p = base.clone();
        p.epoch_fallback = EpochFallback::Static;
        assert_ne!(p.local_digest(), base.local_digest());

        let mut p = base.clone();
        p.ark_id = [1, 2, 3, 4];
        assert_ne!(p.local_digest(), base.local_digest());

        let mut p = base.clone();
        p.mailbox_pubkey = Some(other);
        assert_ne!(p.local_digest(), base.local_digest());

        // Identical payload: identical digest.
        assert_eq!(base.clone().local_digest(), base.local_digest());
    }
}
