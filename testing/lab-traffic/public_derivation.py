"""Read-only script oracle for the declared PUBLIC lab-seed key mode.

No wallet, signature, seed file or funding operation. This implements the
documented HMAC domain and standard BIP32 solely to check public scripts.
The deterministic lab seed is read from the scenario/actor allocation, never
from a wallet private-key file. RIPEMD160 is included because it is optional
in Python/OpenSSL; its public standard vectors are tested independently.
"""
import hashlib
import hmac
from functools import lru_cache
import struct

from oracle import integer, require

_P = 2**256 - 2**32 - 977
_N = 0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141
_GX = 0x79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798
_GY = 0x483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8
_RL = (
    (0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15),
    (7,4,13,1,10,6,15,3,12,0,9,5,2,14,11,8),
    (3,10,14,4,9,15,8,1,2,7,0,6,13,11,5,12),
    (1,9,11,10,0,8,12,4,13,3,7,15,14,5,6,2),
    (4,0,5,9,7,12,2,10,14,1,3,8,11,6,15,13))
_RR = (
    (5,14,7,0,9,2,11,4,13,6,15,8,1,10,3,12),
    (6,11,3,7,0,13,5,10,14,15,8,12,4,9,1,2),
    (15,5,1,3,7,14,6,9,11,8,12,2,10,0,4,13),
    (8,6,4,1,3,11,15,0,5,12,2,13,9,7,10,14),
    (12,15,10,4,1,5,8,7,6,2,13,14,0,3,9,11))
_SL = (
    (11,14,15,12,5,8,7,9,11,13,14,15,6,7,9,8),
    (7,6,8,13,11,9,7,15,7,12,15,9,11,7,13,12),
    (11,13,6,7,14,9,13,15,14,8,13,6,5,12,7,5),
    (11,12,14,15,14,15,9,8,9,14,5,6,8,6,5,12),
    (9,15,5,11,6,8,13,12,5,12,13,14,11,8,5,6))
_SR = (
    (8,9,9,11,13,15,15,5,7,7,8,11,14,14,12,6),
    (9,13,15,7,12,8,9,11,7,7,12,7,6,15,13,11),
    (9,7,15,11,8,6,6,14,12,13,5,14,13,13,7,5),
    (15,5,8,11,14,14,6,14,6,9,12,9,12,5,15,8),
    (8,5,12,9,12,5,14,6,8,13,6,5,15,13,11,11))


def ripemd160(data):
    mask = 0xffffffff
    def rol(n, count):
        n &= mask
        return ((n << count) | (n >> (32 - count))) & mask
    def fun(round_, x, y, z):
        return (x ^ y ^ z, (x & y) | (~x & z), (x | ~y) ^ z,
                (x & z) | (y & ~z), x ^ (y | ~z))[round_]
    message = data + b"\x80" + bytes((55 - len(data)) % 64) + struct.pack("<Q", len(data) * 8)
    state = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476, 0xc3d2e1f0]
    kl = (0, 0x5a827999, 0x6ed9eba1, 0x8f1bbcdc, 0xa953fd4e)
    kr = (0x50a28be6, 0x5c4dd124, 0x6d703ef3, 0x7a6d76e9, 0)
    for offset in range(0, len(message), 64):
        words = struct.unpack("<16I", message[offset:offset + 64])
        left, right = state[:], state[:]
        for j in range(80):
            r, i = divmod(j, 16)
            for registers, order, shifts, constant, f in (
                (left, _RL, _SL, kl[r], r), (right, _RR, _SR, kr[r], 4 - r)
            ):
                a, b, c, d, e = registers
                value = (rol(a + fun(f, b, c, d) + words[order[r][i]] + constant, shifts[r][i]) + e) & mask
                registers[:] = (e, value, b, rol(c, 10), d)
        a, b, c, d, e = left
        aa, bb, cc, dd, ee = right
        state = [(state[1] + c + dd) & mask, (state[2] + d + ee) & mask,
                 (state[3] + e + aa) & mask, (state[4] + a + bb) & mask,
                 (state[0] + b + cc) & mask]
    return struct.pack("<5I", *state)


def _point_add(first, second):
    if first is None:
        return second
    if second is None:
        return first
    x, y = first
    u, v = second
    if x == u:
        if (y + v) % _P == 0:
            return None
        slope = 3 * x * x * pow(2 * y, -1, _P) % _P
    else:
        slope = (v - y) * pow(u - x, -1, _P) % _P
    xx = (slope * slope - x - u) % _P
    return xx, (slope * (x - xx) - y) % _P


def _public_scalar(scalar):
    require(0 < scalar < _N, "BIP32 scalar range")
    point = None
    for bit in bin(scalar)[2:]:
        point = _point_add(point, point)
        if bit == "1":
            point = _point_add(point, (_GX, _GY))
    x, y = point
    return bytes([2 + y % 2]) + x.to_bytes(32, "big")


def _master(seed):
    digest = hmac.digest(b"Bitcoin seed", seed, "sha512")
    key = int.from_bytes(digest[:32], "big")
    require(0 < key < _N, "BIP32 invalid master")
    return key, digest[32:]


def _child(parent, number):
    integer(number, 0, 0xffffffff)
    key, chaincode = parent
    prefix = b"\0" + key.to_bytes(32, "big") if number & 0x80000000 else _public_scalar(key)
    digest = hmac.digest(chaincode, prefix + number.to_bytes(4, "big"), "sha512")
    tweak = int.from_bytes(digest[:32], "big")
    require(tweak < _N and (key + tweak) % _N != 0, "BIP32 invalid child at declared index")
    return (key + tweak) % _N, digest[32:]


def role_label(role):
    kind = role["kind"]
    if kind == "bootstrap_funding":
        require(set(role) == {"kind"}, "bootstrap role fields")
        return "spc-lab/traffic/bootstrap-funding"
    require(kind in {"network_wallet", "user_payment"} and set(role) == {"kind", "index"}, "wallet role fields")
    index = integer(role["index"], 0, 0xffffffff)
    prefix = "network-wallet" if kind == "network_wallet" else "user-payment-wallet"
    return "spc-lab/traffic/" + prefix + "/" + str(index)


@lru_cache(maxsize=32)
def _branch(run_seed, run_id, label, chain):
    seed = hmac.digest(integer(run_seed).to_bytes(8, "big"), run_id.encode("ascii") + b"\0" + label.encode("ascii"), "sha256")
    return _child(_master(seed), (0 if chain == "receive" else 1) | 0x80000000)


def expected_p2pkh_script(run_seed, run_id, role, chain, index, *, hardened_index):
    require(chain in {"receive", "change"}, "unknown derivation chain")
    integer(index, 0, 0x7fffffff)
    require(type(hardened_index) is bool, "explicit source-bound child-index policy required")
    require(isinstance(run_id, str) and 1 <= len(run_id) <= 128 and
            all(c.isascii() and (c.isalnum() or c in "-_.:/@") for c in run_id), "public-seed run id")
    child_number = index | 0x80000000 if hardened_index else index
    public = _public_scalar(_child(_branch(run_seed, run_id, role_label(role), chain), child_number)[0])
    return b"\x76\xa9\x14" + ripemd160(hashlib.sha256(public).digest()) + b"\x88\xac"
