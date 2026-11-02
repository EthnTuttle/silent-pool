# 2026-11-02 - Slice stream format, SV2 client baseline, regtest proof

**Trigger.** Implementing the front, payout and agent roles exposed three gaps
the spec left to `WIRE-FORMATS.md`: the `spc:slices` payload had no byte
definition, the SV2 client/server needed a concrete networking crate, and the
block assembler had to be checked against a real `bitcoind` rather than a mock.

## Decision

1. **`spc:slices` is defined in `WIRE-FORMATS.md` §3.6.** A sealed slice is
   `slice_id`, height, prev hash, job id, ref fees, share count, `U128`
   difficulty, root, a `B4`-length share blob, and per-identity aggregates.
   The share blob is JSON of `spc-share-ext::ShareLeaf` only: no `sp_id`, no
   script, no payout key. It is the retained type-32 payload and the scrub
   target.
2. **SV2 networking uses `stratum-apps` 0.8 `network` helpers** on top of
   `stratum-core` 0.6.0. The authority keypair is a separate `secp256k1` 0.28
   instance, aliased as `secp256k1_legacy`, because `stratum-apps` pins that
   version; the `bitcoin`/`secp256k1` 0.29 types stay on the payout and crypto
   side. The two never share a key object, only bytes on the wire.
3. **Bitcoin block serialization is a raw transaction list.** The assembler
   must not length-prefix each transaction; only the transaction count is
   compact-size. The mock tests did not catch this; the real `bitcoind`
   regtest run did.
4. **Regtest proof.** `testing/regtest-e2e.sh` starts bitcoind, builds a real
   template, runs `spc-pool --roles front` with native SV1 and SV2, mines two
   shares with a synthetic SV1 miner, submits two blocks, and recovers the
   miner's coinbase output with `spc-scan`. The first block has an empty
   window and pays only the pool; the second pays the miner. This is the
   acceptance baseline for later changes.

## Why

- The stream format is the cross-process contract between `front` and
  `payout`; without it the payout role cannot persist or audit a slice, and a
  later format change would silently break the type-32 service.
- Keeping the share blob identity-free is a privacy requirement, not an
  optimization (SPEC §4.2, §5.1). A `ShareRec` blob would have carried
  `sp_id` into a retained, miner-readable service.
- A mock template cannot catch consensus serialization, `getblocktemplate`
  rule flags, or the coinbase/extranonce split. Those are exactly the failures
  that only appear against a node.

## Alternatives rejected

- **JSON for the whole stream**: loses the fixed numeric widths and makes
  `U128`/`B4` ambiguous; the rest of the stream family is binary.
- **Carry `ShareRec` in the blob**: simpler, but leaks identity into the
  type-32 path and the scrub target.
- **Use `stratum-apps`'s payout/template helpers as the pool**: they do not
  carry the SPc coinbase or the PR #203 deltas, and the pinned upstream still
  has the `block_reserved_weight` defect. The network helpers are the useful
  part.

## Edits applied

- `docs/WIRE-FORMATS.md` §3.6; `docs/SPEC.md` changelog for the API signature.
- `UPSTREAM_DEPS.md`: `stratum-core`/`stratum-apps` baseline row.
- `README.md`, `docs/STATUS.md`.
