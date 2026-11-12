//! Concrete cooperative long-operation seam for the separately owned T2-L
//! executor. No fallback miner, synthetic funding or success implementation.
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Clone, Default)]
pub struct OperationControl {
    cancelled: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
}
impl OperationControl {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }
    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
    }
    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapRequest {
    pub command_id: String,
    pub driver_id: String,
    pub node_id: String,
    pub boot_id: String,
    pub deadline_monotonic_ms: u64,
    pub cancel_grace_secs: u64,
}

/// The executor must verify raw canonical distribution confirmation on all
/// declared nodes before returning this evidence. The controller independently
/// checks that boundary before activating any miner/scheduler.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapCompletion {
    pub distribution_txids: Vec<String>,
    pub common_tip_hash: String,
    pub common_tip_height: u64,
}

pub trait BootstrapExecutor: Send + Sync {
    fn execute(
        &self,
        request: BootstrapRequest,
        control: OperationControl,
        host: Arc<dyn BootstrapHost>,
    ) -> Result<BootstrapCompletion, String>;
}

/// T2's concrete controller/actor join. Implementations spend only through the
/// actor's existing journal and obtain each bounded hash permit through the
/// durable controller log. There is no default block-production provider.
pub trait BootstrapHost: Send + Sync {
    fn actor_request(
        &self,
        operation: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HashAuthorization {
    pub operation_id: String,
    pub driver_incarnation: String,
    pub batch_id: u64,
    pub reserved_attempts: u64,
    pub cumulative_reserved: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HashReservationStatus {
    pub authorization: HashAuthorization,
    pub duplicate: bool,
    pub reconciled: bool,
    pub measured_attempts: Option<u64>,
}

/// Local execution consumption, not a budget ledger. The controller remains
/// the sole debit authority. A duplicate/lost-ack status never grants work a
/// second time; old-incarnation authorizations cannot be redeemed at all.
pub struct BatchConsumption {
    operation: String,
    incarnation: String,
    consumed: std::collections::BTreeMap<u64, HashAuthorization>,
}
impl BatchConsumption {
    pub fn new(operation: String, incarnation: String) -> Self {
        Self {
            operation,
            incarnation,
            consumed: std::collections::BTreeMap::new(),
        }
    }
    pub fn consume(&mut self, status: &HashReservationStatus) -> Result<bool, String> {
        let a = &status.authorization;
        if a.operation_id != self.operation
            || a.driver_incarnation != self.incarnation
            || a.batch_id == 0
            || a.reserved_attempts == 0
            || a.cumulative_reserved < a.reserved_attempts
        {
            return Err("hash_authorization_binding_invalid".into());
        }
        if let Some(previous) = self.consumed.get(&a.batch_id) {
            return if previous == a {
                Ok(false)
            } else {
                Err("hash_authorization_conflict".into())
            };
        }
        if self
            .consumed
            .last_key_value()
            .is_some_and(|(id, _)| *id >= a.batch_id)
        {
            return Err("hash_authorization_out_of_order".into());
        }
        self.consumed.insert(a.batch_id, a.clone());
        Ok(!status.duplicate && !status.reconciled)
    }
}

pub fn monotonic_ms() -> std::io::Result<u64> {
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    let secs = u64::try_from(now.tv_sec)
        .map_err(|_| crate::control::invalid("negative monotonic clock"))?;
    secs.checked_mul(1000)
        .and_then(|n| n.checked_add(now.tv_nsec as u64 / 1_000_000))
        .ok_or_else(|| crate::control::invalid("monotonic clock overflow"))
}

pub fn boot_id() -> std::io::Result<String> {
    let id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    let id = id.trim();
    if id.len() != 36 || !id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        return Err(crate::control::invalid("unverifiable Linux boot id"));
    }
    Ok(id.to_owned())
}
