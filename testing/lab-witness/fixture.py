"""EXPLICIT TEST-ONLY witness keys and spend construction (no wallet/funder).

Two publicly known keys, no seed derivation, no persistent ledger, no RPC,
no mining. Funding MUST come from the production T1/T2 signer. Never use
these addresses outside an isolated owned regtest. The oracle does not import
this module. Algorithms follow BIP143/BIP340/BIP341 and RFC6979.
"""
import hashlib
import hmac

P = 2**256 - 2**32 - 977
N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
G = (0x79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798,
     0x483ADA7726A3C4655DA4FBFC0E1108A8FD17B448A68554199C47D08FFB10D4B8)
FIXTURE_KEYS = {"p2wpkh": 1, "p2tr": 2}


def add(a, b):
    if a is None:
        return b
    if b is None:
        return a
    if a[0] == b[0] and (a[1] + b[1]) % P == 0:
        return None
    slope = ((3 * a[0] * a[0]) * pow(2 * a[1], -1, P) if a == b else
             (b[1] - a[1]) * pow(b[0] - a[0], -1, P)) % P
    x = (slope * slope - a[0] - b[0]) % P
    return x, (slope * (a[0] - x) - a[1]) % P


def mul(k, point=G):
    result = None
    while k:
        if k & 1:
            result = add(result, point)
        point = add(point, point)
        k >>= 1
    return result


def h(data):
    return hashlib.sha256(data).digest()


def h2(data):
    return h(h(data))


def tag(name, data):
    prefix = h(name.encode())
    return h(prefix + prefix + data)


def b32(n):
    return n.to_bytes(32, "big")


def varint(n):
    if n < 253:
        return bytes([n])
    if n <= 65535:
        return b"\xfd" + n.to_bytes(2, "little")
    return b"\xfe" + n.to_bytes(4, "little")


def blob(data):
    return varint(len(data)) + data


def pubkey(secret):
    x, y = mul(secret)
    return bytes([2 | (y & 1)]) + b32(x)


def keyhash(secret):
    # This fixture has exactly one P2WPKH key, the public secp256k1 generator.
    # Its published HASH160 avoids depending on OpenSSL's optional RIPEMD160
    # provider; accepting arbitrary wallet keys is deliberately not an API.
    if secret != 1:
        raise ValueError("only the declared P2WPKH fixture key is supported")
    return bytes.fromhex("751e76e8199196d454941c45d1b3a323f1433bd6")


def der_integer(n):
    raw = n.to_bytes((n.bit_length() + 7) // 8, "big")
    if raw[0] & 128:
        raw = b"\0" + raw
    return b"\x02" + bytes([len(raw)]) + raw


def ecdsa(secret, message):
    v, k = b"\x01" * 32, bytes(32)
    data = b32(secret) + b32(int.from_bytes(message, "big") % N)
    k = hmac.digest(k, v + b"\0" + data, "sha256")
    v = hmac.digest(k, v, "sha256")
    k = hmac.digest(k, v + b"\1" + data, "sha256")
    v = hmac.digest(k, v, "sha256")
    for _ in range(16):
        v = hmac.digest(k, v, "sha256")
        nonce = int.from_bytes(v, "big")
        if 0 < nonce < N:
            r = mul(nonce)[0] % N
            s = (pow(nonce, -1, N) * (int.from_bytes(message, "big") + r * secret)) % N
            if r and s:
                encoded = der_integer(r) + der_integer(min(s, N - s))
                return b"\x30" + bytes([len(encoded)]) + encoded
        k = hmac.digest(k, v + b"\0", "sha256")
        v = hmac.digest(k, v, "sha256")
    raise ValueError("fixture signing nonce bound")


def schnorr(secret, message):
    x, y = mul(secret)
    d = secret if y % 2 == 0 else N - secret
    t = bytes(a ^ b for a, b in zip(b32(d), tag("BIP0340/aux", bytes(32))))
    k = int.from_bytes(tag("BIP0340/nonce", t + b32(x) + message), "big") % N
    if not k:
        raise ValueError("fixture zero nonce")
    rx, ry = mul(k)
    k = k if ry % 2 == 0 else N - k
    e = int.from_bytes(tag("BIP0340/challenge", b32(rx) + b32(x) + message), "big") % N
    return b32(rx) + b32((k + e * d) % N)


def address(kind):
    """BIP173/BIP350 bcrt address. No production identity derivation."""
    version = 0 if kind == "p2wpkh" else 1
    program = keyhash(FIXTURE_KEYS[kind]) if version == 0 else b32(mul(FIXTURE_KEYS[kind])[0])
    data, acc, bits = [version], 0, 0
    for value in program:
        acc = (acc << 8) | value
        bits += 8
        while bits >= 5:
            bits -= 5
            data.append((acc >> bits) & 31)
    if bits:
        data.append((acc << (5 - bits)) & 31)
    hrp = "bcrt"
    chk = 1
    for value in [ord(c) >> 5 for c in hrp] + [0] + [ord(c) & 31 for c in hrp] + data + [0] * 6:
        top = chk >> 25
        chk = ((chk & 0x1ffffff) << 5) ^ value
        for i, generator in enumerate((0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3)):
            if (top >> i) & 1:
                chk ^= generator
    chk ^= 1 if version == 0 else 0x2bc830a3
    alphabet = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
    return hrp + "1" + "".join(alphabet[n] for n in data + [(chk >> (5 * (5 - i))) & 31 for i in range(6)])


def script(kind):
    secret = FIXTURE_KEYS[kind]
    return b"\0\x14" + keyhash(secret) if kind == "p2wpkh" else b"\x51\x20" + b32(mul(secret)[0])


def spend(kind, funding_txid, vout, amount_sats, destination_script, fee_sats):
    if kind not in FIXTURE_KEYS or not 0 < fee_sats < amount_sats:
        raise ValueError("fixture spend parameters")
    if len(bytes.fromhex(funding_txid)) != 32 or not 0 <= vout < 2**32:
        raise ValueError("fixture outpoint")
    secret = FIXTURE_KEYS[kind]
    outpoint = bytes.fromhex(funding_txid)[::-1] + vout.to_bytes(4, "little")
    sequence = bytes.fromhex("fdffffff")
    version, locktime = b"\x02\0\0\0", bytes(4)
    output = (amount_sats - fee_sats).to_bytes(8, "little") + blob(destination_script)
    if kind == "p2wpkh":
        scriptcode = b"\x76\xa9\x14" + keyhash(secret) + b"\x88\xac"
        preimage = (version + h2(outpoint) + h2(sequence) + outpoint + blob(scriptcode)
                    + amount_sats.to_bytes(8, "little") + sequence + h2(output) + locktime + b"\x01\0\0\0")
        witness = [ecdsa(secret, h2(preimage)) + b"\x01", pubkey(secret)]
    else:
        # BIP341 SIGHASH_DEFAULT, one input, no annex, ext_flag=0.
        sigmsg = (b"\0" + version + locktime + h(outpoint)
                  + h(amount_sats.to_bytes(8, "little")) + h(blob(script(kind)))
                  + h(sequence) + h(output) + b"\0" + bytes(4))
        witness = [schnorr(secret, tag("TapSighash", b"\0" + sigmsg))]
    body = b"\1" + outpoint + b"\0" + sequence + b"\1" + output
    witness_bytes = varint(len(witness)) + b"".join(blob(item) for item in witness)
    return version + b"\0\1" + body + witness_bytes + locktime
