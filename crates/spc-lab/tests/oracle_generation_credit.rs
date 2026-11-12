mod common;

use spc_lab::evidence::{Accounted, JobActivation, JobIssued, ShareHeader};
use spc_lab::ids::{FrontId, IdentityId, JobId, ShareId};
use spc_lab::oracle::{check_credit, double_sha256, OracleInput, OracleShare, OracleStatus};
use spc_lab::wide::{ReferenceProfile, Target256, WideU128};

fn reference() -> ReferenceProfile {
    ReferenceProfile::parse_hex(
        "0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    )
    .unwrap()
}

fn job(reference: &ReferenceProfile, m: u64, share_target: String) -> JobIssued {
    JobIssued {
        front_id: FrontId::new("front-a").unwrap(),
        channel_id: 1,
        job_id: JobId::new("job-1").unwrap(),
        generation: 7,
        nbits: 0x207f_ffff,
        share_target,
        m_issue: m,
        work_unit_version: 2,
        work_unit_fingerprint: reference.fingerprint(),
        issued_at: 1,
        activation: JobActivation::Active,
    }
}

fn header_for(target: Target256, meeting: bool) -> ShareHeader {
    for nonce in 0..200_000u32 {
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
        if target.hash_meets(&digest) == meeting {
            return header;
        }
    }
    panic!("could not find a header with meeting={meeting}");
}

fn setup(
    m: u64,
) -> (
    ReferenceProfile,
    JobIssued,
    IdentityId,
    OracleShare,
    Accounted,
) {
    let reference = reference();
    let target = reference.share_target(m.trailing_zeros()).unwrap();
    let job = job(&reference, m, target.to_hex());
    let identity = IdentityId::new("id-alice").unwrap();
    let share = OracleShare {
        identity_id: identity.clone(),
        share_id: ShareId::new("share-1").unwrap(),
        job_id: job.job_id.clone(),
        generation: job.generation,
        d: m,
        header: header_for(target, true),
    };
    let accounted = Accounted {
        front_id: FrontId::new("front-a").unwrap(),
        identity_id: identity.clone(),
        share_id: share.share_id.clone(),
        job_id: job.job_id.clone(),
        generation: job.generation,
        nbits: job.nbits,
        share_target: job.share_target.clone(),
        d: WideU128(m as u128),
        slice_id: "slice-1".to_string(),
        diff_sum: WideU128(m as u128),
    };
    (reference, job, identity, share, accounted)
}

#[test]
fn oracle_recomputes_d_from_job_issued_and_catches_wrong_generation() {
    let (reference, job, _identity, share, accounted) = setup(4);
    let input = OracleInput {
        jobs: vec![job.clone()],
        shares: vec![share.clone()],
        accounted: vec![accounted],
        reference: Some(reference.clone()),
        shares_complete: true,
    };
    assert_eq!(check_credit(&input).status, OracleStatus::Verified);

    let mut wrong_generation = input.clone();
    wrong_generation.shares[0].generation = 8;
    let report = check_credit(&wrong_generation);
    assert!(
        matches!(report.status, OracleStatus::Mismatch(_)),
        "{report:?}"
    );
    assert!(report
        .problems
        .iter()
        .any(|problem| problem.contains("generation")));

    let mut wrong_d = input.clone();
    wrong_d.shares[0].d = 2;
    assert!(matches!(
        check_credit(&wrong_d).status,
        OracleStatus::Mismatch(_)
    ));

    let mut wrong_job_target = input;
    wrong_job_target.jobs[0].share_target = reference.share_target(3).unwrap().to_hex();
    assert!(matches!(
        check_credit(&wrong_job_target).status,
        OracleStatus::Mismatch(_)
    ));
}

#[test]
fn oracle_missing_job_is_unknown_not_guessed_credit() {
    let (reference, _job, _identity, share, accounted) = setup(2);
    let input = OracleInput {
        jobs: Vec::new(),
        shares: vec![share],
        accounted: vec![accounted],
        reference: Some(reference),
        shares_complete: true,
    };
    let report = check_credit(&input);
    assert!(
        matches!(report.status, OracleStatus::Unknown(_)),
        "missing job must be unknown: {report:?}"
    );
    assert!(report
        .identities
        .iter()
        .all(|identity| identity.expected_units.get() == 0));
}

#[test]
fn oracle_rejects_a_hash_below_the_job_target() {
    let (reference, job, identity, _share, _accounted) = setup(4);
    let target = Target256::from_hex(&job.share_target).unwrap();
    let below = OracleShare {
        identity_id: identity,
        share_id: ShareId::new("share-below").unwrap(),
        job_id: job.job_id.clone(),
        generation: job.generation,
        d: 4,
        header: header_for(target, false),
    };
    let input = OracleInput {
        jobs: vec![job],
        shares: vec![below],
        accounted: Vec::new(),
        reference: Some(reference),
        shares_complete: true,
    };
    let report = check_credit(&input);
    assert!(matches!(report.status, OracleStatus::Mismatch(_)));
    assert!(report
        .problems
        .iter()
        .any(|problem| problem.contains("does not meet")));
}

#[test]
fn oracle_is_independent_of_the_accounting_crate() {
    // The oracle input types are codecs/types from spc-lab only. This test
    // names the boundary explicitly so a future import of spc-accounting into
    // this path is caught by review.
    let (reference, job, _identity, share, _accounted) = setup(2);
    let input = OracleInput {
        jobs: vec![job],
        shares: vec![share],
        accounted: Vec::new(),
        reference: Some(reference),
        shares_complete: false,
    };
    let report = check_credit(&input);
    assert_eq!(report.status, OracleStatus::Verified);
}

#[test]
fn oracle_handles_two_shares_and_slice_aggregate_credit() {
    let reference = reference();
    let target = reference.share_target(2).unwrap();
    let job = job(&reference, 4, target.to_hex());
    let identity = IdentityId::new("id-alice").unwrap();
    let first = OracleShare {
        identity_id: identity.clone(),
        share_id: ShareId::new("share-1").unwrap(),
        job_id: job.job_id.clone(),
        generation: job.generation,
        d: 4,
        header: header_for(target, true),
    };
    let mut second_header = header_for(target, true);
    second_header.nonce = second_header.nonce.wrapping_add(1);
    // Search for a second distinct meeting header; the target is easy enough
    // that this terminates immediately in practice.
    for _ in 0..100_000 {
        let digest = spc_lab::oracle::double_sha256(&second_header.serialize().unwrap());
        if target.hash_meets(&digest) {
            break;
        }
        second_header.nonce = second_header.nonce.wrapping_add(1);
    }
    let second = OracleShare {
        identity_id: identity.clone(),
        share_id: ShareId::new("share-2").unwrap(),
        job_id: job.job_id.clone(),
        generation: job.generation,
        d: 4,
        header: second_header,
    };
    let aggregate = Accounted {
        front_id: FrontId::new("front-a").unwrap(),
        identity_id: identity.clone(),
        share_id: ShareId::new("share-1").unwrap(),
        job_id: job.job_id.clone(),
        generation: job.generation,
        nbits: job.nbits,
        share_target: job.share_target.clone(),
        d: WideU128(4),
        slice_id: "slice-1".to_string(),
        diff_sum: WideU128(8),
    };
    let input = OracleInput {
        jobs: vec![job.clone()],
        shares: vec![first.clone(), second.clone()],
        accounted: vec![aggregate.clone()],
        reference: Some(reference.clone()),
        shares_complete: true,
    };
    assert_eq!(check_credit(&input).status, OracleStatus::Verified);
    let per_share = OracleInput {
        jobs: vec![job.clone()],
        shares: vec![first, second],
        accounted: vec![
            Accounted {
                diff_sum: WideU128(4),
                ..aggregate.clone()
            },
            Accounted {
                diff_sum: WideU128(4),
                share_id: ShareId::new("share-2").unwrap(),
                ..aggregate
            },
        ],
        reference: Some(reference),
        shares_complete: true,
    };
    assert_eq!(check_credit(&per_share).status, OracleStatus::Verified);
}
