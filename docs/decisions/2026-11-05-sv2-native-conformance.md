# 2026-11-05 - Native SV2 conformance and Noise authority provisioning

## Trigger

The regtest-lab review found that the native listener and agent shared wire
assumptions that an independent `channels_sv2` client rejects: prevhash was sent
before its future job, target U256 byte order was reversed, no target updates
were emitted, and multi-level Merkle paths were reversed. Early channel opens,
extranonce negotiation, template-only header changes and share replies also had
no coherent contract. These are prerequisites for real miners, not new share
work units or accounting-window policy.

## Decision

1. **Job activation and stale policy.** On a `(prev_hash, nbits)` change the
   pool sends one future `NewExtendedMiningJob` (empty `min_ntime`) followed by
   one `SetNewPrevHash` naming it. This includes a same-prevhash nbits change,
   because SV2 carries header bits only in `SetNewPrevHash`. For unchanged
   `(prev_hash, nbits)`, fee, distribution, transaction or version updates send
   only a nonfuture job. Ordinary GBT curtime advancement alone does not change
   the job or its immutable issued `min_ntime`; version/ntime rolling remains
   subject to the existing validation bounds. The current v1 pool supports
   only the newest job: submissions for any older job, including an older
   same-tip job, are explicitly rejected as `stale-share`; a prevhash or nbits
   activation makes every older job stale. Every SV1 republication therefore
   has `clean_jobs = true`. No older generation is silently checked against a
   newer target or template.
2. **Targets.** Every SV2 U256 target is little-endian. `SetTarget` precedes the
   job whose effective target it changes, including the first job after an
   early open. A future job captures the channel target when activated; a
   nonfuture job captures the channel target current when it is received.
   `SetTarget` never retroactively changes an already active job. The per-job
   target and credited-`d` generation therefore remain coherent until that job
   is retired by the latest-only rule above.
3. **Merkle paths.** Coinbase paths are deepest-first in every producer and
   consumer, including SV1 and SV2. A Template Provider path already in that
   order is passed through without reversal. Independent multi-level vectors
   and upstream `channels_sv2` folds are conformance gates.
4. **Channel/extranonce allocation.** One connection has one extended channel
   and one SP identity. Miners use separate connections for separate channels;
   the same identity may operate multiple workers. `min_extranonce_size > 4`
   and a `max_target` below the effective target are rejected. Miner-submitted
   extranonce bytes must be exactly the negotiated four bytes, not include the
   pool prefix. Channel ids increase per front process; the four-byte prefix is
   `front_id || little_endian_u24(counter)`, unique by construction within a
   front process lifetime. Restart uses new process-start jobs; it does not
   claim a persistent cross-restart prefix allocator.
5. **Job identity and front verification.** Bits and version are part of the
   publication key just as prevhash, transaction set and distribution are;
   ordinary curtime is not. When a curtime-only refresh does not publish, all
   other fields of the issued job, especially its `min_ntime`, remain
   immutable. Before accounting, the front checks extranonce length and
   rebuilds the coinbase, deepest-first Merkle root and header hash from the
   named current job and submitted fields. An ingress-supplied hash is not
   evidence on its own.
6. **Stable miner-facing Noise authority.** A front requires
   `[pool] authority_secret_key` in the base58check key-utils encoding and a
   matching `authority_public_key`; missing, malformed or mismatched keys fail
   startup. `spc-agent` pins `[upstream] authority_public_key` and has no
   unauthenticated fallback. Fronts of one pool that should present one
   identity are provisioned with the same pair. This decision does not change
   JDS or node-TDP authority configuration; those interfaces retain their own
   provisioning work.
7. **Correlated SV2 share outcomes.** Native SV2 `SubmitSharesSuccess` is sent
   after the front's correlated `(channel_id, sequence_number)` outcome says the
   share entered the open slice, never merely after queue insertion.
   `new_shares_sum` is the credited `d` from that job's generation. Rejections
   retain their sequence and error code. Success means front/slice acceptance,
   not node block acceptance, canonical inclusion, settlement or payout. The
   SV1 listener's immediate boolean remains an ingress acknowledgement only and
   must not be relabelled as accounting credit.

## Compatibility and boundaries

- Latest-only same-tip submissions are an explicit restriction, not silent
  scheduler behavior. Accepting a bounded same-tip history would require a
  later coordinated front, distribution and slice-generation design and is not
  claimed here.
- This packet makes no change to share-work units, window selection, payout
  tiering, denomination, Ark privacy boundaries, or the L3 design packet.
- JDS/node-TDP provisioning and the separately owned `sv2-tp` branch are
  integration inputs. When merging that branch, remove its reversal of TDP's
  already deepest-first path rather than reviving the old pool order.

## Edits applied

- Native pool SV2 job/target/channel/share handling and front/header/job
  publication paths; SV1/SV2/agent Merkle folds.
- Agent Noise pinning, job state and correlated result handling.
- Pool/agent config validation and examples, plus regtest fixture credentials.
- Independent `channels_sv2` client-state and Merkle vectors, multi-level SV1
  vectors, negotiation/stale/replay/authority/result tests.
- Matching SPEC changelog fragment under `docs/changelog/`; no direct SPEC
  frontmatter edit is made in the packet worktree.
