# Opt-in loopback Grafana + Prometheus for a running regtest lab.
#
# This wrapper is never launched by spc-lab or the self-hosted Ark wrapper;
# an operator starts it alongside an already-started run. It renders the
# Prometheus scrape configuration from the run's actual allocated pool
# api-role listeners (`resources.listeners` keys `front.<id>.api` in the
# run's manifest - the same allocated-endpoint registry the lab's own config
# rendering consumes), so dynamic `127.0.0.1:0` scenario ports need no
# hardcoding. Only those loopback host:port pairs are copied out of the
# manifest; no credential-bearing field is read or re-emitted.
#
# All listeners are loopback-only and all state is ephemeral run-directory
# state under `<run>/observability` (created only after the run manifest
# exists, so a fresh lab run directory stays fresh), matching how the lab
# treats its Postgres/Redis datadirs. Grafana analytics, update checks and
# the news feed are disabled: no external call leaves the host. Anonymous
# admin access is enabled - acceptable only because the listener is loopback
# on a single-operator lab machine.
{ pkgs }:
let
  dashboardsDir = ./grafana/dashboards;

  renderTargets = pkgs.writeText "spc-lab-render-prometheus-targets.py" ''
    """Render a Prometheus file_sd target list from a lab run manifest.

    Reads ONLY `resources.listeners` entries matching `front.<id>.api` and
    emits [{"targets": ["host:port"], "labels": {"front": "<id>"}}]. The run
    manifest is private (it can carry storage URLs); nothing else is copied.
    Exit codes: 0 rendered, 3 manifest unreadable or no API listener yet
    (caller retries), 1 malformed listener, 2 usage.
    """
    import json
    import os
    import re
    import sys


    def main() -> int:
        if len(sys.argv) != 3:
            print("usage: render-targets.py <run-manifest.json> <targets.json>",
                  file=sys.stderr)
            return 2
        try:
            with open(sys.argv[1]) as handle:
                manifest = json.load(handle)
        except (OSError, ValueError) as error:
            print(f"run manifest not readable yet: {error}", file=sys.stderr)
            return 3
        listeners = (manifest.get("resources") or {}).get("listeners") or {}
        groups = []
        for key in sorted(listeners):
            match = re.fullmatch(r"front\.(.+)\.api", key)
            if not match:
                continue
            endpoint = listeners[key]
            host = endpoint.get("host")
            port = endpoint.get("port")
            if not isinstance(host, str) or not isinstance(port, int):
                print(f"listener {key} has no host/port", file=sys.stderr)
                return 1
            if host not in ("127.0.0.1", "::1", "localhost"):
                # v1 scrapes host-profile loopback listeners only; a
                # namespaced topology run publishes addresses Prometheus on
                # the host cannot reach, so skip rather than fail the rest.
                print(f"skipping non-loopback listener {key} ({host})",
                      file=sys.stderr)
                continue
            groups.append({
                "targets": [f"{host}:{port}"],
                "labels": {"front": match.group(1)},
            })
        if not groups:
            print("no front API listener in the run manifest yet",
                  file=sys.stderr)
            return 3
        tmp = sys.argv[2] + ".tmp"
        with open(tmp, "w") as handle:
            json.dump(groups, handle, indent=2)
            handle.write("\n")
        os.replace(tmp, sys.argv[2])
        print(f"rendered {len(groups)} scrape target group(s)")
        return 0


    if __name__ == "__main__":
        sys.exit(main())
  '';

  waitReady = pkgs.writeText "spc-lab-observability-wait-ready.py" ''
    """Poll the given loopback URLs until all answer 2xx or 60s pass."""
    import sys
    import time
    import urllib.request

    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    deadline = time.monotonic() + 60
    pending = list(sys.argv[1:])
    while pending and time.monotonic() < deadline:
        for url in list(pending):
            try:
                with opener.open(url, timeout=2) as response:
                    if 200 <= response.status < 300:
                        pending.remove(url)
            except Exception:
                pass
        if pending:
            time.sleep(1)
    if pending:
        print("not ready after 60s: " + ", ".join(pending), file=sys.stderr)
        sys.exit(1)
  '';
in
pkgs.writeShellApplication {
  name = "spc-lab-observability";
  runtimeInputs = [ pkgs.coreutils pkgs.python3 pkgs.prometheus pkgs.grafana ];
  text = ''
    usage() {
      cat <<'EOF'
    spc-lab-observability --run-dir <dir> [options]

    Launch a loopback-only Prometheus + Grafana pair scraping the pool
    api-role /metrics endpoints of an already-started lab run. State lives
    under <run-dir>/observability and dies with the run directory.

    Options:
      --run-dir <dir>              lab run directory (required)
      --state-dir <dir>            override state dir (default <run-dir>/observability)
      --prometheus-port <port>     Prometheus listener (default 19464)
      --grafana-port <port>        Grafana listener (default 13000)
      --scrape-interval-secs <n>   scrape interval (default 5)
      --wait-secs <n>              wait budget for the run manifest (default 120)
    EOF
    }

    run_dir=""
    state_dir=""
    prometheus_port=19464
    grafana_port=13000
    scrape_interval=5
    wait_secs=120
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --run-dir) run_dir="$2"; shift 2 ;;
        --state-dir) state_dir="$2"; shift 2 ;;
        --prometheus-port) prometheus_port="$2"; shift 2 ;;
        --grafana-port) grafana_port="$2"; shift 2 ;;
        --scrape-interval-secs) scrape_interval="$2"; shift 2 ;;
        --wait-secs) wait_secs="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
      esac
    done
    if [ -z "$run_dir" ]; then
      echo "--run-dir is required" >&2
      usage >&2
      exit 2
    fi

    umask 077
    manifest="$run_dir/run-manifest.json"

    # Wait for the run to exist before creating anything inside its
    # directory: a fresh lab run must see a fresh run dir.
    waited=0
    until [ -s "$manifest" ]; do
      if [ "$waited" -ge "$wait_secs" ]; then
        echo "no run manifest at $manifest after ''${wait_secs}s; start the lab run first" >&2
        exit 1
      fi
      sleep 1
      waited=$((waited + 1))
    done

    state="''${state_dir:-$run_dir/observability}"
    mkdir -p "$state/prometheus/data" "$state/grafana/data" \
      "$state/grafana/logs" "$state/grafana/plugins" \
      "$state/grafana/provisioning/datasources" \
      "$state/grafana/provisioning/dashboards" "$state/logs"

    # Render the scrape targets from the run's actual allocated listeners.
    targets_file="$state/prometheus/targets.json"
    waited=0
    while true; do
      status=0
      python3 ${renderTargets} "$manifest" "$targets_file" || status=$?
      if [ "$status" -eq 0 ]; then break; fi
      if [ "$status" -ne 3 ]; then
        echo "scrape target rendering failed (exit $status)" >&2
        exit "$status"
      fi
      if [ "$waited" -ge "$wait_secs" ]; then
        echo "run manifest never published a front API listener" >&2
        exit 1
      fi
      sleep 2
      waited=$((waited + 2))
    done

    cat > "$state/prometheus/prometheus.yml" <<EOF
    global:
      scrape_interval: ''${scrape_interval}s
    scrape_configs:
      - job_name: spc-pool
        metrics_path: /metrics
        file_sd_configs:
          - files:
              - '$targets_file'
            refresh_interval: 15s
    EOF

    cat > "$state/grafana/provisioning/datasources/spc-lab.yaml" <<EOF
    apiVersion: 1
    datasources:
      - name: spc-lab-prometheus
        uid: spc-lab-prometheus
        type: prometheus
        access: proxy
        url: http://127.0.0.1:$prometheus_port
        isDefault: true
        editable: false
    EOF

    cat > "$state/grafana/provisioning/dashboards/spc-lab.yaml" <<EOF
    apiVersion: 1
    providers:
      - name: spc-lab
        folder: Silent Pool Lab
        type: file
        allowUiUpdates: false
        options:
          path: ${dashboardsDir}
    EOF

    cat > "$state/grafana/grafana.ini" <<EOF
    [server]
    http_addr = 127.0.0.1
    http_port = $grafana_port

    [paths]
    data = $state/grafana/data
    logs = $state/grafana/logs
    plugins = $state/grafana/plugins
    provisioning = $state/grafana/provisioning

    [analytics]
    reporting_enabled = false
    check_for_updates = false
    check_for_plugin_updates = false
    feedback_links_enabled = false

    [news]
    news_feed_enabled = false

    [auth]
    disable_login_form = true

    [auth.anonymous]
    enabled = true
    org_role = Admin

    [security]
    cookie_secure = false
    EOF

    prom_pid=""
    graf_pid=""
    cleanup() {
      for pid in "$prom_pid" "$graf_pid"; do
        if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
          kill "$pid" 2>/dev/null || true
        fi
      done
      wait 2>/dev/null || true
    }
    trap cleanup EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM

    prometheus \
      --config.file="$state/prometheus/prometheus.yml" \
      --storage.tsdb.path="$state/prometheus/data" \
      --storage.tsdb.retention.time=2d \
      --web.listen-address="127.0.0.1:$prometheus_port" \
      > "$state/logs/prometheus.log" 2>&1 &
    prom_pid=$!

    grafana server \
      --homepath ${pkgs.grafana}/share/grafana \
      --config "$state/grafana/grafana.ini" \
      > "$state/logs/grafana.log" 2>&1 &
    graf_pid=$!

    python3 ${waitReady} \
      "http://127.0.0.1:$prometheus_port/-/ready" \
      "http://127.0.0.1:$grafana_port/api/health"

    echo "spc-lab-observability: prometheus=http://127.0.0.1:$prometheus_port"
    echo "spc-lab-observability: grafana=http://127.0.0.1:$grafana_port (anonymous admin, loopback only)"
    echo "spc-lab-observability: dashboards: 'SPc pool - exported metrics' (live families)"
    echo "spc-lab-observability:             'SPc pool - planned metrics' (empty until instrumented)"
    echo "spc-lab-observability: Ctrl-C to stop; state under $state"

    wait -n "$prom_pid" "$graf_pid" || true
    echo "spc-lab-observability: a process exited; shutting down" >&2
  '';
}
