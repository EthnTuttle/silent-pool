# Prometheus alert rules for a deployed spc-pool, encoding the normative
# alarm/page levels of docs/SPEC.md §9 "Monitoring (Prometheus)" and the
# per-family alarm column of docs/OPERATIONS.md §7 "Exported today".
#
# Severity labels:
#   page   - SPEC §9 marks the condition "page"; wake a human.
#   alarm  - SPEC §9 / OPERATIONS §7 mark the condition an alarm.
#   notice - OPERATIONS §7 trend-review conditions; no normative threshold.
#
# Families OPERATIONS §7 lists as "Planned (not exported in this build)"
# (spc_outbox_depth, spc_outbox_age_seconds, spc_tip_divergence_seconds) get
# no rule here: a rule on a never-emitted family can only mislead. Wire each
# as it lands. spc_ark_surplus_sats has no alarm level in SPEC §9 ("for
# operator tooling") and likewise gets no rule.
#
# Two thresholds mirror pool configuration values that are not exported as
# metrics; the module passes them through options:
#   floatTargetSats - [ark] float_target_sats; null omits the rule.
#   maxPayoutSlots  - [accounting] max_payout_slots (example config: 1024).
{ lib
, floatTargetSats ? null
, maxPayoutSlots ? 1024
, streamLagSeconds ? 60
}:
let
  spec9 = "docs/SPEC.md §9 Monitoring (Prometheus)";
  ops7 = "docs/OPERATIONS.md §7 Exported today";
  nearCap = builtins.floor (0.9 * maxPayoutSlots);
in
{
  groups = [
    {
      name = "spc-pool-page";
      rules = [
        {
          alert = "SpcArkOverdue";
          expr = "spc_ark_overdue_total > 0";
          labels.severity = "page";
          annotations = {
            summary = "Ark payouts scheduled past due_height + ark_retry_blocks";
            description = "{{ $value }} Ark payout(s) are overdue. SPEC §9: spc_ark_overdue_total (scheduled past due_height + ark_retry_blocks = page).";
            spec = spec9;
          };
        }
        {
          alert = "SpcArkOrderBadSignature";
          expr = "spc_ark_orders_rejected_total{reason=\"bad-signature\"} > 0";
          labels.severity = "page";
          annotations = {
            summary = "Ark order rejected with bad-signature";
            description = "The ark role rejected a send order for a bad signature; a forged or corrupted order reached it. SPEC §9: spc_ark_orders_rejected_total any bad-signature = page.";
            spec = spec9;
          };
        }
        {
          alert = "SpcPoolExporterDown";
          expr = "up{job=\"spc-pool\"} == 0";
          "for" = "3m";
          labels.severity = "page";
          annotations = {
            summary = "spc-pool /metrics unreachable";
            description = "The api-role exporter is the source of truth (OPERATIONS §7); while it is down every pool family is unknown, never known-empty.";
            spec = ops7;
          };
        }
      ];
    }
    {
      name = "spc-pool-alarm";
      rules = [
        {
          alert = "SpcASendStuck";
          expr = "spc_a_send_age_seconds > 3600";
          "for" = "5m";
          labels.severity = "alarm";
          annotations = {
            summary = "A_send older than 1 h - a stuck height";
            description = "spc_a_send_age_seconds = {{ $value }}s on the worst front. SPEC §9: alarm > 1 h.";
            spec = spec9;
          };
        }
        {
          alert = "SpcASendRepeat";
          expr = "spc_a_send_repeat_total > 0";
          labels.severity = "alarm";
          annotations = {
            summary = "A_send reuse on front {{ $labels.front_id }}";
            description = "spc_a_send_repeat_total{front_id} > 0: A_send reuse breaks unlinkability (MULTI-REGION §10). SPEC §9: alarm > 0.";
            spec = spec9;
          };
        }
        {
          alert = "SpcMetricsScrapeErrors";
          expr = "increase(spc_metrics_scrape_errors_total[15m]) > 0";
          labels.severity = "alarm";
          annotations = {
            summary = "Exporter store-query failures";
            description = "spc_metrics_scrape_errors_total grew: a metrics source query failed and its family was omitted from the scrape (missing means unknown, never empty).";
            spec = ops7;
          };
        }
        {
          alert = "SpcBlocksOrphanedOrPending";
          expr = "increase(spc_blocks_total{status=~\"orphaned|pending\"}[30m]) > 0";
          labels.severity = "alarm";
          annotations = {
            summary = "Block status {{ $labels.status }} growing";
            description = "spc_blocks_total{status={{ $labels.status }}} grew in the last 30m. OPERATIONS §7: alarm on any orphaned/pending growth.";
            spec = ops7;
          };
        }
        {
          alert = "SpcArkDueStalled";
          expr = "delta(spc_ark_due_sats[1h]) > 0 and increase(spc_ark_payouts_total{status=\"sent\"}[1h]) == 0";
          labels.severity = "alarm";
          annotations = {
            summary = "Ark due growing with no sent payouts";
            description = "spc_ark_due_sats grew over the last hour while spc_ark_payouts_total{status=sent} did not move. OPERATIONS §7: growth without sent payouts.";
            spec = ops7;
          };
        }
        {
          alert = "SpcArkPayoutFailedToPending";
          expr = "increase(spc_ark_payouts_total{status=\"failed_to_pending\"}[30m]) > 0";
          labels.severity = "alarm";
          annotations = {
            summary = "Ark payouts failing to pending";
            description = "spc_ark_payouts_total{status=failed_to_pending} grew: sends are failing and falling back to the error rail (SPEC §11.3).";
            spec = ops7;
          };
        }
        {
          alert = "SpcDistributionOutputsNearCap";
          expr = "spc_distribution_outputs_total >= ${toString nearCap}";
          labels.severity = "alarm";
          annotations = {
            summary = "Distribution outputs approaching max_payout_slots";
            description = "spc_distribution_outputs_total = {{ $value }} is at >= 90% of the configured max_payout_slots (${toString maxPayoutSlots}). SPEC §9: approaching max_payout_slots = alarm; an honest build cannot approach it since outputs <= T/rungs[0].";
            spec = spec9;
          };
        }
        {
          alert = "SpcDistributionAllError";
          expr = "(spc_distribution_slots{class=\"error\"} > 0) unless (sum(spc_distribution_slots{class=~\"kept|ark\"}) > 0)";
          "for" = "30m";
          labels.severity = "alarm";
          annotations = {
            summary = "Distribution slots sustained all-error";
            description = "Every distribution slot has been class=error for 30m with no kept/ark slots. OPERATIONS §7: sustained all-error.";
            spec = ops7;
          };
        }
        {
          alert = "SpcStreamLag";
          expr = "spc_stream_lag_seconds > ${toString streamLagSeconds}";
          "for" = "10m";
          labels.severity = "alarm";
          annotations = {
            summary = "Stream {{ $labels.stream }} lagging";
            description = "spc_stream_lag_seconds{stream={{ $labels.stream }}} = {{ $value }}s for 10m. OPERATIONS §7: rising per stream.";
            spec = ops7;
          };
        }
        {
          alert = "SpcRegistrationLag";
          expr = "spc_registration_lag_revs > 0";
          "for" = "30m";
          labels.severity = "alarm";
          annotations = {
            summary = "Registration revisions lagging";
            description = "spc_registration_lag_revs has been > 0 for 30m: a front is behind on ark_delivery propagation and settlements there error-hold (SPEC §11.1).";
            spec = ops7;
          };
        }
        {
          alert = "SpcOrphanedSettlementMissingBlockEvent";
          expr = "increase(spc_orphaned_settlement_total{reason=\"missing_block_event\"}[15m]) > 0";
          labels.severity = "alarm";
          annotations = {
            summary = "Settlement with no block event";
            description = "spc_orphaned_settlement_total{reason=missing_block_event} grew; reconstruct from the on-chain coinbase (SPEC §7.4).";
            spec = ops7;
          };
        }
        {
          alert = "SpcBudgetPinnedAtCeiling";
          expr = "min_over_time(spc_budget_utilisation[30m]) >= 1";
          labels.severity = "alarm";
          annotations = {
            summary = "Weight budget pinned at ceiling";
            description = "spc_budget_utilisation has not left the ceiling for 30m. OPERATIONS §7: pinned at ceiling.";
            spec = ops7;
          };
        }
      ] ++ lib.optional (floatTargetSats != null) {
        alert = "SpcArkFloatBelowTarget";
        expr = "spc_ark_float_sats < ${toString floatTargetSats}";
        "for" = "10m";
        labels.severity = "alarm";
        annotations = {
          summary = "Ark float below float_target_sats";
          description = "spc_ark_float_sats = {{ $value }} < configured float_target_sats (${toString floatTargetSats}). SPEC §9: refill needed; ark boards at the next matured pool output.";
          spec = spec9;
        };
      };
    }
    {
      name = "spc-pool-notice";
      rules = [
        {
          alert = "SpcForfeitedGrowth";
          expr = "increase(spc_forfeited_sats_total[6h]) > 0";
          labels.severity = "notice";
          annotations = {
            summary = "Forfeited sats growing";
            description = "spc_forfeited_sats_total grew by {{ $value }} sats in 6h. OPERATIONS §7: review beyond expected unregistered miners (SPEC §11.1 forfeit rail).";
            spec = ops7;
          };
        }
        {
          alert = "SpcChannelRejectsTrending";
          expr = "increase(spc_channel_reject_total[15m]) > 0";
          "for" = "30m";
          labels.severity = "notice";
          annotations = {
            summary = "Channel rejects trending ({{ $labels.reason }})";
            description = "spc_channel_reject_total{reason={{ $labels.reason }}} kept growing for 30m. OPERATIONS §7: identity-parse rejects trending up.";
            spec = ops7;
          };
        }
        {
          alert = "SpcJdDeclarationRejectsTrending";
          expr = "increase(spc_jd_declaration_reject_total[15m]) > 0";
          "for" = "30m";
          labels.severity = "notice";
          annotations = {
            summary = "JD declaration rejects trending (code {{ $labels.code }})";
            description = "spc_jd_declaration_reject_total{code={{ $labels.code }}} kept growing for 30m. OPERATIONS §7: any single code trending.";
            spec = ops7;
          };
        }
        {
          alert = "SpcArkOrdersRejectedOther";
          expr = "increase(spc_ark_orders_rejected_total{reason!=\"bad-signature\"}[30m]) > 0";
          labels.severity = "notice";
          annotations = {
            summary = "Ark orders rejected ({{ $labels.reason }})";
            description = "Non-signature order rejections grew; the page-level bad-signature case has its own rule.";
            spec = spec9;
          };
        }
      ];
    }
    {
      name = "spc-host";
      rules = [
        {
          alert = "SpcScrapeTargetDown";
          expr = "up{job!=\"spc-pool\"} == 0";
          "for" = "5m";
          labels.severity = "alarm";
          annotations = {
            summary = "Scrape target {{ $labels.job }} down";
            description = "{{ $labels.instance }} (job {{ $labels.job }}) has not answered its scrape for 5m. The pool exporter itself pages via SpcPoolExporterDown.";
            spec = ops7;
          };
        }
        {
          alert = "HostDiskSpaceLow";
          expr = "node_filesystem_avail_bytes{fstype!~\"tmpfs|ramfs|overlay\"} / node_filesystem_size_bytes < 0.10";
          "for" = "15m";
          labels.severity = "alarm";
          annotations = {
            summary = "Filesystem {{ $labels.mountpoint }} below 10% free";
            description = "A full disk stops Postgres, Redis AOF and the Prometheus TSDB; the pool fails closed but stops paying.";
            spec = "docs/OPERATIONS.md §3";
          };
        }
        {
          alert = "HostMemoryLow";
          expr = "node_memory_MemAvailable_bytes / node_memory_MemTotal_bytes < 0.05";
          "for" = "10m";
          labels.severity = "alarm";
          annotations = {
            summary = "Host below 5% available memory";
            description = "Sustained memory pressure on a small VPS precedes OOM kills of the pool roles.";
            spec = "docs/OPERATIONS.md §2";
          };
        }
      ];
    }
  ];
}
