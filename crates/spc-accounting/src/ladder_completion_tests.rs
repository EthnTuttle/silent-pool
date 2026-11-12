use super::*;

fn config() -> TierConfig {
    TierConfig {
        fee_bps: 100,
        dust_limit_sats: 330,
        coinbase_threshold_sats: 330,
        max_payout_slots: 1024,
        budget_wu: 200_000,
        overhead_wu: 224,
    }
}

#[test]
fn worked_vector_groups_coinbase_rows_and_remainder_components() {
    let ids = [[1; 32], [2; 32], [3; 32]];
    let raw = RawPayouts {
        raw: BTreeMap::from([
            (ids[0], 180_991_854),
            (ids[1], 115_178_612),
            (ids[2], 33_004_532),
        ]),
        pool_sats: 3_325_002,
        r_net: 309_375_000,
        f_net: 19_800_000,
    };
    let caps = ids
        .map(|id| {
            (
                id,
                ArkCapability {
                    registered: true,
                    registration_committed: true,
                    dest_known: true,
                },
            )
        })
        .into();
    let routed = route_tiers(&raw, &BTreeMap::new(), &caps, &config(), 332_500_000).unwrap();
    assert_eq!(
        routed.kept,
        vec![
            (ids[0], 180_500_000),
            (ids[1], 115_000_000),
            (ids[2], 33_000_000)
        ]
    );
    assert_eq!(
        routed.ark_due,
        vec![(ids[0], 491_854), (ids[1], 178_612), (ids[2], 4_532)]
    );
    assert_eq!(routed.pay_p, 4_000_000);
    assert!(routed.pending.is_empty());
    let secp = bitcoin::secp256k1::Secp256k1::new();
    let miners: BTreeMap<_, _> = ids
        .into_iter()
        .enumerate()
        .map(|(i, id)| {
            let scan = bitcoin::secp256k1::SecretKey::from_slice(&[i as u8 + 1; 32]).unwrap();
            let spend = bitcoin::secp256k1::SecretKey::from_slice(&[i as u8 + 5; 32]).unwrap();
            (
                id,
                MinerKeys {
                    sp_id: id,
                    b_scan: bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &scan),
                    b_spend: bitcoin::secp256k1::PublicKey::from_secret_key(&secp, &spend),
                },
            )
        })
        .collect();
    let send = SendKey::generate(100).unwrap();
    let destinations = ids.map(|id| (id, vec![id[0]])).into();
    let d = build_distribution(
        100,
        &send,
        &routed,
        &miners,
        &p2tr_script(&[9; 32]),
        &destinations,
    )
    .unwrap();
    assert_eq!(d.outputs.len(), 68); // 67 miner outputs and one pool output.
    assert_eq!(d.entries.len(), 3); // exactly one logical row per identity.
    assert!(d.outputs[1..].windows(2).all(|w| w[0].0 < w[1].0));
    for entry in d.entries {
        assert_eq!(entry.rail, Rail::Coinbase);
        assert_eq!(entry.owed_sats, raw.raw[&entry.sp_id]);
        let decomposition = decompose(entry.owed_sats, 1024).unwrap();
        assert_eq!(entry.p_x.len(), decomposition.outputs.len());
        for (key, amount) in entry.p_x.iter().zip(decomposition.outputs) {
            assert!(d.outputs.contains(&(p2tr_script(key), amount)));
        }
    }
}

#[test]
fn committed_underived_and_unknown_keys_hold_without_forfeit_or_capacity_use() {
    let id = [1; 32];
    let raw = RawPayouts {
        raw: BTreeMap::from([(id, 499_999)]),
        pool_sats: 1,
        r_net: 499_999,
        f_net: 0,
    };
    let caps = BTreeMap::from([(
        id,
        ArkCapability {
            registered: false,
            registration_committed: true,
            dest_known: false,
        },
    )]);
    let r = route_tiers(&raw, &BTreeMap::new(), &caps, &config(), 500_000).unwrap();
    assert_eq!(r.pending, vec![(id, 499_999)]);
    assert!(r.forfeited.is_empty());
    let pending = BTreeMap::from([(id, 6_000_000)]);
    assert!(matches!(
        route_tiers(&raw, &pending, &caps, &config(), 500_000),
        Err(Error::EntitlementExceedsRevenue)
    ));
    let known = BTreeSet::new();
    let r = route_tiers_with_keys(&raw, &pending, &caps, &config(), 500_000, Some(&known)).unwrap();
    assert_eq!(r.pending, vec![(id, 6_499_999)]);
    assert!(r.kept.is_empty());
}

#[test]
fn jd_is_explicit_exact_weight_and_pool_capacity_is_a_hard_failure() {
    let id = [1; 32];
    let raw = RawPayouts {
        raw: BTreeMap::from([(id, 9_500_123)]),
        pool_sats: 10,
        r_net: 9_500_123,
        f_net: 0,
    };
    let cfg = config();
    let jd = route_jd_tiers(&raw, &BTreeMap::new(), &BTreeMap::new(), &cfg, 9_500_133).unwrap();
    assert_eq!(build_weights(&jd, 10).weights, vec![(id, 9_500_123)]);
    let pool = route_tiers(&raw, &BTreeMap::new(), &BTreeMap::new(), &cfg, 9_500_133).unwrap();
    assert_eq!(pool.kept, vec![(id, 9_500_000)]);
    assert_eq!(pool.pending_deltas[&id], 123);
    let mut limited = cfg.clone();
    limited.max_payout_slots = 3;
    assert!(matches!(
        route_tiers(
            &raw,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &limited,
            9_500_133
        ),
        Err(Error::PayoutSlots { required: 4, .. })
    ));
    limited = cfg;
    limited.budget_wu = 224 + 4 * 172 - 1;
    assert_eq!(
        route_tiers(
            &raw,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &limited,
            9_500_133
        ),
        Err(Error::PayoutWeight)
    );
}
