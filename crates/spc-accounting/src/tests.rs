use super::*;
use proptest::prelude::*;

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

fn cfg(budget: u32, slots: usize) -> TierConfig {
    TierConfig {
        fee_bps: 100,
        dust_limit_sats: 330,
        coinbase_threshold_sats: 330,
        max_payout_slots: slots,
        budget_wu: budget,
        overhead_wu: 0,
    }
}

fn empty_raw() -> RawPayouts {
    RawPayouts {
        raw: BTreeMap::new(),
        pool_sats: 0,
        r_net: 0,
        f_net: 0,
    }
}

/// Apply a routing's ledger deltas to a prior pending map, the way the front
/// (`advance_pending`) and settlement do: a balance that reaches zero is
/// removed, never stored negative.
fn apply(routing: &Routing, pending: &BTreeMap<SpId, u64>) -> BTreeMap<SpId, u64> {
    let mut out = pending.clone();
    for (id, delta) in &routing.pending_deltas {
        let next = out.get(id).copied().unwrap_or(0) as i128 + *delta as i128;
        if next > 0 {
            out.insert(*id, u64::try_from(next).unwrap_or(u64::MAX));
        } else {
            out.remove(id);
        }
    }
    out
}

fn assert_net_pool(r: &Routing, raw: &RawPayouts, prior: &BTreeMap<SpId, u64>) {
    let paid = r.kept.iter().map(|(_, n)| u128::from(*n)).sum::<u128>();
    assert_eq!(u128::from(r.pay_p) + paid, u128::from(r.total));
    assert_eq!(
        u128::from(raw.pool_sats) + raw.raw.values().map(|n| u128::from(*n)).sum::<u128>(),
        u128::from(r.total)
    );
    assert_eq!(r.pool_fee, raw.pool_sats);
    let net = i128::from(raw.pool_sats)
        + r.ark_due.iter().map(|(_, n)| i128::from(*n)).sum::<i128>()
        + r.forfeited
            .iter()
            .map(|(_, n)| i128::from(*n))
            .sum::<i128>()
        + r.pending_deltas
            .values()
            .map(|d| i128::from(*d))
            .sum::<i128>();
    assert_eq!(i128::from(r.pay_p), net);
    for id in raw
        .raw
        .keys()
        .chain(prior.keys())
        .copied()
        .collect::<BTreeSet<_>>()
    {
        let p = prior.get(&id).copied().unwrap_or(0);
        let e = i128::from(p) + i128::from(r.pending_deltas.get(&id).copied().unwrap_or(0));
        assert!(e >= 0);
        let value = |rows: &[(SpId, u64)]| {
            rows.iter()
                .filter(|(key, _)| *key == id)
                .map(|(_, n)| i128::from(*n))
                .sum::<i128>()
        };
        assert_eq!(
            value(&r.kept) + value(&r.ark_due) + value(&r.forfeited) + e,
            i128::from(raw.raw.get(&id).copied().unwrap_or(0)) + i128::from(p)
        );
    }
}

#[test]
fn signed_pending_repayment_reduces_net_pool_utxo_but_not_gross_jd_weight() {
    let raw = RawPayouts {
        raw: BTreeMap::from([([1; 32], 4_950_000_000)]),
        pool_sats: 50_000_000,
        r_net: 4_950_000_000,
        f_net: 0,
    };
    let prior = BTreeMap::from([([1; 32], 1_000_000)]);
    let tier = cfg(1_000_000, 2048);
    let pool = route_tiers(&raw, &prior, &BTreeMap::new(), &tier, 5_000_000_000).unwrap();
    assert_eq!(pool.kept, vec![([1; 32], 4_951_000_000)]);
    assert_eq!(pool.pay_p, 49_000_000);
    assert_eq!(pool.pending_deltas[&[1; 32]], -1_000_000);
    assert_net_pool(&pool, &raw, &prior);
    let jd = route_jd_tiers(&raw, &prior, &BTreeMap::new(), &tier, 5_000_000_000).unwrap();
    let weights = build_weights(&jd, raw.pool_sats);
    assert_eq!(weights.pool_weight, 50_000_000);
    assert_eq!(weights.weights, vec![([1; 32], 4_951_000_000)]);
    let total = weights.pool_weight + weights.weights.iter().map(|(_, w)| *w).sum::<u64>();
    assert_eq!(total, 5_001_000_000);
    assert_eq!(
        claim_jd(4_951_000_000, 5_000_000_000, 1_000_000, total).unwrap(),
        4_949_019_996
    );
    assert_eq!(
        claim_jd_v2(4_951_000_000, 5_000_000_000, 1_000_000, total).unwrap(),
        4_949_019_996
    );
    let excessive = BTreeMap::from([([1; 32], 51_000_000)]);
    assert_eq!(
        route_tiers(&raw, &excessive, &BTreeMap::new(), &tier, 5_000_000_000),
        Err(Error::EntitlementExceedsRevenue)
    );
}

#[test]
fn native_r4_failed_candidate_reconstruction_fits_and_reserves_500001() {
    // Exact failed-state inputs from the retained r4 trace/SQL, not a claim
    // that this candidate was accepted by a node before the correction.
    let fingerprint = WorkUnitProfile::lab_default().fingerprint();
    let mut slice = Slice::new_v2(1, 12, [0; 32], 0, 1, 0, fingerprint);
    let mut index = 0;
    for (id, count) in [(1, 23), (2, 22), (3, 22)] {
        for _ in 0..count {
            slice.push_v2(share(id, 1, 0, index)).unwrap();
            index += 1;
        }
    }
    slice.seal();
    let window = select_window_v2(
        &[slice.agg_v2().unwrap()],
        u64::MAX,
        65,
        BTreeMap::new(),
        Some(fingerprint),
    )
    .unwrap();
    assert_eq!(window.d_win, 67);
    let raw = raw_payouts_v2(&window, 5_000_000_000, 0, 100).unwrap();
    assert_eq!(
        raw.raw,
        BTreeMap::from([
            ([1; 32], 1_699_253_731),
            ([2; 32], 1_625_373_134),
            ([3; 32], 1_625_373_134)
        ])
    );
    assert_eq!(raw.pool_sats, 50_000_001);
    let prior = BTreeMap::from([([1; 32], 386_759), ([2; 32], 566_201), ([3; 32], 547_037)]);
    let r = route_tiers(
        &raw,
        &prior,
        &BTreeMap::new(),
        &cfg(1_000_000, 2048),
        5_000_000_000,
    )
    .unwrap();
    assert_eq!(
        r.kept,
        vec![
            ([1; 32], 1_699_500_000),
            ([2; 32], 1_625_500_000),
            ([3; 32], 1_625_500_000)
        ]
    );
    assert_eq!(r.pay_p, 49_500_000);
    assert_eq!(
        apply(&r, &prior),
        BTreeMap::from([([1; 32], 140_490), ([2; 32], 439_335), ([3; 32], 420_171)])
    );
    assert_eq!(
        r.pending_deltas,
        BTreeMap::from([
            ([1; 32], -246_269),
            ([2; 32], -126_866),
            ([3; 32], -126_866)
        ])
    );
    let consumed = r
        .pending_deltas
        .iter()
        .map(|(id, delta)| {
            economic::Consumption::from_delta(*id, prior[id], *delta)
                .unwrap()
                .consumed
        })
        .sum::<u64>();
    assert_eq!(consumed, 500_001);
    assert_net_pool(&r, &raw, &prior);
}

#[test]
fn empty_raw_can_repay_known_prior_while_total_entitlement_above_t_remains_held() {
    let raw = RawPayouts {
        raw: BTreeMap::new(),
        pool_sats: 1_000_000,
        r_net: 1_000_000,
        f_net: 0,
    };
    let prior = BTreeMap::from([([1; 32], 2_000_000), ([2; 32], 600_123)]);
    let r = route_tiers_with_keys(
        &raw,
        &prior,
        &BTreeMap::new(),
        &cfg(1_000_000, 2048),
        1_000_000,
        Some(&BTreeSet::from([[2; 32]])),
    )
    .unwrap();
    assert_eq!(r.owed.values().sum::<u64>(), 2_600_123);
    assert_eq!(r.kept, vec![([2; 32], 500_000)]);
    assert_eq!(r.pending, vec![([1; 32], 2_000_000)]);
    assert_eq!(r.pay_p, 500_000);
    assert_eq!(
        apply(&r, &prior),
        BTreeMap::from([([1; 32], 2_000_000), ([2; 32], 100_123)])
    );
    assert_net_pool(&r, &raw, &prior);
}

fn worked_window() -> (Window, u64, u64) {
    let mut s1 = Slice::new(1, 800_000, [1; 32], 0, 1, 15_000_000);
    s1.push(share(1, 60, 15_000_000, 0)); // Alice
    s1.push(share(2, 40, 15_000_000, 1)); // Bob
    s1.seal();
    let mut s2 = Slice::new(2, 800_000, [1; 32], 0, 2, 18_000_000);
    s2.push(share(1, 50, 18_000_000, 2));
    s2.push(share(2, 30, 18_000_000, 3));
    s2.push(share(3, 20, 19_000_000, 4)); // Carol
    s2.seal();

    let window = Window {
        slices: vec![s2.agg(), s1.agg()],
        t: 2,
        d_win: 200,
        high_water: BTreeMap::new(),
    };
    (window, 312_500_000, 20_000_000)
}

#[test]
fn spec_4_5_worked_example_matches_to_the_sat() {
    let (window, r, f) = worked_window();
    let raw = raw_payouts(&window, r, f, 100).unwrap();
    assert_eq!(raw.raw.get(&[1; 32]), Some(&180_991_854));
    assert_eq!(raw.raw.get(&[2; 32]), Some(&115_178_612));
    assert_eq!(raw.raw.get(&[3; 32]), Some(&33_004_532));
    assert_eq!(raw.pool_sats, 3_325_002);
    assert_eq!(raw.r_net, 309_375_000);
    assert_eq!(raw.f_net, 19_800_000);
}

/// SPEC §4.4 floors the subsidy term **once per identity across the whole
/// window**: `floor(r_net * sum_s d_s / D_win)`. The pre-fix code floored per
/// slice (`floor(r_net * d_s / D_win)` inside the slice loop), which loses every
/// slice's remainder: this window yields 10, not 3 + 3 + 3 = 9.
#[test]
fn subsidy_term_floors_once_per_identity_across_the_window() {
    let mut slices = Vec::new();
    for i in 1..=3u64 {
        let mut s = Slice::new(i, 800_000, [1; 32], 0, 1, 0);
        s.push(share(1, 1, 0, (i - 1) as u32));
        s.seal();
        slices.push(s.agg());
    }
    let window = Window {
        slices,
        t: 3,
        d_win: 3,
        high_water: BTreeMap::new(),
    };
    // r_net = 10, one identity with d = 3 over the window, D_win = 3.
    let raw = raw_payouts(&window, 10, 0, 0).unwrap();
    assert_eq!(raw.raw.get(&[1; 32]), Some(&10));
    assert_eq!(raw.pool_sats, 0);
}

/// The fee term stays per slice with its own floor. Two slices, `F_net = 3`,
/// `t = 2`: `floor(3/2) + floor(3/2) = 2`, not the merged `floor(6/4) = 1`.
#[test]
fn fee_term_is_floored_per_slice() {
    let mut s1 = Slice::new(1, 800_000, [1; 32], 0, 1, 1);
    s1.push(share(1, 1, 1, 0));
    s1.seal();
    let mut s2 = Slice::new(2, 800_000, [1; 32], 0, 1, 1);
    s2.push(share(1, 1, 1, 1));
    s2.seal();
    let window = Window {
        slices: vec![s2.agg(), s1.agg()],
        t: 2,
        d_win: 2,
        high_water: BTreeMap::new(),
    };
    let raw = raw_payouts(&window, 0, 3, 0).unwrap();
    assert_eq!(raw.raw.get(&[1; 32]), Some(&2));
    assert_eq!(raw.pool_sats, 1);
}

/// `DF_S = 0` (a window whose slices declared no fees) cannot attribute the
/// found block's fees. The spec formula divides by `DF_S`, so the term is zero
/// and the sats stay in the pool output rather than panicking.
#[test]
fn zero_slice_fees_leave_the_fee_term_to_the_pool() {
    let mut s = Slice::new(1, 800_000, [1; 32], 0, 1, 0);
    s.push(share(1, 100, 0, 0));
    s.seal();
    let window = Window {
        slices: vec![s.agg()],
        t: 1,
        d_win: 100,
        high_water: BTreeMap::new(),
    };
    let raw = raw_payouts(&window, 1_000, 500, 0).unwrap();
    assert_eq!(raw.raw.get(&[1; 32]), Some(&1_000));
    assert_eq!(raw.pool_sats, 500);
}

#[test]
fn pool_template_collapse_equals_plain_pplns() {
    // One slice, constant fees: the fee term is a difficulty-weighted split.
    let mut s = Slice::new(1, 1, [1; 32], 0, 1, 10_000);
    s.push(share(1, 30, 10_000, 0));
    s.push(share(2, 70, 10_000, 1));
    s.seal();
    let window = Window {
        slices: vec![s.agg()],
        t: 1,
        d_win: 100,
        high_water: BTreeMap::new(),
    };
    let raw = raw_payouts(&window, 100_000, 10_000, 0).unwrap();
    // delta = 0 => plain difficulty-weighted split of the whole reward.
    let total = 110_000u128;
    let a = (total * 30 / 100) as u64;
    let b = (total * 70 / 100) as u64;
    assert_eq!(raw.raw.get(&[1; 32]), Some(&a));
    assert_eq!(raw.raw.get(&[2; 32]), Some(&b));
}

#[test]
fn empty_window_and_bad_fee_bps_are_rejected() {
    let window = Window {
        slices: Vec::new(),
        t: 0,
        d_win: 0,
        high_water: BTreeMap::new(),
    };
    assert_eq!(raw_payouts(&window, 1, 0, 0), Err(Error::EmptyWindow));
    let (window, r, f) = worked_window();
    assert_eq!(raw_payouts(&window, r, f, 10_001), Err(Error::FeeBps));
}

// ---------------------------------------------------------------------------
// Independent plain-PPLNS oracle (SPEC §8.2, §4.4 pool-template collapse)
// ---------------------------------------------------------------------------

/// A window description: per slice, `(job_fees, [(sp_id, d)])`. Pool-template
/// mode is modelled by one constant `job_fees` per slice (`delta = 0`).
#[derive(Clone, Debug)]
struct WindowSpec {
    slices: Vec<(u64, Vec<(u8, u64)>)>,
}

fn build_window(spec: &WindowSpec) -> Window {
    let mut slices = Vec::new();
    let mut d_win = 0u128;
    let mut idx = 0u32;
    for (i, (fees, entries)) in spec.slices.iter().enumerate() {
        let mut slice = Slice::new(i as u64 + 1, 800_000, [1; 32], 0, 1, *fees);
        for (id, d) in entries {
            slice.push(share(*id, *d, *fees, idx));
            idx = idx.saturating_add(1);
        }
        slice.seal();
        d_win = d_win.saturating_add(slice.difficulty);
        slices.push(slice.agg());
    }
    Window {
        t: slices.len(),
        slices,
        d_win,
        high_water: BTreeMap::new(),
    }
}

/// Independent implementation of SPEC §4.4 for a pool-template window, written
/// from the spec text without sharing `raw_payouts`'s code or helpers: it
/// groups per identity outside-in, recomputes `D_win` from the slices, and
/// applies exactly the spec's two floor positions (once per identity for the
/// subsidy, once per slice for the fee).
fn plain_pplns_oracle(
    window: &Window,
    subsidy: u64,
    fees: u64,
    fee_bps: u16,
) -> BTreeMap<SpId, u64> {
    let net = |v: u64| (v as u128) - (v as u128) * (fee_bps as u128) / 10_000;
    let r_net = net(subsidy);
    let f_net = net(fees);
    let d_win: u128 = window.slices.iter().map(|s| s.difficulty).sum();
    let t = window.slices.len() as u128;

    let mut ids: BTreeSet<SpId> = BTreeSet::new();
    for s in &window.slices {
        ids.extend(s.per_id.keys().copied());
    }

    let mut out = BTreeMap::new();
    for id in ids {
        let d_total: u128 = window
            .slices
            .iter()
            .filter_map(|s| s.per_id.get(&id).map(|(d, _)| *d))
            .sum();
        // One floor over the whole window.
        let subsidy_term = r_net * d_total / d_win;
        // One floor per (identity, slice); with delta = 0 the constant fee
        // cancels out of `d*f / DF_S`, so the score is `d / D_S`.
        let mut fee_term = 0u128;
        for s in &window.slices {
            if let Some((d, _)) = s.per_id.get(&id) {
                fee_term += f_net * d / (t * s.difficulty);
            }
        }
        out.insert(id, u64::try_from(subsidy_term + fee_term).unwrap());
    }
    out
}

// ---------------------------------------------------------------------------
// Tier routing
// ---------------------------------------------------------------------------

#[test]
fn tiers_are_disjoint_and_conservation_holds() {
    let raw = RawPayouts {
        raw: BTreeMap::from([
            ([1; 32], 1_000_000),
            ([2; 32], 200),     // below C, registered
            ([3; 32], 150),     // below C, unregistered -> forfeit
            ([4; 32], 900_000), // one base rung plus error-held remainder
        ]),
        pool_sats: 50,
        r_net: 2_000,
        f_net: 0,
    };
    let pending = BTreeMap::from([([1; 32], 10)]);
    let ark = BTreeMap::from([
        (
            [2; 32],
            ArkCapability {
                registered: true,
                registration_committed: true,
                dest_known: true,
            },
        ),
        (
            [3; 32],
            ArkCapability {
                registered: false,
                registration_committed: false,
                dest_known: false,
            },
        ),
    ]);
    let r = route_tiers(&raw, &pending, &ark, &cfg(344, 2), 2_300_000).unwrap();
    assert_eq!(r.kept, vec![([1; 32], 1_000_000), ([4; 32], 500_000)]);
    assert!(r.pending.is_empty());
    assert_eq!(r.pending_deltas[&[4; 32]], 400_000);
    assert_eq!(r.ark_due, vec![([2; 32], 200)]);
    assert_eq!(r.forfeited, vec![([3; 32], 150)]);
    assert_eq!(r.pay_p, 800_000);
    // Rails are disjoint.
    let ids: Vec<SpId> = r
        .kept
        .iter()
        .chain(r.pending.iter())
        .chain(r.ark_due.iter())
        .chain(r.forfeited.iter())
        .map(|(id, _)| *id)
        .collect();
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len());
}

/// SPEC §4.6: `owed(id) = raw(id) + pending(id)` for every id that has either.
/// A prior pending balance with no shares in the current window MUST still be
/// routed; the pre-fix code only looked at `raw.raw` and stranded it.
#[test]
fn pending_only_identity_is_routed_not_stranded() {
    let raw = RawPayouts {
        raw: BTreeMap::from([([1; 32], 1_000_000)]),
        pool_sats: 0,
        r_net: 1_000,
        f_net: 0,
    };
    let pending = BTreeMap::from([
        ([2; 32], 5_000_000), // no shares this window, still coinbase-payable
        ([3; 32], 100),       // registered, below C
        ([4; 32], 900),       // prior error-held sub-base liability is retained
    ]);
    let ark = BTreeMap::from([(
        [3; 32],
        ArkCapability {
            registered: true,
            registration_committed: true,
            dest_known: true,
        },
    )]);
    let r = route_tiers(&raw, &pending, &ark, &cfg(172 * 2, 2), 6_000_100).unwrap();

    assert_eq!(r.kept, vec![([1; 32], 1_000_000), ([2; 32], 5_000_000)]);
    assert_eq!(r.pending, vec![([4; 32], 900)]);
    assert_eq!(r.ark_due, vec![([3; 32], 100)]);
    assert!(r.forfeited.is_empty());
    assert_eq!(r.pay_p, 100);

    // Ledger deltas: a kept id clears its prior balance (the coinbase pays it),
    // a pending id carries the whole owed forward, and an Ark id moves its
    // balance out of pending into `ark_due`.
    assert_eq!(r.pending_deltas.get(&[2; 32]), Some(&-5_000_000));
    assert_eq!(r.pending_deltas.get(&[1; 32]), Some(&0));
    assert_eq!(r.pending_deltas.get(&[3; 32]), Some(&-100));
    assert_eq!(r.pending_deltas.get(&[4; 32]), Some(&0));
}

/// A pending-only identity below `C` with no Ark registration keeps its
/// balance. `pending` is owed by the pool (trimming, failed sends, an
/// unpropagated registration) and SPEC §11.1 says it is never forfeited.
#[test]
fn pending_only_unregistered_below_threshold_is_retained() {
    let raw = empty_raw();
    let pending = BTreeMap::from([([7; 32], 100)]);
    let r = route_tiers(&raw, &pending, &BTreeMap::new(), &cfg(1_000, 10), 0).unwrap();
    assert!(r.forfeited.is_empty());
    assert_eq!(r.pending, vec![([7; 32], 100)]);
    assert_eq!(r.pending_deltas.get(&[7; 32]), Some(&0));
    assert!(r.kept.is_empty() && r.ark_due.is_empty());
    assert_eq!(r.pay_p, 0);
    assert_eq!(apply(&r, &pending).get(&[7; 32]).copied(), Some(100));
}

/// Mixed case: only this block's 40 sats are forfeited; the prior 60 stays
/// owed. The distribution carries one entry per `sp_id`, so the retained
/// balance rides `pending_deltas` instead of a second `pending` entry.
#[test]
fn forfeit_takes_only_the_current_block_raw_portion() {
    let raw = RawPayouts {
        raw: BTreeMap::from([([1; 32], 40)]),
        pool_sats: 0,
        r_net: 40,
        f_net: 0,
    };
    let pending = BTreeMap::from([([1; 32], 60)]);
    let r = route_tiers(&raw, &pending, &BTreeMap::new(), &cfg(10_000, 10), 40).unwrap();
    assert_eq!(r.forfeited, vec![([1; 32], 40)], "raw only, never 100");
    assert!(r.pending.is_empty() && r.kept.is_empty() && r.ark_due.is_empty());
    assert_eq!(r.pending_deltas.get(&[1; 32]), Some(&0));
    assert_eq!(apply(&r, &pending).get(&[1; 32]).copied(), Some(60));
    assert_eq!(r.pay_p, 40);
}

/// Registered cases keep the §11.1 behaviour: the whole owed is routed, and
/// the prior balance moves out of the pending ledger.
#[test]
fn registered_below_threshold_cases_are_unchanged() {
    let raw = RawPayouts {
        raw: BTreeMap::from([([1; 32], 40), ([2; 32], 40)]),
        pool_sats: 0,
        r_net: 80,
        f_net: 0,
    };
    let pending = BTreeMap::from([([1; 32], 60), ([2; 32], 60), ([3; 32], 70)]);
    let ark = BTreeMap::from([
        (
            [1; 32],
            ArkCapability {
                registered: true,
                registration_committed: true,
                dest_known: true,
            },
        ),
        (
            [2; 32],
            ArkCapability {
                registered: true,
                registration_committed: true,
                dest_known: false,
            },
        ),
        (
            [3; 32],
            ArkCapability {
                registered: true,
                registration_committed: true,
                dest_known: true,
            },
        ),
    ]);
    let r = route_tiers(&raw, &pending, &ark, &cfg(10_000, 10), 80).unwrap();
    // Known destination: the whole owed becomes an Ark send due.
    assert_eq!(r.ark_due, vec![([1; 32], 100), ([3; 32], 70)]);
    assert_eq!(r.pending_deltas.get(&[1; 32]), Some(&-60));
    assert_eq!(r.pending_deltas.get(&[3; 32]), Some(&-70));
    // Committed registration without a propagated destination: pending.
    assert_eq!(r.pending, vec![([2; 32], 100)]);
    assert_eq!(r.pending_deltas.get(&[2; 32]), Some(&40));
    assert!(r.forfeited.is_empty() && r.kept.is_empty());
    let after = apply(&r, &pending);
    assert_eq!(after.get(&[1; 32]).copied(), None);
    assert_eq!(after.get(&[2; 32]).copied(), Some(100));
    assert_eq!(after.get(&[3; 32]).copied(), None);
}

/// SPEC §4.6(a): actual full rung payments above T fail, without partial pay.
#[test]
fn coinbase_rejects_over_revenue_full_entitlement_without_partial_payment() {
    let raw = RawPayouts {
        raw: BTreeMap::from([([1; 32], 1_000_000), ([2; 32], 1_000_000)]),
        pool_sats: 0,
        r_net: 200,
        f_net: 0,
    };
    let pending = BTreeMap::from([([1; 32], 1_500_000)]);
    let mut tier = cfg(10_000, 8);
    tier.dust_limit_sats = 1;
    tier.coinbase_threshold_sats = 1;
    assert_eq!(
        route_tiers(&raw, &pending, &BTreeMap::new(), &tier, 2_000_000),
        Err(Error::EntitlementExceedsRevenue)
    );
    assert_eq!(pending[&[1; 32]], 1_500_000); // pure failure did not debit anything.
}

/// An over-T full set fails deterministically instead of choosing a prefix.
#[test]
fn coinbase_over_revenue_failure_is_deterministic_without_prefix_trimming() {
    let raw = RawPayouts {
        raw: BTreeMap::from([
            ([1; 32], 1_500_000),
            ([2; 32], 1_000_000),
            ([3; 32], 500_000),
        ]),
        pool_sats: 0,
        r_net: 250,
        f_net: 0,
    };
    let mut tier = cfg(10_000, 8);
    tier.dust_limit_sats = 1;
    tier.coinbase_threshold_sats = 1;
    for _ in 0..2 {
        assert_eq!(
            route_tiers(&raw, &BTreeMap::new(), &BTreeMap::new(), &tier, 2_000_000),
            Err(Error::EntitlementExceedsRevenue)
        );
    }
}

/// A partial coinbase below `C` would be dust-unsafe, so nothing is paid.
#[test]
fn cap_never_emits_a_sub_threshold_output() {
    let raw = RawPayouts {
        raw: BTreeMap::from([([1; 32], 100)]),
        pool_sats: 0,
        r_net: 100,
        f_net: 0,
    };
    let pending = BTreeMap::from([([1; 32], 500)]);
    // T = 200 < C = 330: the whole balance waits, pay_p keeps the block.
    let r = route_tiers(&raw, &pending, &BTreeMap::new(), &cfg(10_000, 8), 200).unwrap();
    assert!(r.kept.is_empty());
    assert_eq!(r.pay_p, 200);
    assert!(r.pending.is_empty());
    assert_eq!(r.forfeited, vec![([1; 32], 100)]);
    assert_eq!(apply(&r, &pending).get(&[1; 32]).copied(), Some(500));
}

/// Repaying old P can consume the whole current coinbase. The gross fee was
/// already deducted from raw; the signed liability reduction closes net pay_P.
#[test]
fn pending_repayment_does_not_impose_a_second_gross_pool_floor() {
    let raw = RawPayouts {
        raw: BTreeMap::from([([1; 32], 990_000)]),
        pool_sats: 10_000,
        r_net: 990_000,
        f_net: 0,
    };
    let pending = BTreeMap::from([([1; 32], 500_000)]);
    let mut tier = cfg(10_000, 8);
    tier.dust_limit_sats = 1;
    tier.coinbase_threshold_sats = 1;
    let r = route_tiers(&raw, &pending, &BTreeMap::new(), &tier, 1_000_000).unwrap();
    assert_eq!(r.kept, vec![([1; 32], 1_000_000)]);
    assert_eq!(r.pay_p, 0);
    assert_eq!(r.pool_fee, 10_000);
    assert_eq!(r.pending_deltas[&[1; 32]], -10_000);
    assert_eq!(
        i128::from(r.pay_p),
        i128::from(raw.pool_sats) + i128::from(r.pending_deltas[&[1; 32]])
    );
    assert_eq!(apply(&r, &pending)[&[1; 32]], 490_000);
    let mut smaller = raw.clone();
    smaller.pool_sats = 9_999;
    assert_eq!(
        route_tiers(&smaller, &pending, &BTreeMap::new(), &tier, 999_999),
        Err(Error::EntitlementExceedsRevenue)
    );
}

/// No new raw and sub-base prior: the existing error rail preserves all P.
#[test]
fn empty_raw_preserves_sub_base_prior_pending() {
    let raw = RawPayouts {
        raw: BTreeMap::new(),
        pool_sats: 500,
        r_net: 500,
        f_net: 0,
    };
    let pending = BTreeMap::from([([1; 32], 400)]);
    let r = route_tiers(&raw, &pending, &BTreeMap::new(), &cfg(10_000, 8), 500).unwrap();
    assert!(r.kept.is_empty());
    assert_eq!(r.pay_p, 500);
    assert_eq!(r.pool_fee, raw.pool_sats);
    assert_eq!(r.pending_deltas[&[1; 32]], 0);
    assert_eq!(r.pending, vec![([1; 32], 400)]);
    assert_eq!(apply(&r, &pending).get(&[1; 32]).copied(), Some(400));
}

/// A zero balance is on no rail at all (no 0-sat entries).
#[test]
fn zero_balances_are_dropped() {
    let raw = RawPayouts {
        raw: BTreeMap::from([([1; 32], 0)]),
        pool_sats: 0,
        r_net: 0,
        f_net: 0,
    };
    let pending = BTreeMap::from([([2; 32], 0)]);
    let r = route_tiers(&raw, &pending, &BTreeMap::new(), &cfg(1_000, 10), 0).unwrap();
    assert!(r.kept.is_empty() && r.pending.is_empty() && r.ark_due.is_empty());
    assert!(r.forfeited.is_empty());
}

#[test]
fn raising_the_threshold_does_not_change_owed() {
    let raw = RawPayouts {
        raw: BTreeMap::from([([1; 32], 1_000)]),
        pool_sats: 0,
        r_net: 1_000,
        f_net: 0,
    };
    let ark = BTreeMap::from([(
        [1; 32],
        ArkCapability {
            registered: true,
            registration_committed: true,
            dest_known: true,
        },
    )]);
    let low = route_tiers(&raw, &BTreeMap::new(), &ark, &cfg(10_000, 10), 1_000).unwrap();
    let mut high_cfg = cfg(10_000, 10);
    high_cfg.coinbase_threshold_sats = 2_000;
    let high = route_tiers(&raw, &BTreeMap::new(), &ark, &high_cfg, 1_000).unwrap();
    assert!(low.kept.is_empty() && high.kept.is_empty());
    assert_eq!(low.owed, high.owed);
    assert_eq!(low.ark_due, high.ark_due);
    assert_eq!(high.ark_due.iter().map(|(_, v)| v).sum::<u64>(), 1_000);
}

#[test]
fn capacity_rejects_instead_of_trimming() {
    let raw = RawPayouts {
        raw: (1..=50u8).map(|i| ([i; 32], 1_000_000u64)).collect(),
        pool_sats: 0,
        r_net: 50_000,
        f_net: 0,
    };
    let r = route_tiers(
        &raw,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &cfg(172 * 10, 100),
        50_000_000,
    )
    .unwrap_err();
    assert_eq!(r, Error::PayoutWeight);
    let r = route_tiers(
        &raw,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &cfg(172 * 100, 7),
        50_000_000,
    )
    .unwrap_err();
    assert_eq!(
        r,
        Error::PayoutSlots {
            required: 50,
            max: 7
        }
    );
}

#[test]
fn autoscaler_debounces_and_clamps() {
    let mut a = Autoscaler::new(100_000, 50_000, 200_000);
    assert_eq!(a.on_template(90_000, 0), None);
    assert_eq!(a.on_template(90_000, 1), None);
    // Cooldown still binds at t=2 even with the third hit.
    assert_eq!(a.on_template(90_000, 2), None);
    let next = a.on_template(90_000, 301).unwrap();
    assert_eq!(next, 115_000);
    // Down steps take 20 templates.
    for i in 0..19 {
        assert_eq!(a.on_template(10_000, 700 + i), None);
    }
    let down = a.on_template(10_000, 999).unwrap();
    assert_eq!(down, 100_000);
}

// ---------------------------------------------------------------------------
// JD claims and weights (O11, SPEC §4.7)
// ---------------------------------------------------------------------------

#[test]
fn claim_formula_is_revenue_scaled() {
    // T_actual != T: the JDC paid at its own revenue, the pool books the rest.
    let claim = claim_jd(1_000, 900, 0, 1_000).unwrap();
    assert_eq!(claim, 900);
    let (claim, pending) = settle_jd(1_000, 800, 900, 0, 1_000).unwrap();
    assert_eq!(claim, 900);
    assert_eq!(pending, 100);
    // T_actual == T reproduces pool-template mode.
    assert_eq!(
        settle_jd(1_000, 1_000, 1_000, 0, 1_000).unwrap(),
        (1_000, 0)
    );
}

/// `W` is the whole distribution weight (`Σ weight_id + weight_P`) and `F` is
/// the fixed-output total, not the pool weight.
#[test]
fn claim_jd_scales_by_total_weight_and_subtracts_fixed_outputs() {
    // A two-way split: the miner's weight 400, weight_P 600, W = 1_000.
    assert_eq!(claim_jd(400, 1_000, 0, 1_000).unwrap(), 400);
    // T_actual = 800: the claim is revenue-scaled to 320.
    assert_eq!(claim_jd(400, 800, 0, 1_000).unwrap(), 320);
    // Leaving weight_P out of W would over-credit the miner to 800.
    assert_eq!(claim_jd(400, 800, 0, 400).unwrap(), 800);
    // F is subtracted from T_actual before scaling.
    assert_eq!(claim_jd(400, 1_000, 100, 1_000).unwrap(), 360);
    // Claim is monotone in T_actual (the JDC may also beat the pool's
    // template revenue, in which case claim exceeds owed by the same factor).
    let mut last = 0u64;
    for t_actual in [0u64, 1, 100, 500, 700, 1_000, 10_000] {
        let claim = claim_jd(700, t_actual, 0, 1_000).unwrap();
        assert!(claim >= last, "claim must not shrink with revenue");
        last = claim;
    }
    assert_eq!(claim_jd(700, 1_000, 0, 1_000).unwrap(), 700);
    assert_eq!(claim_jd(700, 10_000, 0, 1_000).unwrap(), 7_000);
    // Error paths: zero total weight, F > T_actual.
    assert_eq!(claim_jd(1, 1, 0, 0), Err(Error::Overflow));
    assert_eq!(claim_jd(1, 100, 101, 100), Err(Error::Overflow));
}

/// Settlement reads the mined coinbase: dust-pruned, underpaid, exact and
/// over-paid ids all keep the ledger unsigned.
#[test]
fn settle_jd_handles_dust_pruned_underpaid_and_overpaid_ids() {
    // Dust-pruned by the JDC: nothing on chain, the whole claim is pending.
    assert_eq!(settle_jd(1_000, 0, 900, 0, 1_000).unwrap(), (900, 900));
    // Underpaid: the shortfall is pending.
    assert_eq!(settle_jd(1_000, 700, 900, 0, 1_000).unwrap(), (900, 200));
    // Exact.
    assert_eq!(settle_jd(1_000, 900, 900, 0, 1_000).unwrap(), (900, 0));
    // paid > claim: zero pending, never a debit.
    assert_eq!(settle_jd(1_000, 950, 900, 0, 1_000).unwrap(), (900, 0));
    // pending'(id) is bounded by claim(id) for every paid value.
    for paid in [0u64, 1, 899, 900, 901, 5_000] {
        let (claim, pending) = settle_jd(1_000, paid, 900, 0, 1_000).unwrap();
        assert!(pending <= claim);
    }
}

#[test]
fn build_weights_matches_the_routing() {
    let raw = RawPayouts {
        raw: BTreeMap::from([([1; 32], 1_000), ([4; 32], 900)]),
        pool_sats: 50,
        r_net: 2_000,
        f_net: 0,
    };
    let pending = BTreeMap::from([([2; 32], 200), ([9; 32], 50)]);
    let ark = BTreeMap::from([(
        [2; 32],
        ArkCapability {
            registered: true,
            registration_committed: true,
            dest_known: true,
        },
    )]);
    let routing = route_jd_tiers(&raw, &pending, &ark, &cfg(172, 1), 2_500).unwrap();
    // kept: id 1. Ark: id 2. Trimmed: id 4. Pending-only: id 9 (never forfeited).
    assert_eq!(routing.kept, vec![([1; 32], 1_000)]);
    let weights = build_weights(&routing, 60);
    assert_eq!(weights.weights, vec![([1; 32], 1_000)]);
    assert_eq!(weights.pool_weight, 60 + 200 + 900 + 50);
    // Conservation: Σ weights + weight_P = fee weight + Σ owed.
    let not_kept: u64 = routing
        .pending
        .iter()
        .chain(routing.ark_due.iter())
        .chain(routing.forfeited.iter())
        .map(|(_, v)| *v)
        .sum();
    assert_eq!(
        weights.weights.iter().map(|(_, v)| *v).sum::<u64>() + weights.pool_weight,
        60 + 1_000 + not_kept
    );
    // Entries mirror the rails: Coinbase, Pending (sorted by sp_id), Ark.
    let rails: Vec<Rail> = weights.entries.iter().map(|e| e.rail).collect();
    assert_eq!(
        rails,
        vec![Rail::Coinbase, Rail::Pending, Rail::Pending, Rail::Ark]
    );
    // Weights are the routing's owed for the kept ids, sorted by sp_id.
    assert_eq!(weights.weights, sorted_by_id(&routing.kept));
}

/// JD and pool-template mode share everything but the final scaling: a
/// pool-template distribution must yield the same `owed` as the weights.
#[test]
fn jd_distribution_and_weights_agree_on_exact_kept_ids() {
    let raw = RawPayouts {
        raw: BTreeMap::from([([1; 32], 1_000), ([2; 32], 900), ([3; 32], 100)]),
        pool_sats: 10,
        r_net: 2_000,
        f_net: 0,
    };
    let pending = BTreeMap::from([([2; 32], 100)]);
    let routing =
        route_jd_tiers(&raw, &pending, &BTreeMap::new(), &cfg(172 * 2, 2), 2_100).unwrap();
    assert_eq!(routing.kept, vec![([1; 32], 1_000), ([2; 32], 1_000)]);
    let weights = build_weights(&routing, 25);
    assert_eq!(weights.weights, routing.kept);
    // id 3 (100) is below C and unregistered -> forfeited -> weight_P.
    assert_eq!(weights.pool_weight, 25 + 100);
}

/// JD weights are not capped by the pool-template `T`: a partially paid id
/// keeps its full `owed` weight and O11 scales it at settlement.
#[test]
fn jd_weights_use_full_owed_not_the_capped_coinbase() {
    let raw = RawPayouts {
        raw: BTreeMap::from([([1; 32], 100), ([2; 32], 100)]),
        pool_sats: 0,
        r_net: 200,
        f_net: 0,
    };
    let pending = BTreeMap::from([([1; 32], 150)]);
    let mut tier = cfg(10_000, 8);
    tier.dust_limit_sats = 1;
    tier.coinbase_threshold_sats = 1;
    let routing = route_jd_tiers(&raw, &pending, &BTreeMap::new(), &tier, 200).unwrap();

    // The pool-template coinbase can only pay A 200 of its 250 owed.
    assert_eq!(routing.kept, vec![([1; 32], 200)]);
    assert_eq!(routing.owed.get(&[1; 32]), Some(&250));
    assert_eq!(routing.owed.get(&[2; 32]), Some(&100));

    // The JD weight is the full 250; B's 100 is withheld weight, plus the fee.
    let weights = build_weights(&routing, 5);
    assert_eq!(weights.weights, vec![([1; 32], 250)]);
    assert_eq!(weights.pool_weight, 5 + 100);
    let w: u64 = weights.weights.iter().map(|(_, v)| *v).sum::<u64>() + weights.pool_weight;
    assert_eq!(w, 355);
    // At the JD template revenue the claim is the full weight.
    assert_eq!(claim_jd(weights.weights[0].1, w, 0, w).unwrap(), 250);
}

// ---------------------------------------------------------------------------
// Distribution builder
// ---------------------------------------------------------------------------

fn key_pair(scan_byte: u8, spend_byte: u8) -> (MinerKeys, SpId) {
    let secp = bitcoin::secp256k1::Secp256k1::new();
    let b_scan_sk = bitcoin::secp256k1::SecretKey::from_slice(&[scan_byte; 32]).unwrap();
    let b_spend_sk = bitcoin::secp256k1::SecretKey::from_slice(&[spend_byte; 32]).unwrap();
    let b_scan = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &b_scan_sk);
    let b_spend = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &b_spend_sk);
    let id = spc_crypto::sp_id(&b_scan, &b_spend);
    (
        MinerKeys {
            sp_id: id,
            b_scan,
            b_spend,
        },
        id,
    )
}

#[test]
fn distribution_builds_p2tr_outputs_and_entries() {
    let (keys, id) = key_pair(0x11, 0x12);
    let send = SendKey::generate(800_000).unwrap();
    let routing = Routing {
        owed: BTreeMap::from([(id, 1_000_000)]),
        kept: vec![(id, 1_000_000)],
        pending: vec![],
        ark_due: vec![],
        forfeited: vec![],
        pending_deltas: BTreeMap::new(),
        pay_p: 100,
        total: 1_000_100,
        pool_fee: 100,
    };
    let miners = BTreeMap::from([(id, keys)]);
    let d = build_distribution(
        800_000,
        &send,
        &routing,
        &miners,
        &p2tr_script(&[9; 32]),
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(d.outputs.len(), 2);
    assert_eq!(d.outputs[0].1, 100);
    assert_eq!(d.outputs[1].0.len(), 34);
    assert_eq!(d.outputs[1].0[0], 0x51);
    assert_eq!(d.outputs[1].0[1], 0x20);
    assert_eq!(d.entries.len(), 1);
    assert_eq!(d.entries[0].rail, Rail::Coinbase);
    assert!(!d.entries[0].p_x.is_empty());
    assert_eq!(
        d.fingerprint,
        super::fingerprint(800_000, send.x_only(), &d.outputs)
    );
}

/// SPEC §4.7: `outputs = [pool] ++ [P2TR(P_id, pay(id)) sorted by sp_id]`.
/// The trim order (owed descending) must not leak into the coinbase order.
#[test]
fn distribution_outputs_are_globally_script_sorted_with_logical_identity_rows() {
    let (k1, id1) = key_pair(0x11, 0x12);
    let (k2, id2) = key_pair(0x21, 0x22);
    let (k3, id3) = key_pair(0x31, 0x32);
    let miners = BTreeMap::from([(id1, k1), (id2, k2), (id3, k3)]);
    let send = SendKey::generate(800_000).unwrap();
    let mut ids = [id1, id2, id3];
    ids.sort();
    // Trim order: descending owed, so the sp_id order is not the payout order.
    let routing = Routing {
        owed: BTreeMap::from([
            (ids[2], 3_000_030),
            (ids[1], 1_000_040),
            (ids[0], 500_050),
            ([8; 32], 50),
            ([7; 32], 40),
            ([6; 32], 30),
        ]),
        kept: vec![(ids[2], 3_000_000), (ids[1], 1_000_000), (ids[0], 500_000)],
        pending: vec![([8; 32], 50)],
        ark_due: vec![([7; 32], 40)],
        forfeited: vec![([6; 32], 30)],
        pending_deltas: BTreeMap::new(),
        pay_p: 7,
        total: 4_500_007,
        pool_fee: 7,
    };
    let d = build_distribution(
        800_000,
        &send,
        &routing,
        &miners,
        &p2tr_script(&[9; 32]),
        &BTreeMap::new(),
    )
    .unwrap();
    // Pool output first, then exactly one output per kept id.
    assert_eq!(d.outputs.len(), 4);
    assert_eq!(d.pay_p, 7);
    for (i, id) in ids.iter().enumerate() {
        let entry = d.entries.iter().find(|e| e.sp_id == *id).unwrap();
        assert_eq!(entry.rail, Rail::Coinbase);
        assert_eq!(entry.sp_id, *id);
        let p_x = *entry.p_x.first().expect("coinbase entry carries P_id");
        // The output at index i + 1 belongs to ids[i].
        let expected = {
            let keys = &miners[id];
            let ecdh = send.derive_pool_side(&keys.b_scan).unwrap();
            let tweaks = tier_tweaks(&ecdh, 0).unwrap();
            spc_output_pubkey(&keys.b_spend, &tweaks.t_onchain)
                .unwrap()
                .x_only_public_key()
                .0
                .serialize()
        };
        assert_eq!(p_x, expected);
        assert!(d.outputs.contains(&(p2tr_script(&expected), entry.sats)));
        let _ = i;
    }
    // Non-kept rails ride along in entries only, sorted by sp_id, no outputs.
    assert!(d.outputs[1..].windows(2).all(|w| w[0].0 < w[1].0));
    let rest: Vec<(Rail, SpId)> = d
        .entries
        .iter()
        .filter(|e| e.rail != Rail::Coinbase)
        .map(|e| (e.rail, e.sp_id))
        .collect();
    assert_eq!(
        rest,
        vec![
            (Rail::Forfeit, [6; 32]),
            (Rail::Ark, [7; 32]),
            (Rail::Pending, [8; 32]),
        ]
    );
}

#[test]
fn seal_triggers() {
    let s = Slice::new(1, 1, [1; 32], 0, 1, 100);
    assert_eq!(s.seal_if_needed(&share(1, 1, 100, 0), 50, 10), Seal::Keep);
    assert_eq!(s.seal_if_needed(&share(1, 1, 100, 0), 0, 10), Seal::Keep);
    assert_eq!(
        s.seal_if_needed(&share(1, 1, 150, 0), 50, 10),
        Seal::Seal(SealReason::FeeBand)
    );
    let mut full = Slice::new(1, 1, [1; 32], 0, 1, 0);
    for i in 0..10 {
        full.push(share(1, 1, 0, i));
    }
    assert_eq!(
        full.seal_if_needed(&share(1, 1, 0, 10), 0, 10),
        Seal::Seal(SealReason::MaxShares)
    );
}

#[test]
fn merkle_root_is_deterministic_and_empty_is_zero() {
    assert_eq!(merkle_root(&[]), [0u8; 32]);
    let l: Vec<[u8; 32]> = (0..5).map(|i| [i; 32]).collect();
    assert_eq!(merkle_root(&l), merkle_root(&l));
}

#[test]
fn select_window_skips_the_finding_slice_and_crosses_the_target() {
    let aggs: Vec<SliceAgg> = (0..5u64)
        .map(|i| SliceAgg {
            slice_id: i,
            height: 1,
            prev_hash: [0u8; 32],
            front_id: 0,
            difficulty: 100,
            diff_fee_sum: 0,
            per_id: BTreeMap::new(),
            root: [0u8; 32],
            work_unit_version: WorkUnitVersion::V1,
            work_unit_fingerprint: None,
        })
        .collect();
    // Whole slices newest-first until the target is crossed.
    let w = select_window(&aggs, 2, 250, BTreeMap::new());
    assert_eq!(w.t, 3);
    assert_eq!(w.d_win, 300);
    assert!(!w.slices.iter().any(|s| s.slice_id == 2));
    // A target above the whole chain still excludes only the finding slice.
    let w = select_window(&aggs, 4, u128::MAX, BTreeMap::new());
    assert_eq!(w.t, 4);
    assert!(!w.slices.iter().any(|s| s.slice_id == 4));
}

// ---------------------------------------------------------------------------
// Property tests (SPEC §8.2)
// ---------------------------------------------------------------------------

/// Random pool-template window: 1-6 slices, 1-8 identities, one constant
/// `job_fees` per slice (`delta = 0`).
fn arb_window_spec() -> impl Strategy<Value = WindowSpec> {
    (1usize..=6, 1usize..=8)
        .prop_flat_map(|(n_slices, n_ids)| {
            let slice = (
                1u64..=10_000_000u64, // f_s: constant within the slice
                proptest::collection::vec((0u8..=1u8, 1u64..=10_000u64), n_ids),
            );
            proptest::collection::vec(slice, n_slices).prop_map(|slices| WindowSpec {
                slices: slices
                    .into_iter()
                    .map(|(fees, entries)| {
                        let entries = entries
                            .into_iter()
                            .enumerate()
                            .filter(|(_, (present, _))| *present == 1)
                            .map(|(i, (_, d))| ((i + 1) as u8, d))
                            .collect();
                        (fees, entries)
                    })
                    .collect(),
            })
        })
        .prop_filter("a window needs at least one share", |w| {
            w.slices.iter().any(|(_, entries)| !entries.is_empty())
        })
}

fn owed_of(raw: &RawPayouts, pending: &BTreeMap<SpId, u64>, id: SpId) -> u64 {
    raw.raw
        .get(&id)
        .copied()
        .unwrap_or(0)
        .saturating_add(pending.get(&id).copied().unwrap_or(0))
}

fn rails(routing: &Routing) -> Vec<(&[(SpId, u64)], Rail)> {
    vec![
        (&routing.kept, Rail::Coinbase),
        (&routing.pending, Rail::Pending),
        (&routing.ark_due, Rail::Ark),
        (&routing.forfeited, Rail::Forfeit),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// SPEC §4.4 pool-template collapse: with `delta = 0` the result equals an
    /// independent plain-PPLNS oracle to the sat, and within the spec's per-term
    /// floors it is ordinary difficulty-weighted PPLNS.
    #[test]
    fn pool_template_collapse_matches_independent_pplns_oracle(
        spec in arb_window_spec(),
        subsidy in 1u64..=100_000_000_000,
        fees in 0u64..=1_000_000_000,
        fee_bps in 0u16..=10_000,
    ) {
        let window = build_window(&spec);
        let raw = raw_payouts(&window, subsidy, fees, fee_bps).unwrap();
        let oracle = plain_pplns_oracle(&window, subsidy, fees, fee_bps);
        prop_assert_eq!(&raw.raw, &oracle);

        // Floors only ever return sats to the pool output, and the pool output
        // is exactly the remainder of the block reward.
        let net = |v: u64| (v as u128) - (v as u128) * (fee_bps as u128) / 10_000;
        let combined = net(subsidy) + net(fees);
        let distributed = raw
            .raw
            .values()
            .fold(0u128, |acc, v| acc + *v as u128);
        prop_assert!(distributed <= combined);
        prop_assert_eq!(
            raw.pool_sats as u128,
            subsidy as u128 + fees as u128 - distributed
        );

        // A single-slice window is ordinary difficulty-weighted PPLNS of the
        // whole reward, to within the one sat its two terms can floor away.
        if window.slices.len() == 1 {
            for (id, v) in &raw.raw {
                let d: u128 = window.slices[0]
                    .per_id
                    .get(id)
                    .map(|(d, _)| *d)
                    .unwrap_or(0);
                let pure = combined * d / window.d_win;
                prop_assert!(*v as u128 <= pure, "{} > plain PPLNS {}", v, pure);
                prop_assert!(*v as u128 + 1 >= pure, "{} floors >1 sat below {}", v, pure);
            }
        }
    }

    /// SPEC §4.6 (a)-(e), trim bounds, rail disjointness and ordering, run
    /// through both `raw_payouts` and `route_tiers`.
    #[test]
    fn routing_invariants_hold(
        spec in arb_window_spec(),
        subsidy in 1_000_000u64..=1_000_000_000,
        fees in 0u64..=10_000_000,
        fee_bps in 100u16..=500,
        budget_wu in 0u32..=50_000,
        overhead_wu in 0u32..=1_000,
        slots in 0usize..=256,
        threshold in 330u64..=5_000,
        pending_bits in proptest::collection::vec((0u8..=1u8, 0u64..=50u64), 8),
        ark_bits in proptest::collection::vec(0u8..=2u8, 8),
    ) {
        let window = build_window(&spec);
        let raw = raw_payouts(&window, subsidy, fees, fee_bps).unwrap();
        let total = subsidy + fees;

        let mut pending = BTreeMap::new();
        for (i, (present, amount)) in pending_bits.iter().enumerate() {
            if *present == 1 {
                pending.insert([(i + 1) as u8; 32], *amount);
            }
        }
        let mut ark = BTreeMap::new();
        for (i, code) in ark_bits.iter().enumerate() {
            let id = [(i + 1) as u8; 32];
            match code {
                1 => { ark.insert(id, ArkCapability { registered: true, registration_committed: true, dest_known: true }); }
                2 => { ark.insert(id, ArkCapability { registered: true, registration_committed: true, dest_known: false }); }
                _ => {}
            }
        }
        let tier = TierConfig {
            fee_bps: 100,
            dust_limit_sats: 330,
            coinbase_threshold_sats: threshold,
            max_payout_slots: slots,
            budget_wu,
            overhead_wu,
        };
        let all_ids:BTreeSet<_>=raw.raw.keys().chain(pending.keys()).copied().collect();
        let count:u64=all_ids.iter().map(|id|decomposition_len(owed_of(&raw,&pending,*id))).sum();
        let shaped:u64=all_ids.iter().map(|id|band_amount(owed_of(&raw,&pending,*id)).0).sum();
        let routing = match route_tiers(&raw, &pending, &ark, &tier, total) {
            Ok(r)=>r,
            Err(Error::PayoutSlots { required,max })=>{prop_assert_eq!(required,count);prop_assert_eq!(max,slots);prop_assert!(count>slots as u64);return Ok(());},
            Err(Error::PayoutWeight)=>{prop_assert!(u64::from(overhead_wu)+172*count+if count+2>=253 {8}else{0}>u64::from(budget_wu), "full output set exceeds weight");return Ok(());},
            Err(Error::EntitlementExceedsRevenue)=>{prop_assert!(shaped>total);return Ok(());},
            Err(e)=>return Err(TestCaseError::fail(format!("unexpected routing error {e}"))),
        };

        // (a) SPEC §4.6(a): the coinbase never exceeds the block reward.
        let paid: u64 = routing.kept.iter().map(|(_, v)| *v).sum();
        prop_assert!(paid <= total);
        prop_assert_eq!(paid + routing.pay_p, total);

        // (b) one output per (sp_id, H): `kept` is keyed by sp_id.
        let kept_ids: BTreeSet<SpId> = routing.kept.iter().map(|(id, _)| *id).collect();
        prop_assert_eq!(kept_ids.len(), routing.kept.len());

        // (e) an id is on exactly one rail, and every id with a non-zero owed
        // balance is routed exactly once.
        let mut seen: BTreeSet<SpId> = BTreeSet::new();
        for (group, rail) in rails(&routing) {
            for (id, _) in group {
                if rail==Rail::Ark && kept_ids.contains(id) { continue; }
                prop_assert!(seen.insert(*id), "id on two rails");
            }
        }
        let mut expected: BTreeSet<SpId> = BTreeSet::new();
        for id in raw.raw.keys().chain(pending.keys()) {
            if owed_of(&raw, &pending, *id) > 0 {
                expected.insert(*id);
            }
        }
        prop_assert_eq!(&seen, &expected);
        prop_assert_eq!(
            routing.owed.keys().copied().collect::<BTreeSet<SpId>>(),
            expected.clone(),
            "`owed` covers every routed id"
        );
        for (id, amount) in &routing.owed {
            prop_assert_eq!(*amount, owed_of(&raw, &pending, *id));
        }

        // Per-id partition: the rail amounts plus the new ledger balance equal
        // `owed` exactly, and each rail carries what its rule says. This is
        // what keeps an over-`T` balance from producing an invalid coinbase.
        let applied = apply(&routing, &pending);
        for id in &expected {
            let owed = owed_of(&raw, &pending, *id);
            let raw_part = raw.raw.get(id).copied().unwrap_or(0);
            let old = pending.get(id).copied().unwrap_or(0);
            let new_pending = applied.get(id).copied().unwrap_or(0);
            let kept_sats = routing.kept.iter().find(|(k, _)| k == id).map(|(_, v)| *v);
            let ark_sats = routing.ark_due.iter().find(|(k, _)| k == id).map(|(_, v)| *v);
            let forfeit_sats = routing.forfeited.iter().find(|(k, _)| k == id).map(|(_, v)| *v);
            let pending_sats = routing.pending.iter().find(|(k, _)| k == id).map(|(_, v)| *v);

            if let Some(p) = pending_sats {
                prop_assert_eq!(p, owed, "the pending rail carries the whole balance");
                prop_assert_eq!(new_pending, owed);
            }
            if let Some(f) = forfeit_sats {
                prop_assert_eq!(f, raw_part, "only this block's raw is forfeited");
                prop_assert!(owed < LADDER_BASE_SATS);
                prop_assert_eq!(new_pending, old, "prior pending is never forfeited");
            }
            if let Some(a) = ark_sats {
                prop_assert_eq!(a, if kept_sats.is_some() { owed%LADDER_BASE_SATS } else {owed});
                prop_assert_eq!(new_pending, 0);
            }
            if let Some(p) = kept_sats {
                prop_assert!(p >= threshold);
                prop_assert!(p <= owed);
                prop_assert_eq!(p,owed-owed%LADDER_BASE_SATS);
                prop_assert_eq!(new_pending, owed - p-ark_sats.unwrap_or(0));
            }
            prop_assert_eq!(
                kept_sats.unwrap_or(0)
                    + ark_sats.unwrap_or(0)
                    + forfeit_sats.unwrap_or(0)
                    + new_pending,
                owed,
                "rails + new pending must partition owed"
            );
            let delta = routing.pending_deltas.get(id).copied().unwrap_or(0);
            prop_assert_eq!(new_pending as i128, old as i128 + delta as i128);
        }

        // Successful pool routing contains every payable full denomination set.
        let mut order: Vec<SpId> = expected.iter().copied().collect();
        order.retain(|id|owed_of(&raw,&pending,*id)>=LADDER_BASE_SATS);
        let kept_order: Vec<SpId> = routing.kept.iter().map(|(id, _)| *id).collect();
        prop_assert_eq!(kept_order,order);

        // (c) Generalized ledger identity:
        //   Σ held' + Σ forfeited − Σ pending_old = Σ raw − Σ kept paid
        // where held' = pending' + ark_due'. Delta is signed when old P is paid.
        let held: i128 = applied.values().fold(0i128, |acc, v| acc + *v as i128)
            + routing
                .ark_due
                .iter()
                .fold(0i128, |acc, (_, v)| acc + *v as i128);
        let forfeited: i128 = routing.forfeited.iter().fold(0i128, |acc, (_, v)| acc + *v as i128);
        let pending_old: i128 = pending.values().fold(0i128, |acc, v| acc + *v as i128);
        let lhs = held + forfeited - pending_old;
        let rhs =
            raw.raw.values().fold(0i128, |acc, v| acc + *v as i128) - paid as i128;
        prop_assert_eq!(lhs, rhs);
        prop_assert_eq!(i128::from(routing.pay_p),i128::from(raw.pool_sats)+lhs,
            "net pool output includes signed pending correction, not a second fee reserve");
        prop_assert_eq!(routing.pool_fee,raw.pool_sats,"gross bookkeeping/JD weight source stays unchanged");

        // Ledger apply matches §4.6/§11.1 exactly, and re-running the routing is
        // deterministic (d).
        for (id, amount) in &routing.pending {
            prop_assert_eq!(applied.get(id).copied().unwrap_or(0), *amount);
        }
        for (id, amount) in &routing.kept {
            let owed = owed_of(&raw, &pending, *id);
            let remainder_ark=routing.ark_due.iter().find(|(k,_)|k==id).map_or(0,|(_,v)|*v);
            prop_assert_eq!(applied.get(id).copied().unwrap_or(0), owed - amount-remainder_ark);
        }
        for (id, _) in &routing.ark_due {
            prop_assert_eq!(applied.get(id).copied().unwrap_or(0), 0);
        }
        for (id, _) in &routing.forfeited {
            prop_assert_eq!(applied.get(id).copied().unwrap_or(0), pending.get(id).copied().unwrap_or(0));
        }
        prop_assert_eq!(
            route_tiers(&raw, &pending, &ark, &tier, total).unwrap(),
            routing.clone()
        );

        // Trim never exceeds the slot cap or the weight budget (§4.8).
        prop_assert!(count <= slots as u64);
        if !routing.kept.is_empty() {
            let used = overhead_wu as u64 + 172 * count;
            prop_assert!(used <= budget_wu as u64);
            prop_assert!(overhead_wu <= budget_wu);
        }

        // Deterministic order: owed descending, ties broken by sp_id ascending.
        for pair in routing.kept.windows(2) {
            prop_assert!(
                pair[0].0 < pair[1].0
            );
        }

        // JD weights come from the full owed, never from the capped coinbase
        // amounts (§4.7; O11 scales them at settlement).
        let fee_weight = 7u64;
        let jd=route_jd_tiers(&raw,&pending,&ark,&tier,total).unwrap();
        let weights = build_weights(&jd, fee_weight);
        let expected_weights: Vec<(SpId, u64)> = sorted_by_id(&jd.kept)
            .into_iter()
            .map(|(id, _)| (id, routing.owed[&id]))
            .collect();
        prop_assert_eq!(&weights.weights, &expected_weights);
        let sum_weights: u64 = weights.weights.iter().map(|(_, v)| *v).sum();
        let all_owed: u64 = routing.owed.values().fold(0u64, |acc, v| acc + *v);
        prop_assert_eq!(sum_weights + weights.pool_weight, fee_weight + all_owed);
        prop_assert_eq!(
            weights.pool_weight,
            fee_weight + all_owed - sum_weights,
            "weight_P is the fee plus every not-kept owed"
        );

        // O11 settlement keeps the ledger unsigned for every mined amount.
        let w = sum_weights + weights.pool_weight;
        for (id, owed) in &weights.weights {
            for paid_actual in [0u64, *owed / 2, *owed, owed + 1] {
                let (claim, pending_next) = settle_jd(*owed, paid_actual, total, 0, w).unwrap();
                prop_assert!(pending_next <= claim);
                let _ = id;
            }
        }
    }

    /// SPEC §4.6(a) under carried balances larger than the block reward: the
    /// pre-fix code could emit `Σ kept > T`, a consensus-invalid coinbase.
    /// Conservation, rail disjointness and the per-id partition must hold.
    #[test]
    fn cap_keeps_the_coinbase_within_the_block_reward(
        spec in arb_window_spec(),
        subsidy in 1_000_000u64..=1_000_000_000,
        fees in 0u64..=10_000_000,
        budget_wu in 172u32..=4_000,
        overhead_wu in 0u32..=1_000,
        slots in 1usize..=8,
        threshold in 1u64..=1_000,
        pending_bits in proptest::collection::vec((0u8..=1u8, 0u64..=100u64), 8),
        excess in 1u64..=3,
    ) {
        let window = build_window(&spec);
        let raw = raw_payouts(&window, subsidy, fees, 100).unwrap();
        let total = subsidy + fees;

        // Pending balances at or above the block revenue, so owed overflows T
        // for every identity that carries one.
        let mut pending = BTreeMap::new();
        for (i, (present, extra)) in pending_bits.iter().enumerate() {
            if *present == 1 {
                pending.insert(
                    [(i + 1) as u8; 32],
                    total.saturating_mul(excess).saturating_add(*extra),
                );
            }
        }
        let tier = TierConfig {
            fee_bps: 100,
            dust_limit_sats: 1,
            coinbase_threshold_sats: threshold,
            max_payout_slots: slots,
            budget_wu,
            overhead_wu,
        };
        let result=route_tiers(&raw, &pending, &BTreeMap::new(), &tier, total);
        let all_ids:BTreeSet<_>=raw.raw.keys().chain(pending.keys()).copied().collect();
        let count:u64=all_ids.iter().map(|id|decomposition_len(owed_of(&raw,&pending,*id))).sum();
        let shaped:u64=all_ids.iter().map(|id|band_amount(owed_of(&raw,&pending,*id)).0).sum();
        let routing=match result {
            Ok(r)=>r,
            Err(Error::PayoutSlots {required,max})=>{prop_assert_eq!(required,count);prop_assert_eq!(max,slots);prop_assert!(count>slots as u64);return Ok(());},
            Err(Error::PayoutWeight)=>{prop_assert!(u64::from(overhead_wu)+172*count+if count+2>=253 {8}else{0}>u64::from(budget_wu), "full output set exceeds weight");return Ok(());},
            Err(Error::EntitlementExceedsRevenue)=>{prop_assert!(shaped>total);return Ok(());},
            Err(e)=>return Err(TestCaseError::fail(format!("unexpected routing error {e}"))),
        };

        // Conservation caps actual rung payments at T; old pending is not
        // independently capped by a gross pool-fee/rounding floor.
        let paid: u64 = routing.kept.iter().map(|(_, v)| *v).sum();
        prop_assert!(paid <= total);
        prop_assert_eq!(paid + routing.pay_p, total);
        prop_assert_eq!(routing.pay_p, total - paid);
        let net=i128::from(raw.pool_sats)+routing.ark_due.iter().map(|(_,s)|i128::from(*s)).sum::<i128>()
            +routing.forfeited.iter().map(|(_,s)|i128::from(*s)).sum::<i128>()
            +routing.pending_deltas.values().map(|d|i128::from(*d)).sum::<i128>();
        prop_assert_eq!(i128::from(routing.pay_p),net);
        prop_assert_eq!(routing.pool_fee,raw.pool_sats);

        // One rail per id (settlement rejects duplicate `sp_id` entries).
        let mut seen: BTreeSet<SpId> = BTreeSet::new();
        for (group, _) in rails(&routing) {
            for (id, _) in group {
                prop_assert!(seen.insert(*id), "id on two rails");
            }
        }
        let mut expected: BTreeSet<SpId> = BTreeSet::new();
        for id in raw.raw.keys().chain(pending.keys()) {
            if owed_of(&raw, &pending, *id) > 0 {
                expected.insert(*id);
            }
        }
        prop_assert_eq!(&seen, &expected);
        prop_assert_eq!(
            routing.owed.keys().copied().collect::<BTreeSet<SpId>>(),
            expected.clone(),
            "`owed` covers every routed id"
        );

        // Every owed balance is partitioned by its full rails and error pending.
        let applied = apply(&routing, &pending);
        for id in &expected {
            let owed = owed_of(&raw, &pending, *id);
            let kept_sats = routing.kept.iter().find(|(k, _)| k == id).map(|(_, v)| *v);
            let ark_sats = routing.ark_due.iter().find(|(k, _)| k == id).map(|(_, v)| *v);
            let forfeit_sats = routing.forfeited.iter().find(|(k, _)| k == id).map(|(_, v)| *v);
            let new_pending = applied.get(id).copied().unwrap_or(0);
            if let Some(p) = kept_sats {
                prop_assert!(p >= threshold, "coinbase output below C");
                prop_assert!(p <= owed);
                prop_assert_eq!(new_pending, owed - p);
            }
            if let Some(a) = ark_sats {
                prop_assert_eq!(a, owed);
            }
            if let Some(f) = forfeit_sats {
                prop_assert_eq!(f, raw.raw.get(id).copied().unwrap_or(0));
            }
            prop_assert_eq!(
                kept_sats.unwrap_or(0)
                    + ark_sats.unwrap_or(0)
                    + forfeit_sats.unwrap_or(0)
                    + new_pending,
                owed,
                "rails + new pending must partition owed"
            );
        }
        prop_assert!(routing.kept.len() <= slots);
        if !routing.kept.is_empty() {
            let used = overhead_wu as u64 + 172 * routing.kept.len() as u64;
            prop_assert!(used <= budget_wu as u64);
        }
        prop_assert_eq!(
            route_tiers(&raw, &pending, &BTreeMap::new(), &tier, total).unwrap(),
            routing.clone()
        );
    }

    /// Raising `C` moves value from the coinbase to the Ark/forfeit rails and
    /// never changes `Σ owed` (§11.9); the kept set only shrinks.
    #[test]
    fn jd_raising_the_threshold_moves_value_but_not_owed(
        spec in arb_window_spec(),
        subsidy in 1_000_000u64..=1_000_000_000,
        fees in 0u64..=10_000_000,
        low_c in 330u64..=1_000,
        delta_c in 1u64..=1_000,
        budget_wu in 0u32..=4_000,
        slots in 0usize..=8,
    ) {
        let window = build_window(&spec);
        let raw = raw_payouts(&window, subsidy, fees, 100).unwrap();
        let total = subsidy + fees;
        let high_c = low_c.saturating_add(delta_c);
        let mut low_cfg = cfg(budget_wu, slots);
        low_cfg.coinbase_threshold_sats = low_c;
        let mut high_cfg = low_cfg.clone();
        high_cfg.coinbase_threshold_sats = high_c;
        let low = route_jd_tiers(&raw, &BTreeMap::new(), &BTreeMap::new(), &low_cfg, total).unwrap();
        let high = route_jd_tiers(&raw, &BTreeMap::new(), &BTreeMap::new(), &high_cfg, total).unwrap();

        let owed = |r: &Routing| -> u64 {
            r.kept.iter().chain(r.pending.iter()).chain(r.ark_due.iter()).chain(r.forfeited.iter())
                .map(|(_, v)| *v).sum()
        };
        prop_assert_eq!(owed(&low), owed(&high));
        let low_kept: BTreeSet<SpId> = low.kept.iter().map(|(id, _)| *id).collect();
        for (id, _) in &high.kept {
            prop_assert!(low_kept.contains(id));
        }
    }
}
