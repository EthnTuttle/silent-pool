{ self }:
{ config, lib, pkgs, ... }:
let
  cfg = config.services.spc-agent;
  inherit (lib) mkEnableOption mkOption mkIf types;
in {
  options.services.spc-agent = {
    enable = mkEnableOption "silent-pool spc-agent";

    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.system}.spc-agent;
      description = "spc-agent package.";
    };

    configFile = mkOption {
      type = types.path;
      description = "TOML configuration (see docs/SPEC.md §6.3, §11.10).";
    };

    roles = mkOption {
      type = types.listOf (types.enum [ "translator" "jdc" "wallet" ]);
      default = [ "translator" ];
    };

    environmentFile = mkOption {
      type = types.nullOr types.path;
      default = null;
      description = "Environment file with miner secrets (never the master seed).";
    };

    user = mkOption { type = types.str; default = "spc-agent"; };
    group = mkOption { type = types.str; default = "spc-agent"; };
  };

  config = mkIf cfg.enable {
    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.group;
      home = "/var/lib/spc-agent";
      createHome = true;
    };
    users.groups.${cfg.group} = { };

    systemd.services.spc-agent = {
      description = "silent-pool miner agent";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
      serviceConfig = {
        User = cfg.user;
        Group = cfg.group;
        ExecStart = "${cfg.package}/bin/spc-agent --config ${cfg.configFile} --roles ${lib.concatStringsSep "," cfg.roles}";
        Restart = "on-failure";
        RestartSec = 5;
        StateDirectory = "spc-agent";
        WorkingDirectory = "/var/lib/spc-agent";
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = [ "/var/lib/spc-agent" ];
        LimitNOFILE = 65536;
      } // lib.optionalAttrs (cfg.environmentFile != null) {
        EnvironmentFile = cfg.environmentFile;
      };
    };
  };
}
