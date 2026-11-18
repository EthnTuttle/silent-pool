# Example host for `nixosConfigurations.spc-testnet5-host`.
#
#   nixos-rebuild switch --flake .#spc-testnet5-host --target-host root@<vps>
#
# Everything an operator must change is in the EDIT ME block; the stack
# itself (node, captaind, pool, postgres, redis, nginx, firewall) comes
# from nix/modules/spc-testnet5-stack.nix. Secrets are NOT here and NOT in
# the Nix store - they are plain files on the host (sops-nix friendly);
# provisioning them is step 2 of docs/DEPLOY-TESTNET5.md.
{ modulesPath, ... }:
{
  imports = [ (modulesPath + "/profiles/qemu-guest.nix") ];

  # --------------------------- EDIT ME ------------------------------------
  networking.hostName = "spc-testnet5";

  services.spc-testnet5-stack = {
    enable = true;

    # Public domain for the read-only API (null = plain HTTP on :80).
    domain = null;                       # e.g. "pool.example.org"
    acme.enable = false;                 # true once domain is set
    acme.email = null;                   # e.g. "ops@example.org"

    # Operator-provisioned secret files (see the runbook; sops-nix users
    # point these at their sops-managed paths instead):
    poolConfigFile = "/etc/spc/pool.toml";                    # 0640 root:spc-pool
    rpcCookieFile = "/etc/spc/secrets/rbitcoin-rpc.cookie";   # 0640 root:spc-rpc
    # The remaining secrets are referenced from pool.toml itself:
    #   /etc/spc/ark/seed.hex             ark seed, 0600 spc-pool, dir 0700
    #   /etc/spc/secrets/ark-order-key.hex order-signing key, 0600 root->spc-pool-readable
    # and the pool payout descriptor (derived from the ark seed) is pasted
    # into pool.toml as [pool] pool_payout_descriptor.
  };

  users.users.root.openssh.authorizedKeys.keys = [
    # "ssh-ed25519 AAAA... you@workstation"   # REPLACE
  ];

  # Adjust to the VPS image (these match a typical qemu/KVM guest).
  fileSystems."/" = {
    device = "/dev/disk/by-label/nixos";
    fsType = "ext4";
  };
  boot.loader.grub.device = "/dev/vda";
  # ------------------------- END EDIT ME ----------------------------------

  services.openssh = {
    enable = true;
    settings.PasswordAuthentication = false;
    settings.KbdInteractiveAuthentication = false;
  };

  networking.firewall.enable = true;

  nix.settings.experimental-features = [ "nix-command" "flakes" ];

  system.stateVersion = "25.05";
}
