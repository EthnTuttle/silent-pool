# Aggregate wallet runtime join (source only)

Date: 2026-11-10
Status: API source bound to 0515a6a8; uncompiled; serial primary union pending

## Actual sources and placement

Read only the current primary source rows listed in
`/tmp/opencode/aggregate-wallet-runtime-r2-inputs.json` (18 files, 670,732 bytes).
The manifest file SHA256 is
`3e12918b3ad7cddc399138dfd7040c40dc560dbde75c7e17afce218cbe59c580`.
The exact `crates/spc-lab/src/wallet_bridge.rs` before-row is
`dc6b6fe9c6edd03a6606616074a4344006f1cad3fec68544940969181739de0e`.
No primary file, protected wallet layer, runtime datadir or process was changed.

- `ark_service::validate_runtime` permits one self-hosted, host-local Ark
  backend and rejects topology mode. The join explicitly refuses a persisted
  namespace placement; it cannot mistake another namespace's loopback for host.
- `runtime::render_pool_configs` obtains `front.<front_id>.api` from the leased
  resource manifest and writes `[api].listen` to `private/pool-<front_id>.toml`.
  The API is hosted in the same owned `front/<front_id>` process as SV2.
- `ark_service::provision_pool_roles` writes the public `ark.order_pubkey` to
  each front config and that pool's `private/pool-ark-<pool_id>/pool.toml`.
  Its provisioning record binds the role config's exact SHA256 and pool ID.
  Only the public key/config bytes are consumed; `order_key_path` is never opened.
- `runtime::PostgresHook::after_fronts` calls the existing wallet launch path.
  The narrow hook validates the listener and key join before launch and adds
  `[ark.aggregate_api]` to that existing agent's private config.

## Join and attach checks

`wallet_aggregate_binding.rs` checks run ID/resource directory, unique declared
wallet identity/front, same-pool miner mappings, API role, front ordinal/SV2/API
config equality, unique nonzero loopback leases, Redis/Postgres pool ownership,
front/Ark-role public key and destination equality, role provisioning SHA256,
and public cold-provisioned SP address equality with the scenario.

Before launch, the recorded front process must use the exact config path and
pass the existing supervisor's PID/start-time/run-token/executable checks. Both
SV2 and API sockets must match their exact IPv4 address/port, be owned by that
PID and occupy the controller's actual network namespace. A second ownership
check follows socket inspection. No URL or credential supplied by a browser is
accepted; the origin is constructed solely from that verified API lease.

The private config loader requires canonical run-owned paths, owned 0700
directories and bounded non-symlink 0600 files. Persisted validation compares
origin/key, identity, front authority, upstream/status endpoints, all existing
wallet key/datadir paths and Ark public bindings. It creates no directories,
keys, configuration, process or database. The existing online `WalletKeys::load`
checks the cold-derived B_scan against that same public address before opening
the wallet; the controller reads no scan/spend/master key.

The current-primary patch calls this read-only validation inside
`rebind_ui_registry` before `bind_persisted`. A failed attach leaves that UI
binding unavailable. `require_ready` performs the same validation before any
initial/dynamic mining connection can pass the existing wallet gate. Default
absent aggregate config remains unknown in the agent, and cannot be silently
adopted as a matching aggregate-aware runtime binding.

## Serial union

`/tmp/opencode/aggregate-wallet-runtime-r2.patch` contains only:

1. The narrow hooks in the exact current-primary `wallet_bridge.rs` before-row.
2. New `wallet_aggregate_binding.rs` and `wallet_aggregate_binding_tests.rs`.

The core patch excludes those three paths. Apply the runtime patch against its
hash-bound before rows, or deliberately union these hooks with the owner's
newer UI/T1 startup changes. Do not replace current runtime/engine/frontend
sources with the older wallet bootstrap. The owned wallet tree has the matching
launch/readiness hooks for future local compilation; its older bridge has no
UI-registry hook, which is why the separate primary join is supplied.

Authored tests cover pool/front/API-role mismatch, actual API versus Ark/status
ports, public order-key provenance without a readable private secret, hostile
origins/listeners, namespace refusal, cold identity mismatch, private-source
permissions/symlinks, read-only attach mutation refusal and exact socket
ownership. They have not run. Runtime/funded acceptance requires the integrated
serial source and the assigned heavy build slot.
