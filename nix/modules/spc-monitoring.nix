# Deploy-grade monitoring for a public spc-pool host: loopback-only
# Prometheus + Alertmanager + Grafana, promoted from the lab wrapper
# (nix/lab/observability.nix) to production posture.
#
# Contract with the stack deployment: it does not import that module or read
# its config. The deployment (or the operator) feeds pool endpoints through
# `services.spc-monitoring.scrapeTargets`; the pool api role serves
# GET /metrics (docs/OPERATIONS.md §7) on a loopback port the deployment
# documents. Example:
#
#   services.spc-monitoring = {
#     enable = true;
#     scrapeTargets = [ { jobName = "spc-pool"; targets = [ "127.0.0.1:8080" ]; } ];
#     rbitcoinTarget = "127.0.0.1:9332";   # needs --health-listen + --metrics on the node
#     grafana.adminPasswordFile = "/run/secrets/grafana-admin-password";
#     grafana.secretKeyFile = "/run/secrets/grafana-secret-key";
#     alerts.floatTargetSats = 1000000;    # mirror [ark] float_target_sats
#   };
#
# Native endpoints of the other stack members (checked 2026-11-18):
#   - rbitcoin-node: native Prometheus GET /metrics behind `--health-listen
#     [ADDR]` (default 127.0.0.1:9332) plus `--metrics`; families are
#     prefixed `rbitcoin_*`. Set `rbitcoinTarget` to scrape it.
#   - captaind (bark server): no Prometheus endpoint. Its telemetry is
#     OpenTelemetry OTLP *push* (`otel_collector_endpoint` in
#     captaind.default.toml); scraping it would need an OTel collector with a
#     Prometheus exporter, which this module deliberately does not ship.
#     Skipped.
#
# Exposure: everything binds 127.0.0.1 and no firewall port is opened.
# Grafana has anonymous access disabled and takes its admin password from a
# secret file (never the Nix store). Reach it with an SSH tunnel:
#
#   ssh -N -L 3000:127.0.0.1:3000 operator@pool-host
#
# or front it with a TLS + auth reverse proxy (services.nginx virtualHost
# proxying to 127.0.0.1:<grafana.port>); set `grafana.domain` so Grafana
# renders correct links. Do not open Prometheus or Alertmanager at all: they
# are unauthenticated by design.
#
# Alerting: rules in ./prometheus-rules.nix encode SPEC §9's normative
# alarm/page thresholds (each rule cites the section in its annotations).
# Alertmanager starts receiver-agnostic: the default route drops into a null
# receiver so rules are evaluated and visible without paging anyone. Point
# `alertmanager.webhookUrl` at a webhook bridge (ntfy, matrix-hook, PagerDuty
# events API proxy) to deliver severity page/alarm, or override
# `services.prometheus.alertmanager.configuration` wholesale for email/SMTP -
# the module sets it with mkDefault.
{ config, lib, pkgs, ... }:
let
  cfg = config.services.spc-monitoring;
  inherit (lib) mkEnableOption mkOption mkIf mkDefault types;

  rules = import ./prometheus-rules.nix {
    inherit lib;
    inherit (cfg.alerts) floatTargetSats maxPayoutSlots streamLagSeconds;
  };
  # JSON is valid YAML; promtool accepts it (validated by the prometheus
  # module's build-time check and by the packet's promtool evidence).
  rulesFile = pkgs.writeText "spc-alerts.yml" (builtins.toJSON rules);

  dashboardsDir = ./grafana-dashboards;
in
{
  options.services.spc-monitoring = {
    enable = mkEnableOption "loopback Prometheus + Alertmanager + Grafana for a deployed spc-pool";

    scrapeTargets = mkOption {
      type = types.listOf (types.submodule {
        options = {
          jobName = mkOption {
            type = types.str;
            description = "Prometheus job name. The spc-pool exporter MUST be scraped as \"spc-pool\": dashboards and alert rules select on that job.";
          };
          targets = mkOption {
            type = types.listOf types.str;
            description = "host:port endpoints serving Prometheus text format.";
          };
          metricsPath = mkOption {
            type = types.str;
            default = "/metrics";
            description = "Scrape path.";
          };
          labels = mkOption {
            type = types.attrsOf types.str;
            default = { };
            description = "Static labels attached to every series of this job.";
          };
        };
      });
      default = [ ];
      example = lib.literalExpression ''
        [ { jobName = "spc-pool"; targets = [ "127.0.0.1:8080" ]; } ]
      '';
      description = ''
        Endpoints to scrape. This is the only coupling point with the stack
        deployment: it appends the pool api-role /metrics endpoint(s) here
        (docs/OPERATIONS.md §7). Loopback targets are expected; nothing
        here opens a port.
      '';
    };

    rbitcoinTarget = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "127.0.0.1:9332";
      description = ''
        host:port of an rbitcoin-node health listener started with
        --health-listen and --metrics (families rbitcoin_*). Scraped as job
        "rbitcoin" and shown on the node-health dashboard. null skips it.
        captaind has no Prometheus endpoint (OTLP push only) and is not
        scrapeable here.
      '';
    };

    prometheus = {
      port = mkOption {
        type = types.port;
        default = 9090;
        description = "Prometheus loopback port.";
      };
      scrapeInterval = mkOption {
        type = types.str;
        default = "15s";
        description = "Global scrape interval (the role metric snapshots refresh every 15 s, docs/OPERATIONS.md §7).";
      };
      retention = {
        time = mkOption {
          type = types.str;
          default = "15d";
          description = "TSDB time retention.";
        };
        size = mkOption {
          type = types.str;
          default = "2GB";
          description = "TSDB size cap; whichever of time/size trips first wins. Sized for a small VPS.";
        };
      };
    };

    alertmanager = {
      enable = mkOption {
        type = types.bool;
        default = true;
        description = "Run Alertmanager (loopback). Rules are evaluated either way.";
      };
      port = mkOption {
        type = types.port;
        default = 9093;
        description = "Alertmanager loopback port.";
      };
      webhookUrl = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "http://127.0.0.1:8787/alert";
        description = ''
          Deliver alerts to this webhook. null keeps the null receiver:
          alerts stay visible in Prometheus/Grafana but page nobody. For
          email or anything richer, override
          services.prometheus.alertmanager.configuration (set here with
          mkDefault).
        '';
      };
    };

    grafana = {
      port = mkOption {
        type = types.port;
        default = 3000;
        description = "Grafana loopback port (SSH tunnel or reverse proxy; never opened).";
      };
      adminPasswordFile = mkOption {
        type = types.path;
        description = ''
          File containing the Grafana admin password, readable by the
          grafana user (docs/OPERATIONS.md §4 secrets posture: root-owned
          secret dir, never the Nix store or the repo). No default:
          enabling the module without a real secret must fail evaluation
          rather than ship a known password.
        '';
      };
      secretKeyFile = mkOption {
        type = types.path;
        description = ''
          File containing Grafana's secret_key (database secret encryption;
          the nixpkgs module refuses the historical default). Same posture
          as adminPasswordFile: root-owned secret readable by the grafana
          user, e.g. generated once with `openssl rand -hex 32`. No default.
        '';
      };
      domain = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "grafana.pool.example";
        description = "Public domain when fronted by a reverse proxy; fixes rendered links. null for tunnel-only use.";
      };
    };

    alerts = {
      floatTargetSats = mkOption {
        type = types.nullOr types.ints.unsigned;
        default = null;
        description = ''
          Mirror of the pool's [ark] float_target_sats; the config value is
          not exported as a metric, so the SPEC §9 "spc_ark_float_sats below
          float_target_sats" alarm needs it here. null omits that one rule
          (document why if you leave it off on an ark-enabled deployment).
        '';
      };
      maxPayoutSlots = mkOption {
        type = types.ints.positive;
        default = 1024;
        description = "Mirror of [accounting] max_payout_slots (example config default 1024). The SPEC §9 approaching-cap alarm fires at 90% of it.";
      };
      streamLagSeconds = mkOption {
        type = types.ints.positive;
        default = 60;
        description = "spc_stream_lag_seconds alarm threshold; SPEC §9 says \"rising per stream\" without a number, so this one is operator policy.";
      };
    };

    nodeExporter = {
      enable = mkOption {
        type = types.bool;
        default = true;
        description = "Run node_exporter on loopback for the node-health dashboard.";
      };
      port = mkOption {
        type = types.port;
        default = 9100;
        description = "node_exporter loopback port.";
      };
    };
  };

  config = mkIf cfg.enable {
    services.prometheus = {
      enable = true;
      listenAddress = "127.0.0.1";
      port = cfg.prometheus.port;
      retentionTime = cfg.prometheus.retention.time;
      extraFlags = [ "--storage.tsdb.retention.size=${cfg.prometheus.retention.size}" ];
      globalConfig.scrape_interval = cfg.prometheus.scrapeInterval;
      ruleFiles = [ rulesFile ];

      scrapeConfigs =
        [{
          job_name = "prometheus";
          static_configs = [{ targets = [ "127.0.0.1:${toString cfg.prometheus.port}" ]; }];
        }]
        ++ lib.optional cfg.nodeExporter.enable {
          job_name = "node";
          static_configs = [{ targets = [ "127.0.0.1:${toString cfg.nodeExporter.port}" ]; }];
        }
        ++ lib.optional (cfg.rbitcoinTarget != null) {
          job_name = "rbitcoin";
          static_configs = [{ targets = [ cfg.rbitcoinTarget ]; }];
        }
        ++ map
          (t: {
            job_name = t.jobName;
            metrics_path = t.metricsPath;
            static_configs = [{
              inherit (t) targets;
              labels = t.labels;
            }];
          })
          cfg.scrapeTargets;

      alertmanagers = lib.optional cfg.alertmanager.enable {
        static_configs = [{ targets = [ "127.0.0.1:${toString cfg.alertmanager.port}" ]; }];
      };

      alertmanager = mkIf cfg.alertmanager.enable {
        enable = true;
        listenAddress = "127.0.0.1";
        port = cfg.alertmanager.port;
        # Receiver-agnostic default: evaluate and show everything, page
        # nobody. mkDefault so an operator config can replace it wholesale
        # (email/SMTP, routing trees).
        configuration = mkDefault (
          if cfg.alertmanager.webhookUrl != null then {
            route = {
              receiver = "webhook";
              group_by = [ "alertname" "severity" ];
              group_wait = "30s";
              group_interval = "5m";
              repeat_interval = "4h";
            };
            receivers = [{
              name = "webhook";
              webhook_configs = [{ url = cfg.alertmanager.webhookUrl; }];
            }];
          } else {
            route.receiver = "null";
            receivers = [{ name = "null"; }];
          }
        );
      };

      exporters.node = mkIf cfg.nodeExporter.enable {
        enable = true;
        listenAddress = "127.0.0.1";
        port = cfg.nodeExporter.port;
        enabledCollectors = [ "systemd" ];
      };
    };

    services.grafana = {
      enable = true;
      settings = {
        server = {
          http_addr = "127.0.0.1";
          http_port = cfg.grafana.port;
        } // lib.optionalAttrs (cfg.grafana.domain != null) {
          domain = cfg.grafana.domain;
          root_url = "https://${cfg.grafana.domain}/";
        };
        security = {
          admin_password = "$__file{${cfg.grafana.adminPasswordFile}}";
          secret_key = "$__file{${cfg.grafana.secretKeyFile}}";
        };
        "auth.anonymous".enabled = false;
        analytics = {
          reporting_enabled = false;
          check_for_updates = false;
          check_for_plugin_updates = false;
          feedback_links_enabled = false;
        };
        news.news_feed_enabled = false;
      };
      provision = {
        enable = true;
        datasources.settings = {
          apiVersion = 1;
          datasources = [{
            name = "spc-prometheus";
            uid = "spc-prometheus";
            type = "prometheus";
            access = "proxy";
            url = "http://127.0.0.1:${toString cfg.prometheus.port}";
            isDefault = true;
            editable = false;
          }];
        };
        dashboards.settings = {
          apiVersion = 1;
          providers = [{
            name = "spc";
            folder = "Silent Pool";
            type = "file";
            allowUiUpdates = false;
            disableDeletion = true;
            options.path = dashboardsDir;
          }];
        };
      };
    };
  };
}
