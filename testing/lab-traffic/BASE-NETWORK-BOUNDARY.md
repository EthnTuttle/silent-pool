# Base Runtime phase readiness is separate from host P2P readiness

The actual `/tmp/opencode/AP5-corrected-native-live-r1` run consumed the
`4d3778e8...` bundle / `52e45eab...` CLI. Its corrected Runtime waited on the
original `ap5-native-bootstrap` operation as intended. The next boundary failed:
both operational host nodes had `--no-listen`, disabled seeds/discovery and no
explicit peer connection. The planned graph was not an operational P2P graph.

`base_network.py` reads the externally sealed AP5 archive and preserved source
CommandLog/native records. It independently joins:

- The original controller-principal Begin, sole hash-start, unchanged deadline,
  three reserved/measured attempts and zero unresolved debit.
- One raw native block, authorized by those batches, with verified PoW/Merkle/
  subsidy and hash `5445b184a0a12b3bb375c633b660449aeb0c5c48fee07df689d3abc419dc93ce`.
- Actual retained getbestblockhash response digests: node A at that block and
  node B at regtest genesis. The recorded submitblock path targets only A.
- Actual recorded node PID/start/executable/argv and the owner-observed zero
  peer counts. The latter observations are retained by hash; no raw RPC response
  is fabricated from the summary.
- The actual successful StopRun reply and the original bootstrap's Cancelled
  terminal, including its native debit/cancellation receipt. No user payment
  command was issued.

All 68 recorded actor identities are gone. Four unreadable same-UID processes
keep global cleanup INCONCLUSIVE, and the retained reap error is not a pass.
The run proves the phase-wait correction, not completed funding, full native
wallet auditing, Ark lifecycle or whole-lab acceptance.

The corrected phase-wait admission pin remains valid at that scope. The known
host-network-failing CLI is now refused before this harness can launch another
funding run. The primary owner supplies operational host P2P listeners and
explicit planned peer IDs/ports before profile hashing, plus observed graph
readiness before hashing. Probe-clone isolation stays distinct. A new tested
source/Nix bundle is required; no direct block injection into B is a workaround.

The offline VTXO verifier is already compiled/admitted and does not depend on
this network fix. The full native-audit/Rule B/control bundle remains another
separate pending integration gate.

## Genesis/IBD observer requirements awaiting the versioned producer record

Source-backed finding: `/tmp/opencode/node-ibd-peer-readiness-source/REPORT.md`.
This concerns the later listen/connect-enabled source and its failed pre-hash
readiness gate, separately from the no-listen run above. IBD outbound sockets
are not registered in PeerHub; `getpeerinfo` may show only the accepting inbound
ends of real reciprocal IBD connections. Genesis cannot complete IBD, while
the follow-outbound path waits for that completion. Requiring follow-outbound
RPC rows before the first hash is therefore cyclic for this exact node source.

The consumer must retain the raw RPC direction. A completed inbound handshake
is not an outbound RPC row or authenticated counterpart identity by itself.
For each declared directed dial, the forthcoming producer record must supply:

- The completed receiver handshake and separately owned planned LISTEN socket.
- Both ESTABLISHED endpoints' reversed TCP four-tuples, intersected with each
  designated PID's fd-inode set in its verified network namespace. Endpoint
  socket inodes need not equal each other; ephemeral source ports must not be
  mistaken for the opposite node's listening port.
- Before/after process, namespace, socket and RPC samples binding the same
  run, boot, PID/start/executable, peer identity and endpoints throughout the
  bounded acquisition interval.
- Complete directed-dial coverage and no unplanned peers. Counts, same IP,
  transport labels, UID alone, or a nonempty peer array cannot replace joins.
- A fresh, source/version/method-bound complete observer record before the
  sole original HashStart, with its actual timing and operation relation.

Exact serialized fields and source/version/method tags have not been supplied.
The report's illustrative method name is not an admitted parser schema.
Partial records, unreadable/replaced sockets, connecting rows, missing ownership
joins or unsupported mappings remain unavailable; no readiness pass follows
from this analysis alone.

The required fresh regression uses the exact admitted AP5 launch profile,
proves both directed joins before HashStart, submits the first block only to
the authority node, and independently observes that exact raw block/hash on
the other node before common-index funding credit. Earlier nonempty-peer tests
do not cover this stricter boundary. Parent-reported canonical 350 and successful
reaper results remain scoped facts; the archived failed run lacks the socket
joins and cannot pass retroactively. The independent adapter will consume the
actual complete producer record once its exact contract is handed off.
