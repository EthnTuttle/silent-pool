This packet makes no normative SPEC change: no `changelog:` bullet to splice.
This fragment carries the packet's merge notes for merge-session-owned files.
Branch: `wp-deploy-monitoring` (base `9cb15f6`). Packet-owned files:
`nix/modules/spc-monitoring.nix`, `nix/modules/prometheus-rules.nix`,
`nix/modules/spc-monitoring-test.nix`, `nix/modules/grafana-dashboards/*.json`,
this fragment. The lab wrapper (`nix/lab/observability.nix`,
`nix/lab/grafana/dashboards/*`) is untouched and keeps working.

## Merge note 1: flake.nix (merge-session-owned)

Register the module next to the existing nixosModules entries. The module
takes no `self` argument (it ships nothing from this flake's packages; the
pool endpoint arrives through `services.spc-monitoring.scrapeTargets`):

```nix
        nixosModules = {
          spc-pool = import ./nix/modules/spc-pool.nix { inherit self; };
          spc-agent = import ./nix/modules/spc-agent.nix { inherit self; };
          spc-stack = import ./nix/modules/spc-stack.nix { inherit self; };
          spc-monitoring = import ./nix/modules/spc-monitoring.nix;
          default = self.nixosModules.spc-stack;
        };
```

Optional (merge session's call): expose the eval harness so the gate covers
module regressions without a deploy:

```nix
        nixosConfigurations.spc-monitoring-test =
          import ./nix/modules/spc-monitoring-test.nix { inherit nixpkgs; };
```

## Merge note 2: docs/OPERATIONS.md section 7 (merge-session-owned)

Append after the sentence "`GET /healthz` is the liveness probe." at the end
of section 7:

```markdown
### Deploy monitoring (NixOS)

`nix/modules/spc-monitoring.nix` ships the deploy-grade wrapper for the
tables above: loopback-only Prometheus + Alertmanager + Grafana, the
provisioned operator and node-health dashboards
(`nix/modules/grafana-dashboards/`), and alert rules encoding this section's
alarm/page levels with their SPEC §9 citations
(`nix/modules/prometheus-rules.nix`). The pool `/metrics` endpoint is fed in
through `services.spc-monitoring.scrapeTargets` (job name `spc-pool`); the
testnet5 rbitcoin node's native Prometheus endpoint (`--health-listen` +
`--metrics`) through `rbitcoinTarget`. captaind exposes no Prometheus
endpoint (OTLP push only) and is not scraped. Two alarm thresholds mirror
pool config values that are not exported as metrics - set
`alerts.floatTargetSats` (= `[ark] float_target_sats`) and
`alerts.maxPayoutSlots` (= `[accounting] max_payout_slots`) to match the
deployed `spc-pool.toml`. Grafana is loopback with anonymous access off and
its admin password/secret key from secret files (§4 posture); reach it over
an SSH tunnel or an operator-supplied TLS reverse proxy. Alertmanager
defaults to a null receiver; `alertmanager.webhookUrl` or an operator
override of `services.prometheus.alertmanager.configuration` delivers pages.
The lab wrapper (`nix/lab/observability.nix`) is unchanged and remains the
regtest-lab path.
```

## Merge note 3: docs/DEPLOY-TESTNET5.md (owned by wp-deploy-testnet5)

The sibling packet owns that file; add this as its monitoring section (adjust
the section number to its structure):

```markdown
## Monitoring

Import `nixosModules.spc-monitoring` (flake) or
`nix/modules/spc-monitoring.nix` directly and feed it the stack's documented
loopback pool API port:

    services.spc-monitoring = {
      enable = true;
      scrapeTargets = [ { jobName = "spc-pool"; targets = [ "127.0.0.1:<api-port>" ]; } ];
      rbitcoinTarget = "127.0.0.1:9332";   # start the node with --health-listen --metrics
      grafana.adminPasswordFile = "/run/secrets/grafana-admin-password";
      grafana.secretKeyFile = "/run/secrets/grafana-secret-key";
      alerts.floatTargetSats = <[ark] float_target_sats>;
      alerts.maxPayoutSlots = <[accounting] max_payout_slots>;
    };

Everything binds 127.0.0.1 and no firewall port opens: Grafana (default
:3000, admin login only, password from the secret file) is reached with
`ssh -N -L 3000:127.0.0.1:3000 operator@host` or an operator-supplied TLS +
auth reverse proxy (`grafana.domain` fixes rendered links); Prometheus
(:9090) and Alertmanager (:9093) stay tunnel-only - they are unauthenticated
by design. Dashboards: "SPc pool - operator" (every exported family of
OPERATIONS §7) and "SPc host and node health" (node_exporter + the node's
`rbitcoin_*` families). Alert rules encode SPEC §9's alarm/page thresholds;
Alertmanager starts with a null receiver, so set `alertmanager.webhookUrl`
(or override `services.prometheus.alertmanager.configuration`) before
relying on pages. captaind has no Prometheus endpoint (OTLP push only) and
is not monitored here. Prometheus retention defaults to 15d/2GB for a small
VPS.
```
