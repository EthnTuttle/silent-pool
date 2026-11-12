//! Crash-safe hash authorization in the controller's existing CommandLog.
//! Reserved debit is never refunded; measured work is a separate quantity.

use std::collections::BTreeMap;
use std::io;

use serde::{Deserialize, Serialize};

use super::{write_line, CommandLog, LogLine, State};
use crate::ids::CommandId;

pub(super) fn invalid(reason: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

/// Fresh CLOCK_MONOTONIC reading for guarded budget-start validation.
/// The production path samples the real clock INSIDE the command log
/// state lock at the call site; tests inject a fixed value or a failure
/// through the private override below - never through a caller-supplied
/// timestamp, so a stale reading can never be substituted for the
/// guarded sample.
fn fresh_monotonic_ms() -> io::Result<u64> {
    #[cfg(test)]
    {
        match *TEST_MONOTONIC.lock().unwrap() {
            TestClock::Fixed(v) => return Ok(v),
            TestClock::Fail => {
                return Err(invalid("monotonic clock unreadable (injected)"))
            }
            TestClock::Real => {}
        }
    }
    spc_lab_txd::bootstrap::monotonic_ms()
        .map_err(|e| invalid(&format!("monotonic clock unreadable: {e}")))
}

/// Current Linux boot id for guarded budget-start validation.
fn fresh_boot_id() -> io::Result<String> {
    #[cfg(test)]
    {
        let boot = TEST_BOOT.lock().unwrap();
        if let Some(boot) = boot.as_ref() {
            return Ok(boot.clone());
        }
    }
    spc_lab_txd::bootstrap::boot_id()
        .map_err(|e| invalid(&format!("boot id unreadable: {e}")))
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum TestClock {
    Real,
    Fixed(u64),
    Fail,
}

#[cfg(test)]
static TEST_MONOTONIC: std::sync::Mutex<TestClock> =
    std::sync::Mutex::new(TestClock::Real);

#[cfg(test)]
static TEST_BOOT: std::sync::Mutex<Option<String>> =
    std::sync::Mutex::new(None);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetOperation {
    pub operation_id: CommandId,
    pub driver_id: String,
    pub boot_id: String,
    pub deadline_monotonic_ms: u64,
    pub total_attempts: u64,
    pub hashes_per_second: u64,
    pub cancel_grace_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HashAuthorization {
    pub operation_id: CommandId,
    pub driver_incarnation: String,
    pub batch_id: u64,
    pub reserved_attempts: u64,
    pub cumulative_reserved: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReservationStatus {
    pub authorization: HashAuthorization,
    /// A duplicate is status only: it never starts another execution.
    pub duplicate: bool,
    pub reconciled: bool,
    pub measured_attempts: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HashBudgetSnapshot {
    pub operation: BudgetOperation,
    pub cumulative_reserved: u64,
    pub next_batch_id: u64,
    pub outstanding: Option<HashAuthorization>,
}

#[derive(Clone)]
struct Batch {
    authorization: HashAuthorization,
    reconciled: bool,
    measured: Option<u64>,
}

pub(super) struct BudgetState {
    operation: BudgetOperation,
    cumulative: u64,
    batches: BTreeMap<u64, Batch>,
}

pub(super) fn apply(state: &mut State, record: &LogLine) -> io::Result<()> {
    match record {
        LogLine::HashStart { operation, .. } => {
            if operation.total_attempts == 0 || operation.hashes_per_second == 0
                || operation.cancel_grace_secs == 0 || operation.deadline_monotonic_ms == 0
                || operation.boot_id.is_empty() || operation.driver_id.is_empty()
                || !state.entries.get(&operation.operation_id).is_some_and(|entry| entry.outcome.is_none()
                    && matches!(&entry.command.kind, super::CommandKind::BootstrapFund { driver_id, .. } if driver_id.as_str() == operation.driver_id))
                || state.budgets.contains_key(&operation.operation_id)
            {
                return Err(invalid("invalid or duplicate bootstrap operation"));
            }
            state.budgets.insert(
                operation.operation_id.clone(),
                BudgetState {
                    operation: operation.clone(),
                    cumulative: 0,
                    batches: BTreeMap::new(),
                },
            );
        }
        LogLine::HashReserve {
            operation_id,
            driver_incarnation,
            batch_id,
            reserved_attempts,
            cumulative_reserved,
            ..
        } => {
            let budget = state
                .budgets
                .get_mut(operation_id)
                .ok_or_else(|| invalid("missing bootstrap reservation history"))?;
            let expected_batch = u64::try_from(budget.batches.len())
                .ok()
                .and_then(|n| n.checked_add(1))
                .ok_or_else(|| invalid("batch overflow"))?;
            if driver_incarnation.is_empty()
                || *reserved_attempts == 0
                || *batch_id != expected_batch
                || budget.batches.values().any(|b| !b.reconciled)
                || budget.cumulative.checked_add(*reserved_attempts) != Some(*cumulative_reserved)
                || *cumulative_reserved > budget.operation.total_attempts
            {
                return Err(invalid(
                    "invalid hash reservation arithmetic or outstanding batch",
                ));
            }
            budget.cumulative = *cumulative_reserved;
            budget.batches.insert(
                *batch_id,
                Batch {
                    authorization: HashAuthorization {
                        operation_id: operation_id.clone(),
                        driver_incarnation: driver_incarnation.clone(),
                        batch_id: *batch_id,
                        reserved_attempts: *reserved_attempts,
                        cumulative_reserved: *cumulative_reserved,
                    },
                    reconciled: false,
                    measured: None,
                },
            );
        }
        LogLine::HashReconcile {
            operation_id,
            driver_incarnation,
            batch_id,
            measured_attempts,
            ..
        } => {
            let batch = state
                .budgets
                .get_mut(operation_id)
                .and_then(|b| b.batches.get_mut(batch_id))
                .ok_or_else(|| invalid("reconciliation without reservation"))?;
            if batch.reconciled
                || batch.authorization.driver_incarnation != *driver_incarnation
                || measured_attempts.is_some_and(|n| n > batch.authorization.reserved_attempts)
            {
                return Err(invalid("invalid hash reconciliation"));
            }
            batch.reconciled = true;
            batch.measured = *measured_attempts;
        }
        _ => return Err(invalid("not a traffic budget record")),
    }
    Ok(())
}

fn ready(state: &State) -> io::Result<()> {
    if !state.traffic_strict || state.poisoned || state.next_seq == u64::MAX {
        Err(invalid("bootstrap authorization unavailable"))
    } else {
        Ok(())
    }
}

impl CommandLog {
    pub fn hash_budget_snapshot(&self, operation: &CommandId) -> io::Result<HashBudgetSnapshot> {
        let state = self.state.lock().unwrap();
        ready(&state)?;
        let budget = state
            .budgets
            .get(operation)
            .ok_or_else(|| invalid("unknown hash operation"))?;
        Ok(HashBudgetSnapshot {
            operation: budget.operation.clone(),
            cumulative_reserved: budget.cumulative,
            next_batch_id: (budget.batches.len() as u64)
                .checked_add(1)
                .ok_or_else(|| invalid("batch overflow"))?,
            outstanding: budget
                .batches
                .values()
                .find(|b| !b.reconciled)
                .map(|b| b.authorization.clone()),
        })
    }
    /// UNCHECKED budget start, visible to TESTS ONLY: replay-capable but
    /// without the guarded clock validation. Every production start site
    /// must use start_hash_budget_checked instead; keeping this
    /// test-gated prevents an unchecked-helper bypass.
    #[cfg(test)]
    pub fn start_hash_budget(&self, operation: BudgetOperation) -> io::Result<()> {
        self.start_hash_budget_inner(operation, None)
    }

    /// Checked budget start (contract version 3): the ONLY production
    /// entrypoint for a NEW HashStart record. The command log state lock
    /// is acquired first, then a fresh CLOCK_MONOTONIC reading and the
    /// Linux boot id are sampled INSIDE that guard and validated
    /// immediately before the new HashStart write: the sampled clock
    /// must be strictly BEFORE `operation.deadline_monotonic_ms` (the
    /// never-reset original operation deadline) and BEFORE
    /// `acquisition_deadline_monotonic_ms` (the readiness acquisition
    /// expiry carried out of the gate) - equality is EXPIRED, consistent
    /// with `reserve_hashes` - and the sampled boot id must match the
    /// operation's. A lock wait or stall that carries the clock to
    /// either bound returns a rejected precondition: no HashStart, no
    /// hash debit, no new operation id. An existing identical record
    /// replays idempotently WITHOUT clock validation (it is history,
    /// not a new start) and never rewrites the log. The clock is never
    /// caller-supplied: a stale timestamp cannot be substituted for the
    /// guarded sample.
    pub fn start_hash_budget_checked(
        &self,
        operation: BudgetOperation,
        acquisition_deadline_monotonic_ms: u64,
    ) -> io::Result<()> {
        self.start_hash_budget_inner(operation, Some(acquisition_deadline_monotonic_ms))
    }

    fn start_hash_budget_inner(
        &self,
        operation: BudgetOperation,
        acquisition_deadline: Option<u64>,
    ) -> io::Result<()> {
        let mut state = self.state.lock().unwrap();
        ready(&state)?;
        if let Some(existing) = state.budgets.get(&operation.operation_id) {
            return if existing.operation == operation {
                // Idempotent replay of an existing record: no new
                // HashStart, no clock validation, no log rewrite.
                Ok(())
            } else {
                Err(invalid("bootstrap operation binding conflict"))
            };
        }
        // Guarded fresh-clock validation for a NEW start only; the
        // replay path above is history and never re-validates the
        // current clock.
        if let Some(acquisition_deadline) = acquisition_deadline {
            let now = fresh_monotonic_ms()?;
            let boot = fresh_boot_id()?;
            if boot != operation.boot_id {
                return Err(invalid("bootstrap boot changed before hash start"));
            }
            if now >= operation.deadline_monotonic_ms {
                return Err(invalid(
                    "bootstrap operation deadline expired before hash start",
                ));
            }
            if now >= acquisition_deadline {
                return Err(invalid(
                    "readiness acquisition deadline expired before hash start",
                ));
            }
        }
        // Check the same invariants before writing; replay uses apply again.
        if operation.total_attempts == 0 || operation.hashes_per_second == 0
            || operation.cancel_grace_secs == 0 || operation.deadline_monotonic_ms == 0
            || operation.boot_id.is_empty() || operation.driver_id.is_empty()
            || !state.entries.get(&operation.operation_id).is_some_and(|e| e.outcome.is_none()
                && matches!(&e.command.kind, super::CommandKind::BootstrapFund { driver_id, .. } if driver_id.as_str() == operation.driver_id))
        { return Err(invalid("invalid bootstrap operation")); }
        let record = LogLine::HashStart {
            seq: state.next_seq,
            operation,
        };
        write_line(&mut state, &record)?;
        apply(&mut state, &record)?;
        state.next_seq += 1;
        Ok(())
    }

    /// Caller supplies a fresh CLOCK_MONOTONIC reading and Linux boot id.
    /// Rate pacing is the executor's independent Pacer; debit is a hard cap.
    pub fn reserve_hashes(
        &self,
        operation_id: &CommandId,
        incarnation: &str,
        batch_id: u64,
        attempts: u64,
        boot_id: &str,
        monotonic_ms: u64,
    ) -> io::Result<ReservationStatus> {
        let mut state = self.state.lock().unwrap();
        ready(&state)?;
        let budget = state
            .budgets
            .get(operation_id)
            .ok_or_else(|| invalid("unknown bootstrap operation"))?;
        if boot_id != budget.operation.boot_id
            || monotonic_ms >= budget.operation.deadline_monotonic_ms
        {
            return Err(invalid("bootstrap deadline expired or boot changed"));
        }
        if let Some(batch) = budget.batches.get(&batch_id) {
            if batch.authorization.driver_incarnation != incarnation
                || batch.authorization.reserved_attempts != attempts
            {
                return Err(invalid("old incarnation or conflicting batch"));
            }
            return Ok(ReservationStatus {
                authorization: batch.authorization.clone(),
                duplicate: true,
                reconciled: batch.reconciled,
                measured_attempts: batch.measured,
            });
        }
        if state
            .entries
            .get(operation_id)
            .is_none_or(|e| e.outcome.is_some())
        {
            return Err(invalid("bootstrap operation finished"));
        }
        let cumulative = budget
            .cumulative
            .checked_add(attempts)
            .ok_or_else(|| invalid("hash budget overflow"))?;
        if incarnation.is_empty()
            || attempts == 0
            || cumulative > budget.operation.total_attempts
            || budget.batches.values().any(|b| !b.reconciled)
            || batch_id
                != (budget.batches.len() as u64)
                    .checked_add(1)
                    .ok_or_else(|| invalid("batch overflow"))?
        {
            return Err(invalid("hash budget exhausted or outstanding batch"));
        }
        let authorization = HashAuthorization {
            operation_id: operation_id.clone(),
            driver_incarnation: incarnation.to_owned(),
            batch_id,
            reserved_attempts: attempts,
            cumulative_reserved: cumulative,
        };
        let record = LogLine::HashReserve {
            seq: state.next_seq,
            operation_id: operation_id.clone(),
            driver_incarnation: incarnation.to_owned(),
            batch_id,
            reserved_attempts: attempts,
            cumulative_reserved: cumulative,
        };
        write_line(&mut state, &record)?;
        apply(&mut state, &record)?;
        state.next_seq += 1;
        Ok(ReservationStatus {
            authorization,
            duplicate: false,
            reconciled: false,
            measured_attempts: None,
        })
    }

    /// None means unobservable work after death. It still charges the entire
    /// reservation and closes it without an executed-hash claim or refund.
    pub fn reconcile_hashes(
        &self,
        authorization: &HashAuthorization,
        measured: Option<u64>,
    ) -> io::Result<()> {
        let mut state = self.state.lock().unwrap();
        ready(&state)?;
        let batch = state
            .budgets
            .get(&authorization.operation_id)
            .and_then(|b| b.batches.get(&authorization.batch_id))
            .ok_or_else(|| invalid("unknown hash authorization"))?;
        if batch.authorization != *authorization
            || measured.is_some_and(|n| n > authorization.reserved_attempts)
        {
            return Err(invalid("hash reconciliation binding mismatch"));
        }
        if batch.reconciled {
            return if batch.measured == measured {
                Ok(())
            } else {
                Err(invalid("hash measurement conflict"))
            };
        }
        let record = LogLine::HashReconcile {
            seq: state.next_seq,
            operation_id: authorization.operation_id.clone(),
            driver_incarnation: authorization.driver_incarnation.clone(),
            batch_id: authorization.batch_id,
            measured_attempts: measured,
        };
        write_line(&mut state, &record)?;
        apply(&mut state, &record)?;
        state.next_seq += 1;
        Ok(())
    }

    pub fn hash_accounting(&self, operation_id: &CommandId) -> io::Result<(u64, u64, u64)> {
        let state = self.state.lock().unwrap();
        let budget = state
            .budgets
            .get(operation_id)
            .ok_or_else(|| invalid("unknown bootstrap operation"))?;
        let mut measured = 0u64;
        let mut unknown = 0u64;
        for batch in budget.batches.values() {
            if let Some(n) = batch.measured {
                measured = measured
                    .checked_add(n)
                    .ok_or_else(|| invalid("measurement overflow"))?;
            } else {
                unknown = unknown
                    .checked_add(batch.authorization.reserved_attempts)
                    .ok_or_else(|| invalid("unknown debit overflow"))?;
            }
        }
        Ok((budget.cumulative, measured, unknown))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{CommandKind, LabCommand};
    use crate::ids::{ActorId, NodeId, RunId};
    fn setup(path: &std::path::Path) -> (CommandLog, CommandId) {
        let log = CommandLog::open_traffic(path).unwrap();
        let cmd = LabCommand::new(
            RunId::trusted("run"),
            ActorId::trusted("operator"),
            CommandKind::BootstrapFund {
                driver_id: ActorId::trusted("txd"),
                node_id: NodeId::trusted("node"),
            },
        );
        log.begin_scoped(&cmd, "operator").unwrap();
        log.start_hash_budget(BudgetOperation {
            operation_id: cmd.command_id.clone(),
            driver_id: "txd".into(),
            boot_id: "boot".into(),
            deadline_monotonic_ms: 10000,
            total_attempts: 40,
            hashes_per_second: 40,
            cancel_grace_secs: 30,
        })
        .unwrap();
        (log, cmd.command_id)
    }

    /// Serializes the tests that use the injected clock/boot hooks (the
    /// hooks are process-wide statics, and cargo runs tests in parallel).
    static TEST_HOOK_ORDER: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// A command + Begin record only, for checked-start tests.
    fn checked_setup(path: &std::path::Path) -> (CommandLog, CommandId) {
        let log = CommandLog::open_traffic(path).unwrap();
        let cmd = LabCommand::new(
            RunId::trusted("run"),
            ActorId::trusted("operator"),
            CommandKind::BootstrapFund {
                driver_id: ActorId::trusted("txd"),
                node_id: NodeId::trusted("node"),
            },
        );
        log.begin_scoped(&cmd, "operator").unwrap();
        (log, cmd.command_id)
    }

    fn budget_op(id: &CommandId, deadline: u64) -> BudgetOperation {
        BudgetOperation {
            operation_id: id.clone(),
            driver_id: "txd".into(),
            boot_id: "boot".into(),
            deadline_monotonic_ms: deadline,
            total_attempts: 40,
            hashes_per_second: 40,
            cancel_grace_secs: 30,
        }
    }

    /// (begin records, hash_start records) in the on-disk JSONL log.
    fn record_counts(path: &std::path::Path) -> (usize, usize) {
        let text = std::fs::read_to_string(path).unwrap();
        let mut counts = (0, 0);
        for line in text.lines() {
            if line.contains("\"record\":\"begin\"") {
                counts.0 += 1;
            }
            if line.contains("\"record\":\"hash_start\"") {
                counts.1 += 1;
            }
        }
        counts
    }

    /// Guarded checked budget start (contract version 3): the fresh
    /// clock is sampled inside the state lock; equality with EITHER
    /// deadline is EXPIRED (consistent with reserve_hashes); clock
    /// failure and boot mismatch refuse closed; an earlier clock starts
    /// the budget once; an identical existing record replays
    /// idempotently WITHOUT re-validating the current clock and writes
    /// no second HashStart. Injected clock only - no host clock changes.
    #[test]
    fn checked_budget_start_guarded_clock_semantics() {
        let _serial = TEST_HOOK_ORDER.lock().unwrap();
        let dir = std::env::temp_dir().join(format!(
            "spc-budget-checked-{}-{}",
            std::process::id(),
            CommandId::generate()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("commands.jsonl");
        let (log, id) = checked_setup(&path);
        let operation = budget_op(&id, 10_000);

        // Earlier positive: inside both bounds, exactly one HashStart.
        *TEST_BOOT.lock().unwrap() = Some("boot".into());
        *TEST_MONOTONIC.lock().unwrap() = TestClock::Fixed(1_000);
        log.start_hash_budget_checked(operation.clone(), 9_000).unwrap();
        assert_eq!(record_counts(&path), (1, 1));

        // Idempotent replay: same operation with the clock now past
        // every deadline - history replays without clock validation and
        // without a second HashStart.
        *TEST_MONOTONIC.lock().unwrap() = TestClock::Fixed(50_000);
        log.start_hash_budget_checked(operation.clone(), 9_000).unwrap();
        assert_eq!(record_counts(&path), (1, 1));

        // A different operation bound to the same id still conflicts.
        let conflicting = BudgetOperation {
            deadline_monotonic_ms: 20_000,
            ..operation.clone()
        };
        assert!(log.start_hash_budget_checked(conflicting, 9_000).is_err());
        assert_eq!(record_counts(&path), (1, 1));

        // Fresh command: equality with the original operation deadline
        // is expired even when the carried acquisition bound is later.
        let second = dir.join("second.jsonl");
        let (log2, id2) = checked_setup(&second);
        *TEST_MONOTONIC.lock().unwrap() = TestClock::Fixed(10_000);
        let err = log2
            .start_hash_budget_checked(budget_op(&id2, 10_000), 11_000)
            .unwrap_err();
        assert!(
            err.to_string().contains("operation deadline expired"),
            "{err}"
        );

        // Equality with the carried acquisition expiry is expired too.
        *TEST_MONOTONIC.lock().unwrap() = TestClock::Fixed(9_000);
        let err = log2
            .start_hash_budget_checked(budget_op(&id2, 10_000), 9_000)
            .unwrap_err();
        assert!(
            err.to_string().contains("acquisition deadline expired"),
            "{err}"
        );
        assert_eq!(record_counts(&second), (1, 0));

        // Clock failure refuses closed.
        *TEST_MONOTONIC.lock().unwrap() = TestClock::Fail;
        assert!(
            log2.start_hash_budget_checked(budget_op(&id2, 10_000), 9_000)
                .is_err()
        );

        // Boot mismatch refuses closed.
        *TEST_MONOTONIC.lock().unwrap() = TestClock::Fixed(1_000);
        *TEST_BOOT.lock().unwrap() = Some("another-boot".into());
        let err = log2
            .start_hash_budget_checked(budget_op(&id2, 10_000), 9_000)
            .unwrap_err();
        assert!(err.to_string().contains("boot changed"), "{err}");
        assert_eq!(record_counts(&second), (1, 0));

        // Reset the injection for other tests.
        *TEST_MONOTONIC.lock().unwrap() = TestClock::Real;
        *TEST_BOOT.lock().unwrap() = None;
    }

    /// A command-log state lock held past the deadline while a waiting
    /// checked start blocks: on release the waiting start samples the
    /// fresh clock INSIDE the guard, sees it past the deadline, and
    /// returns a rejected precondition - no HashStart is written, the
    /// original command keeps exactly one Begin, and no hash debit or
    /// new operation id exists. Injected clock; no host clock changes.
    #[test]
    fn checked_start_waiting_on_locked_log_past_deadline_never_writes_hash_start() {
        let _serial = TEST_HOOK_ORDER.lock().unwrap();
        let dir = std::env::temp_dir().join(format!(
            "spc-budget-locked-{}-{}",
            std::process::id(),
            CommandId::generate()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("commands.jsonl");
        let (log, id) = checked_setup(&path);
        let log = std::sync::Arc::new(log);
        *TEST_BOOT.lock().unwrap() = Some("boot".into());
        *TEST_MONOTONIC.lock().unwrap() = TestClock::Fixed(1_000);
        let operation = budget_op(&id, 10_000);
        // Hold the state lock so the started thread blocks; the clock is
        // carried past the deadline BEFORE the lock is released, so the
        // waiting start can only sample the expired reading.
        let guard = log.state.lock().unwrap();
        let waiting = {
            let log = std::sync::Arc::clone(&log);
            std::thread::spawn(move || log.start_hash_budget_checked(operation, 9_000))
        };
        *TEST_MONOTONIC.lock().unwrap() = TestClock::Fixed(50_000);
        drop(guard);
        let err = waiting.join().unwrap().unwrap_err();
        assert!(
            err.to_string().contains("operation deadline expired"),
            "{err}"
        );
        assert_eq!(record_counts(&path), (1, 0));

        // Reset the injection for other tests.
        *TEST_MONOTONIC.lock().unwrap() = TestClock::Real;
        *TEST_BOOT.lock().unwrap() = None;
    }
    #[test]
    fn crash_lost_ack_and_partial_measurement_never_refund() {
        let dir = std::env::temp_dir().join(format!(
            "spc-budget-{}-{}",
            std::process::id(),
            CommandId::generate()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("commands.jsonl");
        let (log, id) = setup(&path);
        let first = log.reserve_hashes(&id, "old", 1, 20, "boot", 1).unwrap();
        assert!(!first.duplicate);
        assert!(
            log.reserve_hashes(&id, "old", 1, 20, "boot", 2)
                .unwrap()
                .duplicate
        );
        assert!(log.reserve_hashes(&id, "old", 2, 20, "boot", 2).is_err());
        drop(log);
        let log = CommandLog::open_traffic(&path).unwrap();
        assert_eq!(log.hash_accounting(&id).unwrap(), (20, 0, 20));
        assert!(log.reserve_hashes(&id, "new", 1, 20, "boot", 3).is_err());
        log.reconcile_hashes(&first.authorization, None).unwrap();
        log.reconcile_hashes(&first.authorization, None).unwrap();
        let second = log.reserve_hashes(&id, "new", 2, 20, "boot", 4).unwrap();
        log.reconcile_hashes(&second.authorization, Some(7))
            .unwrap();
        assert_eq!(log.hash_accounting(&id).unwrap(), (40, 7, 20));
        assert!(log.reserve_hashes(&id, "new", 3, 1, "boot", 5).is_err());
        assert!(log
            .reserve_hashes(&id, "new", 2, 20, "another-boot", 5)
            .is_err());
        assert!(log
            .reserve_hashes(&id, "new", 2, 20, "boot", 10000)
            .is_err());
        drop(log);
        assert_eq!(
            CommandLog::open_traffic(&path)
                .unwrap()
                .hash_accounting(&id)
                .unwrap(),
            (40, 7, 20)
        );
    }
    #[test]
    fn strict_complete_corruption_refuses_without_repair() {
        let dir =
            std::env::temp_dir().join(format!("spc-budget-corrupt-{}", CommandId::generate()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("commands.jsonl");
        let (log, _) = setup(&path);
        drop(log);
        let original = std::fs::read(&path).unwrap();
        let mut corrupt = original.clone();
        corrupt.extend_from_slice(b"{bad}\n");
        std::fs::write(&path, &corrupt).unwrap();
        assert!(CommandLog::open_traffic(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), corrupt);
        let mut tail = original.clone();
        tail.extend_from_slice(b"{\"record\":");
        std::fs::write(&path, &tail).unwrap();
        drop(CommandLog::open_traffic(&path).unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
    #[test]
    fn actual_write_and_sync_failures_poison_authorization() {
        for device in ["/dev/full", "/dev/null"] {
            let dir =
                std::env::temp_dir().join(format!("spc-budget-fault-{}", CommandId::generate()));
            std::fs::create_dir(&dir).unwrap();
            let path = dir.join("commands.jsonl");
            let (log, id) = setup(&path);
            log.state.lock().unwrap().file = std::fs::OpenOptions::new()
                .write(true)
                .open(device)
                .unwrap();
            // /dev/full fails write; /dev/null accepts write/flush but rejects sync_data.
            assert!(log.reserve_hashes(&id, "inc", 1, 20, "boot", 1).is_err());
            assert!(log.state.lock().unwrap().poisoned);
            assert!(log.reserve_hashes(&id, "inc", 1, 20, "boot", 2).is_err());
            assert_eq!(log.hash_accounting(&id).unwrap(), (0, 0, 0));
        }
    }
}
