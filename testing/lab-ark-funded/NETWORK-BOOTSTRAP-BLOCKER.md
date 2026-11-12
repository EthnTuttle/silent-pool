# Actual corrected-runtime blocker: no operational P2P link

Run: `/tmp/opencode/AP5-corrected-native-live-r1`.
Bundle: `/tmp/opencode/runtime-fix-diagnostic-r1/bundle.json`, SHA256
`4d3778e8d4c65cffa95af483828fe1013a1d4307da32b3469801ce1624f036fd`.
CLI SHA256: `52e45eabfbcbc53921c670a19b36cb6940a7228f5d8b4bc15c43556bedce402b`.
Actual source: `/nix/store/11icfby7ni0185p093hxmbsahyl0p2dl-source`.
The run used the immutable AP5-probe-and-pid-handoff-r1 harness. No bound
runtime/harness source was edited while it executed.

## What the actual run proved

The previous Runtime asynchronous-phase fix worked: the declared original
bootstrap id entered Running and remained in progress instead of immediately
failing the phase. The canonical probe was recognized and all declared actor
roles were present, including two nodes, two pool Ark roles, four wallets,
four miner processes and two tx drivers. Worker hashing was not admitted as
completed bootstrap/paid work.

The native bootstrap authority produced a block on node A:

- A: height 1, hash `5445b184a0a12b3bb375c633b660449aeb0c5c48fee07df689d3abc419dc93ce`.
- B: height 0, regtest genesis.
- Both `getpeerinfo` observations: zero peers.
- Both actual recorded node argvs: `--no-listen --no-seeds --no-discover
  --max-outbound 1`, without an explicit peer connection.

The live source at `crates/spc-lab/src/runtime.rs:1726` still renders
`--no-listen` for operational host nodes. The bootstrap executor at
`crates/spc-lab-txd/src/native_bootstrap.rs:704-719` submits to its authority
node, then requires a common canonical index; lines 752-772 verify the block
on every node before observing any funding. With both listeners disabled and
no peer connection, A's block cannot establish that required common view.
The configuration's peer-wiring claim is not an observed working P2P link.

## Bounded stop and scope

Stopped through the existing authenticated `stop_run` command. Its receipt
is Succeeded; the original `ap5-native-bootstrap` finished Cancelled with
`bootstrap cancelled`. No replacement bootstrap id or deadline reset was
used. No RPC generate, manual cross-node block injection, source/argv mutation,
second mining authority, balance credit or user-payment command was issued.

Observed budget records: one hash-start at 40 H/s, three reserved attempts,
three measured attempts, zero unknown reconciliations. These are preserved
observations, not an independent native-audit verdict. Four ordinary funding
transfers and their three-confirmation proofs were not reached.

All 68 recorded process identities are gone; all recorded listeners released;
no run-file references or locks remain in the captured census. The same four
preexisting unreadable same-UID PIDs keep global cleanup INCONCLUSIVE. The
harness's post-stop reap error is retained, not relabelled as success.

Evidence is in `/tmp/opencode/AP5-verifier-build-r1/`:
`live-bootstrap-chain-observation.json`, `live-node-network-arguments.json`,
`live-stop-intent.json`, `live-stop-reply.json`, `live-execution-receipt.json`.
The run's sealed events, command journal and private datadirs remain intact.

## Minimum producer correction

Render the actual owned loopback P2P listeners and explicit node-to-node
connection for operational host nodes using the leased endpoint map. Do so
before finalizing/hashing launch profiles. Keep the cloned probe node isolated
through the existing declared profile substitutions. Do not patch running
argv/profile claims, rely on discovery, or relay blocks manually to bypass the
common-chain check. A regression must exercise actual two-node propagation and
common canonical bootstrap observation. The pinned node/server binaries need
not be changed; updated runtime source/test/Nix binding is required.

This is separate from the corrected asynchronous phase wait, R4 pool-control,
Rule B, Ark delivery and offline verifier conformance. No full native, Ark or
whole-lab acceptance is claimed.
