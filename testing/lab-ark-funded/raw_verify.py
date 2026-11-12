"""Local raw Bitcoin evidence verification; never accepts a producer status.

Contextual difficulty and whole-campaign work accounting stay with T3/T4b.
This module checks raw transaction/block identities, Merkle/witness commitments,
input values/maturity and evidence links used by the AP5 component verifier.
"""
import hashlib
from common import require


def dh(raw):
    return hashlib.sha256(hashlib.sha256(raw).digest()).digest()


def compact(n):
    if n < 253:
        return bytes([n])
    width, tag = (2, 253) if n <= 65535 else (4, 254) if n <= 4294967295 else (8, 255)
    return bytes([tag]) + n.to_bytes(width, "little")


class Reader:
    def __init__(self, raw):
        self.raw, self.pos = raw, 0

    def take(self, n):
        require(0 <= n <= 4_000_000 and self.pos + n <= len(self.raw), "truncated/oversized Bitcoin encoding")
        value = self.raw[self.pos:self.pos+n]
        self.pos += n
        return value

    def number(self, n):
        return int.from_bytes(self.take(n), "little")

    def size(self):
        first = self.number(1)
        if first < 253:
            return first
        value = self.number({253: 2, 254: 4, 255: 8}[first])
        require(value >= {253: 253, 254: 65536, 255: 4294967296}[first], "nonminimal CompactSize")
        return value

    def blob(self):
        return self.take(self.size())


def transaction(reader):
    start = reader.pos
    version = reader.take(4)
    count = reader.size()
    witness = count == 0
    if witness:
        require(reader.number(1) == 1, "unsupported witness flag")
        count = reader.size()
    require(0 < count <= 100000, "empty/oversized input list")
    vin_start = reader.pos
    inputs = []
    for _ in range(count):
        inputs.append({"txid": reader.take(32)[::-1].hex(), "vout": reader.number(4),
                       "script": reader.blob().hex(), "sequence": reader.number(4)})
    wire_inputs = reader.raw[vin_start:reader.pos]
    output_count = reader.size()
    require(0 < output_count <= 100000, "empty/oversized output list")
    output_start = reader.pos
    outputs = [{"sats": reader.number(8), "script": reader.blob().hex()} for _ in range(output_count)]
    wire_outputs = reader.raw[output_start:reader.pos]
    require(sum(o["sats"] for o in outputs) <= 2_100_000_000_000_000, "outputs exceed Bitcoin money range")
    witnesses = []
    if witness:
        for _ in inputs:
            n = reader.size()
            require(n <= 100000, "oversized witness stack")
            witnesses.append([reader.blob() for _ in range(n)])
        require(any(witnesses), "superfluous witness serialization")
    locktime = reader.take(4)
    raw = reader.raw[start:reader.pos]
    stripped = version + compact(count) + wire_inputs + compact(output_count) + wire_outputs + locktime
    weight = len(stripped) * 3 + len(raw)
    coinbase = len(inputs) == 1 and inputs[0]["txid"] == "00" * 32 and inputs[0]["vout"] == 0xffffffff
    require(coinbase or len({(i["txid"], i["vout"]) for i in inputs}) == len(inputs), "duplicate inputs")
    return {"raw": raw.hex(), "txid": dh(stripped)[::-1].hex(), "wtxid": dh(raw)[::-1].hex(),
            "vin": inputs, "vout": outputs, "witness": witnesses, "coinbase": coinbase,
            "weight": weight, "vsize": (weight + 3) // 4}


def parse_tx(raw_hex):
    r = Reader(bytes.fromhex(raw_hex))
    result = transaction(r)
    require(r.pos == len(r.raw), "trailing transaction bytes")
    return result


def merkle(leaves):
    require(leaves, "empty Merkle tree")
    while len(leaves) > 1:
        if len(leaves) % 2:
            leaves = leaves + leaves[-1:]
        leaves = [dh(leaves[i] + leaves[i+1]) for i in range(0, len(leaves), 2)]
    return leaves[0]


def parse_block(raw_hex):
    r = Reader(bytes.fromhex(raw_hex))
    header = r.take(80)
    count = r.size()
    require(0 < count <= 100000, "block transaction count")
    txs = [transaction(r) for _ in range(count)]
    require(r.pos == len(r.raw) and txs[0]["coinbase"] and not any(t["coinbase"] for t in txs[1:]), "block/coinbase shape")
    require(merkle([bytes.fromhex(t["txid"])[::-1] for t in txs]) == header[36:68], "raw block Merkle mismatch")
    bits = int.from_bytes(header[72:76], "little")
    exp, word = bits >> 24, bits & 0x7fffff
    require(word and not bits & 0x800000 and exp <= 34 and not (word > 255 and exp > 33) and not (word > 65535 and exp > 32), "invalid compact target")
    target = (word >> (8 * (3-exp))) if exp <= 3 else word << (8 * (exp-3))
    require(0 < target < 2**256 and int.from_bytes(dh(header), "little") <= target, "raw block PoW mismatch")
    if any(t["witness"] for t in txs):
        commits = [bytes.fromhex(o["script"])[6:38] for o in txs[0]["vout"]
                   if o["script"].startswith("6a24aa21a9ed") and len(o["script"]) >= 76]
        reserved = txs[0]["witness"]
        require(commits and len(reserved) == 1 and len(reserved[0]) == 1 and len(reserved[0][0]) == 32, "missing witness commitment/reserved value")
        root = merkle([bytes(32)] + [bytes.fromhex(t["wtxid"])[::-1] for t in txs[1:]])
        require(commits[-1] == dh(root + reserved[0][0]), "raw witness commitment mismatch")
    return {"hash": dh(header)[::-1].hex(), "previous": header[4:36][::-1].hex(), "transactions": txs}


def transaction_record(record, maturity):
    tx = parse_tx(record["raw"])
    require(tx["txid"] == record["txid"], "raw transaction id mismatch")
    block = parse_block(record["raw_block"])
    require(block["hash"] == record["block_hash"], "raw block id mismatch")
    require(any(t["raw"] == tx["raw"] for t in block["transactions"]), "raw transaction not included in canonical evidence block")
    previous = {(p["txid"], p["vout"]): p for p in record["inputs"]}
    require(len(previous) == len(tx["vin"]), "input proof census mismatch")
    total = 0
    for vin in tx["vin"]:
        p = previous.get((vin["txid"], vin["vout"]))
        require(p is not None, "missing input transaction")
        parent = parse_tx(p["raw"])
        require(parent["txid"] == vin["txid"] and vin["vout"] < len(parent["vout"]), "input raw transaction mismatch")
        parent_block = parse_block(p["raw_block"])
        require(parent_block["hash"] == p["block_hash"] and any(t["raw"] == parent["raw"] for t in parent_block["transactions"]), "input canonical block proof mismatch")
        require(p["height"] <= record["height"], "input from future block")
        if parent["coinbase"]:
            require(p["height"] > 0 and record["height"] >= p["height"] + maturity, "immature/genesis coinbase input")
        amount = parent["vout"][vin["vout"]]["sats"]
        require(amount == p["amount_sats"], "input amount differs from raw bytes")
        total += amount
    fee = total - sum(o["sats"] for o in tx["vout"])
    require(fee >= 0 and fee == record["fee_sats"], "raw fee mismatch")
    return {"txid": tx["txid"], "wtxid": tx["wtxid"], "fee_sats": fee,
            "vsize": tx["vsize"], "block_hash": block["hash"], "height": record["height"]}
