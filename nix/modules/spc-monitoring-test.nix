# Eval-only validation host for nix/modules/spc-monitoring.nix: a minimal
# nixosSystem embedding the module with every contract option exercised
# (pool scrape target, rbitcoin target, float/slots alert mirrors, secret
# file path). It is never deployed or built; instantiating its toplevel
# proves the module and the generated Prometheus/Grafana/Alertmanager
# configuration evaluate, and the NixOS prometheus module's build-time
# promtool check covers the rendered config when the toplevel is built.
#
#   nix eval --impure --expr \
#     '(import ./nix/modules/spc-monitoring-test.nix {
#        nixpkgs = (builtins.getFlake (toString ./.)).inputs.nixpkgs;
#      }).config.system.build.toplevel.drvPath'
#
# The merge session may wire this as `nixosConfigurations.spc-monitoring-test`
# in flake.nix (shared file; hunk shipped as a merge note).
{ nixpkgs }:
nixpkgs.lib.nixosSystem {
  system = "x86_64-linux";
  modules = [
    (import ./spc-monitoring.nix)
    ({ ... }: {
      boot.loader.grub.enable = false;
      fileSystems."/" = { device = "none"; fsType = "tmpfs"; };
      system.stateVersion = "25.05";

      services.spc-monitoring = {
        enable = true;
        scrapeTargets = [{
          jobName = "spc-pool";
          targets = [ "127.0.0.1:8080" ];
        }];
        rbitcoinTarget = "127.0.0.1:9332";
        grafana.adminPasswordFile = "/run/secrets/grafana-admin-password";
        grafana.secretKeyFile = "/run/secrets/grafana-secret-key";
        alerts = {
          floatTargetSats = 1000000; # mirror of [ark] float_target_sats
          maxPayoutSlots = 1024; # mirror of [accounting] max_payout_slots
        };
      };
    })
  ];
}
