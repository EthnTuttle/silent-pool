use std::collections::BTreeMap;
use std::time::Instant;

use bitcoin::pow::Target;
use stratum_core::mining_sv2::{NewExtendedMiningJob, SetNewPrevHash, SetTarget};
use thiserror::Error;

use crate::ids::JobId;
use crate::target::{decode_compact_target, target_from_le_bytes, TargetError};

#[derive(Clone, Debug)]
pub struct PrevContext {
    pub prev_hash: [u8; 32],
    pub min_ntime: u32,
    pub nbits: u32,
    pub network_target: Target,
    pub received_at: Instant,
}

#[derive(Clone, Debug)]
pub struct PendingJob {
    pub job_id: JobId,
    pub version: u32,
    pub version_rolling_allowed: bool,
    pub merkle_path: Vec<[u8; 32]>,
    pub coinbase_tx_prefix: Vec<u8>,
    pub coinbase_tx_suffix: Vec<u8>,
    pub extranonce_prefix: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct JobTemplate {
    pub job_id: JobId,
    pub version: u32,
    pub version_rolling_allowed: bool,
    pub merkle_path: Vec<[u8; 32]>,
    pub coinbase_tx_prefix: Vec<u8>,
    pub coinbase_tx_suffix: Vec<u8>,
    pub extranonce_prefix: Vec<u8>,
    pub prev_hash: [u8; 32],
    pub min_ntime: u32,
    pub nbits: u32,
    pub network_target: Target,
    pub share_target: Target,
    pub target_generation: u64,
    pub activated_at: Instant,
}

#[derive(Clone, Debug, Default)]
pub struct StateChanges {
    pub activated: Vec<JobTemplate>,
    pub invalidated: Vec<JobId>,
    pub target_changed: bool,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("message for channel {actual}; expected {expected} or group {group}")]
    WrongChannel {
        expected: u32,
        group: u32,
        actual: u32,
    },
    #[error("job {0} is already present")]
    DuplicateJob(JobId),
    #[error("future job bound reached")]
    FutureJobBound,
    #[error("active job received before any SetNewPrevHash")]
    MissingPrevHash,
    #[error("SetNewPrevHash names job {0}, which was not a pending future job")]
    UnknownFutureJob(JobId),
    #[error("active job min_ntime {actual} precedes prevhash min_ntime {minimum}")]
    NtimeRegression { actual: u32, minimum: u32 },
    #[error("target generation exhausted")]
    TargetGenerationExhausted,
    #[error("invalid extranonce size {0}")]
    ExtranonceSize(usize),
    #[error(transparent)]
    Target(#[from] TargetError),
}

/// Independent SV2 job/prevhash/target state. At most one extended channel is
/// open on a connection; group broadcasts addressed to its group are accepted.
pub struct ProtocolState {
    channel_id: u32,
    group_channel_id: u32,
    channel_target: Target,
    target_generation: u64,
    extranonce_prefix: Vec<u8>,
    extranonce_size: usize,
    latest_prev: Option<PrevContext>,
    future_jobs: BTreeMap<JobId, PendingJob>,
    active_job: Option<JobTemplate>,
}

impl ProtocolState {
    pub fn new(
        channel_id: u32,
        group_channel_id: u32,
        initial_target: Target,
        extranonce_prefix: Vec<u8>,
        extranonce_size: usize,
    ) -> Result<Self, ProtocolError> {
        if !(4..=32).contains(&extranonce_size) {
            return Err(ProtocolError::ExtranonceSize(extranonce_size));
        }
        Ok(Self {
            channel_id,
            group_channel_id,
            channel_target: initial_target,
            target_generation: 0,
            extranonce_prefix,
            extranonce_size,
            latest_prev: None,
            future_jobs: BTreeMap::new(),
            active_job: None,
        })
    }

    fn accepts_channel(&self, actual: u32) -> bool {
        actual == self.channel_id || actual == self.group_channel_id
    }

    fn check_channel(&self, actual: u32) -> Result<(), ProtocolError> {
        if self.accepts_channel(actual) {
            Ok(())
        } else {
            Err(ProtocolError::WrongChannel {
                expected: self.channel_id,
                group: self.group_channel_id,
                actual,
            })
        }
    }

    pub fn channel_id(&self) -> u32 {
        self.channel_id
    }

    pub fn group_channel_id(&self) -> u32 {
        self.group_channel_id
    }

    pub fn channel_target(&self) -> Target {
        self.channel_target
    }

    pub fn target_generation(&self) -> u64 {
        self.target_generation
    }

    pub fn extranonce_size(&self) -> usize {
        self.extranonce_size
    }

    pub fn active_job(&self) -> Option<&JobTemplate> {
        self.active_job.as_ref()
    }

    pub fn future_jobs(&self) -> impl Iterator<Item = (&JobId, &PendingJob)> {
        self.future_jobs.iter()
    }

    pub fn future_job_count(&self) -> usize {
        self.future_jobs.len()
    }

    pub fn latest_prev(&self) -> Option<&PrevContext> {
        self.latest_prev.as_ref()
    }

    pub fn on_new_extended(
        &mut self,
        message: NewExtendedMiningJob,
    ) -> Result<StateChanges, ProtocolError> {
        self.check_channel(message.channel_id)?;
        let job_id = JobId::from(message.job_id);
        if self.future_jobs.contains_key(&job_id)
            || self
                .active_job
                .as_ref()
                .is_some_and(|job| job.job_id == job_id)
        {
            return Err(ProtocolError::DuplicateJob(job_id));
        }
        let pending = PendingJob {
            job_id,
            version: message.version,
            version_rolling_allowed: message.version_rolling_allowed,
            merkle_path: message
                .merkle_path
                .iter()
                .map(|hash| {
                    let mut bytes = [0u8; 32];
                    bytes.copy_from_slice(hash.as_bytes());
                    bytes
                })
                .collect(),
            coinbase_tx_prefix: message.coinbase_tx_prefix.as_bytes().to_vec(),
            coinbase_tx_suffix: message.coinbase_tx_suffix.as_bytes().to_vec(),
            extranonce_prefix: self.extranonce_prefix.clone(),
        };
        let min_ntime = message.min_ntime.into_inner();
        if let Some(min_ntime) = min_ntime {
            let prev = self
                .latest_prev
                .clone()
                .ok_or(ProtocolError::MissingPrevHash)?;
            if min_ntime < prev.min_ntime {
                return Err(ProtocolError::NtimeRegression {
                    actual: min_ntime,
                    minimum: prev.min_ntime,
                });
            }
            let active = self.activate(pending, &prev, min_ntime, Instant::now());
            let invalidated = self
                .active_job
                .as_ref()
                .map(|job| vec![job.job_id])
                .unwrap_or_default();
            self.active_job = Some(active.clone());
            Ok(StateChanges {
                activated: vec![active],
                invalidated,
                target_changed: false,
            })
        } else {
            if self.future_jobs.len() >= 256 {
                return Err(ProtocolError::FutureJobBound);
            }
            self.future_jobs.insert(pending.job_id, pending);
            Ok(StateChanges::default())
        }
    }

    pub fn on_set_new_prev_hash(
        &mut self,
        message: SetNewPrevHash,
    ) -> Result<StateChanges, ProtocolError> {
        self.check_channel(message.channel_id)?;
        let job_id = JobId::from(message.job_id);
        let network_target = decode_compact_target(message.nbits)?;
        if !self.future_jobs.contains_key(&job_id) {
            return Err(ProtocolError::UnknownFutureJob(job_id));
        }
        let mut prev_hash = [0u8; 32];
        prev_hash.copy_from_slice(message.prev_hash.as_bytes());
        let prev = PrevContext {
            prev_hash,
            min_ntime: message.min_ntime,
            nbits: message.nbits,
            network_target,
            received_at: Instant::now(),
        };
        let pending = self
            .future_jobs
            .remove(&job_id)
            .expect("presence checked above");
        let active = self.activate(pending, &prev, message.min_ntime, prev.received_at);
        let mut invalidated: Vec<JobId> = self.future_jobs.keys().copied().collect();
        if let Some(old) = &self.active_job {
            invalidated.push(old.job_id);
        }
        invalidated.sort_unstable();
        self.future_jobs.clear();
        self.active_job = Some(active.clone());
        self.latest_prev = Some(prev);
        Ok(StateChanges {
            activated: vec![active],
            invalidated,
            target_changed: false,
        })
    }

    fn activate(
        &self,
        pending: PendingJob,
        prev: &PrevContext,
        min_ntime: u32,
        activated_at: Instant,
    ) -> JobTemplate {
        // SetTarget semantics: a future job captures the channel target here,
        // at activation; an already-active job never passes through this again.
        JobTemplate {
            job_id: pending.job_id,
            version: pending.version,
            version_rolling_allowed: pending.version_rolling_allowed,
            merkle_path: pending.merkle_path,
            coinbase_tx_prefix: pending.coinbase_tx_prefix,
            coinbase_tx_suffix: pending.coinbase_tx_suffix,
            extranonce_prefix: pending.extranonce_prefix,
            prev_hash: prev.prev_hash,
            min_ntime,
            nbits: prev.nbits,
            network_target: prev.network_target,
            share_target: self.channel_target,
            target_generation: self.target_generation,
            activated_at,
        }
    }

    pub fn on_set_target(&mut self, message: SetTarget) -> Result<StateChanges, ProtocolError> {
        self.check_channel(message.channel_id)?;
        let target = target_from_le_bytes(message.maximum_target.as_bytes())?;
        self.channel_target = target;
        self.target_generation = self
            .target_generation
            .checked_add(1)
            .ok_or(ProtocolError::TargetGenerationExhausted)?;
        // Do not mutate active_job.share_target. Pending future jobs carry no
        // frozen target and capture this value in activate().
        Ok(StateChanges {
            target_changed: true,
            ..StateChanges::default()
        })
    }

    /// A prefix update applies only to jobs received after this call.
    pub fn set_extranonce_prefix(
        &mut self,
        channel_id: u32,
        prefix: Vec<u8>,
    ) -> Result<(), ProtocolError> {
        if channel_id != self.channel_id {
            return Err(ProtocolError::WrongChannel {
                expected: self.channel_id,
                group: self.group_channel_id,
                actual: channel_id,
            });
        }
        self.extranonce_prefix = prefix;
        Ok(())
    }
}
