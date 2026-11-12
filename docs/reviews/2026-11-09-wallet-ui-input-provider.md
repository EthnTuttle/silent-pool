# Private wallet UI input provider

Date: 2026-11-09
Status: implemented; focused combined gate exit 0; funded provider acceptance NOT RUN

## Source ownership and bootstrap

Sole-owned follow-up tree:
`/home/ethan/.local/share/opencode/worktree/ad8886/lab-wallet-ui-provider`,
git base `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.

Verified the exact manifest recipe and every path/size/mode/SHA256 of all 690
rows in `/tmp/opencode/AP3b-audit-fix-r2-manifest-full.json`, full aggregate
`0b0c5fd7d5b52e314a0a1aec818b13eb9d134aafa2ca93734310bed028949468`,
approved code aggregate
`a270582ddfe2e0a20f11de5316b67b913f3a5a6404419f029633bd19b63a01a3`.
Imported only those rows (11,166,147 bytes); no Git, target, runtime datadir or
extra source snapshot. The approved wallet bridge tree and its receipts stay
immutable. Baseline manifest: `/tmp/opencode/wallet-ui-provider-baseline-manifest.json`.

Read the exact UI handoff and UI report provider/integration sections. The
implemented interface matches its closed allowlist without a new public route.

## Exact implemented interface

File: `<actual configured ark.datadir>/ui-inputs.json`.
AP-3b runtime example: `<run>/private/wallet-alice/state/ui-inputs.json`.

```json
{
  "schema": "spc-agent-private-inputs/v1",
  "updated_at_ms": 1794199618000,
  "inputs": [
    {
      "id": "010101010101010101010101010101010101010101010101010101010101010101010101010101",
      "amount_sats": "9007199254740993",
      "state": "spendable",
      "expiry_height": 1000,
      "rail": "derived"
    }
  ]
}
```

The example is illustrative, not a funded receipt. ID is lowercase full
36-byte outpoint hex (72 characters); money is canonical decimal-string u64,
including values above JavaScript's exact integer range; rail is `derived` or
`normal`. Only `spendable` rows with known actual expiry are emitted. The DTO
allows nullable expiry, but unknown expiry is excluded from this selectable
projection. No key/seed/xprv, proof/script, cookie, private path or third-party
identity appears. Empty `inputs` means no currently eligible proven inputs and
disables actions; it does not assert a zero balance. Missing file is provider
unavailable. UI freshness remains <=15 seconds.

## Implementation

- `wallet/ui_inputs.rs`: bounded proof-observation scratch and the private
  publisher. This cache is not another wallet or balance ledger. The start of
  each existing proof pass supplies source time; a slow completion cannot
  refresh old proof timestamps. Missing/stale/overflowed batches fail closed.
- `live_receive.rs`: captures only the actual SP observations after the existing
  canonical epoch/anchor, owner, mailbox and server-state checks and ledger
  commit. Failed validation invalidates that batch. No extra SP RPC pass.
- `maintenance.rs`: captures ordinary input data from the existing successfully
  validated full stock VTXOs in `proven_stock_balance`, retaining its existing
  sum/policy check. Publication excludes actual reservations, valid nonterminal
  queued owner commands and stock pending-round inputs. No refresh/offboard
  arithmetic, counter, signature or key behavior changes.
- `live_bark.rs`: holds only ephemeral observation batches in the existing
  live wallet. No second wallet instance/persister/RPC pump.
- `wallet/mod.rs`: binds the publisher to the actual settings datadir under the
  existing writer, invalidates the inert file before each tick's validation or
  commands, and publishes only after successful current scan and stock balance
  proof validation with no uncertain operation. Read-model failure is auxiliary
  unavailability, not new funding authority. Existing owner command submission
  still performs actual proof/ownership/reservation checks.
- File hygiene: canonical owned non-symlink 0700 wallet root and existing
  private database metadata checks; fixed filename; max 4096 rows and 1 MiB;
  pinned Bark atomic writer creates its own exclusive 0600 temp, writes/fsyncs,
  renames, then this provider fsyncs the directory and rechecks root identity.
  Failures never publish timestamp-new old available inputs. Unsafe targets
  are refused; removing an inert symlink does not modify its target.

## Actual verification

`/tmp/opencode/wallet-ui-provider-r1-gate.log`: **overall exit 0**.
The nonblocking shared heavy-build lock was held; inside-lock target/store
availability was 83,459,698,688 bytes with a 5-GiB incremental estimate and
>=20-GiB residual. Used the assigned `/tmp/opencode/T1a-target`, verified Nix
1.93 compiler/Cargo paths and existing complete read-only vendor source.

| Step | Exit | Result |
|---|---:|---|
| agent ark all-target check | 0 | compiled |
| focused input-provider tests | 0 | 9 passed |
| existing status tests | 0 | 5 passed |
| agent format check | 0 | clean |
| agent ark all-target clippy, -D warnings | 0 | clean |

Tests cover exact SP/normal DTO and u64 >2^53; reservation/spent/unknown
exclusion and SP-to-stock handover without double counting; unvalidated versus
successful empty inventory; stale/changed/duplicate/too-many inputs; queued
manual/pending inputs; private files and wrong datadir/symlink/root permissions;
large bodies; failed validation without timestamp refresh; and concurrent
readers seeing only complete atomic JSON. Verified-selector-state fixtures are
explicitly synthetic and do not claim funded blockchain/Ark validation.

Every gate step captured matching full/code source manifests before/after.
Tested full aggregate:
`bf265de078fa39a2f7608a05512b522e90b18d3ec362bd9960fb318b3ecbdd0a`;
tested/final code aggregate:
`7119fd63576efd950e0c2f1c7f687dd7e0cbed95565992097bae89292bf79cc0`.
The decision/fragment/report added after the gate only change documentation/full
hash; the final receipt verifies unchanged tested code. Full 153-test reruns
and the old Nix unfunded fixture were not needed for this additive observation.
Actual funded input-provider validation remains for the integrated native chain
and funded Ark wallets; no baseline unfunded observation is relabeled.

## Parent/UI integration and packet

Use the already agreed registry hook from
`/tmp/opencode/live-wallet-ui-provider-handoff.md`: the declared identity/front,
leased status endpoint and canonical `dir.join("state")` datadir. That hook is
not changed here. The UI adapter can consume this exact file/DTO. No browser
body chooses the producer path, and the observation never authorizes a spend.

Handoff files: `/tmp/opencode/wallet-ui-provider-manifest-{full,code}.json`,
`wallet-ui-provider-delta.json`, `wallet-ui-provider.patch`,
`wallet-ui-provider-receipts.json`, and `wallet-ui-provider-interface.json`.
Transfer only the provider delta over the approved 690-row AP-3b source.
No SPEC edit, primary/old-tree write, subagent, commit, pin/server change,
inspection-controller access or cache cleanup occurred.
