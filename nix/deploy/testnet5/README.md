# Testnet5 deployment assets (packet wp-deploy-testnet5)

Operator-side inputs for `nixosConfigurations.spc-testnet5-host`; the
runbook is [`docs/DEPLOY-TESTNET5.md`](../../../docs/DEPLOY-TESTNET5.md).

- `host-example.nix` - the example host wired into the flake's
  `nixosConfigurations.spc-testnet5-host`; edit the "EDIT ME" block.
- `pool.toml.example` - the operator-provisioned spc-pool config
  (install as `/etc/spc/pool.toml`; holds secrets, never in the store).
- `spc-testnet5-keygen.py` - offline, stdlib-only generator for every
  secret the stack needs (SV2 authority pair, ark seed + payout
  descriptor, order key, RPC cookie, ark_id). Runs built-in known-answer
  self-tests (BIP32 vector 1, the repo's published throwaway authority
  pair) on every invocation.
- `captaind.defaults.toml` - byte-exact copy of `captaind.default.toml`
  from the pinned bark testnet5 fork (`EthnTuttle/bark` @ `8c38e07`,
  sha256 `b04817bf78cdaf85caab2dc49f96ead08727fe62e7760ed37f7955159b5a27d2`).
  `nix/modules/captaind-testnet5.nix` uses it as the complete config base
  (captaind requires every key present) and validates the rendered result
  with `captaind check-config` before every start, so a future re-pin that
  changes the default config fails loudly. Re-copy it from
  `<bark-src>/server/captaind.default.toml` whenever the fork pin moves.
