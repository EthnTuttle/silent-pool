# Public BIP95 Testnet5 rbitcoin node service (deployment flavor of the
# pinned nix/dev/testnet5-node.nix package: same binary, deploy config).
#
# Differences from the dev fixture (nix/dev/testnet5/node.conf.in), on
# purpose:
#   - P2P listens publicly (this host is a public pool front and may serve
#     inbound peers); the dev fixture is outbound-only.
#   - RPC stays loopback TCP with Core-format cookie auth. The cookie file
#     is operator-provisioned (sops-friendly), never generated into the
#     store; captaind and spc-pool authenticate with its contents.
#   - `rpc_client_version` is set to 310000 by default: the captaind
#     testnet5 fork gates on `getnetworkinfo.version >= 310000`
#     ("upgrade to v31"), and the node's real supported default is 190000.
#     This is the config-level compatibility claim the bark-v31 profile
#     work validated (docs/decisions/2026-11-08-lab-bark-v31-profile.md);
#     the binary is not patched. Set it to null to report the real default.
{ self }:
{ config, lib, pkgs, ... }:
let
  cfg = config.services.rbitcoin-testnet5;
  inherit (lib) mkEnableOption mkOption mkIf types;
  nodeConf = pkgs.writeText "rbitcoin-testnet5.conf" (''
    network=testnet5
    datadir=${cfg.dataDir}/data
    listen=${cfg.p2pAddress}:${toString cfg.p2pPort}
    max_outbound=${toString cfg.maxOutbound}
    max_inbound=${toString cfg.maxInbound}
    rpc_listen=${cfg.rpcAddress}:${toString cfg.rpcPort}
    rpc_socket=${cfg.dataDir}/data/rpc.sock
    rpc_cookie_file=${cfg.rpcCookieFile}
    rpc_cookie_perms=group
    log_level=${cfg.logLevel}
  ''
  + lib.optionalString (cfg.rpcClientVersion != null) ''
    rpc_client_version=${toString cfg.rpcClientVersion}
  ''
  + lib.optionalString (cfg.healthListen != null) ''
    health_listen=${cfg.healthListen}
    metrics=${if cfg.metrics then "1" else "0"}
  ''
  + lib.optionalString cfg.shIndex ''
    sh_index=1
  ''
  + lib.optionalString (cfg.esploraListen != null) ''
    esplora_listen=${cfg.esploraListen}
  ''
  + lib.optionalString (cfg.electrumListen != null) ''
    electrum_listen=${cfg.electrumListen}
  ''
  + lib.concatMapStrings (peer: ''
    seed_node=${peer}
  '') cfg.seedNodes
  + cfg.extraConfig);
in {
  options.services.rbitcoin-testnet5 = {
    enable = mkEnableOption "pinned rbitcoin Testnet5 node (public P2P, loopback RPC)";

    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.system}.rbitcoin-node-testnet5-dev;
      description = "Pinned rbitcoin node package (nix/dev/testnet5-node.nix).";
    };

    dataDir = mkOption {
      type = types.str;
      default = "/var/lib/rbitcoin-testnet5";
      readOnly = true;
      description = "State root (systemd StateDirectory; fixed).";
    };

    p2pAddress = mkOption { type = types.str; default = "0.0.0.0"; };
    p2pPort = mkOption {
      type = types.port;
      default = 18353;
      description = "Public BIP95 Testnet5 P2P port.";
    };
    maxOutbound = mkOption { type = types.int; default = 8; };
    maxInbound = mkOption {
      type = types.int;
      default = 64;
      description = "Inbound peer slots (public pool host serves the network).";
    };

    rpcAddress = mkOption {
      type = types.str;
      default = "127.0.0.1";
      description = "RPC bind address. Loopback only; captaind and spc-pool dial it locally.";
    };
    rpcPort = mkOption { type = types.port; default = 18352; };

    rpcCookieFile = mkOption {
      type = types.str;
      default = "/etc/spc/secrets/rbitcoin-rpc.cookie";
      description = ''
        Operator-provisioned Core-format `username:password` cookie
        (single line, no trailing newline, mode 0640 root:spc-rpc). The
        node READS this file; it does not generate it. captaind points
        its `[bitcoind] cookie` here and spc-pool carries the same
        credentials as `[template] rpc_user`/`rpc_pass`.
      '';
    };

    rpcClientVersion = mkOption {
      type = types.nullOr types.int;
      default = 310000;
      description = ''
        Value reported as `getnetworkinfo.version` (node conf
        `rpc_client_version`). captaind requires >= 310000; the node's
        real default is 190000. null removes the override.
      '';
    };

    seedNodes = mkOption {
      type = types.listOf types.str;
      # No DNS/fixed seeds exist at the source pin; this published peer is
      # the bootstrap recorded in nix/dev/testnet5/network.json. Discovery
      # stays on, so the addrman grows past it.
      default = [ "lapis.erisian.com.au:18353" ];
      description = "Bootstrap peers (`seed_node=`, additive; discovery stays on).";
    };

    logLevel = mkOption { type = types.str; default = "info"; };

    healthListen = mkOption {
      type = types.nullOr types.str;
      default = "127.0.0.1:9332";
      description = ''
        `health_listen` address: /healthz plus (with metrics) Prometheus
        /metrics (`rbitcoin_*` families). This is the address the
        monitoring packet's `services.spc-monitoring.rbitcoinTarget`
        scrapes; keep it loopback. null disables the listener.
      '';
    };
    metrics = mkOption {
      type = types.bool;
      default = true;
      description = "Serve Prometheus /metrics on the health listener (requires healthListen).";
    };

    # Explorer support (packet wp-explorer-testnet5): the pinned node
    # serves esplora/electrum from these flags; the explorer deployment
    # enables them and fronts the HTTP side itself. Loopback recommended;
    # no firewall port is opened here either way.
    shIndex = mkOption {
      type = types.bool;
      default = false;
      description = "Enable the scripthash index (`sh_index`; required by the esplora/electrum listeners).";
    };
    esploraListen = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "127.0.0.1:3003";
      description = "`esplora_listen` address for the embedded esplora HTTP API; null disables.";
    };
    electrumListen = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "127.0.0.1:50001";
      description = "`electrum_listen` address for the embedded electrum server; null disables.";
    };

    extraConfig = mkOption {
      type = types.lines;
      default = "";
      description = "Extra raw node.conf lines (see the node's apply_kv keys).";
    };

    user = mkOption { type = types.str; default = "rbitcoin-t5"; };
    group = mkOption { type = types.str; default = "rbitcoin-t5"; };

    openFirewall = mkOption {
      type = types.bool;
      default = true;
      description = "Open the public P2P port.";
    };
  };

  config = mkIf cfg.enable {
    assertions = [
      {
        assertion = lib.hasPrefix "127." cfg.rpcAddress || cfg.rpcAddress == "::1";
        message = "services.rbitcoin-testnet5.rpcAddress must stay loopback; the RPC carries cookie credentials for captaind and spc-pool only.";
      }
      {
        assertion = cfg.metrics -> cfg.healthListen != null;
        message = "services.rbitcoin-testnet5.metrics requires healthListen (the node refuses --metrics without --health-listen).";
      }
      {
        assertion = (cfg.esploraListen != null || cfg.electrumListen != null) -> cfg.shIndex;
        message = "services.rbitcoin-testnet5: esploraListen/electrumListen need shIndex = true (the scripthash index backs both).";
      }
    ];

    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.group;
      home = cfg.dataDir;
      # Reads the shared RPC cookie (0640 root:spc-rpc).
      extraGroups = [ "spc-rpc" ];
    };
    users.groups.${cfg.group} = { };
    users.groups.spc-rpc = { };

    systemd.services.rbitcoin-testnet5 = {
      description = "rbitcoin Testnet5 node (pinned, public P2P, loopback RPC)";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
      preStart = ''
        mkdir -p ${cfg.dataDir}/data
        if [ ! -r ${lib.escapeShellArg cfg.rpcCookieFile} ]; then
          echo "rbitcoin-testnet5: RPC cookie ${cfg.rpcCookieFile} is missing or unreadable." >&2
          echo "Provision it first (docs/DEPLOY-TESTNET5.md, 'Secrets')." >&2
          exit 1
        fi
      '';
      serviceConfig = {
        User = cfg.user;
        Group = cfg.group;
        SupplementaryGroups = [ "spc-rpc" ];
        ExecStart = "${cfg.package}/bin/rbitcoin-node --conf ${nodeConf}";
        Restart = "on-failure";
        RestartSec = 5;
        StateDirectory = "rbitcoin-testnet5";
        WorkingDirectory = cfg.dataDir;
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = [ cfg.dataDir ];
        LimitNOFILE = 65536;
      };
    };

    networking.firewall.allowedTCPPorts =
      lib.optionals cfg.openFirewall [ cfg.p2pPort ];
  };
}
