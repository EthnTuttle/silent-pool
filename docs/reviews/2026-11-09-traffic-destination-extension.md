# T1-DEST - bounded legacy-input destination extension

Packet date: 2026-11-09. Implementation and verification performed in the
sole-owned `lab-traffic-witness-destinations` worktree. Check receipt timestamps
use the host clock (2026-11-10 UTC).

## Source and authority

- Base commit: `b0ab4ca34fda07c61a6335d05f598eaacc69678c`.
- Bootstrap: exactly 706 files / 11,312,078 bytes from the final tested T1/J1
  manifest. Every SHA256 and executable mode was checked before copying;
  the source matched the freeze, so archive fallback was unnecessary.
- Baseline full: `e53118dd3869db3722b4a28bf0028103604dcb7d162320c07fcf9607579dfaaa`.
- Baseline code: `8b1bbdc9267c4fd7a97592f2424e8e16e648e647cfa6e75033ad360be457be71`.
- Source manifest file SHA256:
  `79c0c00ae9052a93a0ff3f03b04a4e5ae418b6d811ae251408ad4832f79bc503`.
- Exact receipt and baseline rows: `/tmp/opencode/T1-DEST-bootstrap.json`.
- Authority: current AGENTS, DESIGN, SPEC (including §11.3 fee/privacy rules),
  architecture/wire/roadmap/prototype and upstream references; approved
  traffic-runtime plan §6 SHA256
  `44daaa2baa41a4b93eb46a9e69dfdcf2bba1e0ce73367e27e83a87ab3a6b3307`.
- Normative record: `docs/decisions/2026-11-09-traffic-destination-extension.md`
  and its SPEC frontmatter fragment. No control/history/key version bump.

## Implemented ownership and shared hunks

| Path | T1-DEST-owned hunk |
|---|---|
| `crates/spc-lab/src/traffic/commands.rs` | `SendUserPayment` docs and `validate_destination_address` output-template admission; existing `require_network(Regtest)` retained |
| `crates/spc-lab/src/traffic/spec.rs` | Input-only spend-gate comment, no schema/policy change to `spend_types` |
| `crates/spc-lab-txd/src/builder.rs` | Shared exact-template output predicate, false witness-owner guard, `build_and_sign` destination guard only |
| `crates/spc-lab-txd/src/wallet.rs` | `checked_destination` uses `require_network(Regtest)` and output predicate; `validate_request` invokes output/owner guard before command begin |
| `crates/spc-lab-txd/src/admit.rs` | Output predicate only, after unchanged legacy input signature verification |
| `crates/spc-lab-txd/src/actor.rs` | Early output-address preflight through `checked_destination`, before chain observation can append; complete request checks and rescan retained |
| `crates/spc-lab-txd/src/error.rs` | Accurate unsupported-output diagnostic and false witness-owner error |
| `crates/spc-lab-txd/src/adapter.rs` | Destination policy documentation |
| `crates/spc-lab-txd/src/tests.rs`, `tests/actor_control.rs`, `tests/destination_outputs.rs`, `tests/common/destinations.rs` | Scoped core/address/real actor-process tests and public address corpus |
| `crates/spc-lab/tests/{traffic_contract,traffic_destinations}.rs` | Controller destination validation and shared corpus |
| `docs/SPEC.md` | One additive frontmatter changelog entry; serial merge with T2's distinct entry |

The T2 independent nine-path installer has no executable overlap: Cargo.lock,
txd Cargo.toml/lib.rs, native_bootstrap.rs/tests and engine/traffic_runtime.rs
retain their baseline bytes in this tree. Its new native-bootstrap files are
not imported here. Only SPEC frontmatter needs the usual additive integration.
The observer runtime implementation was not edited. The actor's narrow early
output preflight prevents invalid address intent from triggering journaled
chain observation; the normal accepted-payment rescan remains mandatory.

## Boundary audit

The admitted templates are exactly P2PKH, P2WPKH and P2TR. P2SH/P2WSH,
OP_RETURN, nonstandard scripts, wrong-network/malformed addresses, witness
version 2, and version-1 programs of 31/33 bytes reject. All address-bearing
paths use rust-bitcoin's regtest check. Raw scripts do not encode a network;
their validated journal/run is regtest-bound.

All `is_p2pkh` occurrences were classified: `SignableUtxo::resolve_indexed`,
the builder's input loop, raw admission's previous-script check, and journal
`FundingObserved` admission remain input/owned-funding restrictions. Wallet
keys and receive/change derivation remain P2PKH. A caller cannot tag a new
witness destination as owned, and raw admission supplies no owner tag.

Selection already serialized each actual destination; no fee algorithm edit
was needed. Tests check the P2TR +9-byte and P2WPKH -3-byte differences against
P2PKH, plus a one-input P2TR payment with exactly bound-plus-dust funding.
Final signed consensus bytes, independent raw parser/SIGHASH_ALL verification,
txid, measured weight/vbytes, and input-minus-output fees are asserted.
Existing golden raw/signature vectors are unchanged.

The public stock Bark address fixture is
`bcrt1pm96lcuz7whnewynm6xkv389pys2v7vjlh88w0dxfdyxka6rd2emqar8un5`,
from the `onchain_funding_address` field of the AP3b unfunded fixture's
`pool-ark-pool-a/wallet/provision.json`. Only that public field was extracted.
Its exact script is pinned as
`5120d975fc705e75e797127bd1acc89ca12414cf325fb9cee7b4c9690d6ee86d5676`.
A deterministic standard address pair is generated separately for each network.

## Verification

The offline tests use explicitly labeled fixture prevouts, real legacy
signatures and consensus decoding. The actor-process fixture serves public
genesis reads, supplies no RPC funding, captures actual send bytes and returns
transport UNKNOWN. It is not a funded-node acceptance claim.

The checker is `/tmp/opencode/T1-DEST-check.sh` / `T1-DEST-check.py`, using
nonblocking `flock -n -o`, the existing `/tmp/opencode/T1a-target`, Nix Rust/
Cargo 1.93.0, and the existing read-only vendor config. It verifies code bytes
before/after each invocation, checks target/store space after lock and before
each command (50 GiB start, 5 GiB estimated growth, >=20 GiB residual), uses
bounded background checks and kills the owned process group on timeout.
No source snapshot was ingested into Nix.

Result: **IMPLEMENTED; scoped checks PASS.** Exact final artifact hashes are
recorded in `/tmp/opencode/T1-DEST-handoff.json` and the full/code/baseline-delta
manifests. The delta-only archive preserves the handed-off source bytes.

| Check | Actual result | Receipt directory under `/tmp/opencode/` |
|---|---|---|
| Full `cargo test -p spc-lab-txd` | 96 top-level passed + 1 explicitly spawned fault child; 2 child entry points ignored by ordinary discovery as before | `T1-DEST-check-20261110T032338Z` |
| Controller `traffic_contract`, `traffic_destinations`, `traffic_history_conformance` | 49 + 1 + 7 passed | same |
| Final actor-only regression after early output preflight | 6 passed, including new no-chain invalid-address ordering test | `T1-DEST-check-20261110T032659Z` |
| Scoped lib/bin/new-test clippy, `-D warnings` | PASS | same |
| `cargo fmt -p spc-lab-txd -p spc-lab -- --check` | PASS | same |
| `git diff --check` and reverse delta patch check | PASS; receipt in final handoff | final lightweight verification |

The only executable/test files changed after the full core/controller pass
were `actor.rs` and `tests/actor_control.rs`; the final six actor tests and
clippy cover those exact bytes. The per-step code-row receipts prove that
attribution. No passing broad suite was rerun after that narrow correction.
Across the final scoped suites, 154 distinct top-level test instances pass
(plus the explicitly spawned fault child); repeated actor instances are not
counted twice. Final target/store free space was 89,302,732,800 bytes before
fmt, comfortably above the enforced limits.

Retained initial failures: the default offline Cargo index lacked the locked
`ar_archive_writer` version; using the existing vendor config resolved it.
Two new test DTO accesses were corrected to `StoredCommand.request` (the
existing `SignedTxFacts` has no destination list). One new assertion initially
expected conflicts to leave journal bytes unchanged; the existing contract
correctly appends `command_conflict`. The test now verifies exactly that
append while preserving the bound request/signature/reservation. Production
replay/conflict semantics were not changed.

## Remaining integrated proof

Native-funded Bitcoin-node acceptance and funded stock Bark receive/board/
refresh/offboard are pending T2/AP-5 integration. T2's reported node GBT target
`207fffff` versus bound `200fffff` profile mismatch remains an independent
node-owner issue; this packet does not weaken that check. AP-5 can consume
this bounded output admission through the existing manual/send API after
native funding and maturity. No funds were sent and the user's inspection
runs/data were untouched. No commit, pin movement or server change occurred.
