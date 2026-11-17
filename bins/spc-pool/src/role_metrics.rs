//! Role-sourced metric snapshots (SPEC §9, OPERATIONS §7).
//!
//! The `api` role's exporter renders what it can see: Postgres (via the
//! `Store`) and its own configuration. The remaining SPEC §9 families are
//! known only to the `front`, `payout` and `ark` roles. Each role publishes
//! a small JSON snapshot of its own gauges and counters to a Redis key under
//! `spc:metrics:*` (plain `SET` with a TTL, no new stream or wire format);
//! the api role gathers the fresh snapshots at scrape time. An expired
//! snapshot makes its families **absent** - a missing series means
//! "unknown", never "empty" - exactly the store-query contract in
//! [`crate::metrics`].
//!
//! Privacy (SPEC §11.3): every value here is an aggregate count, age or
//! sats figure. No identity field (`sp_id`, `B_scan`, `B_spend`, `P_i`,
//! block hash) appears in any snapshot, so nothing attributable crosses
//! into - or out of - the `ark` role through this surface.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use spc_store::redis::Streams;

use crate::metrics::MetricsSnapshot;

/// One key per front: `spc:metrics:front:<front_id>`.
pub const FRONT_KEY_PREFIX: &str = "spc:metrics:front:";
/// The single `ark` role's snapshot.
pub const ARK_KEY: &str = "spc:metrics:ark";
/// The single `payout` role's snapshot.
pub const PAYOUT_KEY: &str = "spc:metrics:payout";

/// How often each role publishes its snapshot.
pub const PUBLISH_INTERVAL: Duration = Duration::from_secs(15);
/// Snapshot TTL: four missed publications and the snapshot expires, making
/// the role's families unknown rather than stale.
pub const SNAPSHOT_TTL_SECS: u64 = 60;

/// The streams whose consumer-group delivery lag the api samples
/// (`spc_stream_lag_seconds{stream}`; the SPEC §5.2 group-consumed streams).
pub const LAG_STREAMS: &[&str] = &[
    "spc:slices",
    "spc:blocks",
    "spc:arkreg",
    "spc:front:events",
    "spc:ark:orders",
    "spc:ark:receipts",
];

/// A shared, labelled counter family (e.g. rejects by reason). Cheap to
/// clone into listener tasks; the publisher snapshots it.
#[derive(Clone, Debug, Default)]
pub struct CounterMap(Arc<Mutex<BTreeMap<String, u64>>>);

impl CounterMap {
    pub fn incr(&self, label: &str) {
        if let Ok(mut map) = self.0.lock() {
            *map.entry(label.to_string()).or_insert(0) += 1;
        }
    }

    pub fn snapshot(&self) -> BTreeMap<String, u64> {
        self.0.lock().map(|m| m.clone()).unwrap_or_default()
    }
}

/// What one `front` knows at publish time. Counters are process-lifetime
/// monotonic; gauges describe the current job/distribution.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FrontSample {
    pub front_id: u8,
    /// Seconds since the current height's `a_send` was generated.
    pub a_send_age_seconds: Option<f64>,
    /// `ser_x(A_send)` repeats across heights within this front's own
    /// sequence (SPEC §2.2).
    pub a_send_repeat_total: u64,
    /// Shares in the open slice (0 when no slice is open).
    pub open_slice_shares: u64,
    pub slices_sealed_total: BTreeMap<String, u64>,
    /// Current distribution's entries per class (`kept`/`ark`/`error`).
    pub distribution_slots: Option<BTreeMap<String, u64>>,
    pub distribution_outputs_total: Option<u64>,
    pub budget_wu: Option<u64>,
    pub budget_utilisation: Option<f64>,
    pub channel_reject_total: BTreeMap<String, u64>,
    pub jd_declaration_reject_total: BTreeMap<String, u64>,
    pub ext32_requests_total: BTreeMap<String, u64>,
    /// Highest `spc:front:events` revision applied (bootstrap or live).
    pub applied_rev: u64,
}

/// What the `ark` role knows: float level and terminal order rejections.
/// Aggregates only - the role never sees an identity, and none leaves it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ArkSample {
    /// Spendable sats in the float (`None` when the transport cannot
    /// report a balance, e.g. the dev command transport).
    pub float_sats: Option<u64>,
    /// Matured pool revenue not boarded (`None` when unknown).
    pub surplus_sats: Option<u64>,
    pub orders_rejected_total: BTreeMap<String, u64>,
}

/// What `payout` knows: the committed event revision, overdue scheduled Ark
/// sends, and orphan settlements.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PayoutSample {
    /// The persisted `spc:front:events` watermark.
    pub committed_rev: Option<u64>,
    /// Scheduled Ark payouts past `due_height + ark_retry_blocks` (`None`
    /// when the chain view was unavailable at publish time).
    pub ark_overdue_total: Option<u64>,
    pub orphaned_settlement_total: BTreeMap<String, u64>,
}

type RedisResult<T> = std::result::Result<T, spc_store::redis::RedisError>;

pub async fn publish_front(streams: &Streams, sample: &FrontSample) -> RedisResult<()> {
    let key = format!("{FRONT_KEY_PREFIX}{}", sample.front_id);
    publish(streams, &key, sample).await
}

pub async fn publish_ark(streams: &Streams, sample: &ArkSample) -> RedisResult<()> {
    publish(streams, ARK_KEY, sample).await
}

pub async fn publish_payout(streams: &Streams, sample: &PayoutSample) -> RedisResult<()> {
    publish(streams, PAYOUT_KEY, sample).await
}

async fn publish<T: Serialize>(streams: &Streams, key: &str, sample: &T) -> RedisResult<()> {
    let json = serde_json::to_string(sample).expect("role metric samples serialize");
    streams.kv_set_ex(key, &json, SNAPSHOT_TTL_SECS).await
}

/// Everything one scrape gathered from Redis. `errors` counts failed
/// sources (connection, scan or undecodable snapshot), to be added to
/// `spc_metrics_scrape_errors_total`.
#[derive(Debug, Default)]
pub struct Gathered {
    pub fronts: Vec<FrontSample>,
    pub ark: Option<ArkSample>,
    pub payout: Option<PayoutSample>,
    /// `(stream, lag_seconds)` for each stream whose lag is known.
    pub stream_lags: Vec<(String, f64)>,
    pub errors: u32,
}

/// Gather the fresh role snapshots and the per-stream lags. Each failed
/// source counts one error and leaves its families absent; a missing key
/// (role not running, snapshot expired) is not an error.
pub async fn gather(streams: &Streams) -> Gathered {
    let mut out = Gathered::default();

    match streams.scan_keys(&format!("{FRONT_KEY_PREFIX}*")).await {
        Ok(keys) => {
            for key in keys {
                match read_sample::<FrontSample>(streams, &key).await {
                    Ok(Some(sample)) => out.fronts.push(sample),
                    Ok(None) => {} // expired between SCAN and GET
                    Err(()) => out.errors += 1,
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "metrics: front snapshot scan failed");
            out.errors += 1;
        }
    }
    match read_sample::<ArkSample>(streams, ARK_KEY).await {
        Ok(sample) => out.ark = sample,
        Err(()) => out.errors += 1,
    }
    match read_sample::<PayoutSample>(streams, PAYOUT_KEY).await {
        Ok(sample) => out.payout = sample,
        Err(()) => out.errors += 1,
    }
    for stream in LAG_STREAMS {
        match streams.group_lag_ms(stream).await {
            Ok(Some(ms)) => out
                .stream_lags
                .push((stream.to_string(), ms as f64 / 1000.0)),
            Ok(None) => {} // stream or group absent: unknown, not zero
            Err(e) => {
                tracing::warn!(error = %e, stream, "metrics: stream lag failed");
                out.errors += 1;
            }
        }
    }
    out
}

async fn read_sample<T: for<'de> Deserialize<'de>>(
    streams: &Streams,
    key: &str,
) -> std::result::Result<Option<T>, ()> {
    match streams.kv_get(key).await {
        Ok(Some(json)) => match serde_json::from_str(&json) {
            Ok(sample) => Ok(Some(sample)),
            Err(e) => {
                tracing::warn!(error = %e, key, "metrics: undecodable role snapshot");
                Err(())
            }
        },
        Ok(None) => Ok(None),
        Err(e) => {
            tracing::warn!(error = %e, key, "metrics: role snapshot read failed");
            Err(())
        }
    }
}

/// Merge the gathered role snapshots into one scrape.
///
/// Aggregation across fronts follows the alarm each family serves:
/// counters and sizes **sum** (pool-wide totals), ages and utilisation take
/// the **max** (the worst front is the alarming one), and the per-front
/// repeat counter keeps its `front_id` label (the one labelled front family,
/// MULTI-REGION §10). `spc_registration_lag_revs` is payout's committed
/// revision minus the slowest front's applied revision.
pub fn merge_into(snap: &mut MetricsSnapshot, gathered: &Gathered) {
    snap.scrape_errors += gathered.errors;
    if !gathered.stream_lags.is_empty() {
        snap.stream_lag_seconds = Some(gathered.stream_lags.clone());
    }

    if !gathered.fronts.is_empty() {
        let fronts = &gathered.fronts;
        snap.a_send_age_seconds = fronts
            .iter()
            .filter_map(|f| f.a_send_age_seconds)
            .fold(None, |acc: Option<f64>, age| {
                Some(acc.map_or(age, |a| a.max(age)))
            });
        snap.a_send_repeat_by_front = Some(
            fronts
                .iter()
                .map(|f| (f.front_id.to_string(), f.a_send_repeat_total))
                .collect(),
        );
        snap.open_slice_shares = Some(fronts.iter().map(|f| f.open_slice_shares).sum());
        snap.slices_sealed_by_reason =
            Some(sum_maps(fronts.iter().map(|f| &f.slices_sealed_total)));
        let slot_maps: Vec<_> = fronts
            .iter()
            .filter_map(|f| f.distribution_slots.as_ref())
            .collect();
        if !slot_maps.is_empty() {
            snap.distribution_slots = Some(sum_maps(slot_maps.into_iter()));
        }
        snap.distribution_outputs =
            sum_present(fronts.iter().map(|f| f.distribution_outputs_total));
        snap.budget_wu = fronts.iter().filter_map(|f| f.budget_wu).max();
        snap.budget_utilisation = fronts
            .iter()
            .filter_map(|f| f.budget_utilisation)
            .fold(None, |acc: Option<f64>, u| {
                Some(acc.map_or(u, |a| a.max(u)))
            });
        snap.channel_reject_by_reason =
            Some(sum_maps(fronts.iter().map(|f| &f.channel_reject_total)));
        snap.jd_declaration_reject_by_code = Some(sum_maps(
            fronts.iter().map(|f| &f.jd_declaration_reject_total),
        ));
        snap.ext32_requests_by_msg = Some(sum_maps(fronts.iter().map(|f| &f.ext32_requests_total)));
    }

    if let Some(ark) = &gathered.ark {
        snap.ark_float_sats = ark.float_sats;
        snap.ark_surplus_sats = ark.surplus_sats;
        snap.ark_orders_rejected_by_reason = Some(to_vec(&ark.orders_rejected_total));
    }

    if let Some(payout) = &gathered.payout {
        snap.ark_overdue_total = payout.ark_overdue_total;
        snap.orphaned_settlement_by_reason = Some(to_vec(&payout.orphaned_settlement_total));
        if let (Some(committed), Some(slowest)) = (
            payout.committed_rev,
            gathered.fronts.iter().map(|f| f.applied_rev).min(),
        ) {
            snap.registration_lag_revs = Some(committed.saturating_sub(slowest));
        }
    }
}

fn sum_present<I: Iterator<Item = Option<u64>>>(values: I) -> Option<u64> {
    values.fold(None, |acc, v| match (acc, v) {
        (None, None) => None,
        (acc, v) => Some(acc.unwrap_or(0) + v.unwrap_or(0)),
    })
}

fn sum_maps<'a, I: Iterator<Item = &'a BTreeMap<String, u64>>>(maps: I) -> Vec<(String, u64)> {
    let mut total: BTreeMap<String, u64> = BTreeMap::new();
    for map in maps {
        for (k, v) in map {
            *total.entry(k.clone()).or_insert(0) += v;
        }
    }
    total.into_iter().collect()
}

fn to_vec(map: &BTreeMap<String, u64>) -> Vec<(String, u64)> {
    map.iter().map(|(k, v)| (k.clone(), *v)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn front(front_id: u8, applied_rev: u64) -> FrontSample {
        FrontSample {
            front_id,
            a_send_age_seconds: Some(10.0 * f64::from(front_id) + 1.0),
            a_send_repeat_total: u64::from(front_id),
            open_slice_shares: 5,
            slices_sealed_total: BTreeMap::from([("fee_band".to_string(), 2)]),
            distribution_slots: Some(BTreeMap::from([("kept".to_string(), 3)])),
            distribution_outputs_total: Some(4),
            budget_wu: Some(400_000),
            budget_utilisation: Some(0.25 * f64::from(front_id)),
            channel_reject_total: BTreeMap::from([("invalid-user-identity".to_string(), 1)]),
            jd_declaration_reject_total: BTreeMap::from([("invalid-job".to_string(), 1)]),
            ext32_requests_total: BTreeMap::from([("get_window".to_string(), 1)]),
            applied_rev,
        }
    }

    /// Counters and sizes sum; ages and utilisation take the worst front;
    /// the repeat counter keeps one labelled sample per front.
    #[test]
    fn merge_aggregates_fronts_per_family_semantics() {
        let gathered = Gathered {
            fronts: vec![front(0, 7), front(1, 9)],
            ..Default::default()
        };
        let mut snap = MetricsSnapshot::default();
        merge_into(&mut snap, &gathered);
        assert_eq!(snap.a_send_age_seconds, Some(11.0));
        assert_eq!(
            snap.a_send_repeat_by_front,
            Some(vec![("0".to_string(), 0), ("1".to_string(), 1)])
        );
        assert_eq!(snap.open_slice_shares, Some(10));
        assert_eq!(
            snap.slices_sealed_by_reason,
            Some(vec![("fee_band".to_string(), 4)])
        );
        assert_eq!(snap.distribution_slots, Some(vec![("kept".to_string(), 6)]));
        assert_eq!(snap.distribution_outputs, Some(8));
        assert_eq!(snap.budget_wu, Some(400_000));
        assert_eq!(snap.budget_utilisation, Some(0.25));
        assert_eq!(
            snap.channel_reject_by_reason,
            Some(vec![("invalid-user-identity".to_string(), 2)])
        );
        // No payout snapshot: the lag is unknown, not zero.
        assert_eq!(snap.registration_lag_revs, None);
        assert_eq!(snap.scrape_errors, 0);
    }

    /// Registration lag is committed minus the *slowest* front, and the
    /// payout/ark families pass through with their maps rendered.
    #[test]
    fn merge_computes_registration_lag_from_the_slowest_front() {
        let gathered = Gathered {
            fronts: vec![front(0, 7), front(1, 9)],
            payout: Some(PayoutSample {
                committed_rev: Some(12),
                ark_overdue_total: Some(1),
                orphaned_settlement_total: BTreeMap::from([("competing_branch".to_string(), 2)]),
            }),
            ark: Some(ArkSample {
                float_sats: Some(1_000_000),
                surplus_sats: None,
                orders_rejected_total: BTreeMap::from([("bad-signature".to_string(), 1)]),
            }),
            ..Default::default()
        };
        let mut snap = MetricsSnapshot::default();
        merge_into(&mut snap, &gathered);
        assert_eq!(snap.registration_lag_revs, Some(5));
        assert_eq!(snap.ark_overdue_total, Some(1));
        assert_eq!(
            snap.orphaned_settlement_by_reason,
            Some(vec![("competing_branch".to_string(), 2)])
        );
        assert_eq!(snap.ark_float_sats, Some(1_000_000));
        assert_eq!(snap.ark_surplus_sats, None);
        assert_eq!(
            snap.ark_orders_rejected_by_reason,
            Some(vec![("bad-signature".to_string(), 1)])
        );
    }

    /// An empty gather leaves every role family absent and adds its errors
    /// to the scrape-error counter.
    #[test]
    fn merge_of_nothing_changes_nothing_but_errors() {
        let gathered = Gathered {
            errors: 2,
            ..Default::default()
        };
        let mut snap = MetricsSnapshot::default();
        merge_into(&mut snap, &gathered);
        assert_eq!(snap.scrape_errors, 2);
        assert_eq!(snap.a_send_repeat_by_front, None);
        assert_eq!(snap.stream_lag_seconds, None);
        assert_eq!(snap.ark_orders_rejected_by_reason, None);
        assert_eq!(snap.orphaned_settlement_by_reason, None);
    }

    /// Snapshots round-trip through their JSON form, and a sample written
    /// by a newer build (extra fields) still decodes.
    #[test]
    fn samples_round_trip_and_tolerate_new_fields() {
        let sample = front(3, 42);
        let json = serde_json::to_string(&sample).unwrap();
        let back: FrontSample = serde_json::from_str(&json).unwrap();
        assert_eq!(back.front_id, 3);
        assert_eq!(back.applied_rev, 42);
        assert_eq!(back.a_send_age_seconds, Some(31.0));

        let forward = r#"{"front_id":9,"new_field_from_v2":true}"#;
        let decoded: FrontSample = serde_json::from_str(forward).unwrap();
        assert_eq!(decoded.front_id, 9);
        assert_eq!(decoded.a_send_repeat_total, 0);
    }

    #[test]
    fn counter_map_increments_and_snapshots() {
        let counters = CounterMap::default();
        counters.incr("invalid-job");
        counters.incr("invalid-job");
        counters.incr("internal-error");
        let clone = counters.clone();
        clone.incr("invalid-job");
        assert_eq!(
            counters.snapshot(),
            BTreeMap::from([
                ("invalid-job".to_string(), 3),
                ("internal-error".to_string(), 1)
            ])
        );
    }
}
