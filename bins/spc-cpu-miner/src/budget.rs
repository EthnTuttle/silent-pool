use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HashBudget {
    pub hashes_per_second: Option<u64>,
    pub total_attempts: Option<u64>,
}

impl HashBudget {
    pub fn validate(&self) -> Result<(), BudgetError> {
        if self.hashes_per_second.is_none() && self.total_attempts.is_none() {
            return Err(BudgetError::Empty);
        }
        if self.hashes_per_second == Some(0) {
            return Err(BudgetError::ZeroRate);
        }
        if self.total_attempts == Some(0) {
            return Err(BudgetError::ZeroAttempts);
        }
        Ok(())
    }

    pub fn remaining(&self, attempted: u64) -> u64 {
        self.total_attempts
            .map(|total| total.saturating_sub(attempted))
            .unwrap_or(u64::MAX)
    }

    pub fn allows(&self, attempted: u64) -> bool {
        self.remaining(attempted) > 0
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum BudgetError {
    #[error("a rate or total attempt budget is required")]
    Empty,
    #[error("hashes_per_second must be greater than zero")]
    ZeroRate,
    #[error("total_attempts must be greater than zero")]
    ZeroAttempts,
}

/// Calculate sleep required after `attempted` real hashes. Every attempt,
/// including the first, is derived from elapsed rate credit; no synthetic
/// attempt or background credit is created.
pub fn pacing_delay(hps: u64, attempted: u64, active_elapsed: Duration) -> Duration {
    if hps == 0 {
        return Duration::ZERO;
    }
    let elapsed_ns = active_elapsed.as_nanos();
    let allowed = elapsed_ns.saturating_mul(u128::from(hps)) / 1_000_000_000;
    let attempted = u128::from(attempted);
    if attempted <= allowed {
        return Duration::ZERO;
    }
    let debt = attempted - allowed;
    let nanos = debt
        .saturating_mul(1_000_000_000)
        .saturating_add(u128::from(hps) - 1)
        / u128::from(hps);
    Duration::from_nanos(nanos.min(u128::from(u64::MAX)) as u64)
}

/// Saturated means two complete measurement samples below 95% of the requested
/// rate while work was available and the pacer did not have to sleep. It does
/// not mean "paused", "no job" or "budget exhausted".
pub fn saturation_decision(samples: u32, budget_hps: u64, achieved_hps: u64, paced: bool) -> bool {
    samples >= 2 && !paced && achieved_hps.saturating_mul(20) < budget_hps.saturating_mul(19)
}

#[derive(Clone, Debug)]
pub struct Pacer {
    attempted: u64,
    active_elapsed: Duration,
    rate_hps: Option<u64>,
    rate_epoch_attempted: u64,
    rate_epoch_elapsed: Duration,
    resumed_at: Option<Instant>,
    sample_started_at: Instant,
    sample_started_attempted: u64,
    sample_started_elapsed: Duration,
    sample_count: u32,
    sample_paced: bool,
    paced: bool,
    saturated: bool,
}

impl Pacer {
    pub fn new(now: Instant) -> Self {
        Self {
            attempted: 0,
            active_elapsed: Duration::ZERO,
            rate_hps: None,
            rate_epoch_attempted: 0,
            rate_epoch_elapsed: Duration::ZERO,
            resumed_at: Some(now),
            sample_started_at: now,
            sample_started_attempted: 0,
            sample_started_elapsed: Duration::ZERO,
            sample_count: 0,
            sample_paced: false,
            paced: false,
            saturated: false,
        }
    }

    pub fn attempted(&self) -> u64 {
        self.attempted
    }

    pub fn elapsed(&self, now: Instant) -> Duration {
        self.active_elapsed
            + self
                .resumed_at
                .map(|start| now.saturating_duration_since(start))
                .unwrap_or(Duration::ZERO)
    }

    pub fn pause(&mut self, now: Instant) {
        if let Some(start) = self.resumed_at.take() {
            self.active_elapsed += now.saturating_duration_since(start);
        }
        self.paced = false;
        self.saturated = false;
    }

    pub fn resume(&mut self, now: Instant) {
        if self.resumed_at.is_none() {
            self.resumed_at = Some(now);
            self.sample_started_at = now;
            self.sample_started_elapsed = self.active_elapsed;
            self.sample_started_attempted = self.attempted;
            self.sample_count = 0;
            self.sample_paced = false;
        }
    }

    pub fn note_attempts(&mut self, attempts: u64) {
        self.attempted = self.attempted.saturating_add(attempts);
    }

    /// Attempts currently covered by the requested rate. Batches are capped to
    /// this credit before hashing, including the first batch.
    pub fn available_attempts(&mut self, budget: &HashBudget, now: Instant) -> u64 {
        self.rebase_budget(budget, now);
        let Some(hps) = budget.hashes_per_second else {
            return u64::MAX;
        };
        let epoch_elapsed = self.epoch_elapsed(now);
        let epoch_attempted = self.attempted - self.rate_epoch_attempted;
        let allowed = epoch_elapsed.as_nanos().saturating_mul(u128::from(hps)) / 1_000_000_000;
        allowed
            .saturating_sub(u128::from(epoch_attempted))
            .min(u128::from(u64::MAX)) as u64
    }

    fn epoch_elapsed(&self, now: Instant) -> Duration {
        self.elapsed(now).saturating_sub(self.rate_epoch_elapsed)
    }

    fn rebase_budget(&mut self, budget: &HashBudget, now: Instant) {
        if self.rate_hps == budget.hashes_per_second {
            return;
        }
        self.rate_hps = budget.hashes_per_second;
        self.rate_epoch_attempted = self.attempted;
        self.rate_epoch_elapsed = self.elapsed(now);
        self.sample_started_at = now;
        self.sample_started_attempted = self.attempted;
        self.sample_started_elapsed = self.elapsed(now);
        self.sample_count = 0;
        self.sample_paced = false;
        self.paced = false;
        self.saturated = false;
    }

    /// Sleep required before the next real attempt when its rate credit is
    /// exhausted. This is evaluated before hashing, not only after a burst.
    pub fn required_delay(&mut self, budget: &HashBudget, now: Instant) -> Duration {
        if self.available_attempts(budget, now) > 0 {
            return Duration::ZERO;
        }
        let Some(hps) = budget.hashes_per_second else {
            return Duration::ZERO;
        };
        pacing_delay(
            hps,
            self.attempted - self.rate_epoch_attempted + 1,
            self.epoch_elapsed(now),
        )
    }

    pub fn note_pace_sleep(&mut self) {
        self.paced = true;
        self.sample_paced = true;
    }

    pub fn finish_sample(&mut self, now: Instant, budget: &HashBudget) {
        let sample_elapsed = now.saturating_duration_since(self.sample_started_at);
        if sample_elapsed < Duration::from_millis(250) {
            return;
        }
        let sample_attempts = self.attempted - self.sample_started_attempted;
        let active_before = self.elapsed(now);
        let active_sample = active_before.saturating_sub(self.sample_started_elapsed);
        let achieved = if active_sample.is_zero() {
            0
        } else {
            (u128::from(sample_attempts) * 1_000_000_000 / active_sample.as_nanos())
                .min(u128::from(u64::MAX)) as u64
        };
        let low_cpu_sample = !self.sample_paced
            && budget
                .hashes_per_second
                .is_some_and(|hps| achieved.saturating_mul(20) < hps.saturating_mul(19));
        self.sample_count = if low_cpu_sample {
            self.sample_count.saturating_add(1)
        } else {
            0
        };
        self.saturated = budget
            .hashes_per_second
            .map(|hps| saturation_decision(self.sample_count, hps, achieved, self.sample_paced))
            .unwrap_or(false);
        self.sample_started_at = now;
        self.sample_started_attempted = self.attempted;
        self.sample_started_elapsed = active_before;
        self.sample_count = self.sample_count.min(2);
        self.sample_paced = false;
    }

    pub fn saturated(&self) -> bool {
        self.saturated
    }

    pub fn paced(&self) -> bool {
        self.paced
    }
}
