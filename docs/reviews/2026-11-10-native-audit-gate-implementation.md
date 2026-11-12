# Native audit gate implementation - 2026-11-10

Status: source implemented and formatted; **UNCOMPILED, runtime acceptance
unproven**. Parent retains the heavy-build slot. No Cargo check/test, clippy,
Nix build or joined mining run was performed for this packet.

## Delivered source

- Frozen public r3 codec: `/tmp/opencode/native-job-gate-public-r3/handoff.json`.
  Codec SHA256 `39c75d44c8ff16cf1e08b873eca91ad2988f9da1937ef18265e62d537d8abd99`.
  Its request, receipt, lifecycle DTOs, signing preimages and AddConnection
  config remain byte-identical to the AP3 handoff.
- The native connection actor runs one cancellable wallet exchange alongside
  the dedicated Noise reader. Candidate replacement and proof expiry revoke
  per-hash worker permits. Invalid evidence leaves work inactive; retries use
  fresh epochs/nonces and retain existing hash-budget accounting.
- Public verification consumes exact source-feed/2 evidence and delegates to
  `verify_public_evidence`. Signed window facts reach activation events.
  Complete audited underfill is eligible; full campaign acceptance still needs Q.
- Controller AddConnection hooks capture supervised process/config facts and
  publish private peer registries before sending the command. Attach validates
  persisted instance/process/socket/registry bindings. Private input reads use
  no-follow, nonblocking opens, opened-file metadata and a bounded read.
- Decision, SPEC changelog entry and `docs/NATIVE-JOB-GATE.md` document the
  private protocol and ordering requirements.

## Source provenance

The worktree was bootstrapped from the verified 682-row baseline at
`/tmp/opencode/pool-wallet-control-packet-r1/baseline.json`, digest
`4561a2df12d9980ee1a16ee285e6667a94d807af7b9c5aa6d30bc81a7ef012b6`.
Git's larger working delta includes that baseline; it is not this packet's
owned delta. Source-feed/2 and its pure accounting/scanner dependencies were
selectively imported with provenance in
`/tmp/opencode/native-gate-source-feed-v2-layer.json`.

The final handoff identifies baseline-relative changes, imported dependencies
and packet-owned files separately. The current-primary runtime patch is based
on captured engine bytes, not on replacement of the primary runtime. It needs
serial integration against the parent's current engine and AP3 fields.

## Verification completed

- `rustfmt --check --edition 2021 --config skip_children=true` passed for all
  owned/touched Rust modules, including CPU integration tests and runtime hooks.
- `git diff --check` passed.
- Source dependency inspection found an acyclic internal production graph and
  the CPU-to-scan and scan-to-accounting lock edges. This is not Cargo resolution.
- Public frozen r3 hashes are checked again when constructing the final handoff.

## Authored tests awaiting execution

Codec tests cover request correlation, independent authorities, window-v2
tampering/omission, strict JSON, wallet restart and kernel peer mismatch.
Real Noise/CPU fixtures cover commit-before-first-hash, wrong/expired proofs,
new job/prevhash/target/extranonce cancellation, active-work revocation,
disconnect/reconnect, pause/resume, expiry renewal and fresh-attempt retry.
Private-file tests cover atomic publication and live/dangling symlink refusal.
Fixture authorities do not establish full-source or real wallet acceptance.

## Remaining acceptance

1. Assigned guarded build slot: use `/tmp/opencode/T1a-target`,
   `CARGO_INCREMENTAL=0`, nonblocking heavy-build flock and required disk checks.
   Run codec tests, CPU native tests, relevant lab checks and clippy. Fix actual
   failures before marking any test or compilation passed.
2. AP3 integration: the existing wallet owns service lifecycle, actual instance
   creation, kernel authorizer and loaded-key signing. It uses
   `RequestAudit::from_wallet` and consuming
   `verify_received(&job, request.nonce, deadline)` through its existing owner
   channel. The baseline's serialized scenario lacks AP3 `wallet_agents`; the
   focused runtime helper fails closed until those declarations are integrated.
3. Joined QA: existing four wallets/four native miners, declared 40 H/s total,
   funding and fresh registration barriers, actual independent source-feed/2
   collector history, wallet COMMITTED receipts and event/first-hash ordering.
   Verify lifecycle/candidate failure cases and demonstrate Q reached for full
   campaign acceptance. This proof has not run.
