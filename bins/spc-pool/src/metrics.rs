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
