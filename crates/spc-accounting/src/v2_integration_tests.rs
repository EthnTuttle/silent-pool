use super::*;

fn share(id: u8, d: u64, fees: u64, idx: u32) -> ShareRec {
    ShareRec {
        sp_id: [id; 32],
        d,
        job_fees: fees,
        job_id: 1,
        nonce: idx,
        ntime: 1_700_000_000,
        version: 0x2000_0000,
        extranonce: vec![idx as u8; 8],
        share_index: idx,
    }
}

const FP: [u8; 32] = [7; 32];

fn v2_slice(id: u64) -> Slice {
    Slice::new_v2(id, 800_000, [1; 32], 0, id, 17, FP)
}

fn v2_agg(id: u64, work: u128) -> SliceAgg {
    SliceAgg {
        slice_id: id,
        height: 800_000,
        prev_hash: [1; 32],
        front_id: 0,
        difficulty: work,
        diff_fee_sum: work,
        per_id: BTreeMap::from([([1; 32], (work, work))]),
        root: [0; 32],
        work_unit_version: WorkUnitVersion::V2,
        work_unit_fingerprint: Some(FP),
    }
}

#[test]
fn v2_slice_credit_binds_job_generation_and_checked_aggregates() {
    let mut slice = v2_slice(1);
    slice.push_v2(share(1, 2, 17, 0)).unwrap();
    slice.push_v2(share(1, 4, 23, 1)).unwrap();
    slice.seal();
    let agg = slice.agg_v2().unwrap();
    assert_eq!(agg.difficulty, 6);
    assert_eq!(agg.per_id.get(&[1; 32]), Some(&(6, 126)));
    assert_eq!(agg.work_unit_version, WorkUnitVersion::V2);
    assert_eq!(agg.work_unit_fingerprint, Some(FP));

    let mut other = v2_slice(2);
    other.push_v2(share(1, 1, 0, 2)).unwrap();
    assert_eq!(
        other.difficulty, 1,
        "a later job does not reprice earlier work"
    );
}

#[test]
fn v2_slice_and_window_overflow_are_checked_without_mutation() {
    let mut slice = v2_slice(1);
    slice.difficulty = u128::MAX;
    assert_eq!(
        slice.push_v2(share(1, 1, 0, 0)),
        Err(WorkUnitError::SliceWorkOverflow)
    );
    assert!(slice.shares.is_empty());
    assert_eq!(slice.difficulty, u128::MAX);

    let mut fees = v2_slice(2);
    fees.push_v2(share(1, u64::MAX, u64::MAX, 0)).unwrap();
    fees.push_v2(share(1, u64::MAX, u64::MAX, 1)).unwrap();
    assert_eq!(fees.agg_v2(), Err(WorkUnitError::SliceFeeWorkOverflow));

    let aggs = vec![v2_agg(1, u128::MAX - 1), v2_agg(2, 2)];
    assert_eq!(
        select_window_v2(&aggs, 99, u128::MAX, BTreeMap::new(), Some(FP)),
        Err(WorkUnitError::WindowWorkOverflow)
    );
}

#[test]
fn v2_window_derives_underfill_overshoot_and_rejects_mixed_profiles() {
    let aggs = vec![v2_agg(3, 10), v2_agg(2, 10), v2_agg(1, 10)];
    let w = select_window_v2(&aggs, 99, 25, BTreeMap::new(), Some(FP)).unwrap();
    assert_eq!(w.d_win, 30);
    assert_eq!(w.t, 3);
    assert!(w.slices.iter().any(|s| s.slice_id == 1));
    assert!(!w.underfilled(25));
    assert_eq!(w.target_overshoot(25), 5);

    let w = select_window_v2(&aggs, 99, 50, BTreeMap::new(), Some(FP)).unwrap();
    assert!(w.underfilled(50));
    assert_eq!(w.underfill(50), 20);

    let mut mixed = aggs;
    mixed[1].work_unit_version = WorkUnitVersion::V1;
    mixed[1].work_unit_fingerprint = None;
    assert_eq!(
        select_window_v2(&mixed, 99, 25, BTreeMap::new(), Some(FP)),
        Err(WorkUnitError::WorkUnitRestoreMismatch { slice_id: 2 })
    );
}

#[test]
fn v2_payout_products_match_legacy_on_normal_windows_and_fail_wide() {
    let mut one = v2_slice(1);
    one.push_v2(share(1, 30, 10_000, 0)).unwrap();
    let mut two = v2_slice(2);
    two.push_v2(share(2, 70, 10_000, 1)).unwrap();
    one.seal();
    two.seal();
    let aggs = vec![two.agg_v2().unwrap(), one.agg_v2().unwrap()];
    let window = select_window_v2(&aggs, 99, 100, BTreeMap::new(), Some(FP)).unwrap();
    let raw = raw_payouts_v2(&window, 100_000, 10_000, 0).unwrap();
    let legacy = raw_payouts(&window, 100_000, 10_000, 0).unwrap();
    assert_eq!(raw, legacy);

    let window = Window {
        slices: vec![v2_agg(1, u128::MAX), v2_agg(2, u128::MAX)],
        t: 2,
        d_win: u128::MAX,
        high_water: BTreeMap::new(),
    };
    assert_eq!(
        raw_payouts_v2(&window, u64::MAX, u64::MAX, 100),
        Err(WorkUnitError::PayoutProductOverflow)
    );
}

#[test]
fn v2_jd_products_are_checked() {
    assert_eq!(claim_jd_v2(400, 1_000, 100, 1_000).unwrap(), 360);
    assert_eq!(
        claim_jd_v2(1, 100, 101, 100),
        Err(WorkUnitError::PayoutProductOverflow)
    );
}

#[test]
fn v2_raw_payout_names_the_offending_slice_and_empty_window() {
    let mut offender = v2_agg(2, 1);
    offender.work_unit_version = WorkUnitVersion::V1;
    offender.work_unit_fingerprint = None;
    let window = Window {
        slices: vec![v2_agg(1, 1), offender],
        t: 2,
        d_win: 2,
        high_water: BTreeMap::new(),
    };
    assert_eq!(
        raw_payouts_v2(&window, 100, 0, 0),
        Err(WorkUnitError::WorkUnitRestoreMismatch { slice_id: 2 })
    );
    let empty = Window {
        slices: vec![],
        t: 0,
        d_win: 0,
        high_water: BTreeMap::new(),
    };
    assert_eq!(
        raw_payouts_v2(&empty, 100, 0, 0),
        Err(WorkUnitError::PayoutWindowEmpty)
    );
}
