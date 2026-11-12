"""Independent legacy P2PKH SIGHASH_ALL proof for the funded v3 case.

No signer, wallet, production helper or node call. The node's early version
policy rejection is deliberately not used as signature evidence.
"""
import hashlib

from oracle import Chain, ChainPrefix, Inconclusive, Reader, compact_size, decode_transaction, integer, require, sha256d, unhex
from public_derivation import _P, _N, _GX, _GY, _point_add, ripemd160


def _multiply(scalar, point):
    result = None
    for bit in bin(scalar % _N)[2:]:
        result = _point_add(result, result)
        if bit == "1":
            result = _point_add(result, point)
    return result


def public_point(encoded):
    if len(encoded) == 33 and encoded[0] in (2, 3):
        x = int.from_bytes(encoded[1:], "big")
        require(x < _P, "ECDSA public x coordinate range")
        y2 = (x**3 + 7) % _P
        y = pow(y2, (_P + 1) // 4, _P)
        require(y * y % _P == y2, "ECDSA public key is not on secp256k1")
        if y % 2 != encoded[0] % 2:
            y = _P - y
    elif len(encoded) == 65 and encoded[0] == 4:
        x, y = int.from_bytes(encoded[1:33], "big"), int.from_bytes(encoded[33:], "big")
        require(x < _P and y < _P and y * y % _P == (x**3 + 7) % _P, "uncompressed ECDSA public key range/curve")
    else:
        raise Inconclusive("unsupported legacy public-key encoding")
    return x, y


def der_scalars(signature):
    require(8 <= len(signature) <= 72 and signature[0] == 0x30 and signature[1] == len(signature) - 2,
            "noncanonical DER sequence")
    reader = Reader(signature[2:])
    values = []
    for _ in range(2):
        require(reader.uint(1) == 2, "DER scalar tag")
        encoded = reader.take(reader.uint(1))
        require(encoded and not encoded[0] & 0x80 and
                (len(encoded) == 1 or encoded[0] != 0 or encoded[1] & 0x80), "DER scalar sign/minimal encoding")
        value = int.from_bytes(encoded, "big")
        require(0 < value < _N, "ECDSA scalar range")
        values.append(value)
    reader.end()
    require(values[1] <= _N // 2, "non-low-S production signature")
    return values


def _push(reader):
    length = reader.uint(1)
    require(1 <= length <= 75, "nonminimal or unsupported P2PKH scriptSig push")
    return reader.take(length)


def sighash_all_preimage(tx, index, script_code):
    integer(index, 0, len(tx["inputs"]) - 1)
    result = tx["stripped"][:4] + compact_size(len(tx["inputs"]))
    for i, item in enumerate(tx["inputs"]):
        txid, vout = item["outpoint"].split(":")
        script = script_code if i == index else b""
        result += (unhex(txid, 32)[::-1] + integer(int(vout), 0, 0xffffffff).to_bytes(4, "little")
                   + compact_size(len(script)) + script + item["sequence"].to_bytes(4, "little"))
    result += compact_size(len(tx["outputs"]))
    for output in tx["outputs"]:
        result += output["value"].to_bytes(8, "little") + compact_size(len(output["script"])) + output["script"]
    return result + tx["stripped"][-4:] + b"\x01\x00\x00\x00"


def verify_legacy_sighash_all(raw, view, *, expected_version):
    require(isinstance(view, (Chain, ChainPrefix)), "signature proof requires independently decoded raw chain")
    tx = decode_transaction(raw)
    require(not tx["coinbase"] and not tx["witness"], "funded version case must use legacy production inputs")
    version = int.from_bytes(tx["stripped"][:4], "little", signed=True)
    require(version == expected_version, "signed transaction version mismatch")
    proofs = []
    input_total = 0
    for index, item in enumerate(tx["inputs"]):
        point = item["outpoint"]
        if point not in view.utxos:
            raise Inconclusive("signature input lacks canonical unspent provenance at send anchor")
        txid, vout = point.split(":")
        if txid not in view.transactions:
            raise Inconclusive("signature input raw parent transaction missing")
        parent_block, parent_claim = view.transactions[txid]
        parent = decode_transaction(parent_claim["raw"])
        require(parent["txid"] == txid, "raw prevout transaction id mismatch")
        vout = integer(int(vout), 0, len(parent["outputs"]) - 1)
        prev = parent["outputs"][vout]
        require(view.utxos[point]["value"] == prev["value"] and view.utxos[point]["script"] == prev["script"],
                "raw prevout differs from canonical UTXO index")
        require(not parent["coinbase"] or view.tip["height"] - parent_block["height"] >= view.policy.coinbase_maturity,
                "signature input coinbase is immature at observed send anchor")
        script = prev["script"]
        require(len(script) == 25 and script[:3] == b"\x76\xa9\x14" and script[-2:] == b"\x88\xac", "signature input is not P2PKH")
        reader = Reader(item["script"])
        signed, public = _push(reader), _push(reader)
        reader.end()
        require(signed[-1] == 1, "funded v3 proof requires SIGHASH_ALL without ANYONECANPAY")
        require(ripemd160(hashlib.sha256(public).digest()) == script[3:23], "input public key does not own raw prevout")
        point_public = public_point(public)
        r, s = der_scalars(signed[:-1])
        message = sha256d(sighash_all_preimage(tx, index, script))
        inverse = pow(s, -1, _N)
        check = _point_add(_multiply(int.from_bytes(message, "big") * inverse, (_GX, _GY)),
                           _multiply(r * inverse, point_public))
        require(check is not None and check[0] % _N == r, "SIGHASH_ALL ECDSA verification failed")
        input_total += prev["value"]
        proofs.append({"outpoint": item["outpoint"], "input_index": index, "prevout_value_sats": prev["value"],
                       "canonical_height": parent_block["height"], "coinbase": parent["coinbase"],
                       "sighash_sha256d": message.hex()})
    fee = input_total - sum(o["value"] for o in tx["outputs"])
    require(fee >= 0, "signature proof has negative raw fee")
    return {"scope": "raw_legacy_signatures_and_prevouts", "txid": tx["txid"], "raw_sha256": hashlib.sha256(raw).hexdigest(),
            "transaction_version": version, "sighash_all_inputs_verified": len(proofs), "inputs": proofs,
            "fee_sats": fee, "vbytes": tx["vbytes"], "node_acceptance_or_rejection_inferred": False}
