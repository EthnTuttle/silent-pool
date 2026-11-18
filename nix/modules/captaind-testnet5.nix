# Self-hosted captaind (bark server) on Testnet5, from the adopted
# network-identity-only fork (nix/lab/ark-testnet5.nix; decision
# docs/decisions/2026-11-17-bark-testnet5-fork.md). Self-hosting is
# spec-legal on testnet5 (SPEC §11.3/§11.9: no third-party Ark server
# exists there); the trust posture is sender-equals-server and is a
# test-network posture only.
#
# The config is generated from the fork's pinned captaind.default.toml
# (vendored byte-exact at nix/deploy/testnet5/captaind.defaults.toml;
# captaind requires a complete config file) with deploy overrides merged
# on top. `captaind check-config` validates the rendered file against the
# actual binary before every start, so drift between the vendored
# defaults and a future re-pin fails loudly, not silently.
{ self }:
{ config, lib, pkgs, ... }:
let
  cfg = config.services.captaind-testnet5;
  inherit (lib) mkEnableOption mkOption mkIf types;
  tomlFormat = pkgs.formats.toml { };
  defaults = builtins.fromTOML (builtins.readFile ../deploy/testnet5/captaind.defaults.toml);
  rendered = lib.recursiveUpdate (lib.recursiveUpdate defaults {
    data_dir = "${cfg.dataDir}/data";
    network = "testnet5";
    rpc = {
      public_address = cfg.publicAddress;
      admin_address = cfg.adminAddress;
      integration_address = cfg.integrationAddress;
    };
    postgres = {
      host = cfg.postgres.host;
      port = cfg.postgres.port;
      name = cfg.postgres.database;
      user = cfg.postgres.user;
      max_connections = 10;
    };
    bitcoind = {
      url = cfg.bitcoind.url;
      cookie = cfg.bitcoind.cookieFile;
    };
    otel_deployment_name = "spc_testnet5";
  }) cfg.settings;
  configFile = tomlFormat.generate "captaind-testnet5.toml" rendered;
in {
  options.services.captaind-testnet5 = {
    enable = mkEnableOption "self-hosted captaind (bark server) on Testnet5";

    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.system}.captaind-testnet5;
      description = "captaind package (testnet5 fork, nix/lab/ark-testnet5.nix).";
    };

    dataDir = mkOption {
      type = types.str;
      default = "/var/lib/captaind-testnet5";
      readOnly = true;
      description = "State root (systemd StateDirectory; fixed). Holds the server wallet seed - back it up.";
    };

    # Loopback by design: the public gRPC endpoint is consumed by the
    # co-located spc-pool ark role and by the operator's bark CLI on this
    # host. Publishing an open Ark service is a separate operator decision
    # outside this packet.
    publicAddress = mkOption { type = types.str; default = "127.0.0.1:3535"; };
    adminAddress = mkOption {
      type = types.str;
      default = "127.0.0.1:3536";
      description = "Admin gRPC. Unauthenticated by design and as privileged as shell access; never expose it.";
    };
    integrationAddress = mkOption { type = types.str; default = "127.0.0.1:3537"; };

    bitcoind = {
      url = mkOption {
        type = types.str;
        default = "http://127.0.0.1:18352";
        description = "Testnet5 node RPC endpoint (services.rbitcoin-testnet5).";
      };
      cookieFile = mkOption {
        type = types.str;
        default = "/etc/spc/secrets/rbitcoin-rpc.cookie";
        description = "Core-format RPC cookie shared with the node (0640 root:spc-rpc).";
      };
    };

    postgres = {
      host = mkOption {
        type = types.str;
        default = "/run/postgresql";
        description = "Postgres host; a path selects the local Unix socket (peer auth).";
      };
      port = mkOption { type = types.port; default = 5432; };
      # ensureDBOwnership requires database == role; peer auth requires
      # role == system user, so all three share the service user's name.
      database = mkOption { type = types.str; default = "captaind"; };
      user = mkOption { type = types.str; default = "captaind"; };
      manage = mkOption {
        type = types.bool;
        default = true;
        description = "Provision the database/role on the local services.postgresql.";
      };
    };

    settings = mkOption {
      type = tomlFormat.type;
      default = { };
      description = ''
        Raw captaind.toml overrides merged over the vendored pinned
        defaults (vtxo lifetimes, fees, round timing, limits, ...).
      '';
    };

    user = mkOption { type = types.str; default = "captaind"; };
    group = mkOption { type = types.str; default = "captaind"; };
  };

  config = mkIf cfg.enable {
    assertions = [{
      assertion = lib.hasPrefix "127." cfg.adminAddress;
      message = "services.captaind-testnet5.adminAddress must stay loopback: the admin gRPC is unauthenticated and as privileged as shell access.";
    }];

    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.group;
      home = cfg.dataDir;
      # Reads the shared node RPC cookie (0640 root:spc-rpc).
      extraGroups = [ "spc-rpc" ];
    };
    users.groups.${cfg.group} = { };
    users.groups.spc-rpc = { };

    services.postgresql = mkIf cfg.postgres.manage {
      enable = lib.mkDefault true;
      ensureDatabases = [ cfg.postgres.database ];
      ensureUsers = [{
        name = cfg.postgres.user;
        ensureDBOwnership = true;
      }];
    };

    systemd.services.captaind-testnet5 = {
      description = "captaind (bark Ark server), Testnet5 fork, self-hosted";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" "postgresql.service" "rbitcoin-testnet5.service" ];
      wants = [ "network-online.target" "rbitcoin-testnet5.service" ];
      requires = [ "postgresql.service" ];
      preStart = ''
        if [ ! -r ${lib.escapeShellArg cfg.bitcoind.cookieFile} ]; then
          echo "captaind-testnet5: node RPC cookie ${cfg.bitcoind.cookieFile} is missing or unreadable." >&2
          exit 1
        fi
        ${cfg.package}/bin/captaind check-config ${configFile}
        # First boot: create the server (wallet seed + schema). The seed
        # lands under ${cfg.dataDir}/data; it is hot - back it up.
        if [ ! -e ${cfg.dataDir}/data/mnemonic ]; then
          ${cfg.package}/bin/captaind --config ${configFile} create
        fi
      '';
      serviceConfig = {
        User = cfg.user;
        Group = cfg.group;
        SupplementaryGroups = [ "spc-rpc" ];
        ExecStart = "${cfg.package}/bin/captaind --config ${configFile} start";
        Restart = "on-failure";
        RestartSec = 5;
        StateDirectory = "captaind-testnet5";
        WorkingDirectory = cfg.dataDir;
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = [ cfg.dataDir ];
        LimitNOFILE = 65536;
      };
    };
  };
}
