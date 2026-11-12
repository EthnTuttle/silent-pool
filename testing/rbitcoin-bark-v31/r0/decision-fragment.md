# 2026-11-08 - rbitcoin named RPC compat profile `bark-v31` (PROPOSED fragment)

> **Status: PROPOSED, not binding.** This is the decision fragment from the
> AP-1b candidate packet `testing/rbitcoin-bark-v31/`. It becomes a binding
> record under `docs/decisions/` (with a `SPEC.md` frontmatter changelog
> entry) only if the parent Fable review approves adoption of the node
> patch. Nothing in this packet installs the profile anywhere, changes a
> pin, or adopts the candidate source.

**Trigger.** AP-1 version-gate blocker
(`docs/plans/.../2026-11-07-lab-ark-services.md` section 5.4 and table row
"rbitcoin reports a bark-compatible version"): bark `c419159` requires
`getnetworkinfo.version >= 310000` (server) / `>= 290000` (wallet); the
approved L2 rbitcoin reports `190000`, so every Ark acceptance run fails the
version gate. AP-1a (approved read-only probe, contract
`f4fc3e043f420870675095ab24144ba14e0afba6cb9ad69d818d99b1bccb2f62`) proved
the gate failure is honest and the only unresolved surface policy failure on
the tested read-only surface.

## Decision (proposed)

Adopt, for the lab rbitcoin node only, the **named** RPC compatibility
profile `bark-v31`, selected by `--rpc-compat-profile bark-v31` (or
`RBITCOIN_RPC_COMPAT_PROFILE=bark-v31` for env-launched owned nodes, e.g.
the AP-1a probe's `OwnedNode`):

- `getnetworkinfo.version` reports exactly `310000` as an **explicit RPC
  compatibility claim**. There is no free-form `--rpc-compat-version`
  integer; `bark-v31` is the only accepted value and any other name is a
  startup error before effects.
- The real rbitcoin software identity is preserved: `subversion` keeps the
  true `/rbitcoin:<version>/` string, `protocolversion` stays 70016.
- `getnetworkinfo` carries a structured
  `rbitcoin_compat {profile:"bark-v31", claimed_version:310000,
  package_dialect:"core-is-child-with-parents"}` object, and the node logs
  a startup banner
  `rpc_compat_profile=bark-v31 claimed_version=310000 software=rbitcoin software_version=<real>`.
- The profile selects the **existing** Core `IsChildWithParents` package
  dialect for `submitpackage` (the `RBITCOIN_RPC_PACKAGE_DIALECT=1` code
  path). One policy resolution path: the profile turns the dialect on, and
  an explicitly-off `RBITCOIN_RPC_PACKAGE_DIALECT=0` conflicts with the
  profile's claim and fails closed at startup.
- The profile is valid **only on regtest**; other networks fail at config
  validation, before the store opens.
- Default is **off**: `getnetworkinfo.version` stays `190000`, no
  `rbitcoin_compat` object, dialect unchanged.

## Scope of the claim (deliberately narrow)

The profile claims RPC-shape compatibility with the **tested read-only
RPC surface** only (the AP-1a probe's 40-check surface: availability,
shape, id-echo, envelope forms, package-dialect recognition). It is **not**
a claim that the node is Bitcoin Core v31, implements every v31 behavioral
difference, or passes funded package behavior. Funded behavior stays
`not_proven` until the AP-5 funded runs; `full_acceptance` stays false in
read-only mode. The `getmempoolinfo maxdatacarriersize: null` divergence
from the selected v30 modelling consumer is a **separate traffic limitation**
(T0 owner's statement), explicitly not patched and not laundered by this
profile.

## Why

- Named profile, not an integer knob: a free-form version is a general
  spoofing surface (peer review R4, plan resolution 4). The name binds the
  claim to a reviewed, probed contract.
- Honest identity: `subversion` keeps the real rbitcoin identity so no
  evidence can call the binary "stock Core" or hide what software ran; the
  claim is explicit and machine-readable in `rbitcoin_compat` plus the
  startup log.
- Fail closed: unknown profile, non-regtest network, and dialect conflict
  are startup errors, never silent guesses or silent overrides.
- No bark fork: bark stays stock at `c419159`; the compatibility report is
  fixed on the rbitcoin side where the claim can be probed every acceptance
  run (plan section 5.4 Option A).

## Consequences (if adopted)

- The AP-4 surface gate can pass the version floor **only** when the
  profile-on node is probed fresh at acceptance start; a version-only pass
  remains invalid (every method still `pass|fail|not_proven`).
- A second profile name would be a new decision; the single accepted name
  also means CLI and env forms cannot disagree (both parse or both fail).
- The node patch is additive to the approved L2 snapshot (`d2f37b91...`);
  adoption requires the parent to apply the exact patch file from this
  packet to a fresh L2-derived tree and re-bind binary/source hashes. No
  `UPSTREAM_DEPS.md` or pin change is part of this decision.

## Edits applied (proposed; in the candidate, not the primary)

- `crates/rbitcoin-rpc/src/methods/net.rs`: `RpcCompatProfile` enum
  (`BarkV31`: name, claimed version 310000, dialect label), `getnetworkinfo`
  version claim + `rbitcoin_compat` object.
- `crates/rbitcoin-rpc/src/methods/mod.rs`, `src/server.rs`:
  `RpcContext`/`RpcConfig` carry `compat_profile: Option<RpcCompatProfile>`.
- `crates/rbitcoin-node/src/config.rs`: `RpcOpts.compat_profile`,
  `NodeEnv.rpc_compat_profile`, tri-state `rpc_package_dialect`,
  `resolve_rpc_env` (one policy path), `apply_kv` key, regtest-only
  validation.
- `crates/rbitcoin-node/src/cli.rs`: usage, env/profile resolution via
  `resolve_rpc_env` with fail-closed conflicts.
- `crates/rbitcoin-node/src/run.rs`: RPC config wiring + startup banner.
- Tests: counterfactual first (default off 190000; profile-on 310000 +
  real identity + dialect enforced; unknown profile, non-regtest, and
  dialect conflict fail; new metadata exact).
