#!/usr/bin/env python3
"""Offline secret/key material generator for the Testnet5 pool host.

Self-contained (Python 3 stdlib only, no network, no third-party
packages); every invocation first re-runs built-in known-answer tests
(BIP32 test vector 1 and the repo's published throwaway SV2 authority
pair) and aborts on any mismatch.

Subcommands (see docs/DEPLOY-TESTNET5.md "Secrets"):

  authority        SV2 Noise authority pair (key-utils base58check), for
                   [pool] authority_public_key / authority_secret_key.
  ark-seed         64-byte hex ark seed for [ark] seed_path.
  descriptor       tr(<account tpub>/0/*) payout descriptor derived from
                   an ark seed file ([pool] pool_payout_descriptor); the
                   account path defaults to the pool's m/86'/1'/1'.
  order-key        BIP340 order-signing key: 32-byte hex secret for
                   [ark] order_key_path and the x-only public key for
                   [ark] order_pubkey.
  ark-id           first 4 bytes of SHA256(server_pubkey) as 8 hex chars,
                   for [ark] ark_id.
  rpc-cookie       Core-format "user:password" RPC cookie line.

Generated secrets print to stdout (or --out FILE, written 0600); place
them per the runbook and never commit them.
"""

import argparse
import hashlib
import hmac
import os
import secrets
import struct
import sys

# --- secp256k1 (affine, double-and-add; fine for offline keygen) ----------

P = 2**256 - 2**32 - 977
N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
G = (
    0x79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798,
    0x483ADA7726A3C4655DA4FBFC0E1108A8FD17B448A68554199C47D08FFB10D4B8,
)


def _point_add(p, q):
    if p is None:
        return q
    if q is None:
        return p
    if p[0] == q[0] and (p[1] + q[1]) % P == 0:
        return None
    if p == q:
        lam = (3 * p[0] * p[0]) * pow(2 * p[1], -1, P) % P
    else:
        lam = (q[1] - p[1]) * pow(q[0] - p[0], -1, P) % P
    x = (lam * lam - p[0] - q[0]) % P
    return (x, (lam * (p[0] - x) - p[1]) % P)


def _point_mul(k, point=G):
    result = None
    while k:
        if k & 1:
            result = _point_add(result, point)
        point = _point_add(point, point)
        k >>= 1
    return result


def pub_compressed(secret_int):
    x, y = _point_mul(secret_int)
    return bytes([2 + (y & 1)]) + x.to_bytes(32, "big")


# --- RIPEMD-160 (hashlib first; pure-python fallback for OpenSSL 3) --------


def _ripemd160_pure(data):
    # RFC-style reference implementation, verified by the self-test below.
    def rol(x, n):
        return ((x << n) | (x >> (32 - n))) & 0xFFFFFFFF

    r1 = [11, 14, 15, 12, 5, 8, 7, 9, 11, 13, 14, 15, 6, 7, 9, 8,
          7, 6, 8, 13, 11, 9, 7, 15, 7, 12, 15, 9, 11, 7, 13, 12,
          11, 13, 6, 7, 14, 9, 13, 15, 14, 8, 13, 6, 5, 12, 7, 5,
          11, 12, 14, 15, 14, 15, 9, 8, 9, 14, 5, 6, 8, 6, 5, 12,
          9, 15, 5, 11, 6, 8, 13, 12, 5, 12, 13, 14, 11, 8, 5, 6]
    r2 = [8, 9, 9, 11, 13, 15, 15, 5, 7, 7, 8, 11, 14, 14, 12, 6,
          9, 13, 15, 7, 12, 8, 9, 11, 7, 7, 12, 7, 6, 15, 13, 11,
          9, 7, 15, 11, 8, 6, 6, 14, 12, 13, 5, 14, 13, 13, 7, 5,
          15, 5, 8, 11, 14, 14, 6, 14, 6, 9, 12, 9, 12, 5, 15, 8,
          8, 5, 12, 9, 12, 5, 14, 6, 8, 13, 6, 5, 15, 13, 11, 11]
    z1 = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
          7, 4, 13, 1, 10, 6, 15, 3, 12, 0, 9, 5, 2, 14, 11, 8,
          3, 10, 14, 4, 9, 15, 8, 1, 2, 7, 0, 6, 13, 11, 5, 12,
          1, 9, 11, 10, 0, 8, 12, 4, 13, 3, 7, 15, 14, 5, 6, 2,
          4, 0, 5, 9, 7, 12, 2, 10, 14, 1, 3, 8, 11, 6, 15, 13]
    z2 = [5, 14, 7, 0, 9, 2, 11, 4, 13, 6, 15, 8, 1, 10, 3, 12,
          6, 11, 3, 7, 0, 13, 5, 10, 14, 15, 8, 12, 4, 9, 1, 2,
          15, 5, 1, 3, 7, 14, 6, 9, 11, 8, 12, 2, 10, 0, 4, 13,
          8, 6, 4, 1, 3, 11, 15, 0, 5, 12, 2, 13, 9, 7, 10, 14,
          12, 15, 10, 4, 1, 5, 8, 7, 6, 2, 13, 14, 0, 3, 9, 11]

    def f(j, x, y, z):
        if j < 16:
            return x ^ y ^ z
        if j < 32:
            return (x & y) | (~x & z)
        if j < 48:
            return (x | ~y) ^ z
        if j < 64:
            return (x & z) | (y & ~z)
        return x ^ (y | ~z)

    def k1(j):
        return [0x00000000, 0x5A827999, 0x6ED9EBA1, 0x8F1BBCDC, 0xA953FD4E][j // 16]

    def k2(j):
        return [0x50A28BE6, 0x5C4DD124, 0x6D703EF3, 0x7A6D76E9, 0x00000000][j // 16]

    h = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0]
    msg = bytearray(data)
    msg_len = len(data)
    msg.append(0x80)
    while len(msg) % 64 != 56:
        msg.append(0)
    msg += struct.pack("<Q", msg_len * 8)

    for off in range(0, len(msg), 64):
        x = struct.unpack("<16I", msg[off:off + 64])
        a1, b1, c1, d1, e1 = h
        a2, b2, c2, d2, e2 = h
        for j in range(80):
            t = (rol((a1 + f(j, b1, c1, d1) + x[z1[j]] + k1(j)) & 0xFFFFFFFF,
                     r1[j]) + e1) & 0xFFFFFFFF
            a1, e1, d1, c1, b1 = e1, d1, rol(c1, 10), b1, t
            t = (rol((a2 + f(79 - j, b2, c2, d2) + x[z2[j]] + k2(j)) & 0xFFFFFFFF,
                     r2[j]) + e2) & 0xFFFFFFFF
            a2, e2, d2, c2, b2 = e2, d2, rol(c2, 10), b2, t
        t = (h[1] + c1 + d2) & 0xFFFFFFFF
        h = [t,
             (h[2] + d1 + e2) & 0xFFFFFFFF,
             (h[3] + e1 + a2) & 0xFFFFFFFF,
             (h[4] + a1 + b2) & 0xFFFFFFFF,
             (h[0] + b1 + c2) & 0xFFFFFFFF]
    return struct.pack("<5I", *h)


def ripemd160(data):
    try:
        return hashlib.new("ripemd160", data).digest()
    except (ValueError, TypeError):
        return _ripemd160_pure(data)


def hash160(data):
    return ripemd160(hashlib.sha256(data).digest())


# --- base58check ------------------------------------------------------------

B58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def b58check_encode(payload):
    data = payload + hashlib.sha256(hashlib.sha256(payload).digest()).digest()[:4]
    num = int.from_bytes(data, "big")
    out = ""
    while num:
        num, rem = divmod(num, 58)
        out = B58_ALPHABET[rem] + out
    for byte in data:
        if byte:
            break
        out = "1" + out
    return out


def b58check_decode(text):
    num = 0
    for ch in text:
        num = num * 58 + B58_ALPHABET.index(ch)
    raw = num.to_bytes((num.bit_length() + 7) // 8, "big")
    pad = 0
    for ch in text:
        if ch != "1":
            break
        pad += 1
    raw = b"\x00" * pad + raw
    payload, check = raw[:-4], raw[-4:]
    if hashlib.sha256(hashlib.sha256(payload).digest()).digest()[:4] != check:
        raise ValueError("bad base58check checksum")
    return payload


# --- BIP32 ------------------------------------------------------------------

XPUB_MAINNET = bytes.fromhex("0488B21E")
TPUB_TESTNET = bytes.fromhex("043587CF")  # testnet family; testnet5 uses it
HARDENED = 0x80000000


def bip32_master(seed):
    digest = hmac.new(b"Bitcoin seed", seed, hashlib.sha512).digest()
    key = int.from_bytes(digest[:32], "big")
    if not 0 < key < N:
        raise ValueError("invalid master key for this seed")
    return key, digest[32:]


def bip32_ckd_priv(key, chain, index):
    if index >= HARDENED:
        data = b"\x00" + key.to_bytes(32, "big") + index.to_bytes(4, "big")
    else:
        data = pub_compressed(key) + index.to_bytes(4, "big")
    digest = hmac.new(chain, data, hashlib.sha512).digest()
    tweak = int.from_bytes(digest[:32], "big")
    child = (tweak + key) % N
    if tweak >= N or child == 0:
        raise ValueError("invalid child (astronomically unlikely); re-derive")
    return child, digest[32:]


def parse_path(path):
    if path in ("m", "m/"):
        return []
    if not path.startswith("m/"):
        raise ValueError(f"derivation path must start with m/: {path}")
    out = []
    for part in path[2:].split("/"):
        hardened = part.endswith("'") or part.endswith("h") or part.endswith("H")
        idx = int(part.rstrip("'hH"))
        out.append(idx + HARDENED if hardened else idx)
    return out


def derive_account_xpub(seed, path, version):
    key, chain = bip32_master(seed)
    parent_pub = None
    index = 0
    for index in parse_path(path):
        parent_pub = pub_compressed(key)
        key, chain = bip32_ckd_priv(key, chain, index)
    depth = len(parse_path(path))
    fingerprint = hash160(parent_pub)[:4] if parent_pub else b"\x00" * 4
    payload = (version + bytes([depth]) + fingerprint
               + index.to_bytes(4, "big") + chain + pub_compressed(key))
    return b58check_encode(payload)


# --- SV2 key-utils authority encoding (crates/spc-lab/src/secrets.rs) ------


def encode_authority_public(xonly32):
    return b58check_encode(bytes([1, 0]) + xonly32)


def encode_authority_secret(secret32):
    return b58check_encode(secret32)


# --- known-answer self-tests (run on every invocation) ----------------------


def self_test():
    # BIP32 test vector 1.
    seed = bytes.fromhex("000102030405060708090a0b0c0d0e0f")
    assert derive_account_xpub(seed, "m/0'", XPUB_MAINNET) == (
        "xpub68Gmy5EdvgibQVfPdqkBBCHxA5htiqg55crXYuXoQRKfDBFA1WEjWgP6LHhwBZeNK1"
        "VTsfTFUHCdrfp1bgwQ9xv5ski8PX9rL2dZXvgGDnw"
    ), "BIP32 vector 1 m/0' failed"
    assert derive_account_xpub(seed, "m/0'/1/2'", XPUB_MAINNET) == (
        "xpub6D4BDPcP2GT577Vvch3R8wDkScZWzQzMMUm3PWbmWvVJrZwQY4VUNgqFJPMM3No2dF"
        "DFGTsxxpG5uJh7n7epu4trkrX7x7DogT5Uv6fcLW5"
    ), "BIP32 vector 1 m/0'/1/2' failed"
    # The repo's published throwaway SV2 authority pair
    # (config/spc-pool.example.toml): secret must re-derive the public.
    secret = b58check_decode("mkDLTBBRxdBv998612qipDYoTK3YUrqLe8uWw7gu3iXbSrn2n")
    assert len(secret) == 32
    xonly = pub_compressed(int.from_bytes(secret, "big"))[1:]
    assert encode_authority_public(xonly) == (
        "9auqWEzQDVyd2oe1JVGFLMLHZtCo2FFqZwtKA5gd9xbuEu7PH72"
    ), "SV2 authority key-utils encoding failed"
    # Pure ripemd160 agrees with the test vector "abc".
    assert _ripemd160_pure(b"abc").hex() == (
        "8eb208f7e05d987a9b044a8e98c6b087f15a0bfc"
    ), "pure ripemd160 failed"


# --- subcommands -------------------------------------------------------------


def write_secret_or_print(text, out, newline=True):
    if out:
        fd = os.open(out, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w") as fh:
            fh.write(text + ("\n" if newline else ""))
        print(f"wrote {out} (0600)")
    else:
        print(text)


def cmd_authority(_args):
    while True:
        secret = secrets.token_bytes(32)
        if 0 < int.from_bytes(secret, "big") < N:
            break
    xonly = pub_compressed(int.from_bytes(secret, "big"))[1:]
    print(f"authority_public_key = \"{encode_authority_public(xonly)}\"")
    print(f"authority_secret_key = \"{encode_authority_secret(secret)}\"")


def cmd_ark_seed(args):
    write_secret_or_print(secrets.token_bytes(64).hex(), args.out)


def cmd_descriptor(args):
    seed = bytes.fromhex(open(args.seed_file).read().strip())
    if len(seed) != 64:
        sys.exit(f"{args.seed_file}: expected 64 bytes of hex (128 chars), got {len(seed)}")
    xpub = derive_account_xpub(seed, args.account, TPUB_TESTNET)
    print(f"pool_payout_descriptor = \"tr({xpub}/0/*)\"")


def cmd_order_key(args):
    while True:
        secret = secrets.token_bytes(32)
        if 0 < int.from_bytes(secret, "big") < N:
            break
    xonly = pub_compressed(int.from_bytes(secret, "big"))[1:]
    print(f"order_pubkey = \"{xonly.hex()}\"")
    write_secret_or_print(secret.hex(), args.out)


def cmd_ark_id(args):
    pubkey = bytes.fromhex(args.server_pubkey)
    if len(pubkey) != 33:
        sys.exit("server_pubkey must be 33 bytes (66 hex chars, compressed)")
    print(f"ark_id = \"{hashlib.sha256(pubkey).hexdigest()[:8]}\"")


def cmd_rpc_cookie(args):
    # Core cookie format: exactly "user:password", NO trailing newline
    # (the node refuses a cookie that ends with a line ending).
    write_secret_or_print(f"{args.user}:{secrets.token_hex(24)}", args.out,
                          newline=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("authority").set_defaults(func=cmd_authority)
    p = sub.add_parser("ark-seed")
    p.add_argument("--out")
    p.set_defaults(func=cmd_ark_seed)
    p = sub.add_parser("descriptor")
    p.add_argument("--seed-file", required=True)
    p.add_argument("--account", default="m/86'/1'/1'")
    p.set_defaults(func=cmd_descriptor)
    p = sub.add_parser("order-key")
    p.add_argument("--out")
    p.set_defaults(func=cmd_order_key)
    p = sub.add_parser("ark-id")
    p.add_argument("--server-pubkey", required=True)
    p.set_defaults(func=cmd_ark_id)
    p = sub.add_parser("rpc-cookie")
    p.add_argument("--user", default="spcpool")
    p.add_argument("--out")
    p.set_defaults(func=cmd_rpc_cookie)
    args = parser.parse_args()
    self_test()
    args.func(args)


if __name__ == "__main__":
    main()
