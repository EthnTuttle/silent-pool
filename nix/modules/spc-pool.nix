{ self }:
{ config, lib, pkgs, ... }:
let
  cfg = config.services.spc-pool;
  inherit (lib) mkEnableOption mkOption mkIf mkDefault types;
in {
  options.services.spc-pool = {
    enable = mkEnableOption "silent-pool spc-pool";

    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.system}.spc-pool;
      description = "spc-pool package.";
    };

    configFile = mkOption {
      type = types.path;
      description = "TOML configuration (see docs/SPEC.md §6.3).";
    };

    roles = mkOption {
      type = types.listOf (types.enum [ "front" "payout" "api" "ark" ]);
      default = [ "front" "payout" "api" ];
      description = "Roles this process runs.";
    };

    environmentFile = mkOption {
      type = types.nullOr types.path;
      default = null;
      description = "Environment file with secrets (database/Redis URLs, order key path).";
    };

    user = mkOption { type = types.str; default = "spc-pool"; };
    group = mkOption { type = types.str; default = "spc-pool"; };

    openFirewall = mkOption {
      type = types.bool;
      default = false;
      description = "Open the SV1 and SV2 listener ports.";
    };

    sv2Port = mkOption { type = types.port; default = 34254; };
    sv1Port = mkOption { type = types.port; default = 3333; };
    apiPort = mkOption { type = types.port; default = 8080; };
  };

  config = mkIf cfg.enable {
    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.group;
      home = "/var/lib/spc-pool";
      createHome = true;
    };
    users.groups.${cfg.group} = { };

    systemd.services.spc-pool = {
      description = "silent-pool pool";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" "postgresql.service" "redis-spc.service" ];
      wants = [ "network-online.target" ];
      serviceConfig = {
        User = cfg.user;
        Group = cfg.group;
        ExecStart = "${cfg.package}/bin/spc-pool --config ${cfg.configFile} --roles ${lib.concatStringsSep "," cfg.roles}";
        Restart = "on-failure";
        RestartSec = 5;
        StateDirectory = "spc-pool";
        WorkingDirectory = "/var/lib/spc-pool";
        # a_send must never hit disk; the service only writes logs.
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = [ "/var/lib/spc-pool" ];
        LimitNOFILE = 65536;
      } // lib.optionalAttrs (cfg.environmentFile != null) {
        EnvironmentFile = cfg.environmentFile;
      };
    };

    networking.firewall.allowedTCPPorts =
      lib.optionals cfg.openFirewall ([ cfg.sv2Port cfg.sv1Port cfg.apiPort ]);
  };
}
