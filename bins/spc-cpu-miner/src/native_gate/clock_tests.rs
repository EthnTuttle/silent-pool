use super::*;

fn sample(start: Instant, elapsed_ms: u64, wall_ms: u64) -> ClockSample {
    ClockSample {
        monotonic: start + Duration::from_millis(elapsed_ms),
        wall_ms,
    }
}

#[test]
fn rollback_between_verification_and_activation_cannot_extend_original_admission() {
    let start = Instant::now();
    let admission = start + Duration::from_secs(10);
    let verified = sample(start, 9_000, 109_000);
    let activated = sample(start, 9_001, 100_001);
    let deadline = activation_deadline(110_000, admission, verified, activated).unwrap();
    assert_eq!(deadline, admission);
    let permit = AuditPermit {
        proof: VerifiedActivation {
            request_digest: [1; 32],
            candidate_epoch: 1,
            job_id: 1,
            generation: 1,
            checkpoint_digest: [2; 32],
            consumed_digest: [3; 32],
            committed_digest: [4; 32],
            expires_at_unix_ms: 110_000,
            window: spc_scan::source_audit::WindowFacts::from_work(9, 9),
        },
        deadline,
        valid: AtomicBool::new(true),
        lease: RwLock::new(()),
    };
    assert!(permit.valid_at(sample(start, 9_999, 101_000)));
    assert!(!permit.valid_at(sample(start, 10_000, 101_001)));
    assert!(!permit.valid_at(sample(start, 18_000, 109_001)));
}

#[test]
fn completion_at_or_after_admission_is_expired_even_after_rollback() {
    let start = Instant::now();
    for elapsed in [10_000, 10_001] {
        assert!(activation_deadline(
            110_000,
            start + Duration::from_secs(10),
            sample(start, 9_000, 109_000),
            sample(start, elapsed, 100_000)
        )
        .is_err());
    }
}

#[test]
fn shorter_proof_deadline_survives_both_normal_handoff_and_rollback() {
    let start = Instant::now();
    let verified = sample(start, 9_000, 109_000);
    for wall in [109_250, 100_000] {
        let deadline = activation_deadline(
            109_500,
            start + Duration::from_secs(10),
            verified,
            sample(start, 9_250, wall),
        )
        .unwrap();
        assert_eq!(deadline, start + Duration::from_millis(9_500));
    }
    assert!(activation_deadline(
        109_500,
        start + Duration::from_secs(10),
        verified,
        sample(start, 9_500, 100_000)
    )
    .is_err());
}

#[test]
fn forward_wall_jump_expires_completion_without_touching_host_clock() {
    let start = Instant::now();
    assert!(activation_deadline(
        110_000,
        start + Duration::from_secs(10),
        sample(start, 1_000, 101_000),
        sample(start, 2_000, 110_000)
    )
    .is_err());
}

#[test]
fn observed_forward_expiry_cannot_be_resurrected_by_rollback() {
    let start = Instant::now();
    let permit = AuditPermit {
        proof: VerifiedActivation {
            request_digest: [1; 32],
            candidate_epoch: 1,
            job_id: 1,
            generation: 1,
            checkpoint_digest: [2; 32],
            consumed_digest: [3; 32],
            committed_digest: [4; 32],
            expires_at_unix_ms: 110_000,
            window: spc_scan::source_audit::WindowFacts::from_work(9, 9),
        },
        deadline: start + Duration::from_secs(10),
        valid: AtomicBool::new(true),
        lease: RwLock::new(()),
    };
    assert!(permit.valid_at(sample(start, 1_000, 101_000)));
    assert!(!permit.valid_at(sample(start, 2_000, 110_000)));
    assert!(!permit.valid_at(sample(start, 2_001, 100_000)));
}
