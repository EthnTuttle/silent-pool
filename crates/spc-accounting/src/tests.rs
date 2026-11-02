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

#[test]
fn tiers_are_disjoint_and_conservation_holds() {
    let raw = RawPayouts {
        raw: BTreeMap::from([
            ([1; 32], 1_000),
            ([2; 32], 200), // below C, registered
            ([3; 32], 150), // below C, unregistered -> forfeit
            ([4; 32], 900), // above C, trimmed by budget
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
                dest_known: true,
            },
        ),
        (
            [3; 32],
            ArkCapability {
                registered: false,
                dest_known: false,
            },
        ),
    ]);
    let r = route_tiers(&raw, &pending, &ark, &cfg(172, 1), 2_300).unwrap();
    // Only the largest (id 1) is kept; id 4 is trimmed, id 2 is Ark, id 3 forfeit.
    assert_eq!(r.kept, vec![([1; 32], 1_010)]);
    assert_eq!(r.pending, vec![([4; 32], 900)]);
    assert_eq!(r.ark_due, vec![([2; 32], 200)]);
    assert_eq!(r.forfeited, vec![([3; 32], 150)]);
    assert_eq!(r.pay_p, 2_300 - 1_010);
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
            dest_known: true,
        },
    )]);
    let low = route_tiers(&raw, &BTreeMap::new(), &ark, &cfg(10_000, 10), 1_000).unwrap();
    let mut high_cfg = cfg(10_000, 10);
    high_cfg.coinbase_threshold_sats = 2_000;
    let high = route_tiers(&raw, &BTreeMap::new(), &ark, &high_cfg, 1_000).unwrap();
    assert_eq!(low.kept.iter().map(|(_, v)| v).sum::<u64>(), 1_000);
    assert_eq!(high.ark_due.iter().map(|(_, v)| v).sum::<u64>(), 1_000);
}

#[test]
fn trim_never_exceeds_budget_or_slots() {
    let raw = RawPayouts {
        raw: (1..=50u8).map(|i| ([i; 32], 1_000u64)).collect(),
        pool_sats: 0,
        r_net: 50_000,
        f_net: 0,
    };
    let r = route_tiers(
        &raw,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &cfg(172 * 10, 100),
        50_000,
    )
    .unwrap();
    assert_eq!(r.kept.len(), 10);
    let r = route_tiers(
        &raw,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &cfg(172 * 100, 7),
        50_000,
    )
    .unwrap();
    assert_eq!(r.kept.len(), 7);
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

#[test]
fn distribution_builds_p2tr_outputs_and_entries() {
    let secp = bitcoin::secp256k1::Secp256k1::new();
    let b_scan_sk = bitcoin::secp256k1::SecretKey::from_slice(&[0x11; 32]).unwrap();
    let b_spend_sk = bitcoin::secp256k1::SecretKey::from_slice(&[0x12; 32]).unwrap();
    let b_scan = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &b_scan_sk);
    let b_spend = bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &b_spend_sk);
    let id = spc_crypto::sp_id(&b_scan, &b_spend);

    let send = SendKey::generate(800_000).unwrap();
    let routing = Routing {
        kept: vec![(id, 1_000)],
        pending: vec![],
        ark_due: vec![],
        forfeited: vec![],
        pending_deltas: BTreeMap::new(),
        pay_p: 100,
        total: 1_100,
        pool_fee: 100,
    };
    let miners = BTreeMap::from([(
        id,
        MinerKeys {
            sp_id: id,
            b_scan,
            b_spend,
        },
    )]);
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
    assert!(d.entries[0].p_x.is_some());
    assert_eq!(
        d.fingerprint,
        super::fingerprint(800_000, send.x_only(), &d.outputs)
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

proptest! {
    #[test]
    fn conservation_and_tier_disjointness(
        r in 1u64..100_000_000,
        f in 0u64..10_000_000,
        n in 1usize..20,
        budget in 172u32..10_000,
    ) {
        let mut raw = BTreeMap::new();
        for i in 0..n {
            raw.insert([i as u8; 32], (i as u64 + 1) * 1_000);
        }
        let raw = RawPayouts { raw, pool_sats: 0, r_net: r, f_net: f };
        let total = r + f;
        let c = TierConfig {
            fee_bps: 100,
            dust_limit_sats: 330,
            coinbase_threshold_sats: 330,
            max_payout_slots: 256,
            budget_wu: budget,
            overhead_wu: 0,
        };
        let routing = route_tiers(&raw, &BTreeMap::new(), &BTreeMap::new(), &c, total).unwrap();
        let paid: u64 = routing.kept.iter().map(|(_, v)| *v).sum();
        prop_assert_eq!(paid + routing.pay_p, total);
        let mut ids = Vec::new();
        for group in [&routing.kept, &routing.pending, &routing.ark_due, &routing.forfeited] {
            for (id, _) in group { ids.push(*id); }
        }
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        prop_assert_eq!(sorted.len(), ids.len());
    }
}
