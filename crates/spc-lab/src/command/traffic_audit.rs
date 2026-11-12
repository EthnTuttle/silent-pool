//! Private completeness evidence in the existing strict CommandLog. No funds,
//! reservations or authority are reconstructed here. Missing results remain
//! unresolved across restart and can never produce a complete watermark.
use super::{write_line, CommandLog, LogLine};
use serde::{Deserialize, Serialize};
use spc_lab_txd::control::{ForwardContext, SignedCheckpoint};
use std::collections::BTreeMap;
use std::io;

fn invalid(s: &str) -> io::Error {
    super::traffic_budget::invalid(s)
}
pub fn hash(value: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(value))
}
fn digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub run_id: String,
    pub driver_id: String,
    pub incarnation: String,
    pub controller_sha256: String,
    pub actor_sha256: String,
    pub acceptance_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attempt {
    pub incarnation: String,
    pub rpc_request_id: u64,
    pub node_id: String,
    pub method: String,
    pub send_sequence: Option<u64>,
    /// SHA256 of the exact compact JSON RPC params (never auth/cookies).
    pub payload_sha256: String,
    pub context: ForwardContext,
    pub permit_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookMode {
    HoldBeforeIntent,
    LoseOneResponse,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum HookAction {
    Arm { mode: HookMode, timeout_ms: u64 },
    Release { raw_sha256: String },
    Cancel,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookKey {
    pub driver_id: String,
    pub incarnation: String,
    pub command_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "transition", rename_all = "snake_case", deny_unknown_fields)]
pub enum HookEvent {
    Armed {
        mode: HookMode,
        deadline_ms: u64,
        boot_id: String,
    },
    Signed {
        raw_sha256: String,
        journal_seq: u64,
    },
    Released {
        raw_sha256: String,
    },
    Cancelled,
    ResponseDiscarded {
        attempt_seq: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    Session {
        binding: Session,
    },
    ForwardAttempt {
        attempt: Attempt,
    },
    ForwardResult {
        attempt_seq: u64,
        outcome: String,
        response_sha256: String,
        /// True only for deliberate acceptance injection. False does not
        /// claim the actor received/acknowledged its pipe response.
        response_suppressed: bool,
    },
    Watermark {
        incarnation: String,
        last_attempt_seq: u64,
        completed_count: u64,
        permit_revision: u64,
        closed: bool,
        command_id: Option<String>,
    },
    MeasuredBoundary {
        command_id: String,
        tip_hash: String,
        tip_height: u64,
    },
    MinerAttempt {
        run_id: String,
        miner_id: String,
        incarnation: String,
        request_id: String,
        payload_sha256: String,
        controller_sha256: String,
        actor_sha256: String,
    },
    MinerResult {
        attempt_seq: u64,
        succeeded: bool,
    },
    Hook {
        key: HookKey,
        detail: HookEvent,
    },
}

#[derive(Clone, Default)]
pub(super) struct AuditState {
    sessions: BTreeMap<String, Stream>,
    attempts: BTreeMap<u64, (Attempt, Option<bool>)>,
    miners: BTreeMap<u64, bool>,
    hooks: BTreeMap<(String, String), Hook>,
    measured: Option<u64>,
}
#[derive(Clone)]
struct Stream {
    binding: Session,
    request: u64,
    last: u64,
    completed: u64,
    pending: u64,
    revision: u64,
    closed: bool,
}
#[derive(Clone)]
struct Hook {
    key: HookKey,
    mode: HookMode,
    deadline: u64,
    boot: String,
    signed: Option<String>,
    released: bool,
    cancelled: bool,
    consumed: bool,
}

impl AuditState {
    fn apply(&mut self, seq: u64, event: &Event) -> io::Result<()> {
        match event {
            Event::Session { binding: b } => {
                if b.run_id.is_empty()
                    || b.driver_id.is_empty()
                    || b.incarnation.is_empty()
                    || !digest(&b.controller_sha256)
                    || !digest(&b.actor_sha256)
                    || self.sessions.contains_key(&b.incarnation)
                    || self.sessions.values().any(|s| s.binding.run_id != b.run_id)
                {
                    return Err(invalid("audit session binding"));
                }
                self.sessions.insert(
                    b.incarnation.clone(),
                    Stream {
                        binding: b.clone(),
                        request: 0,
                        last: 0,
                        completed: 0,
                        pending: 0,
                        revision: 0,
                        closed: false,
                    },
                );
            }
            Event::ForwardAttempt { attempt: a } => {
                let s = self
                    .sessions
                    .get_mut(&a.incarnation)
                    .ok_or_else(|| invalid("audit session missing"))?;
                if a.rpc_request_id <= s.request
                    || !digest(&a.payload_sha256)
                    || a.permit_revision != s.revision
                    || a.node_id.is_empty()
                    || a.method.is_empty()
                {
                    return Err(invalid("audit attempt correlation"));
                }
                if a.method == "sendrawtransaction"
                    && (s.closed
                        || a.send_sequence.is_none_or(|n| n == 0)
                        || a.context.command_id.as_ref().is_none_or(|s| s.is_empty())
                        || a.context
                            .intent_seq
                            .is_none_or(|n| n == 0 || n > a.context.journal_seq)
                        || a.context
                            .chain_revision
                            .is_none_or(|n| n > a.context.journal_seq)
                        || a.context.raw_sha256.as_ref().is_none_or(|s| !digest(s)))
                {
                    return Err(invalid("audit send context missing"));
                }
                s.request = a.rpc_request_id;
                s.last = seq;
                s.pending += 1;
                self.attempts.insert(seq, (a.clone(), None));
            }
            Event::ForwardResult {
                attempt_seq,
                outcome,
                response_sha256,
                response_suppressed,
            } => {
                if !matches!(
                    outcome.as_str(),
                    "ok" | "node_rejected" | "unknown" | "gate_closed"
                ) || !digest(response_sha256)
                {
                    return Err(invalid("audit result shape"));
                }
                let (a, result) = self
                    .attempts
                    .get_mut(attempt_seq)
                    .ok_or_else(|| invalid("audit orphan result"))?;
                if result.replace(*response_suppressed).is_some() {
                    return Err(invalid("audit duplicate result"));
                }
                let s = self.sessions.get_mut(&a.incarnation).unwrap();
                s.pending -= 1;
                s.completed += 1;
                // Completed unsuppressed calls need no replay map entry. An
                // absent id still rejects duplicate/orphan end records. Keep
                // only an explicitly lost result until its hook consumes it.
                if !*response_suppressed {
                    self.attempts.remove(attempt_seq);
                }
            }
            Event::Watermark {
                incarnation,
                last_attempt_seq,
                completed_count,
                permit_revision,
                closed,
                ..
            } => {
                let s = self
                    .sessions
                    .get_mut(incarnation)
                    .ok_or_else(|| invalid("audit watermark session"))?;
                if s.pending != 0
                    || *last_attempt_seq != s.last
                    || *completed_count != s.completed
                    || *permit_revision != s.revision + 1
                {
                    return Err(invalid("audit incomplete watermark"));
                }
                s.revision = *permit_revision;
                s.closed = *closed;
            }
            Event::MeasuredBoundary { tip_hash, .. } => {
                if self.measured.is_some() || !digest(tip_hash) {
                    return Err(invalid("audit measured boundary"));
                }
                self.measured = Some(seq);
            }
            Event::MinerAttempt {
                payload_sha256,
                controller_sha256,
                actor_sha256,
                request_id,
                ..
            } => {
                if self.measured.is_none()
                    || !digest(payload_sha256)
                    || !digest(controller_sha256)
                    || !digest(actor_sha256)
                    || request_id.is_empty()
                {
                    return Err(invalid(
                        "AddConnection before measured boundary or missing binding",
                    ));
                }
                self.miners.insert(seq, false);
            }
            Event::MinerResult { attempt_seq, .. } => {
                let done = self
                    .miners
                    .get_mut(attempt_seq)
                    .ok_or_else(|| invalid("audit orphan miner result"))?;
                if *done {
                    return Err(invalid("audit duplicate miner result"));
                }
                self.miners.remove(attempt_seq);
            }
            Event::Hook { key, detail } => {
                let s = self
                    .sessions
                    .get(&key.incarnation)
                    .ok_or_else(|| invalid("hook session missing"))?;
                if !s.binding.acceptance_enabled
                    || key.driver_id != s.binding.driver_id
                    || key.command_id.is_empty()
                {
                    return Err(invalid("hook disabled or binding mismatch"));
                }
                let slot = (key.driver_id.clone(), key.command_id.clone());
                if let HookEvent::Armed {
                    mode,
                    deadline_ms,
                    boot_id,
                } = detail
                {
                    if self.hooks.contains_key(&slot)
                        || self.hooks.len() >= 64
                        || *deadline_ms == 0
                        || boot_id.is_empty()
                    {
                        return Err(invalid("hook duplicate or bound"));
                    }
                    self.hooks.insert(
                        slot,
                        Hook {
                            key: key.clone(),
                            mode: mode.clone(),
                            deadline: *deadline_ms,
                            boot: boot_id.clone(),
                            signed: None,
                            released: false,
                            cancelled: false,
                            consumed: false,
                        },
                    );
                } else {
                    let h = self
                        .hooks
                        .get_mut(&slot)
                        .ok_or_else(|| invalid("hook not armed"))?;
                    if h.key != *key || h.cancelled || h.consumed {
                        return Err(invalid("hook stale/terminal"));
                    }
                    match detail {
                        HookEvent::Signed {
                            raw_sha256,
                            journal_seq,
                        } => {
                            if h.signed.is_some() || !digest(raw_sha256) || *journal_seq == 0 {
                                return Err(invalid("hook signed binding"));
                            }
                            h.signed = Some(raw_sha256.clone());
                        }
                        HookEvent::Released { raw_sha256 } => {
                            if h.mode != HookMode::HoldBeforeIntent
                                || h.released
                                || h.signed.as_ref() != Some(raw_sha256)
                            {
                                return Err(invalid("hook release binding"));
                            }
                            h.released = true;
                        }
                        HookEvent::Cancelled => h.cancelled = true,
                        HookEvent::ResponseDiscarded { attempt_seq } => {
                            let (a, result) = self
                                .attempts
                                .get(attempt_seq)
                                .ok_or_else(|| invalid("hook result absent"))?;
                            if h.mode != HookMode::LoseOneResponse
                                || *result != Some(true)
                                || a.incarnation != key.incarnation
                                || a.context.command_id.as_ref() != Some(&key.command_id)
                                || a.context.raw_sha256 != h.signed
                            {
                                return Err(invalid("hook response correlation"));
                            }
                            h.consumed = true;
                            self.attempts.remove(attempt_seq);
                        }
                        _ => return Err(invalid("hook transition")),
                    }
                }
            }
        }
        Ok(())
    }
}

pub(super) fn replay(state: &mut super::State, seq: u64, event: &Event) -> io::Result<()> {
    if !state.traffic_strict {
        return Err(invalid("audit requires strict command log"));
    }
    state.audit.apply(seq, event)
}

impl CommandLog {
    #[cfg(test)]
    pub(crate) fn fail_audit_writes(&self) {
        self.state.lock().unwrap().file = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .unwrap();
    }
    pub fn audit(&self, event: Event) -> io::Result<u64> {
        let mut state = self.state.lock().unwrap();
        if !state.traffic_strict || state.poisoned || state.next_seq == u64::MAX {
            return Err(invalid("audit log unavailable"));
        }
        let seq = state.next_seq;
        let mut next = state.audit.clone();
        next.apply(seq, &event)?;
        let record = LogLine::TrafficAudit {
            seq,
            version: 1,
            boot_id: spc_lab_txd::bootstrap::boot_id()?,
            monotonic_ms: spc_lab_txd::bootstrap::monotonic_ms()?,
            event,
        };
        write_line(&mut state, &record)?;
        state.audit = next;
        state.next_seq += 1;
        Ok(seq)
    }
    pub fn audit_revision(&self, incarnation: &str) -> io::Result<u64> {
        let state = self.state.lock().unwrap();
        if state.poisoned {
            return Err(invalid("audit poisoned"));
        }
        state
            .audit
            .sessions
            .get(incarnation)
            .map(|s| s.revision)
            .ok_or_else(|| invalid("audit session missing"))
    }
    pub fn audit_watermark(
        &self,
        incarnation: &str,
        closed: bool,
        command_id: Option<String>,
    ) -> io::Result<u64> {
        let event = {
            let state = self.state.lock().unwrap();
            let s = state
                .audit
                .sessions
                .get(incarnation)
                .ok_or_else(|| invalid("audit session missing"))?;
            Event::Watermark {
                incarnation: incarnation.into(),
                last_attempt_seq: s.last,
                completed_count: s.completed,
                permit_revision: s.revision + 1,
                closed,
                command_id,
            }
        };
        self.audit(event)
    }
    pub fn acceptance_action(&self, key: HookKey, action: HookAction) -> io::Result<u64> {
        let detail = match action {
            HookAction::Arm { mode, timeout_ms } => {
                if !(100..=120_000).contains(&timeout_ms) {
                    return Err(invalid("hook timeout bound"));
                }
                HookEvent::Armed {
                    mode,
                    deadline_ms: spc_lab_txd::bootstrap::monotonic_ms()?
                        .checked_add(timeout_ms)
                        .ok_or_else(|| invalid("hook deadline overflow"))?,
                    boot_id: spc_lab_txd::bootstrap::boot_id()?,
                }
            }
            HookAction::Release { raw_sha256 } => HookEvent::Released { raw_sha256 },
            HookAction::Cancel => HookEvent::Cancelled,
        };
        self.audit(Event::Hook { key, detail })
    }
    /// Only delays or cancels. No response is an admission grant.
    pub fn acceptance_checkpoint(
        &self,
        key: HookKey,
        signed: &SignedCheckpoint,
    ) -> io::Result<&'static str> {
        let h = self
            .state
            .lock()
            .unwrap()
            .audit
            .hooks
            .get(&(key.driver_id.clone(), key.command_id.clone()))
            .cloned();
        let Some(h) = h else { return Ok("proceed") };
        if h.key != key || h.cancelled {
            return Ok("cancel");
        }
        if h.consumed {
            return Ok("proceed");
        }
        if h.boot != spc_lab_txd::bootstrap::boot_id()?
            || spc_lab_txd::bootstrap::monotonic_ms()? >= h.deadline
        {
            self.audit(Event::Hook {
                key,
                detail: HookEvent::Cancelled,
            })?;
            return Ok("cancel");
        }
        if h.signed.as_ref().is_some_and(|s| s != &signed.raw_sha256) {
            return Err(invalid("hook signed bytes changed"));
        }
        if h.signed.is_none() {
            self.audit(Event::Hook {
                key,
                detail: HookEvent::Signed {
                    raw_sha256: signed.raw_sha256.clone(),
                    journal_seq: signed.journal_seq,
                },
            })?;
        }
        Ok(if h.mode == HookMode::HoldBeforeIntent && !h.released {
            "hold"
        } else {
            "proceed"
        })
    }
    pub fn acceptance_lose_response(&self, key: &HookKey, raw_sha256: Option<&str>) -> bool {
        self.state
            .lock()
            .unwrap()
            .audit
            .hooks
            .get(&(key.driver_id.clone(), key.command_id.clone()))
            .is_some_and(|h| {
                h.key == *key
                    && h.mode == HookMode::LoseOneResponse
                    && !h.cancelled
                    && !h.consumed
                    && h.signed.as_deref() == raw_sha256
                    && raw_sha256.is_some()
            })
    }

    /// A hook is only an additional veto, never a send authorization. Caller
    /// holds the forwarding mutex, also held by operator hook transitions.
    pub fn acceptance_forward_allowed(
        &self,
        key: &HookKey,
        raw_sha256: Option<&str>,
    ) -> io::Result<bool> {
        let state = self.state.lock().unwrap();
        if state.poisoned {
            return Err(invalid("audit poisoned"));
        }
        let Some(h) = state
            .audit
            .hooks
            .get(&(key.driver_id.clone(), key.command_id.clone()))
        else {
            return Ok(true);
        };
        Ok(h.key == *key
            && !h.cancelled
            && h.boot == spc_lab_txd::bootstrap::boot_id()?
            && spc_lab_txd::bootstrap::monotonic_ms()? < h.deadline
            && h.signed.as_deref() == raw_sha256
            && raw_sha256.is_some()
            && (h.mode == HookMode::LoseOneResponse || h.released))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> Session {
        Session {
            run_id: "run".into(),
            driver_id: "driver".into(),
            incarnation: "epoch".into(),
            controller_sha256: "aa".repeat(32),
            actor_sha256: "bb".repeat(32),
            acceptance_enabled: true,
        }
    }
    fn key() -> HookKey {
        HookKey {
            driver_id: "driver".into(),
            incarnation: "epoch".into(),
            command_id: "payment".into(),
        }
    }
    fn attempt() -> Attempt {
        Attempt {
            incarnation: "epoch".into(),
            rpc_request_id: 1,
            node_id: "node".into(),
            method: "sendrawtransaction".into(),
            send_sequence: Some(1),
            payload_sha256: "cc".repeat(32),
            permit_revision: 0,
            context: ForwardContext {
                command_id: Some("payment".into()),
                journal_seq: 12,
                chain_revision: Some(10),
                intent_seq: Some(12),
                raw_sha256: Some("dd".repeat(32)),
            },
        }
    }
    fn path(label: &str) -> std::path::PathBuf {
        std::path::PathBuf::from("/tmp/opencode").join(format!(
            "traffic-audit-{}-{label}.jsonl",
            super::super::CommandId::generate()
        ))
    }
    #[test]
    fn unresolved_attempt_cannot_be_completed_by_restart_or_watermark() {
        let p = path("replay");
        let log = CommandLog::open_traffic(&p).unwrap();
        log.audit(Event::Session { binding: session() }).unwrap();
        let seq = log
            .audit(Event::ForwardAttempt { attempt: attempt() })
            .unwrap();
        assert!(log.audit_watermark("epoch", true, None).is_err());
        drop(log);
        let log = CommandLog::open_traffic(&p).unwrap();
        assert!(log.audit_watermark("epoch", true, None).is_err());
        log.audit(Event::ForwardResult {
            attempt_seq: seq,
            outcome: "unknown".into(),
            response_sha256: "ee".repeat(32),
            response_suppressed: false,
        })
        .unwrap();
        assert!(log
            .audit(Event::ForwardResult {
                attempt_seq: seq,
                outcome: "unknown".into(),
                response_sha256: "ee".repeat(32),
                response_suppressed: false
            })
            .is_err());
        log.audit_watermark("epoch", true, Some("pause".into()))
            .unwrap();
        drop(log);
        CommandLog::open_traffic(&p).unwrap();
        assert!(CommandLog::open(&p).is_err());
        let mut bytes = std::fs::read_to_string(&p).unwrap();
        bytes = bytes.replace("\"completed_count\":1", "\"completed_count\":0");
        std::fs::write(&p, bytes).unwrap();
        assert!(CommandLog::open_traffic(&p).is_err());
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn hold_release_is_hash_and_incarnation_bound_and_restart_conservative() {
        let p = path("hold");
        let log = CommandLog::open_traffic(&p).unwrap();
        log.audit(Event::Session { binding: session() }).unwrap();
        log.acceptance_action(
            key(),
            HookAction::Arm {
                mode: HookMode::HoldBeforeIntent,
                timeout_ms: 120000,
            },
        )
        .unwrap();
        assert!(log
            .acceptance_action(
                key(),
                HookAction::Release {
                    raw_sha256: "dd".repeat(32)
                }
            )
            .is_err());
        let signed = SignedCheckpoint {
            command_id: "payment".into(),
            raw_sha256: "dd".repeat(32),
            journal_seq: 11,
        };
        assert_eq!(log.acceptance_checkpoint(key(), &signed).unwrap(), "hold");
        assert!(log
            .acceptance_action(
                key(),
                HookAction::Release {
                    raw_sha256: "ee".repeat(32)
                }
            )
            .is_err());
        drop(log);
        let log = CommandLog::open_traffic(&p).unwrap();
        assert_eq!(log.acceptance_checkpoint(key(), &signed).unwrap(), "hold");
        let mut other = key();
        other.incarnation = "new".into();
        assert_eq!(log.acceptance_checkpoint(other, &signed).unwrap(), "cancel");
        log.acceptance_action(
            key(),
            HookAction::Release {
                raw_sha256: signed.raw_sha256.clone(),
            },
        )
        .unwrap();
        assert_eq!(
            log.acceptance_checkpoint(key(), &signed).unwrap(),
            "proceed"
        );
        log.acceptance_action(key(), HookAction::Cancel).unwrap();
        assert!(!log
            .acceptance_forward_allowed(&key(), Some(&signed.raw_sha256))
            .unwrap());
        assert_eq!(log.acceptance_checkpoint(key(), &signed).unwrap(), "cancel");
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn response_loss_is_exactly_once_and_requires_actual_completed_attempt() {
        let p = path("loss");
        let log = CommandLog::open_traffic(&p).unwrap();
        log.audit(Event::Session { binding: session() }).unwrap();
        log.acceptance_action(
            key(),
            HookAction::Arm {
                mode: HookMode::LoseOneResponse,
                timeout_ms: 120000,
            },
        )
        .unwrap();
        let signed = SignedCheckpoint {
            command_id: "payment".into(),
            raw_sha256: "dd".repeat(32),
            journal_seq: 11,
        };
        assert_eq!(
            log.acceptance_checkpoint(key(), &signed).unwrap(),
            "proceed"
        );
        assert!(log.acceptance_lose_response(&key(), Some(&signed.raw_sha256)));
        assert!(log
            .audit(Event::Hook {
                key: key(),
                detail: HookEvent::ResponseDiscarded { attempt_seq: 900 }
            })
            .is_err());
        let seq = log
            .audit(Event::ForwardAttempt { attempt: attempt() })
            .unwrap();
        assert!(log
            .audit(Event::Hook {
                key: key(),
                detail: HookEvent::ResponseDiscarded { attempt_seq: seq }
            })
            .is_err());
        log.audit(Event::ForwardResult {
            attempt_seq: seq,
            outcome: "ok".into(),
            response_sha256: "ee".repeat(32),
            response_suppressed: true,
        })
        .unwrap();
        log.audit(Event::Hook {
            key: key(),
            detail: HookEvent::ResponseDiscarded { attempt_seq: seq },
        })
        .unwrap();
        assert!(!log.acceptance_lose_response(&key(), Some(&signed.raw_sha256)));
        drop(log);
        let log = CommandLog::open_traffic(&p).unwrap();
        assert!(!log.acceptance_lose_response(&key(), Some(&signed.raw_sha256)));
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn logging_failure_poison_and_add_connection_before_boundary_refuse() {
        let p = path("fault");
        let log = CommandLog::open_traffic(&p).unwrap();
        log.audit(Event::Session { binding: session() }).unwrap();
        let miner = Event::MinerAttempt {
            run_id: "run".into(),
            miner_id: "miner".into(),
            incarnation: "m1".into(),
            request_id: "wire1".into(),
            payload_sha256: "aa".repeat(32),
            controller_sha256: "bb".repeat(32),
            actor_sha256: "cc".repeat(32),
        };
        assert!(log.audit(miner.clone()).is_err());
        log.audit(Event::MeasuredBoundary {
            command_id: "bootstrap".into(),
            tip_hash: "dd".repeat(32),
            tip_height: 102,
        })
        .unwrap();
        log.audit(miner).unwrap();
        log.fail_audit_writes();
        assert!(log
            .audit(Event::Session {
                binding: Session {
                    incarnation: "epoch2".into(),
                    ..session()
                }
            })
            .is_err());
        assert!(log.audit_revision("epoch").is_err());
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn disabled_hooks_and_missing_forward_context_refuse() {
        let p = path("disabled");
        let log = CommandLog::open_traffic(&p).unwrap();
        log.audit(Event::Session {
            binding: Session {
                acceptance_enabled: false,
                ..session()
            },
        })
        .unwrap();
        assert!(log
            .acceptance_action(
                key(),
                HookAction::Arm {
                    mode: HookMode::HoldBeforeIntent,
                    timeout_ms: 1000
                }
            )
            .is_err());
        let mut a = attempt();
        a.context.intent_seq = None;
        assert!(log.audit(Event::ForwardAttempt { attempt: a }).is_err());
        std::fs::remove_file(p).unwrap();
    }

    #[test]
    fn expired_hook_is_a_veto_and_new_run_binding_refuses() {
        let p = path("expiry");
        let log = CommandLog::open_traffic(&p).unwrap();
        log.audit(Event::Session { binding: session() }).unwrap();
        log.audit(Event::Hook {
            key: key(),
            detail: HookEvent::Armed {
                mode: HookMode::HoldBeforeIntent,
                deadline_ms: 1,
                boot_id: spc_lab_txd::bootstrap::boot_id().unwrap(),
            },
        })
        .unwrap();
        let signed = SignedCheckpoint {
            command_id: "payment".into(),
            raw_sha256: "dd".repeat(32),
            journal_seq: 11,
        };
        assert_eq!(log.acceptance_checkpoint(key(), &signed).unwrap(), "cancel");
        assert!(!log
            .acceptance_forward_allowed(&key(), Some(&signed.raw_sha256))
            .unwrap());
        assert!(log
            .audit(Event::Session {
                binding: Session {
                    run_id: "different".into(),
                    incarnation: "epoch2".into(),
                    ..session()
                }
            })
            .is_err());
        std::fs::remove_file(p).unwrap();
    }

    #[test]
    fn acceptance_command_is_operator_only_even_for_own_driver_target() {
        use crate::access::{check_command_scope, AccessClaim, ACCESS_CLAIM_SCHEMA};
        use crate::ids::{ActorId, RunId};
        let mut claim:AccessClaim=serde_json::from_value(serde_json::json!({
            "schema":ACCESS_CLAIM_SCHEMA,"run_id":"run","principal":"alice","token":"aa".repeat(32),
            "actor":"alice","may":["send_user_payment"],"user_id":"alice","miner_id":"miner","worker_ids":["worker"]
        })).unwrap();
        let command = super::super::LabCommand::new(
            RunId::trusted("run"),
            ActorId::trusted("alice"),
            super::super::CommandKind::TrafficAcceptance {
                key: key(),
                control: HookAction::Arm {
                    mode: HookMode::HoldBeforeIntent,
                    timeout_ms: 1000,
                },
            },
        );
        assert!(check_command_scope(&claim, &command).is_err());
        claim.may = vec!["operator".into()];
        claim.principal = "operator".into();
        assert!(check_command_scope(&claim, &command).is_ok());
    }
}
