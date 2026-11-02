{ self }:
{ config, lib, ... }:
{
  imports = [
    (import ./spc-pool.nix { inherit self; })
    (import ./spc-agent.nix { inherit self; })
  ];

  # Durable state for the SPc side. The `ark` role deliberately does not use
  # either service (docs/SPEC.md §11.3).
  services.postgresql = {
    enable = lib.mkDefault true;
    ensureDatabases = [ "spc" ];
    ensureUsers = [{
      name = "spc";
      ensureDBOwnership = true;
    }];
  };

  services.redis.servers.spc = {
    enable = lib.mkDefault true;
    port = lib.mkDefault 6379;
    save = [ ];
    appendOnly = true;
    appendFsync = "everysec";
  };

  services.spc-pool.enable = lib.mkDefault true;
}
