# Self-hosted stock Ark infrastructure in the lab

Date: 2026-11-08
Status: adopted by explicit owner deployment directive; implementation validation separate

## Decision

The expanded lab builds and owns its Ark server from the stock Bark repository
at `c419159fa54085572f8ca72d2ba1716556a9d3eb`, through Nix. No hosted-provider
endpoint is a lab fallback. This is a lab deployment requirement, not an Ark
protocol fork or a change to production's independent-server threat model.
SPEC §11.3's identity boundary and vanilla-server interoperability remain
load-bearing.

One host-local provider has its own Postgres cluster, mnemonic and config.
Each pool has a separate `spc-pool --roles ark` process, independent seed,
wallet SQLite, order-verification key and restricted Redis credentials. Pool
attribution databases are not provider storage and their credentials never
enter an Ark-role config. Actor ownership and cleanup use the existing lab
supervisor and parent-death helper.

The startup order is nodes/pool storage, provider storage/create/start, actual
GetArkInfo policy binding, pool-wallet provisioning, then fronts. The AP-4
432-block/100-depth/3-confirmation/20000-sat board profile is explicit. Server
fee values come from actual ArkInfo, with its JSON fee schedule SHA256 bound
in the private startup artifact. This is not the AP-5 fee-reserve proof.

## Scope and status semantics

`lab-ark-infrastructure.json` is an unfunded infrastructure scenario, with
zero user/miner actors. It does not claim the four-user acceptance profile.
The native funding driver and registered-agent/miner bridge must be integrated
before a mining roster can run in this profile. This is an unimplemented
runtime interface, not a workflow approval hold. Existing non-Ark scenarios
retain their mining behavior.

The server wallet's observed zero balance yields `initialized_unfunded`.
A positive balance yields `funding_observed_not_validated`, never `ready`.
An absent or invalid balance is an error, not zero. Pool wallet startup writes
an actual wallet funding address and observed spendable float to a private
provisioning artifact. A target of 115000000 sats is never a balance claim.

The named `bark-v31` node profile is selected only by this local Ark scenario.
The default L2 node pin and consensus remain unchanged. Source Nix packaging
of the stock server does not permit modifications to that server's protocol.

## References / edits

- SPEC §9 lab deployment supplement (frontmatter/body fragment in
  `docs/changelog/2026-11-08-self-hosted-lab-ark.md`, for the shared-doc writer).
- SPEC §11.3 and §11.5 continue to govern boundaries and wallet ownership.
- `nix/lab/ark.nix`, `nix/lab/ark-runtime.nix`, lab schema/runtime/inventory.
- Validation receipt: `docs/reviews/2026-11-08-self-hosted-ark-infrastructure.md`.
