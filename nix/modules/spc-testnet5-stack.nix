# The public Testnet5 pool host, in one composable module:
#
#   rbitcoin Testnet5 node  (public P2P, loopback RPC, cookie auth)
#   captaind-testnet5       (self-hosted Ark server, loopback gRPC, own DB)
#   spc-pool                (roles front,payout,api,ark in ONE process -
#                            SPEC §9 dev/testnet topology; ark co-location
#                            and server_is_self are spec-legal on testnet5
#                            per decision 2026-11-17-bark-testnet5-fork.md)
#   PostgreSQL + Redis      (the pool's durable state, SPEC §9)
#   nginx                   (public READ-ONLY API; /metrics stays loopback)
#
# Listener contract (documented for the monitoring packet):
#   SV2  0.0.0.0:34254      public (Noise)
#   SV1  0.0.0.0:3333       public (plain TCP)
#   JDP  127.0.0.1:34264    loopback (open it separately for remote JDCs)
#   API  127.0.0.1:8080     loopback; nginx publishes it read-only and
#                           REFUSES /metrics
#   METRICS                 http://127.0.0.1:8080/metrics (Prometheus
#                           0.0.4, served by the api role). The monitoring
#                           packet scrapes THIS loopback endpoint; nothing
#                           here builds dashboards.
#
# Chain-identity initialization order (SPEC §2.1/§5.1/§5.3): the payout
# role alone initializes the SQL and Redis chain bindings
# (Store::initialize_chain, bind_redis_namespace(initialize=true)); every
# other role only verifies them. This stack runs all four roles in one
# process whose startup sequence runs payout's initialization before the
# front/api consumers, so ordering is inherent - and the module asserts
# "payout" stays in the roles list. If roles are ever split into separate
# processes/hosts, the payout process MUST start (and bind) first.
{ self }:
{ config, lib, pkgs, options, ... }:
let
  cfg = config.services.spc-testnet5-stack;
  inherit (lib) mkEnableOption mkOption mkIf types;
in {
  imports = [
    (import ./spc-pool.nix { inherit self; })
    (import ./rbitcoin-testnet5-node.nix { inherit self; })
    (import ./captaind-testnet5.nix { inherit self; })
  ];

  options.services.spc-testnet5-stack = {
    enable = mkEnableOption "public silent-pool Testnet5 stack (node + captaind + pool + nginx)";

    poolConfigFile = mkOption {
      type = types.str;
      default = "/etc/spc/pool.toml";
      description = ''
        Operator-provisioned spc-pool TOML (never in the Nix store: it
        carries the Noise authority secret and node RPC credentials).
        Start from nix/deploy/testnet5/pool.toml.example; the listener
        addresses there are this module's contract (SV2 0.0.0.0:34254,
        SV1 0.0.0.0:3333, API 127.0.0.1:8080). Mode 0640 root:spc-pool.
      '';
    };

    rpcCookieFile = mkOption {
      type = types.str;
      default = "/etc/spc/secrets/rbitcoin-rpc.cookie";
      description = "Node RPC cookie (user:pass, 0640 root:spc-rpc), shared by node/captaind; pool.toml carries the same values.";
    };

    domain = mkOption {
      type = types.nullOr types.str;
      default = null;
      description = ''
        Public domain for the read-only API vhost. null serves the API on
        plain HTTP port 80 (default vhost) - fine for a first soak;
        set a domain (and acme) for TLS.
      '';
    };

    acme = {
      enable = mkOption {
        type = types.bool;
        default = false;
        description = "Let's Encrypt TLS for the API vhost (requires domain and acmeEmail).";
      };
      email = mkOption {
        type = types.nullOr types.str;
        default = null;
        description = "ACME account email.";
      };
    };

    metricsPort = mkOption {
      type = types.port;
      default = 8080;
      readOnly = true;
      description = ''
        Loopback metrics contract: http://127.0.0.1:8080/metrics (the api
        role's listener; pool.toml [api] listen must match). Read-only
        documentation of the contract, hence fixed.
      '';
    };

    openFirewall = mkOption {
      type = types.bool;
      default = true;
      description = "Open SV2 (34254), SV1 (3333), HTTP(S) and the node P2P port.";
    };
  };

  config = mkIf cfg.enable (lib.mkMerge [ {
    assertions = [
      {
        assertion = lib.elem "payout" config.services.spc-pool.roles;
        message = "spc-testnet5-stack: the roles list must include \"payout\" - it alone initializes the SQL/Redis chain-identity bindings the other roles verify (SPEC §5.1/§5.3).";
      }
      {
        assertion = lib.elem "ark" config.services.spc-pool.roles;
        message = "spc-testnet5-stack: the roles list must include \"ark\" - testnet5 has no third-party Ark server, the co-located ark role is the payout rail (decision 2026-11-17-bark-testnet5-fork.md).";
      }
      {
        assertion = cfg.acme.enable -> (cfg.domain != null && cfg.acme.email != null);
        message = "spc-testnet5-stack: acme.enable requires both domain and acme.email.";
      }
    ];

    # --- node + ark server -------------------------------------------------
    services.rbitcoin-testnet5 = {
      enable = true;
      rpcCookieFile = cfg.rpcCookieFile;
      openFirewall = cfg.openFirewall;
    };

    services.captaind-testnet5 = {
      enable = true;
      bitcoind.cookieFile = cfg.rpcCookieFile;
    };

    # --- pool durable state (SPEC §9: Postgres + Redis only) ---------------
    # Role name == database name == system user, so local socket peer auth
    # works with ensureDBOwnership and no password material exists.
    services.postgresql = {
      enable = lib.mkDefault true;
      ensureDatabases = [ "spc-pool" ];
      ensureUsers = [{
        name = "spc-pool";
        ensureDBOwnership = true;
      }];
    };

    services.redis.servers.spc = {
      enable = lib.mkDefault true;
      port = lib.mkDefault 6379;      # loopback bind (NixOS default)
      save = [ ];
      appendOnly = true;
      appendFsync = "everysec";
    };

    # --- the pool itself ----------------------------------------------------
    services.spc-pool = {
      enable = true;
      # rpc + ark features: template source "rpc" and the stock bark-wallet
      # ark transport (SPEC §11.10) are both required on testnet5.
      package = self.packages.${pkgs.system}.spc-pool-ark;
      roles = [ "front" "payout" "api" "ark" ];
      configFile = cfg.poolConfigFile;
      # Firewall handled below: the module option would also open the API
      # port, which must stay loopback behind nginx.
      openFirewall = false;
    };

    systemd.services.spc-pool = {
      after = [
        "redis-spc.service"
        "rbitcoin-testnet5.service"
        "captaind-testnet5.service"
      ];
      wants = [
        "redis-spc.service"
        "rbitcoin-testnet5.service"
        "captaind-testnet5.service"
      ];
      # Fail fast with a pointer instead of a parse error when the
      # operator config is not provisioned yet.
      preStart = ''
        if [ ! -r ${lib.escapeShellArg cfg.poolConfigFile} ]; then
          echo "spc-pool: ${cfg.poolConfigFile} is missing or unreadable." >&2
          echo "Provision it first (docs/DEPLOY-TESTNET5.md, 'Secrets')." >&2
          exit 1
        fi
      '';
    };

    # --- public read-only API; /metrics stays loopback ----------------------
    services.nginx = {
      enable = true;
      recommendedProxySettings = true;
      virtualHosts.${if cfg.domain != null then cfg.domain else "_"} = {
        default = cfg.domain == null;
        enableACME = cfg.acme.enable;
        forceSSL = cfg.acme.enable;
        locations = {
          # The metrics contract is loopback-only (monitoring packet
          # scrapes 127.0.0.1:8080/metrics directly). Refuse it here so
          # the public vhost can never leak it.
          "= /metrics".extraConfig = "return 404;";
          "/" = {
            proxyPass = "http://127.0.0.1:${toString cfg.metricsPort}";
            extraConfig = ''
              # The API is read-only plus the POST /api/sp/challenge
              # auth bootstrap; refuse other write verbs at the edge.
              limit_except GET HEAD POST { deny all; }
            '';
          };
        };
      };
    };

    security.acme = mkIf cfg.acme.enable {
      acceptTerms = true;
      defaults.email = cfg.acme.email;
    };

    networking.firewall.allowedTCPPorts = lib.optionals cfg.openFirewall (
      [
        config.services.spc-pool.sv2Port
        config.services.spc-pool.sv1Port
        80
      ]
      ++ lib.optional cfg.acme.enable 443
    );
  }
  # --- monitoring packet feed (services.spc-monitoring) --------------------
  # Takes effect only when that packet's module is imported too; the option
  # set is probed (optionalAttrs, not mkIf: the key must vanish entirely
  # when absent) so this stack does not hard-depend on it. Job name
  # "spc-pool" is the monitoring packet's contract (its dashboards and
  # alert rules select on it); rbitcoinTarget is the node's health
  # listener (--health-listen + --metrics, rbitcoin_* families).
  (lib.optionalAttrs (options.services ? spc-monitoring) {
    services.spc-monitoring = {
      scrapeTargets = [{
        jobName = "spc-pool";
        targets = [ "127.0.0.1:${toString cfg.metricsPort}" ];
      }];
      rbitcoinTarget = lib.mkDefault config.services.rbitcoin-testnet5.healthListen;
    };
  }) ]);
}
