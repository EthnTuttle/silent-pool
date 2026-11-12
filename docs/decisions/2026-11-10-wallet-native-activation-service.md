# Native source activation inside the existing wallet

Date: 2026-11-10
Status: implemented source; compilation and integrated acceptance pending

## Authority

Parent assignment for the direct native SV2 pre-hash gate, together with the
native owner's frozen codec/runtime handoff and ladder owner's loaded-key
SourceFeed v2 handoff. This extends wallet-role integration for SPEC §11.5;
the serial owner applies the accompanying frontmatter fragment.

## Decision

- Use exactly `spc.native-job-gate/1` with SourceFeed v2 evidence. Import the
  pure codec `39c75d44c8ff16cf1e08b873eca91ad2988f9da1937ef18265e62d537d8abd99`.
  Native outer nonce passes unchanged to the consuming shared auditor.
- A private 0600 Unix socket lives under the existing owned 0700 wallet state
  directory. The actual wallet lifecycle generates a fresh nonzero instance ID
  and atomically publishes the declared WalletInstance only after binding and
  starting its bounded executor. PID/start/boot/executable/run-token/config
  hashes and socket device/inode are observed locally. No new wallet or seed.
- Decode a bounded strict request, derive CPU facts from SO_PEERCRED and /proc,
  then call the shared PeerRegistry authorizer BEFORE the auditor. Registry
  and paths are controller-owned, private and bounded. Recheck peer, config,
  instance, expiry and current candidate before response release.
- An exact completed request may replay its cached response only until its
  effective expiry. In-flight/failed duplicates cannot remint. Changed requests
  at the same epoch and live nonce reuse reject. Newer authorized generation/
  session/candidate cancels older work; wallet restart changes instance binding.
- Run bounded collector/source verification on a dedicated thread in each
  existing wallet process. A bounded in-process factory borrows the wallet's
  original B_scan to construct one request-owned, consuming shared auditor.
  Main-loop activation signing receives typed request/evidence, never a raw
  caller digest. Reverify public evidence and full job ID, compute the codec's
  activation digest, sign with loaded B_scan and erase the temporary keypair.
  No Ark key, SP spend key, database or maintenance operation lock is passed.
- Approved carries only the exact capability/ActivationReceipt. Unknown codes
  are the codec's closed enum; malformed/unbound transport may close. Missing
  collector/history is never warmup success. Effective expiry is the minimum
  of request and checkpoint expiry; SourceFeed v1 has no activation fallback.
- Optional `ark.native_audit` is rendered from declared public collector/pool
  source authorities and run/pool/front/identity scope. Fixed lifecycle paths
  are derived from the actual wallet datadir. The controller's existing native
  helper publishes peer/config/connection bindings before every AddConnection.
  Attach validates persisted state without provisioning or new allowances.
- Current deployment is host-local. Planned topology is refused, and kernel
  peer observation requires the same actual network namespace. This does not
  claim isolation against arbitrary same-UID host processes reading files.

## Verification boundary

The 708-row aggregate and loaded-717 source freezes and approved 0b0/e97 trees
remain preserved. Tests authored here use a mock collector and fixture wallet
startup metadata; a real subprocess supplies Unix/proc CPU facts. Source is
uncompiled while the other owner holds the heavy turn. Actual native CPU
first-hash ordering, full-Q and funded acceptance remain integrated test gates.
