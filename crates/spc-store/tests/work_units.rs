use spc_store::models::*;
use spc_store::Store;
use uuid::Uuid;

fn id() -> [u8; 32] {
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    out[16..].copy_from_slice(Uuid::new_v4().as_bytes());
    out
}

async fn store() -> Option<Store> {
    let url = std::env::var("SPC_TEST_DATABASE_URL").ok()?;
    let store = Store::connect(&url).await.expect("connect");
    store.migrate().await.expect("migrate");
    Some(store)
}

#[tokio::test]
async fn work_unit_columns_roundtrip_legacy_defaults_and_check_constraints() {
    let Some(store) = store().await else { return };
    let fp = id();
    let reference_fingerprint = [0x55; 32];
    let snapshot_fingerprint = id();
    let window_target = "65".to_string();

    store
        .insert_slice(&SliceRow {
            slice_id: u64::from_be_bytes(fp[..8].try_into().unwrap()) & 0x7fff_ffff_ffff_ffff,
            height: 800_000,
            prev_hash: fp,
            ref_job_id: 7,
            ref_fees: 0,
            n_shares: 3,
            work_unit_version: WorkUnitVersion::V2,
            work_unit_fingerprint: Some(reference_fingerprint),
            difficulty: window_target.clone(),
            root: [2; 32],
            shares: vec![],
            credits: vec![],
        })
        .await
        .unwrap();
    store
        .insert_snapshot(&SnapshotRow {
            fingerprint: snapshot_fingerprint,
            height: 800_001,
            a_send_x: [4; 32],
            distribution_id: None,
            entries: serde_json::json!([]),
            high_water: serde_json::json!({}),
            work_unit_version: WorkUnitVersion::V2,
            work_unit_fingerprint: Some(reference_fingerprint),
            window_rule: WindowRule::NetworkBlockWork,
            window_target: Some(window_target),
        })
        .await
        .unwrap();

    let frozen = store
        .snapshot(&snapshot_fingerprint)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(frozen.work_unit_version, WorkUnitVersion::V2);
    assert_eq!(frozen.work_unit_fingerprint, Some(reference_fingerprint));
    assert_eq!(frozen.window_rule, WindowRule::NetworkBlockWork);
    assert_eq!(frozen.window_target.as_deref(), Some("65"));
    let tail = store.window_tail(50).await.unwrap();
    let updated = tail
        .iter()
        .find(|s| {
            s.slice_id == u64::from_be_bytes(fp[..8].try_into().unwrap()) & 0x7fff_ffff_ffff_ffff
        })
        .unwrap();
    assert_eq!(updated.work_unit_version, WorkUnitVersion::V2);
    assert_eq!(updated.work_unit_fingerprint, Some(reference_fingerprint));
    assert_eq!(updated.difficulty, "65");

    let slice_key = i64::try_from(updated.slice_id).unwrap();
    for (sql, restore) in [
        ("work_unit_version = 1", "work_unit_version = 2"),
        ("work_unit_fingerprint = NULL", "work_unit_fingerprint = $2"),
        ("work_unit_version = 9", "work_unit_version = 2"),
    ] {
        let result = sqlx::query(&format!("UPDATE slice SET {sql} WHERE slice_id = $1"))
            .bind(slice_key)
            .bind(&reference_fingerprint[..])
            .execute(store.pool())
            .await;
        assert!(result.is_err(), "CHECK must reject {sql}");
        sqlx::query(&format!("UPDATE slice SET {restore} WHERE slice_id = $1"))
            .bind(slice_key)
            .bind(&reference_fingerprint[..])
            .execute(store.pool())
            .await
            .unwrap();
    }
    for invalid in [vec![0u8; 31], vec![0u8; 32]] {
        let result = sqlx::query("UPDATE slice SET work_unit_fingerprint = $2 WHERE slice_id = $1")
            .bind(slice_key)
            .bind(&invalid)
            .execute(store.pool())
            .await;
        assert!(
            result.is_err(),
            "slice fingerprint must be 32 nonzero bytes: {} bytes",
            invalid.len()
        );
    }
    for invalid in [vec![0u8; 31], vec![0u8; 32]] {
        let result = sqlx::query(
            "UPDATE distribution_snapshot SET work_unit_fingerprint = $2 WHERE fingerprint = $1",
        )
        .bind(&snapshot_fingerprint[..])
        .bind(&invalid)
        .execute(store.pool())
        .await;
        assert!(
            result.is_err(),
            "snapshot fingerprint must be 32 nonzero bytes: {} bytes",
            invalid.len()
        );
    }

    let bad_rule =
        sqlx::query("UPDATE distribution_snapshot SET window_rule = 0 WHERE fingerprint = $1")
            .bind(&snapshot_fingerprint[..])
            .execute(store.pool())
            .await;
    assert!(bad_rule.is_err(), "v2 cannot use legacy window_rule");
    sqlx::query("UPDATE distribution_snapshot SET window_rule = 1 WHERE fingerprint = $1")
        .bind(&snapshot_fingerprint[..])
        .execute(store.pool())
        .await
        .unwrap();

    // A raw legacy insert omits every new field: defaults preserve v1 + NULL,
    // without conversion or a zero placeholder fingerprint.
    let legacy: i64 = (i64::from_be_bytes(fp[8..16].try_into().unwrap()) & 0x07ff_ffff) + 1;
    sqlx::query(
        "INSERT INTO slice
            (slice_id, height, prev_hash, ref_job_id, ref_fees, n_shares,
             difficulty, root, shares, sealed_at)
         VALUES ($1, 1, $2, 0, 0, 0, '16'::numeric, $3, $4, now())",
    )
    .bind(legacy)
    .bind(&fp[..])
    .bind(&fp[..])
    .bind(&fp[..])
    .execute(store.pool())
    .await
    .unwrap();
    let tail = store.window_tail(100).await.unwrap();
    let legacy = tail.iter().find(|s| s.slice_id == legacy as u64).unwrap();
    assert_eq!(legacy.work_unit_version, WorkUnitVersion::V1);
    assert_eq!(legacy.work_unit_fingerprint, None);
}

#[tokio::test]
async fn mixed_version_scrub_uses_stored_row_horizons_in_either_payout_mode() {
    let Some(store) = store().await else { return };
    let sp_id = id();
    store
        .upsert_identity(&SpIdentityRow {
            sp_id,
            b_scan: vec![2; 33],
            b_spend: vec![3; 33],
            network: 3,
        })
        .await
        .unwrap();

    let base = u64::from_be_bytes(id()[..8].try_into().unwrap()) & 0x7fff_ffff_ffff_ff00;
    let heights = [5_903u32, 5_904, 6_000, 9_983, 9_984];
    let mut ids = Vec::new();
    let mut expected = std::collections::BTreeSet::new();
    for (version_index, version) in [WorkUnitVersion::V1, WorkUnitVersion::V2]
        .into_iter()
        .enumerate()
    {
        for (height_index, height) in heights.into_iter().enumerate() {
            let slice_id = base + (version_index * heights.len() + height_index + 1) as u64;
            ids.push(slice_id as i64);
            let v2 = version == WorkUnitVersion::V2;
            if (v2 && height > 5_903) || (!v2 && height >= 9_984) {
                expected.insert(slice_id as i64);
            }
            store
                .insert_slice(&SliceRow {
                    slice_id,
                    height,
                    prev_hash: [3; 32],
                    ref_job_id: 1,
                    ref_fees: 0,
                    n_shares: 1,
                    work_unit_version: version,
                    work_unit_fingerprint: v2.then_some([0x55; 32]),
                    difficulty: "1".into(),
                    root: [2; 32],
                    shares: vec![],
                    credits: vec![SliceCreditRow {
                        sp_id,
                        diff_sum: "1".into(),
                        diff_fee_sum: "0".into(),
                    }],
                })
                .await
                .unwrap();
        }
    }

    // Tip 10000: legacy cutoff 9984, v2 cutoff 5904. The deletion must be
    // independent of which profile the active payout is configured for.
    // The second run also proves the same rows/cutoffs are idempotent.
    for _ in 0..2 {
        store.scrub(10_000, 16, 4_096, 100).await.unwrap();
        let rows = sqlx::query(
            "SELECT slice_id FROM slice_credit WHERE slice_id = ANY($1) ORDER BY slice_id",
        )
        .bind(&ids)
        .fetch_all(store.pool())
        .await
        .unwrap();
        let actual: std::collections::BTreeSet<i64> = rows
            .into_iter()
            .map(|row| sqlx::Row::try_get::<i64, _>(&row, "slice_id").unwrap())
            .collect();
        assert_eq!(
            actual, expected,
            "stored row version must select its own horizon"
        );
    }
}
