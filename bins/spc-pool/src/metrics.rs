//! Prometheus text-format exporter for the `api` role (SPEC §6.1, §9).
//!
//! The `api` role is read-only: it sees Postgres (via the `Store`) and its
//! own static configuration, nothing else. This module renders exactly what
//! those sources know - the SPEC §9 names the role can source truthfully -
//! and omits the rest rather than fabricating them (see
//! `docs/OPERATIONS.md` §7 for the omission list). A failed source query
//! omits its family and increments `spc_metrics_scrape_errors_total`.

use std::time::{SystemTime, UNIX_EPOCH};

/// Everything one scrape can render. `None` fields are sources that failed;
/// the renderer omits them instead of rendering a misleading zero, and the
/// gatherer counts them in `scrape_errors`.
#[derive(Debug, Default)]
pub struct MetricsSnapshot {
    /// Seconds since the process started (`/proc` on Linux; `None` when the
    /// true value cannot be read).
    pub uptime_secs: Option<f64>,
    /// Store queries that failed during this scrape.
    pub scrape_errors: u32,
    /// `[accounting] window_n`.
    pub window_n: Option<u32>,
    /// `[accounting] coinbase_threshold_sats` (the tier threshold `C`).
    pub coinbase_threshold_sats: Option<u64>,
    /// `[budget] floor_wu`.
    pub budget_floor_wu: Option<u64>,
    /// `[budget] ceiling_wu`.
    pub budget_ceiling_wu: Option<u64>,
    /// Found blocks per status (`pending`/`confirmed`/`orphaned`).
    pub blocks_by_status: Option<Vec<(String, u64)>>,
    /// `Sum pending_balance.sats + Sum ark_payout.sats WHERE status =
    /// 'scheduled'` - everything the pool holds for miners (SPEC §6.1).
    pub pending_liability_sats: Option<u64>,
    /// Sats booked to the Ark rail and not yet sent.
    pub ark_due_sats: Option<u64>,
    /// Ark payouts per status
    /// (`scheduled`/`sent`/`failed_to_pending`).
    pub ark_payouts_by_status: Option<Vec<(String, u64)>>,
    /// Cumulative forfeited sats (SPEC §11.1).
    pub forfeited_sats_total: Option<u64>,
    /// Age of the current height's `a_send` (front-sourced; worst front).
    pub a_send_age_seconds: Option<f64>,
    /// `ser_x(A_send)` repeats across heights, one sample per front
    /// (SPEC §2.2, MULTI-REGION §10: the `front_id` label is the one
    /// labelled front metric).
    pub a_send_repeat_by_front: Option<Vec<(String, u64)>>,
    /// Shares in the open slice(s) (front-sourced; summed over fronts).
    pub open_slice_shares: Option<u64>,
    /// Sealed slices per reason (front-sourced counters).
    pub slices_sealed_by_reason: Option<Vec<(String, u64)>>,
    /// Current distribution's entries per class (`kept`/`ark`/`error`).
    pub distribution_slots: Option<Vec<(String, u64)>>,
    /// Outputs in the current distribution (approaching `max_payout_slots`
    /// is the alarm).
    pub distribution_outputs: Option<u64>,
    /// The weight budget the current distribution was built against.
    pub budget_wu: Option<u64>,
    /// Used weight over `budget_wu` for the current distribution.
    pub budget_utilisation: Option<f64>,
    /// Scheduled Ark payouts past `due_height + ark_retry_blocks`
    /// (payout-sourced; page).
    pub ark_overdue_total: Option<u64>,
    /// Spendable sats in the `ark` role's float.
    pub ark_float_sats: Option<u64>,
    /// Matured pool revenue not boarded (operator tooling).
    pub ark_surplus_sats: Option<u64>,
    /// Terminally rejected Ark orders per reason (`ark`-role counters).
    pub ark_orders_rejected_by_reason: Option<Vec<(String, u64)>>,
    /// Channel-open rejections per reason (front listeners).
    pub channel_reject_by_reason: Option<Vec<(String, u64)>>,
    /// JD declaration declines per wire `error_code` (front JDS).
    pub jd_declaration_reject_by_code: Option<Vec<(String, u64)>>,
    /// Type-32 requests per message (front SV2 listener).
    pub ext32_requests_by_msg: Option<Vec<(String, u64)>>,
    /// Per-stream consumer-group delivery lag, seconds (api-sourced from
    /// Redis; only streams whose lag is known).
    pub stream_lag_seconds: Option<Vec<(String, f64)>>,
    /// `payout`'s committed event revision minus the slowest front's applied
    /// revision (0 when every front has caught up).
    pub registration_lag_revs: Option<u64>,
    /// Orphan settlements per reason (payout-sourced counters).
    pub orphaned_settlement_by_reason: Option<Vec<(String, u64)>>,
}

/// Render the snapshot as Prometheus text format 0.0.4.
///
/// The output is deterministic: families are emitted in a fixed order and
/// status families are sorted, with the schema-known statuses zero-filled
/// so alert expressions see `0` rather than an absent series. A family whose
/// source failed is omitted entirely - a missing series means "unknown",
/// never "empty".
pub fn render(snap: &MetricsSnapshot) -> String {
    let mut out = String::new();

    if let Some(up) = snap.uptime_secs {
        family(
            &mut out,
            "spc_process_uptime_seconds",
            "Seconds since this process started.",
            "gauge",
        );
        sample(
            &mut out,
            "spc_process_uptime_seconds",
            &[],
            format!("{up:.3}"),
        );
    }

    family(
        &mut out,
        "spc_metrics_scrape_errors_total",
        "Store queries that failed during this scrape.",
        "counter",
    );
    sample(
        &mut out,
        "spc_metrics_scrape_errors_total",
        &[],
        snap.scrape_errors,
    );

    if let Some(by_status) = &snap.blocks_by_status {
        family(
            &mut out,
            "spc_blocks_total",
            "Found blocks by settlement status.",
            "counter",
        );
        for (status, n) in zero_filled(by_status, &["pending", "confirmed", "orphaned"]) {
            sample(&mut out, "spc_blocks_total", &[("status", &status)], n);
        }
    }

    if let Some(sats) = snap.pending_liability_sats {
        family(
            &mut out,
            "spc_pending_liability_sats",
            "Everything the pool holds for miners (pending + scheduled Ark).",
            "gauge",
        );
        sample(&mut out, "spc_pending_liability_sats", &[], sats);
    }

    if let Some(sats) = snap.ark_due_sats {
        family(
            &mut out,
            "spc_ark_due_sats",
            "Sats booked to the Ark rail and not yet sent.",
            "gauge",
        );
        sample(&mut out, "spc_ark_due_sats", &[], sats);
    }

    if let Some(by_status) = &snap.ark_payouts_by_status {
        family(
            &mut out,
            "spc_ark_payouts_total",
            "Ark payouts by status.",
            "counter",
        );
        for (status, n) in zero_filled(by_status, &["scheduled", "sent", "failed_to_pending"]) {
            sample(&mut out, "spc_ark_payouts_total", &[("status", &status)], n);
        }
    }

    if let Some(sats) = snap.forfeited_sats_total {
        family(
            &mut out,
            "spc_forfeited_sats_total",
            "Cumulative forfeited sats (unregistered below the threshold).",
            "counter",
        );
        sample(&mut out, "spc_forfeited_sats_total", &[], sats);
    }

    if let Some(n) = snap.window_n {
        family(
            &mut out,
            "spc_config_window_n",
            "Configured accounting window size (slices).",
            "gauge",
        );
        sample(&mut out, "spc_config_window_n", &[], n);
    }

    if let Some(sats) = snap.coinbase_threshold_sats {
        family(
            &mut out,
            "spc_config_coinbase_threshold_sats",
            "Configured tier threshold C (sats).",
            "gauge",
        );
        sample(&mut out, "spc_config_coinbase_threshold_sats", &[], sats);
    }

    if let Some(wu) = snap.budget_floor_wu {
        family(
            &mut out,
            "spc_config_budget_floor_wu",
            "Configured payout weight budget floor (weight units).",
            "gauge",
        );
        sample(&mut out, "spc_config_budget_floor_wu", &[], wu);
    }

    if let Some(wu) = snap.budget_ceiling_wu {
        family(
            &mut out,
            "spc_config_budget_ceiling_wu",
            "Configured payout weight budget ceiling (weight units).",
            "gauge",
        );
        sample(&mut out, "spc_config_budget_ceiling_wu", &[], wu);
    }

    if let Some(age) = snap.a_send_age_seconds {
        family(
            &mut out,
            "spc_a_send_age_seconds",
            "Age of the current height's a_send (worst front).",
            "gauge",
        );
        sample(&mut out, "spc_a_send_age_seconds", &[], format!("{age:.3}"));
    }

    if let Some(by_front) = &snap.a_send_repeat_by_front {
        family(
            &mut out,
            "spc_a_send_repeat_total",
            "ser_x(A_send) repeats across heights within one front's sequence (SPEC \u{a7}2.2; any growth breaks unlinkability).",
            "counter",
        );
        for (front_id, n) in sorted(by_front) {
            sample(
                &mut out,
                "spc_a_send_repeat_total",
                &[("front_id", &front_id)],
                n,
            );
        }
    }

    if let Some(n) = snap.open_slice_shares {
        family(
            &mut out,
            "spc_open_slice_shares",
            "Shares in the open slice (summed over fronts).",
            "gauge",
        );
        sample(&mut out, "spc_open_slice_shares", &[], n);
    }

    if let Some(by_reason) = &snap.slices_sealed_by_reason {
        family(
            &mut out,
            "spc_slices_sealed_total",
            "Sealed slices by reason.",
            "counter",
        );
        for (reason, n) in zero_filled(
            by_reason,
            &[
                "fee_band",
                "max_shares",
                "chain_moved",
                "distribution_change",
            ],
        ) {
            sample(
                &mut out,
                "spc_slices_sealed_total",
                &[("reason", &reason)],
                n,
            );
        }
    }

    if let Some(by_class) = &snap.distribution_slots {
        family(
            &mut out,
            "spc_distribution_slots",
            "Current distribution's entries by class.",
            "gauge",
        );
        for (class, n) in zero_filled(by_class, &["kept", "ark", "error"]) {
            sample(&mut out, "spc_distribution_slots", &[("class", &class)], n);
        }
    }

    if let Some(n) = snap.distribution_outputs {
        family(
            &mut out,
            "spc_distribution_outputs_total",
            "Outputs in the current distribution (alarm when approaching max_payout_slots).",
            "gauge",
        );
        sample(&mut out, "spc_distribution_outputs_total", &[], n);
    }

    if let Some(wu) = snap.budget_wu {
        family(
            &mut out,
            "spc_budget_wu",
            "Weight budget the current distribution was built against.",
            "gauge",
        );
        sample(&mut out, "spc_budget_wu", &[], wu);
    }

    if let Some(util) = snap.budget_utilisation {
        family(
            &mut out,
            "spc_budget_utilisation",
            "Used weight over budget_wu for the current distribution.",
            "gauge",
        );
        sample(
            &mut out,
            "spc_budget_utilisation",
            &[],
            format!("{util:.4}"),
        );
    }

    if let Some(n) = snap.ark_overdue_total {
        family(
            &mut out,
            "spc_ark_overdue_total",
            "Scheduled Ark payouts past due_height + ark_retry_blocks (page).",
            "gauge",
        );
        sample(&mut out, "spc_ark_overdue_total", &[], n);
    }

    if let Some(sats) = snap.ark_float_sats {
        family(
            &mut out,
            "spc_ark_float_sats",
            "Spendable sats in the ark role's float.",
            "gauge",
        );
        sample(&mut out, "spc_ark_float_sats", &[], sats);
    }

    if let Some(sats) = snap.ark_surplus_sats {
        family(
            &mut out,
            "spc_ark_surplus_sats",
            "Matured pool revenue not boarded.",
            "gauge",
        );
        sample(&mut out, "spc_ark_surplus_sats", &[], sats);
    }

    if let Some(by_reason) = &snap.ark_orders_rejected_by_reason {
        family(
            &mut out,
            "spc_ark_orders_rejected_total",
            "Terminally rejected Ark orders by reason (any bad-signature pages).",
            "counter",
        );
        for (reason, n) in zero_filled(
            by_reason,
            &[
                "bad-signature",
                "too-large",
                "bad-address",
                "wrong-server",
                "daily-cap",
                "duplicate",
                "ambiguous",
                "transport",
            ],
        ) {
            sample(
                &mut out,
                "spc_ark_orders_rejected_total",
                &[("reason", &reason)],
                n,
            );
        }
    }

    if let Some(by_reason) = &snap.channel_reject_by_reason {
        family(
            &mut out,
            "spc_channel_reject_total",
            "Channel-open rejections by reason.",
            "counter",
        );
        for (reason, n) in zero_filled(by_reason, &["invalid-user-identity"]) {
            sample(
                &mut out,
                "spc_channel_reject_total",
                &[("reason", &reason)],
                n,
            );
        }
    }

    if let Some(by_code) = &snap.jd_declaration_reject_by_code {
        family(
            &mut out,
            "spc_jd_declaration_reject_total",
            "JD declaration declines by wire error_code.",
            "counter",
        );
        for (code, n) in zero_filled(
            by_code,
            &[
                "invalid-job",
                "invalid-payout-distribution",
                "stale-payout-distribution",
                "no-payout-distribution",
                "internal-error",
            ],
        ) {
            sample(
                &mut out,
                "spc_jd_declaration_reject_total",
                &[("code", &code)],
                n,
            );
        }
    }

    if let Some(by_msg) = &snap.ext32_requests_by_msg {
        family(
            &mut out,
            "spc_ext32_requests_total",
            "Type-32 (share accounting) requests by message.",
            "counter",
        );
        for (msg, n) in zero_filled(
            by_msg,
            &["get_window", "get_shares", "get_transactions_in_job"],
        ) {
            sample(&mut out, "spc_ext32_requests_total", &[("msg", &msg)], n);
        }
    }

    if let Some(lags) = &snap.stream_lag_seconds {
        family(
            &mut out,
            "spc_stream_lag_seconds",
            "Consumer-group delivery lag per Redis stream.",
            "gauge",
        );
        for (stream, lag) in sorted(lags) {
            sample(
                &mut out,
                "spc_stream_lag_seconds",
                &[("stream", &stream)],
                format!("{lag:.3}"),
            );
        }
    }

    if let Some(revs) = snap.registration_lag_revs {
        family(
            &mut out,
            "spc_registration_lag_revs",
            "Committed front-event revision minus the slowest front's applied revision.",
            "gauge",
        );
        sample(&mut out, "spc_registration_lag_revs", &[], revs);
    }

    if let Some(by_reason) = &snap.orphaned_settlement_by_reason {
        family(
            &mut out,
            "spc_orphaned_settlement_total",
            "Orphan settlements by reason (any missing_block_event: reconstruct per SPEC \u{a7}7.4).",
            "counter",
        );
        for (reason, n) in zero_filled(
            by_reason,
            &[
                "competing_branch",
                "economic_displaced",
                "missing_block_event",
            ],
        ) {
            sample(
                &mut out,
                "spc_orphaned_settlement_total",
                &[("reason", &reason)],
                n,
            );
        }
    }

    out
}

/// Emit the `# HELP` and `# TYPE` lines of one metric family.
fn family(out: &mut String, name: &str, help: &str, kind: &str) {
    out.push_str("# HELP ");
    out.push_str(name);
    out.push(' ');
    out.push_str(help);
    out.push('\n');
    out.push_str("# TYPE ");
    out.push_str(name);
    out.push(' ');
    out.push_str(kind);
    out.push('\n');
}

/// Emit one sample line. Label values are escaped per the 0.0.4 format.
fn sample(out: &mut String, name: &str, labels: &[(&str, &str)], value: impl std::fmt::Display) {
    out.push_str(name);
    if !labels.is_empty() {
        out.push('{');
        for (i, (k, v)) in labels.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(k);
            out.push_str("=\"");
            out.push_str(&escape_label_value(v));
            out.push('"');
        }
        out.push('}');
    }
    out.push(' ');
    out.push_str(&value.to_string());
    out.push('\n');
}

/// Escape a label value per Prometheus text format 0.0.4: backslash, double
/// quote and line feed.
fn escape_label_value(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out
}

/// Sort a labelled sample list by label so the output is deterministic.
fn sorted<V: Clone>(samples: &[(String, V)]) -> Vec<(String, V)> {
    let mut out = samples.to_vec();
    out.sort_by(|(a, _), (b, _)| a.cmp(b));
    out
}

/// Sort a per-status count list with the schema-known statuses first
/// (zero-filled when absent) and any unexpected status passed through after.
fn zero_filled(by_status: &[(String, u64)], known: &[&str]) -> Vec<(String, u64)> {
    let mut out: Vec<(String, u64)> = known
        .iter()
        .map(|k| {
            let n = by_status
                .iter()
                .find(|(s, _)| s == k)
                .map(|(_, n)| *n)
                .unwrap_or(0);
            ((*k).to_string(), n)
        })
        .collect();
    for (status, n) in by_status {
        if !known.contains(&status.as_str()) {
            out.push((status.clone(), *n));
        }
    }
    out
}

/// Linux userspace clock ticks per second (`sysconf(_SC_CLK_TCK)`, fixed at
/// 100 by the kernel's `USER_HZ` on every architecture NixOS supports).
const CLK_TCK: f64 = 100.0;

/// Process uptime in seconds, or `None` when the true value cannot be read.
///
/// On Linux this is computed from `/proc/self/stat` (field 22, start time in
/// clock ticks since boot, plus the boot epoch from `/proc/stat`), so it is
/// correct even on the very first scrape. Elsewhere there is no
/// dependency-free source, and the series is omitted rather than
/// approximated.
pub fn process_uptime_secs() -> Option<f64> {
    let start = process_start_unix_secs()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs_f64();
    Some((now - start).max(0.0))
}

fn process_start_unix_secs() -> Option<f64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // The comm field (2) may contain spaces and parentheses; everything
    // after the final ')' is whitespace-separated fields starting at field 3
    // (state). `starttime` is field 22, i.e. index 19 of those.
    let fields = stat.rsplit_once(')')?.1.split_whitespace();
    let start_ticks: f64 = fields.into_iter().nth(19)?.parse().ok()?;
    let boot = boot_time_unix_secs()?;
    Some(boot + start_ticks / CLK_TCK)
}

fn boot_time_unix_secs() -> Option<f64> {
    let stat = std::fs::read_to_string("/proc/stat").ok()?;
    for line in stat.lines() {
        if let Some(rest) = line.strip_prefix("btime ") {
            return rest.trim().parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_snapshot() -> MetricsSnapshot {
        MetricsSnapshot {
            uptime_secs: Some(12.5),
            scrape_errors: 0,
            window_n: Some(8),
            coinbase_threshold_sats: Some(330),
            budget_floor_wu: Some(50_000),
            budget_ceiling_wu: Some(400_000),
            blocks_by_status: Some(vec![
                ("confirmed".into(), 2),
                ("pending".into(), 1),
                ("orphaned".into(), 3),
            ]),
            pending_liability_sats: Some(123_456),
            ark_due_sats: Some(789),
            ark_payouts_by_status: Some(vec![("sent".into(), 3), ("scheduled".into(), 1)]),
            forfeited_sats_total: Some(1_000),
            a_send_age_seconds: Some(42.25),
            a_send_repeat_by_front: Some(vec![("0".into(), 0), ("1".into(), 2)]),
            open_slice_shares: Some(17),
            slices_sealed_by_reason: Some(vec![("fee_band".into(), 4), ("max_shares".into(), 9)]),
            distribution_slots: Some(vec![("kept".into(), 5), ("ark".into(), 3)]),
            distribution_outputs: Some(6),
            budget_wu: Some(400_000),
            budget_utilisation: Some(0.0043),
            ark_overdue_total: Some(0),
            ark_float_sats: Some(1_000_000),
            ark_surplus_sats: Some(250_000),
            ark_orders_rejected_by_reason: Some(vec![("bad-signature".into(), 1)]),
            channel_reject_by_reason: Some(vec![("invalid-user-identity".into(), 2)]),
            jd_declaration_reject_by_code: Some(vec![("invalid-job".into(), 1)]),
            ext32_requests_by_msg: Some(vec![("get_window".into(), 7)]),
            stream_lag_seconds: Some(vec![("spc:slices".into(), 0.0), ("spc:blocks".into(), 1.5)]),
            registration_lag_revs: Some(0),
            orphaned_settlement_by_reason: Some(vec![("competing_branch".into(), 1)]),
        }
    }

    /// Assert the text is well-formed 0.0.4: every non-comment line is a
    /// sample for a family that already has `# HELP` then `# TYPE`, every
    /// value parses as a float, and no sample series repeats.
    fn assert_well_formed(text: &str) {
        let mut helped: Vec<&str> = Vec::new();
        let mut typed: Vec<&str> = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        for line in text.lines() {
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("# HELP ") {
                let name = rest.split_whitespace().next().unwrap();
                assert!(!helped.contains(&name), "duplicate HELP for {name}");
                helped.push(name);
            } else if let Some(rest) = line.strip_prefix("# TYPE ") {
                let name = rest.split_whitespace().next().unwrap();
                assert!(helped.contains(&name), "TYPE for unhelped {name}");
                assert!(!typed.contains(&name), "duplicate TYPE for {name}");
                let kind = rest.split_whitespace().nth(1).unwrap();
                assert!(
                    matches!(
                        kind,
                        "counter" | "gauge" | "histogram" | "summary" | "untyped"
                    ),
                    "bad type {kind}"
                );
                typed.push(name);
            } else if !line.starts_with('#') {
                let (series, value) = line
                    .rsplit_once(' ')
                    .unwrap_or_else(|| panic!("sample without value: {line}"));
                let name = series.split('{').next().unwrap();
                assert!(typed.contains(&name), "sample for untyped {name}: {line}");
                assert!(value.parse::<f64>().is_ok(), "non-numeric value: {line}");
                assert!(!seen.contains(&series), "duplicate sample: {line}");
                seen.push(series);
            }
        }
        assert!(!helped.is_empty(), "no families rendered");
    }

    #[test]
    fn renders_the_required_families_with_types() {
        let text = render(&full_snapshot());
        assert_well_formed(&text);
        for name in [
            "spc_process_uptime_seconds",
            "spc_metrics_scrape_errors_total",
            "spc_blocks_total",
            "spc_pending_liability_sats",
            "spc_ark_due_sats",
            "spc_ark_payouts_total",
            "spc_forfeited_sats_total",
            "spc_config_window_n",
            "spc_config_coinbase_threshold_sats",
            "spc_config_budget_floor_wu",
            "spc_config_budget_ceiling_wu",
            "spc_a_send_age_seconds",
            "spc_a_send_repeat_total",
            "spc_open_slice_shares",
            "spc_slices_sealed_total",
            "spc_distribution_slots",
            "spc_distribution_outputs_total",
            "spc_budget_wu",
            "spc_budget_utilisation",
            "spc_ark_overdue_total",
            "spc_ark_float_sats",
            "spc_ark_surplus_sats",
            "spc_ark_orders_rejected_total",
            "spc_channel_reject_total",
            "spc_jd_declaration_reject_total",
            "spc_ext32_requests_total",
            "spc_stream_lag_seconds",
            "spc_registration_lag_revs",
            "spc_orphaned_settlement_total",
        ] {
            assert!(
                text.contains(&format!("# TYPE {name} ")),
                "missing {name} in:\n{text}"
            );
        }
        assert!(text.contains("spc_blocks_total{status=\"confirmed\"} 2\n"));
        assert!(text.contains("spc_pending_liability_sats 123456\n"));
        assert!(text.contains("spc_forfeited_sats_total 1000\n"));
        assert!(!text.contains("not yet wired"));
        assert!(!text.contains("placeholder"));
        assert!(!text.contains("TODO"));
    }

    /// The per-front repeat counter keeps its `front_id` label (SPEC §9 via
    /// MULTI-REGION §10: the only labelled front metric) and renders a `0`
    /// sample, so the `> 0` alarm expression sees data, not absence.
    #[test]
    fn a_send_families_render_age_and_labelled_repeats() {
        let text = render(&full_snapshot());
        assert!(text.contains("spc_a_send_age_seconds 42.250\n"));
        assert!(text.contains("spc_a_send_repeat_total{front_id=\"0\"} 0\n"));
        assert!(text.contains("spc_a_send_repeat_total{front_id=\"1\"} 2\n"));
    }

    #[test]
    fn seal_reasons_and_distribution_classes_are_zero_filled() {
        let text = render(&full_snapshot());
        for reason in [
            "fee_band",
            "max_shares",
            "chain_moved",
            "distribution_change",
        ] {
            assert!(
                text.contains(&format!("spc_slices_sealed_total{{reason=\"{reason}\"}} ")),
                "missing reason {reason} in:\n{text}"
            );
        }
        assert!(text.contains("spc_slices_sealed_total{reason=\"chain_moved\"} 0\n"));
        for class in ["kept", "ark", "error"] {
            assert!(
                text.contains(&format!("spc_distribution_slots{{class=\"{class}\"}} ")),
                "missing class {class} in:\n{text}"
            );
        }
        assert!(text.contains("spc_distribution_slots{class=\"error\"} 0\n"));
        assert!(text.contains("spc_distribution_outputs_total 6\n"));
        assert!(text.contains("spc_budget_wu 400000\n"));
        assert!(text.contains("spc_budget_utilisation 0.0043\n"));
    }

    /// The `bad-signature` reason pages; it must render as an explicit `0`
    /// even when nothing was ever rejected for it.
    #[test]
    fn ark_role_families_zero_fill_the_paging_reason() {
        let snap = MetricsSnapshot {
            ark_float_sats: Some(42),
            ark_orders_rejected_by_reason: Some(vec![("transport".into(), 3)]),
            ..Default::default()
        };
        let text = render(&snap);
        assert_well_formed(&text);
        assert!(text.contains("spc_ark_float_sats 42\n"));
        assert!(text.contains("spc_ark_orders_rejected_total{reason=\"bad-signature\"} 0\n"));
        assert!(text.contains("spc_ark_orders_rejected_total{reason=\"transport\"} 3\n"));
        // No surplus source: the family is absent, not zero.
        assert!(!text.contains("spc_ark_surplus_sats"));
    }

    #[test]
    fn reject_counters_zero_fill_their_known_labels() {
        let text = render(&full_snapshot());
        assert!(text.contains("spc_channel_reject_total{reason=\"invalid-user-identity\"} 2\n"));
        for code in [
            "invalid-job",
            "invalid-payout-distribution",
            "stale-payout-distribution",
            "no-payout-distribution",
            "internal-error",
        ] {
            assert!(
                text.contains(&format!(
                    "spc_jd_declaration_reject_total{{code=\"{code}\"}} "
                )),
                "missing code {code} in:\n{text}"
            );
        }
        assert!(text.contains("spc_jd_declaration_reject_total{code=\"invalid-job\"} 1\n"));
        for msg in ["get_window", "get_shares", "get_transactions_in_job"] {
            assert!(
                text.contains(&format!("spc_ext32_requests_total{{msg=\"{msg}\"}} ")),
                "missing msg {msg} in:\n{text}"
            );
        }
    }

    /// Stream lag renders one labelled sample per stream whose lag is known,
    /// sorted; an unknown stream is simply absent.
    #[test]
    fn stream_lag_renders_known_streams_only() {
        let text = render(&full_snapshot());
        assert!(text.contains("spc_stream_lag_seconds{stream=\"spc:blocks\"} 1.500\n"));
        assert!(text.contains("spc_stream_lag_seconds{stream=\"spc:slices\"} 0.000\n"));
        let blocks = text.find("stream=\"spc:blocks\"").unwrap();
        let slices = text.find("stream=\"spc:slices\"").unwrap();
        assert!(blocks < slices, "stream samples must be sorted");
        assert!(!text.contains("spc:front:events"));
    }

    #[test]
    fn payout_sourced_families_render() {
        let text = render(&full_snapshot());
        assert!(text.contains("spc_ark_overdue_total 0\n"));
        assert!(text.contains("spc_registration_lag_revs 0\n"));
        assert!(text.contains("spc_orphaned_settlement_total{reason=\"competing_branch\"} 1\n"));
        assert!(text.contains("spc_orphaned_settlement_total{reason=\"missing_block_event\"} 0\n"));
    }

    #[test]
    fn omits_failed_sources_and_counts_them() {
        let snap = MetricsSnapshot {
            scrape_errors: 3,
            ..Default::default()
        };
        let text = render(&snap);
        assert_well_formed(&text);
        assert!(text.contains("spc_metrics_scrape_errors_total 3\n"));
        // A failed query omits its family entirely: absent means unknown,
        // not zero.
        assert!(!text.contains("spc_blocks_total"));
        assert!(!text.contains("spc_pending_liability_sats"));
        assert!(!text.contains("spc_ark_due_sats"));
        assert!(!text.contains("spc_ark_payouts_total"));
        assert!(!text.contains("spc_forfeited_sats_total"));
        assert!(!text.contains("spc_process_uptime_seconds"));
        assert!(!text.contains("spc_a_send_age_seconds"));
        assert!(!text.contains("spc_stream_lag_seconds"));
        assert!(!text.contains("spc_ark_float_sats"));
        assert!(!text.contains("spc_registration_lag_revs"));
    }

    #[test]
    fn zero_fills_the_schema_known_statuses() {
        let snap = MetricsSnapshot {
            blocks_by_status: Some(Vec::new()),
            ark_payouts_by_status: Some(Vec::new()),
            ..Default::default()
        };
        let text = render(&snap);
        assert_well_formed(&text);
        for status in ["pending", "confirmed", "orphaned"] {
            assert!(text.contains(&format!("spc_blocks_total{{status=\"{status}\"}} 0\n")));
        }
        for status in ["scheduled", "sent", "failed_to_pending"] {
            assert!(text.contains(&format!("spc_ark_payouts_total{{status=\"{status}\"}} 0\n")));
        }
    }

    #[test]
    fn passes_unexpected_statuses_through() {
        let snap = MetricsSnapshot {
            blocks_by_status: Some(vec![("weird".into(), 4), ("confirmed".into(), 1)]),
            ..Default::default()
        };
        let text = render(&snap);
        assert_well_formed(&text);
        assert!(text.contains("spc_blocks_total{status=\"weird\"} 4\n"));
        assert!(text.contains("spc_blocks_total{status=\"confirmed\"} 1\n"));
    }

    #[test]
    fn escapes_quotes_backslashes_and_newlines_in_label_values() {
        let snap = MetricsSnapshot {
            blocks_by_status: Some(vec![("we\"ird\\\nx".into(), 7)]),
            ..Default::default()
        };
        let text = render(&snap);
        assert_well_formed(&text);
        // `"` -> `\"`, `\` -> `\\`, line feed -> `\n`.
        assert!(
            text.lines()
                .any(|l| l == r#"spc_blocks_total{status="we\"ird\\\nx"} 7"#),
            "unescaped line in:\n{text}"
        );
    }

    #[test]
    fn process_uptime_is_plausible_or_absent() {
        if let Some(up) = process_uptime_secs() {
            assert!((0.0..60.0 * 60.0 * 24.0 * 365.0).contains(&up));
        }
    }
}
