//! Independent v2 credit checker.
//!
//! This module intentionally does not import `spc-accounting`. It recomputes
//! the share hash from the recorded 80-byte header, checks it against the job's
//! recorded share target, derives `d` from `job_issued.m_issue`, and compares
//! the exact sum with the recorded slice credit. A missing or ambiguous job is
//! `unknown`, not a guessed credit.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::evidence::{Accounted, JobIssued, ShareHeader, Submitted};
use crate::ids::{IdentityId, JobId, ShareId};
use crate::wide::{ReferenceProfile, Target256, WideU128};

#[derive(Debug, Clone)]
pub struct OracleShare {
    pub identity_id: IdentityId,
    pub share_id: ShareId,
    pub job_id: JobId,
    pub generation: u64,
    pub d: u64,
    pub header: ShareHeader,
}

impl OracleShare {
    pub fn from_submitted(submitted: &Submitted, d: u64) -> Self {
        Self {
            identity_id: submitted.identity_id.clone(),
            share_id: submitted.share_id.clone(),
            job_id: submitted.job_id.clone(),
            generation: submitted.generation,
            d,
            header: submitted.header.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OracleInput {
    pub jobs: Vec<JobIssued>,
    pub shares: Vec<OracleShare>,
    pub accounted: Vec<Accounted>,
    /// The v2 reference profile, when the caller wants the job's share target
    /// checked against the exact family as well.
    pub reference: Option<ReferenceProfile>,
    /// True when `shares` covers every share in the compared credit sums.
    pub shares_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OracleStatus {
    Verified,
    Mismatch(Vec<String>),
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OracleIdentity {
    pub identity_id: IdentityId,
    pub expected_units: WideU128,
    pub recorded_units: Option<WideU128>,
    pub sampled_shares: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OracleReport {
    pub status: OracleStatus,
    pub identities: Vec<OracleIdentity>,
    pub problems: Vec<String>,
}

/// Recompute each sampled share and compare exact credit per identity.
pub fn check_credit(input: &OracleInput) -> OracleReport {
    let mut problems = Vec::new();
    let mut unknown = Vec::new();
    let mut jobs: BTreeMap<&str, &JobIssued> = BTreeMap::new();
    for job in &input.jobs {
        if jobs.insert(job.job_id.as_str(), job).is_some() {
            unknown.push(format!(
                "job {} has more than one job_issued record",
                job.job_id
            ));
        }
    }

    if input.shares.is_empty() {
        return OracleReport {
            status: OracleStatus::Unknown("no shares were sampled".to_string()),
            identities: Vec::new(),
            problems,
        };
    }

    let mut expected: BTreeMap<IdentityId, u128> = BTreeMap::new();
    let mut sampled: BTreeMap<IdentityId, u64> = BTreeMap::new();
    let mut seen_shares = std::collections::BTreeSet::new();
    for share in &input.shares {
        if !seen_shares.insert(share.share_id.clone()) {
            problems.push(format!(
                "share {} appears twice in the sample",
                share.share_id
            ));
        }
        let Some(job) = jobs.get(share.job_id.as_str()) else {
            unknown.push(format!(
                "share {} names job {} with no job_issued record",
                share.share_id, share.job_id
            ));
            continue;
        };
        if share.generation != job.generation {
            problems.push(format!(
                "share {} credit belongs to generation {}, but job {} is generation {}",
                share.share_id, share.generation, job.job_id, job.generation
            ));
            continue;
        }
        if share.header.nbits != job.nbits {
            problems.push(format!(
                "share {} header nbits={:#010x} does not match job {} nbits={:#010x}",
                share.share_id, share.header.nbits, job.job_id, job.nbits
            ));
            continue;
        }
        let target = match Target256::from_hex(&job.share_target) {
            Ok(target) => target,
            Err(error) => {
                unknown.push(format!(
                    "job {} share target is invalid: {error}",
                    job.job_id
                ));
                continue;
            }
        };
        if let Some(reference) = &input.reference {
            if !job.m_issue.is_power_of_two() || job.m_issue == 0 {
                problems.push(format!(
                    "job {} m_issue={} is not a power-of-two family member",
                    job.job_id, job.m_issue
                ));
                continue;
            }
            let j = job.m_issue.trailing_zeros();
            match reference.share_target(j) {
                Ok(expected_target) if expected_target == target => {}
                Ok(expected_target) => problems.push(format!(
                    "job {} share target 0x{} is not the family target 0x{} for m={}",
                    job.job_id,
                    target.to_hex(),
                    expected_target.to_hex(),
                    job.m_issue
                )),
                Err(error) => {
                    unknown.push(format!("job {} family check failed: {error}", job.job_id))
                }
            }
        }
        let header = match share.header.serialize() {
            Ok(header) => header,
            Err(error) => {
                problems.push(format!(
                    "share {} header is invalid: {error}",
                    share.share_id
                ));
                continue;
            }
        };
        let digest = double_sha256(&header);
        if !target.hash_meets(&digest) {
            problems.push(format!(
                "share {} hash does not meet job {} share target",
                share.share_id, job.job_id
            ));
            continue;
        }
        let expected_d = u128::from(job.m_issue);
        if u128::from(share.d) != expected_d {
            problems.push(format!(
                "share {} counted d={}, job {} says d={}",
                share.share_id, share.d, job.job_id, job.m_issue
            ));
            continue;
        }
        *expected.entry(share.identity_id.clone()).or_insert(0) += expected_d;
        *sampled.entry(share.identity_id.clone()).or_insert(0) += 1;
    }

    let mut recorded: BTreeMap<IdentityId, u128> = BTreeMap::new();
    for accounted in &input.accounted {
        if let Some(job) = jobs.get(accounted.job_id.as_str()) {
            if accounted.generation != job.generation {
                problems.push(format!(
                    "accounted share {} records generation {}, job {} is generation {}",
                    accounted.share_id, accounted.generation, job.job_id, job.generation
                ));
            }
        } else {
            unknown.push(format!(
                "accounted share {} names job {} with no job_issued record",
                accounted.share_id, accounted.job_id
            ));
        }
        *recorded.entry(accounted.identity_id.clone()).or_insert(0) += accounted.diff_sum.get();
    }

    let mut identities = Vec::new();
    for (identity, units) in &expected {
        let recorded_units = recorded.get(identity).copied().map(WideU128);
        if let Some(recorded_units) = recorded_units {
            if input.shares_complete && recorded_units.get() != *units {
                problems.push(format!(
                    "identity {identity} recorded {} units but the oracle recomputes {units}",
                    recorded_units.get()
                ));
            }
        }
        identities.push(OracleIdentity {
            identity_id: identity.clone(),
            expected_units: WideU128(*units),
            recorded_units,
            sampled_shares: sampled.get(identity).copied().unwrap_or(0),
        });
    }
    for identity in recorded.keys() {
        if !expected.contains_key(identity) {
            identities.push(OracleIdentity {
                identity_id: identity.clone(),
                expected_units: WideU128(0),
                recorded_units: recorded.get(identity).copied().map(WideU128),
                sampled_shares: 0,
            });
            if input.shares_complete && unknown.is_empty() {
                problems.push(format!(
                    "identity {identity} has recorded credit but no sampled share"
                ));
            }
        }
    }

    let status = if !problems.is_empty() {
        OracleStatus::Mismatch(problems.clone())
    } else if !unknown.is_empty() {
        OracleStatus::Unknown(unknown.join("; "))
    } else {
        OracleStatus::Verified
    };
    OracleReport {
        status,
        identities,
        problems,
    }
}

pub fn double_sha256(header: &[u8; 80]) -> [u8; 32] {
    let first = Sha256::digest(header);
    let second = Sha256::digest(first);
    let mut out = [0u8; 32];
    out.copy_from_slice(&second);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::*;

    fn header_meeting(target: Target256) -> ShareHeader {
        for nonce in 0..100_000u32 {
            let header = ShareHeader {
                version: 0x2000_0000,
                prev_hash: "00".repeat(32),
                merkle_root: "11".repeat(32),
                ntime: 1_700_000_000,
                nbits: 0x207f_ffff,
                nonce,
            };
            let bytes = header.serialize().unwrap();
            let digest = double_sha256(&bytes);
            if target.hash_meets(&digest) {
                return header;
            }
        }
        panic!("could not find a meeting header");
    }

    #[test]
    fn recomputes_exact_credit_and_catches_wrong_generation() {
        let reference = ReferenceProfile::parse_hex(
            "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        )
        .unwrap();
        let target = reference.share_target(2).unwrap();
        let job = JobIssued {
            front_id: FrontId::new("front-a").unwrap(),
            channel_id: 1,
            job_id: JobId::new("job-1").unwrap(),
            generation: 7,
            nbits: 0x207f_ffff,
            share_target: target.to_hex(),
            m_issue: 4,
            work_unit_version: 2,
            work_unit_fingerprint: reference.fingerprint(),
            issued_at: 1,
            activation: crate::evidence::JobActivation::Active,
        };
        let identity = IdentityId::new("id-a").unwrap();
        let share = OracleShare {
            identity_id: identity.clone(),
            share_id: ShareId::new("share-1").unwrap(),
            job_id: job.job_id.clone(),
            generation: 7,
            d: 4,
            header: header_meeting(target),
        };
        let accounted = Accounted {
            front_id: FrontId::new("front-a").unwrap(),
            identity_id: identity.clone(),
            share_id: share.share_id.clone(),
            job_id: job.job_id.clone(),
            generation: 7,
            nbits: 0x207f_ffff,
            share_target: target.to_hex(),
            d: WideU128(4),
            slice_id: "slice-1".to_string(),
            diff_sum: WideU128(4),
        };
        let input = OracleInput {
            jobs: vec![job.clone()],
            shares: vec![share.clone()],
            accounted: vec![accounted],
            reference: Some(reference.clone()),
            shares_complete: true,
        };
        assert_eq!(check_credit(&input).status, OracleStatus::Verified);

        let mut wrong = input.clone();
        wrong.shares[0].generation = 8;
        assert!(matches!(
            check_credit(&wrong).status,
            OracleStatus::Mismatch(_)
        ));

        let mut missing = input;
        missing.jobs.clear();
        assert!(matches!(
            check_credit(&missing).status,
            OracleStatus::Unknown(_)
        ));
    }
}
