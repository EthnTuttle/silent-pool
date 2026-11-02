# SPc: silent payments in a coinbase

Status: draft for BIP review. This is the standalone text of `docs/SPEC.md` §2.
`DESIGN.md` is authoritative for the derivation and tier policy; where this
document and `DESIGN.md` disagree, `DESIGN.md` wins.

## 1. Motivation

BIP 352 silent payments cannot be used in a coinbase: the shared secret is
derived from the sum of input public keys, and a coinbase input has no key. SPc
supplies a nonce that a coinbase can carry instead: a per-height ephemeral
public key `A_send` in the coinbase itself. The block height, which BIP 34
already forces into every coinbase, makes the nonce consensus-unique.

The result keeps BIP 352's receiver properties for the coinbase tier: each
miner's output is a one-time P2TR key, unlinkable to the miner's address or to
the miner's outputs in other blocks by a third-party chain observer. It does
not hide amounts, and it does not hide anything from the pool that validated
the shares.

## 2. Notation

- `ser_P(P)`: 33-byte SEC1 compressed point.
- `ser_x(P)`: 32-byte x-coordinate.
- `ser32(i)`: 4-byte big-endian.
- `hash_tag(m)`: BIP 340 tagged hash, `SHA256(SHA256(tag) || SHA256(tag) || m)`.
- `G`, `n`: secp256k1 generator and group order.
- `H`: the height of the block whose coinbase carries the outputs.
- `k`: output index within one `(receiver, H)`; `0` in v0.

Every hash-to-scalar (`input_hash`, `t_onchain`, `t_ark`, `t_mb`) **fails** if
the digest is `0` or `>= n`. The value is never reduced modulo `n`. A point at
infinity also fails. A sender that fails `input_hash` draws a new `A_send`; a
per-receiver failure omits that receiver from the tier and does not affect any
other output.

## 3. Receiver identity

A receiver publishes a BIP 352 v0 address unchanged:

```
addr = bech32m("sp" | "tsp", version=0, ser_P(B_scan) || ser_P(B_spend))
```

`sp` is mainnet; `tsp` is testnet4, signet or regtest. The payload is exactly 66
bytes and the address is 116 or 117 characters. SPc does not require a new
address version: a v0 receiver key pair is used directly.

The pool's internal ledger key is:

```
sp_id = hash_SPc/Id(ser_P(B_scan) || ser_P(B_spend))
```

`sp_id` is 32 bytes and never appears on the wire or in a coinbase. It is the
only account key; an optional `.worker` suffix on a submitted identity is
statistics-only and is stripped at the first `.`.

## 4. Sending key

For each height `H`:

```
a_raw  <- CSPRNG 32 bytes, reduced mod n, reject 0
A_raw   = a_raw * G
a_send  = n - a_raw  if A_raw has odd Y, else a_raw
A_send  = a_send * G
```

`a_send` MUST come from an OS CSPRNG. It MUST NOT be derived from a seed, an
xprv, a descriptor, or an earlier `a_send`: additive-tweak ladders let one
secret regenerate every key and make erasure impossible. It exists only in the
pool's front-process memory, is zeroised when the height advances, and is never
written to disk, logs, Redis or Postgres. `ser_x(A_send)` is public and may be
stored.

A restart mid-height generates a new `a_send` for the same `H`. Outputs differ
and both are scannable. Reusing one `a_send` across two heights is not a
correctness failure because the height is bound into `input_hash`, but it
widens forward secrecy; a pool MUST alarm if `ser_x(A_send)` repeats across
heights in its own sequence.

## 5. Derivation

One shared secret per `(receiver, H)`:

```
input_hash = hash_SPc/Inputs(ser32(H) || ser_P(A_send))
ecdh       = ser_P(input_hash * a_send * B_scan)   (sender)
           = ser_P(input_hash * b_scan * A_send)   (receiver)

t_onchain  = hash_SPc/TierSpc(ser_P(ecdh) || ser32(k))
P          = B_spend + t_onchain * G
```

The coinbase output is `OP_1 <ser_x(P)>`, a P2TR key-path output. The receiver
spends with `d = b_spend + t_onchain (mod n)`, negated if `P` has odd Y, exactly
as BIP 352 §Spending.

The tag is the domain separator. `SPc/SharedSecret` is reserved for an optional
32-byte digest of `ecdh` and is not a tier input. A second tier (Ark) uses
`SPc/TierArk` over the same `ecdh`; the two tags MUST differ, because
`P = B_spend + t*G` and `U = B_ark + t*G` coexisting would expose
`B_ark - B_spend`.

`k = 0` in v0: one output per `(receiver, H)`.

## 6. Carriers

`A_send` is carried in one of two places. A scanner tries the scriptSig first,
then the `OP_RETURN`.

### 6.1 scriptSig (pool-template mode)

The coinbase input script is exactly:

```
<BIP34 height push> OP_PUSHBYTES_32 <ser_x(A_send)> <extranonce>
```

The extranonce is part of the Stratum job's rolled bytes, so the key is
structurally outside them. The Template Provider reserves 100 bytes of
scriptSig; the budget is:

```
100 - height_push_len - 33 - extranonce_len >= 0
```

`height_push_len` is the minimal BIP 34 script-number push: 1 byte for heights
up to 16, 2 for 17..255 (255 needs a sign byte), 3 for 256..65535, 4 for
65536..16777215, and 5 for heights at or above 16777216. A v0 pool SHOULD
allocate at most 16 bytes of extranonce, leaving at least 47 bytes spare.

### 6.2 OP_RETURN (Job Declaration mode)

When the miner builds the coinbase, the pool's only pool-to-coinbase data
channel is an additional output:

```
OP_RETURN OP_PUSHBYTES_35 0x53 0x50 0x63 <ser_x(A_send)>
```

`0x53 0x50 0x63` is ASCII `SPc`. The script is 37 bytes and the output value is
0. It costs 8 (value) + 1 (script length) + 37 = 46 bytes, 184 WU, of real
block space. A pool-template coinbase MUST NOT emit it: it would be a
protocol fingerprint with no benefit.

## 7. Scanning

For each block, given `(header, coinbase, merkle_branch)`:

```
REQUIRE merkle_branch proves the coinbase at index 0 under header.merkle_root
H = parse_bip34_height(coinbase.vin[0].scriptSig)
A = scriptsig_push32_after_height(coinbase) ?? op_return_SPc(coinbase.vout)
if A is None: skip
for h in {H, H-1}:
    ih = hash_SPc/Inputs(ser32(h) || 0x02 || A)
    ecdh = ser_P(ih * b_scan * lift_x_even(A))
    t = hash_SPc/TierSpc(ser_P(ecdh) || ser32(0))
    P = B_spend + t*G
    if OP_1 <ser_x(P)> is in coinbase.vout: record (txid, vout, amount, t)
```

The `H-1` tolerance is belt-and-braces for a declaration built before a height
change. A pool MUST NOT rely on it to accept a stale Job Declaration; the grace
window is same-height-only.

A light client needs the 80-byte header, the ~12×32-byte branch stack, the
coinbase and `A_send`: roughly 500 bytes per block, verifiable against the
header. Spending needs `b_spend + t`; exporting `(outpoint, t)` pairs is safe
because the outputs were found by derivation, not by notification.

## 8. Security considerations

- **Third-party chain observers** cannot link a coinbase output to a receiver
  address or to that receiver's outputs in other blocks without `a_send`.
  Compromise of one `a_send` reveals one height's payee set and nothing else;
  it authorises no funds.
- **The pool** sees every identity and every amount, as it must to account. SPc
  does not attempt to hide this; operational scrubbing is the mitigation.
- **Amounts are not hidden.** A stable hashrate share produces a stable
  `amount_i / total` across blocks, which is a fingerprint independent of
  addresses.
- **The coinbase is public before it is mined.** Miners hashing a template see
  the outputs; this is unavoidable and not a privacy loss beyond the amount
  channel.
- **Grinding.** `input_hash` commits to `A_send`, so a sender cannot reuse an
  earlier payment's shared secret by choosing `A_send` adaptively. A naive
  fixed-point search is not feasible; a real proof obligation is recorded in
  the test vectors.
- **Eligibility.** SPc deliberately violates BIP 352's input-eligibility gate
  (a coinbase has no spendable input key). A future BIP should either amend 352
  for coinbase scope or define a new informational BIP; the construction does
  not depend on that choice because its tags are disjoint.

## 9. Test vectors

The frozen vector is `crates/spc-crypto/tests/vectors.rs`, ported from the
validated prototype in `spc_ark/src/lib.rs`:

- seed `0x42`×32, height `800000`, `a_send = 0x07`×32, `k = 0`;
- `input_hash`, both sides' `ecdh`, `t_onchain`, `t_ark`, `P`, `U`, the mailbox
  hint `D`, the blinded mailbox id, and the resulting `ark1` address;
- negative controls: odd-Y `A_send` without negation misses; `t_onchain` and
  `t_ark` differ; a zero or `>= n` hash fails; a bare `bc1q` address, wrong
  HRP, bad checksum, wrong version and truncated address are rejected.

`crates/spc-crypto/tests/bip352_address.rs` additionally checks the address
parser and encoder against the BIP 352 `send_and_receive_test_vectors.json`
reference address.

## 10. References

- BIP 352 Silent Payments.
- BIP 340 Schnorr signatures, BIP 341 taproot, BIP 34 coinbase height.
- `DESIGN.md` in this repository (authoritative derivation and tier policy).
- `docs/SPEC.md` §2 (normative project text) and §11 (Ark tier).
