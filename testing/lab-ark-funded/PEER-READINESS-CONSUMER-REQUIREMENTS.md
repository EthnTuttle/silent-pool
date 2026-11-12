# Consumer preparation for the producer's replacement readiness observer

Status: evidence requirements only. The producer owns the proof version,
record layout, observer implementation and tests. No replacement parser or
admission registration is implemented by this document. Implementation waits
for its actual tested source-bound schema/bundle; the old 2484 bundle is not
to be rerun while that work is pending.

Input: `/tmp/opencode/node-ibd-peer-readiness-source/REPORT.md`, SHA256
`a814a0ac95cdee39619a63d51fbbb68a26d833a07b1e2a020a7b512dc002de06`.
Its `source-hashes.json` binds the cited production source and old evidence.
The report's illustrative proof name is not an assigned wire schema.

## Source explanation and historical boundary

The pinned node's IBD outbound PeerSlots are outside PeerHub. The opposite
acceptor registers inbound sessions in PeerHub, which alone is projected by
getpeerinfo. Literal-IP follow-outbound setup awaits IBD completion; height
zero explicitly prevents that completion. Requiring follow-outbound RPC rows
before the first hash is therefore cyclic. IBD already handshakes and requests
headers at genesis; it is completion that is held there.

This explains the failure without establishing who owned the old inbound
ephemeral endpoints. The failed run did not capture their socket-owner joins
or complete handshake markers. Preserve its failed pre-hash result and seal
`6fe9f92fb118c0eaf26e3245db9c225529a4b222705c4bf66fa4de69e064b768`.
No historical reconstruction, count-based acceptance, inbound relabelling or
new fields backfilled into its sealed records can supply the missing proof.

## Evidence the consumer must be able to verify

These obligations describe semantics, not JSON keys or an invented DTO.
Map them to the producer's actual versioned records before writing a parser.

| Obligation | Evidence needed from the actual producer | Report reference |
| --- | --- | --- |
| Source and executable identity | Exact observer source/root, node source/binary, CLI, actual argv/environment/profile, manifest and completed test/log bindings. Expected protocol/subver/services derive from that source. | Exact bindings; Node source trace |
| Both directed connections | Independently establish A -> B and B -> A from declared connect/listen endpoints; only then claim the required graph coverage. Preserve reported inbound direction. | Minimal contract items 3-5 |
| Owned actors | Designated D and R identities, boot id, PID/start ticks, executable hash, run/launch generation and verified namespace before and after acquisition. UID or peer user-agent alone is insufficient. | Minimal contract item 1 |
| Owned listeners and RPC | Exact receiver P2P LISTEN address/port and fd-inode ownership; RPC obtained through the receiver's bound owned control/listener. A listener is not a connection. | Minimal contract items 1-2 |
| Completed handshake | Typed inbound=true, peer id, addrbind equal to R's planned listener, addr equal to D's ephemeral endpoint; protocol 70016, source-expected subver and typed services, positive received VERSION/VERACK accounting markers. | Which public fields prove handshake completion? |
| Both ESTABLISHED endpoints | D's owned fd inode joins local D_ephemeral -> remote R_listener; R independently joins the exact reverse tuple. Both state 01, nonzero inodes; endpoint inodes need not equal each other. | Minimal contract item 4 |
| Temporal coherence | Kernel and identity samples bracket RPC; retain same peer identity/endpoints and each side's socket ownership/state across the bounded monotonic interval. Evidence is fresh for the current operation/incarnations. | Minimal contract item 6 |
| Namespace and completeness | Correct designated namespace TCP view intersected with each designated PID's fd-inode set, complete address/family/port decoding, declared endpoint coverage and no unplanned P2P connections. | Kernel/namespace caveats |
| Ordering | Complete fresh proof immediately before the sole original bootstrap HashStart. Stale or incomplete attempts cannot be combined across retries; retries remain within the original deadline. | Minimal contract item 6; Required fresh regression |
| First real block | After readiness, authority-only first-block submission to A, independent observation/verification of that exact first block on B, before funding credit/activation and maturity/allocation/transfers. | Required fresh regression |

Retain received getheaders on the same inbound row as strong post-handshake
corroboration if the producer's declared proof uses it. Its mandatory/optional
encoding must be explicit in that schema. Do not substitute sent-headers
counters: the pinned production tree does not populate that message map.
Positive VERSION/VERACK accounting is not an exact BIP324 wire-length check.
RPC compatibility version 310000 is not P2P protocol version 70016.
Transport=v2, relaytxes, peer counts, same genesis or nonzero traffic alone do
not establish the proof. Positive synced heights are not required at genesis.

Keep token/environment secrets out of public evidence. Identity checks may
verify ownership privately, but published records must not expose credentials.

## Parser and replay integration after producer delivery

1. Read the actual observer code, DTO/version, persistence path, journal event,
   temporal bounds, retry semantics and test records together. Bind all to the
   delivered source/Nix binaries. Determine exactly which namespace/topology
   forms the implementation supports; reject unsupported forms.
2. Use the actual closed schema with strict types and no legacy fallback that
   invents missing fields. Unknown proof versions, malformed direction or
   missing ownership/bracket samples cannot yield ready. Preserve raw inputs
   sufficient for offline validation, not just a producer success boolean.
3. Bind the proof artifact and corresponding journal records into the sealed
   archive. Check matching operation/run/incarnations and ordering against the
   original command and hash-budget journal. Do not compare unrelated clocks
   as if they shared a monotonic origin; follow the actual producer clock and
   sequence bindings.
4. Recompute declared directed coverage and validate the two endpoint joins
   from retained samples. A proof from another operation, source or generation
   cannot be reused. Keep graph readiness and later block propagation as
   distinct evidence requirements.
5. Qualify only completed actual source/test inventories. Do not guess the new
   gate total or reuse 350 merely because the old closure passed. Preserve the
   old source's exact 350 qualification and its one excluded fixture generator.

## Negative-case requirements to map to actual producer/consumer tests

- Connecting row: version 0, absent VERSION/VERACK markers, malformed boolean,
  wrong protocol/subver/services or missing peer id/addrbind.
- Same IP with wrong port or process; correct namespace-wide TCP tuple owned
  only by another process; listener-only evidence; only one socket endpoint.
- Incorrect reversal, family/address decoding or port suffix matching;
  comparing the two endpoint inodes for equality instead of tuple reversal.
- Wrong/reused PID, start ticks, boot/namespace/launch generation; stale peer
  id or inode; socket disappearance/state change between bracket samples.
- Unreadable required fd/table/identity, unsupported namespace/host-PID
  translation, wildcard listener or undeclared NAT/proxy mapping.
- SYN_SENT/TIME_WAIT/CLOSE_WAIT, absent direction, duplicated/unplanned peers,
  only one of the two directed dials, or a false complete graph declaration.
- Stale proof or proof after HashStart; operation mismatch, deadline reset,
  duplicate bootstrap start, altered source/log or unsealed substituted proof.
- A correct readiness proof with B never observing A's first real block must
  still fail the separate propagation/funding gate.

Duplicate fd aliases of the same inode are valid and must be deduplicated,
not confused with extra connections. Unsupported or incomplete observations
stay unknown/unready under the producer's bounded retry policy.

## Fresh-run and scope requirements

The decisive regression/run uses fresh nodes with exact admitted AP5 argv,
environment and binary bindings. Earlier nonempty-peer propagation tests and
the nine rendered-argv fixture tests did not exercise this strict observer.
After the replacement bundle passes source-qualified canonical admission,
freeze the consumer harness and execute the authorized diagnostic. Preserve
the original bootstrap ID/principal/deadline and single 40 H/s authority.
Require first-block A-to-B observation, then maturity/96 allocations and the
four external transfers with three confirmations on both nodes.

No addnode/addconnection, manual block relay to B, generate, mocktime, cloned
chain or extra hashing authority may be introduced to satisfy readiness.
Preparation adds no RPCs or process inspection. The four global unreadable
processes remain separate from designated-node proof and are not manipulated.
Successful reap and compiled-verifier evidence remain preserved. Native audit,
Rule B, control integration and Ark lifecycle acceptance remain outside this
diagnostic's claims.
