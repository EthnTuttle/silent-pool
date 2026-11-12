"""Independent, read-only Bitcoin evidence oracle. No production module imports.

Hashes are display-endian strings at interfaces, internal bytes in Merkle trees.
Amounts only come from consensus bytes. Missing evidence is never a zero.
"""
from dataclasses import dataclass
from collections.abc import Mapping
from decimal import Decimal
import hashlib
import json
import struct


class Inconclusive(Exception):
    pass


class Invalid(Exception):
    pass


def require(ok, reason):
    if not ok:
        raise Invalid(reason)


def integer(value, low=0, high=(1 << 64) - 1):
    require(type(value) is int and low <= value <= high, "integer/range")
    return value


def strict_json(data):
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, "duplicate JSON key")
            result[key] = value
        return result
    def constant(_):
        raise Invalid("non-finite JSON")
    return json.loads(data, object_pairs_hook=pairs, parse_float=Decimal,
                      parse_constant=constant)


def unhex(value, length=None):
    require(isinstance(value, str) and len(value) % 2 == 0 and
            all(c in "0123456789abcdef" for c in value), "noncanonical hex")
    data = bytes.fromhex(value)
    require(length is None or len(data) == length, "hex length")
    return data


def sha256d(data):
    return hashlib.sha256(hashlib.sha256(data).digest()).digest()


def compact_size(n):
    integer(n)
    if n < 253:
        return bytes([n])
    if n <= 0xffff:
        return b"\xfd" + n.to_bytes(2, "little")
    if n <= 0xffffffff:
        return b"\xfe" + n.to_bytes(4, "little")
    return b"\xff" + n.to_bytes(8, "little")


class Reader:
    def __init__(self, data):
        self.data, self.pos = data, 0

    def take(self, n):
        require(0 <= n <= len(self.data) - self.pos, "truncated consensus bytes")
        result = self.data[self.pos:self.pos + n]
        self.pos += n
        return result

    def uint(self, n):
        return int.from_bytes(self.take(n), "little")

    def count(self, maximum=4_000_000):
        marker = self.uint(1)
        if marker < 253:
            value = marker
        else:
            size, minimum = {253: (2, 253), 254: (4, 65536), 255: (8, 1 << 32)}[marker]
            value = self.uint(size)
            require(value >= minimum, "noncanonical CompactSize")
        require(value <= maximum, "CompactSize resource bound")
        return value

    def blob(self):
        return self.take(self.count())

    def end(self):
        require(self.pos == len(self.data), "trailing consensus bytes")


MAX_MONEY = 21_000_000 * 100_000_000


def address_script(address):
    """Strict regtest P2PKH/P2WPKH/P2TR destination decoder for the oracle."""
    require(isinstance(address, str) and 1 <= len(address) <= 90, "address length")
    if not address.lower().startswith("bcrt1"):
        alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
        value = 0
        for c in address:
            require(c in alphabet, "base58 character")
            value = value * 58 + alphabet.index(c)
        raw = bytes(len(address) - len(address.lstrip("1"))) + value.to_bytes((value.bit_length() + 7) // 8, "big")
        require(len(raw) == 25 and raw[0] == 111 and sha256d(raw[:-4])[:4] == raw[-4:], "regtest P2PKH checksum/version")
        return b"\x76\xa9\x14" + raw[1:21] + b"\x88\xac"
    require(address == address.lower() or address == address.upper(), "mixed case bech32")
    hrp, data = address.lower().rsplit("1", 1)
    require(hrp == "bcrt" and len(data) >= 7, "regtest witness address")
    alphabet = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
    require(all(c in alphabet for c in data), "bech32 character")
    values = [alphabet.index(c) for c in data]
    chk = 1
    for n in [ord(c) >> 5 for c in hrp] + [0] + [ord(c) & 31 for c in hrp] + values:
        top, chk = chk >> 25, ((chk & 0x1ffffff) << 5) ^ n
        for i, g in enumerate((0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3)):
            if top & (1 << i):
                chk ^= g
    version = values[0]
    require(version in (0, 1) and chk == (1 if version == 0 else 0x2bc830a3), "witness version/checksum")
    acc, bits, program = 0, 0, bytearray()
    for n in values[1:-6]:
        acc = (acc << 5) | n
        bits += 5
        if bits >= 8:
            bits -= 8
            program.append((acc >> bits) & 255)
    require(bits < 5 and (acc & ((1 << bits) - 1)) == 0, "noncanonical witness padding")
    require(len(program) == (20 if version == 0 else 32), "unsupported witness program length")
    return bytes([0 if version == 0 else 0x51, len(program)]) + bytes(program)


def money_sum(values):
    total = 0
    for value in values:
        total += integer(value, 0, MAX_MONEY)
        require(total <= MAX_MONEY, "amount sum overflow")
    return total


def transaction(reader):
    start = reader.pos
    version = reader.take(4)
    count = reader.count()
    witness = count == 0
    if witness:
        require(reader.uint(1) == 1, "unknown witness flag")
        count = reader.count()
    require(count > 0 and count <= 100_000, "empty/oversize inputs")
    inputs = []
    vin_start = reader.pos
    for _ in range(count):
        prev = reader.take(32)[::-1].hex()
        index = reader.uint(4)
        script = reader.blob()
        sequence = reader.uint(4)
        inputs.append({"outpoint": f"{prev}:{index}", "script": script,
                       "sequence": sequence})
    vins = reader.data[vin_start:reader.pos]
    out_start = reader.pos
    out_count = reader.count(100_000)
    require(out_count > 0, "empty outputs")
    outputs = [{"value": reader.uint(8), "script": reader.blob()}
               for _ in range(out_count)]
    money_sum(o["value"] for o in outputs)
    vouts = reader.data[out_start:reader.pos]
    stacks = []
    if witness:
        for _ in inputs:
            stacks.append([reader.blob() for _ in range(reader.count(100_000))])
        require(any(stacks), "superfluous witness serialization")
    locktime = reader.take(4)
    raw = reader.data[start:reader.pos]
    stripped = version + compact_size(count) + vins + vouts + locktime
    txid = sha256d(stripped)
    weight = 3 * len(stripped) + len(raw)
    coinbase = len(inputs) == 1 and inputs[0]["outpoint"] == "0" * 64 + ":4294967295"
    if coinbase:
        require(2 <= len(inputs[0]["script"]) <= 100, "coinbase script length")
    else:
        require(all(i["outpoint"] != "0" * 64 + ":4294967295" for i in inputs), "null prevout")
    require(len({i["outpoint"] for i in inputs}) == len(inputs), "duplicate input")
    return {"raw": raw, "stripped": stripped, "txid_bytes": txid,
            "txid": txid[::-1].hex(), "wtxid_bytes": sha256d(raw),
            "wtxid": sha256d(raw)[::-1].hex(), "inputs": inputs,
            "outputs": outputs, "witness": stacks, "coinbase": coinbase,
            "weight": weight, "vbytes": (weight + 3) // 4}


def decode_transaction(raw):
    reader = Reader(raw)
    tx = transaction(reader)
    reader.end()
    return tx


def merkle(leaves):
    require(bool(leaves) and all(len(x) == 32 for x in leaves), "Merkle leaves")
    level = list(leaves)
    while len(level) > 1:
        require(all(level[i] != level[i + 1] for i in range(0, len(level) - 1, 2)),
                "mutated Merkle tree")
        if len(level) % 2:
            level.append(level[-1])
        level = [sha256d(level[i] + level[i + 1]) for i in range(0, len(level), 2)]
    return level[0]


def target(bits):
    integer(bits, 0, 0xffffffff)
    exponent, mantissa = bits >> 24, bits & 0x7fffff
    require(not bits & 0x800000 and mantissa != 0, "negative/zero compact target")
    value = mantissa >> (8 * (3 - exponent)) if exponent <= 3 else mantissa << (8 * (exponent - 3))
    require(0 < value < 1 << 256, "compact target overflow/underflow")
    return value


def to_compact(value):
    require(0 < value < 1 << 256, "target range")
    size = (value.bit_length() + 7) // 8
    mantissa = value << (8 * (3 - size)) if size <= 3 else value >> (8 * (size - 3))
    if mantissa & 0x800000:
        mantissa >>= 8
        size += 1
    return (size << 24) | mantissa


def header(raw, pow_limit_bits):
    require(len(raw) == 80, "header must be 80 bytes")
    bits = int.from_bytes(raw[72:76], "little")
    threshold = target(bits)
    require(threshold <= target(pow_limit_bits), "target exceeds bound pow limit")
    digest = sha256d(raw)
    require(int.from_bytes(digest, "little") <= threshold, "header PoW")
    return {"raw": raw, "hash": digest[::-1].hex(), "prev": raw[4:36][::-1].hex(),
            "time": int.from_bytes(raw[68:72], "little"), "bits": bits}


@dataclass(frozen=True)
class Policy:
    genesis: str
    pow_limit_bits: int
    initial_target_bits: int
    regtest_pow_bits_overlay: int | None
    no_pow_retargeting: bool
    allow_min_difficulty_blocks: bool
    target_spacing: int
    target_timespan: int
    difficulty_adjustment_interval: int
    enforce_bip94: bool
    retarget_floor_bits: int | None
    subsidy_initial_sats: int
    subsidy_halving_interval: int
    coinbase_maturity: int

    def __post_init__(self):
        unhex(self.genesis, 32)
        for flag in (self.no_pow_retargeting, self.allow_min_difficulty_blocks, self.enforce_bip94):
            require(type(flag) is bool, "missing policy boolean")
        for n in (self.target_spacing, self.target_timespan, self.subsidy_halving_interval, self.coinbase_maturity):
            integer(n, 1)
        integer(self.difficulty_adjustment_interval)
        require(self.difficulty_adjustment_interval == self.target_timespan // self.target_spacing,
                "inconsistent spacing/interval")
        limit = target(self.pow_limit_bits)
        for bits in (self.initial_target_bits, self.regtest_pow_bits_overlay, self.retarget_floor_bits):
            if bits is not None:
                require(target(bits) <= limit, "policy target above limit")
        integer(self.subsidy_initial_sats, 0, MAX_MONEY)

    def subsidy(self, height):
        halvings = integer(height) // self.subsidy_halving_interval
        return self.subsidy_initial_sats >> halvings if halvings < 64 else 0


def expected_bits(policy, ancestors, candidate_time):
    """Contextual Core min-difficulty plus declared rbitcoin regtest overlay.

    ancestors is genesis-first; every time/bits comes from raw headers, never
    GBT. A boundary precedes the long-gap rule. Overlay only replaces the
    pow-limit result away from a boundary and away from a legitimate gap.
    """
    if not ancestors:
        raise Inconclusive("missing parent ancestry")
    height = len(ancestors)
    parent = ancestors[-1]
    interval = policy.difficulty_adjustment_interval
    boundary = interval != 0 and height % interval == 0
    gap = candidate_time > parent["time"] + 2 * policy.target_spacing
    if boundary:
        if policy.no_pow_retargeting:
            bits = parent["bits"]
        else:
            first = ancestors[height - interval]
            elapsed = max(0, parent["time"] - first["time"])
            elapsed = max(policy.target_timespan // 4, min(elapsed, policy.target_timespan * 4))
            base = first["bits"] if policy.enforce_bip94 else parent["bits"]
            ceiling = target(policy.retarget_floor_bits or policy.pow_limit_bits)
            bits = to_compact(min(ceiling, target(base) * elapsed // policy.target_timespan))
    elif policy.allow_min_difficulty_blocks and gap:
        bits = policy.pow_limit_bits
    else:
        index = height - 1
        if policy.allow_min_difficulty_blocks and interval:
            while index % interval and ancestors[index]["bits"] == policy.pow_limit_bits:
                index -= 1
        bits = ancestors[index]["bits"]
    if (policy.regtest_pow_bits_overlay is not None and bits == policy.pow_limit_bits
            and not boundary and not (policy.allow_min_difficulty_blocks and gap)):
        bits = policy.regtest_pow_bits_overlay
    return bits


COMMITMENT_PREFIX = bytes.fromhex("6a24aa21a9ed")


def witness_commitment(txs, production=True):
    cb = txs[0]
    matches = [(i, o) for i, o in enumerate(cb["outputs"])
               if len(o["script"]) >= 38 and o["script"].startswith(COMMITMENT_PREFIX)]
    has_witness = any(t["witness"] for t in txs[1:])
    if not matches:
        require(not any(t["witness"] for t in txs), "witness without commitment")
        return None
    index, output = matches[-1]  # BIP141: highest matching output, not first.
    require(len(cb["witness"]) == 1 and len(cb["witness"][0]) == 1
            and len(cb["witness"][0][0]) == 32, "coinbase reserved value")
    root = merkle([bytes(32)] + [t["wtxid_bytes"] for t in txs[1:]])
    reserved = cb["witness"][0][0]
    require(output["script"][6:38] == sha256d(root + reserved), "witness commitment mismatch")
    if production:
        require(len(matches) == 1 and index == len(cb["outputs"]) - 1 and output["value"] == 0,
                "pool commitment must be unique, zero-valued, last")
    return {"index": index, "root": root.hex(), "reserved": reserved.hex(),
            "witness_transactions": sum(bool(t["witness"]) for t in txs[1:]),
            "mixed": has_witness and any(not t["witness"] for t in txs[1:])}


def decode_block(raw, policy, production=False):
    require(len(raw) <= 4_000_000, "block byte bound")
    reader = Reader(raw)
    head = header(reader.take(80), policy.pow_limit_bits)
    count_start = reader.pos
    count = reader.count(100_000)
    count_bytes = reader.pos - count_start
    require(count > 0, "empty block")
    txs = [transaction(reader) for _ in range(count)]
    reader.end()
    require(txs[0]["coinbase"] and all(not t["coinbase"] for t in txs[1:]), "coinbase position")
    require(len({t["txid"] for t in txs}) == count, "duplicate transaction")
    require(merkle([t["txid_bytes"] for t in txs]) == raw[36:68], "txid Merkle mismatch")
    weight = (80 + count_bytes) * 4 + sum(t["weight"] for t in txs)
    require(weight <= 4_000_000, "block weight")
    return {**head, "transactions": txs, "weight": weight,
            "commitment": witness_commitment(txs, production)}


class Chain:
    """An immutable anchored view rebuilt independently from raw blocks.

    Genesis output is deliberately unspendable. Reorgs instantiate a new
    Chain; previously captured views are retained to verify journal history.
    """
    def __init__(self, policy, blocks):
        self.policy, self.blocks = policy, []
        self.utxos, self.outputs, self.transactions, self.by_hash = {}, {}, {}, {}
        self.spent_at = {}
        for height, raw in enumerate(blocks):
            block = decode_block(raw, policy)
            block["height"] = height
            if height == 0:
                require(block["hash"] == policy.genesis, "genesis binding")
            else:
                require(block["prev"] == self.blocks[-1]["hash"], "broken chain")
                require(block["bits"] == expected_bits(policy, self.blocks, block["time"]),
                        "contextual expected nBits")
                times = sorted(b["time"] for b in self.blocks[-11:])
                require(block["time"] > times[len(times) // 2], "median time past")
            fees = 0
            for position, tx in enumerate(block["transactions"]):
                require(tx["txid"] not in self.transactions, "duplicate historical txid")
                if position:
                    inputs = []
                    for inp in tx["inputs"]:
                        outpoint = inp["outpoint"]
                        if outpoint not in self.utxos:
                            raise Invalid("missing/spent canonical prevout")
                        prevout = self.utxos.pop(outpoint)
                        self.spent_at[outpoint] = height
                        require(not prevout["coinbase"] or height - prevout["height"] >= policy.coinbase_maturity,
                                "immature coinbase spend")
                        inputs.append(prevout["value"])
                    fee = money_sum(inputs) - money_sum(o["value"] for o in tx["outputs"])
                    require(fee >= 0, "negative transaction fee")
                    fees = money_sum([fees, fee])
                    tx["fee"] = fee
                for index, output in enumerate(tx["outputs"]):
                    point = f"{tx['txid']}:{index}"
                    fact = {**output, "height": height, "block_hash": block["hash"], "coinbase": position == 0}
                    self.outputs[point] = fact
                    if height and not output["script"].startswith(b"\x6a"):
                        self.utxos[point] = fact
                self.transactions[tx["txid"]] = (block, tx)
            block["fees"] = fees
            block["subsidy"] = policy.subsidy(height)
            block["claim"] = money_sum(o["value"] for o in block["transactions"][0]["outputs"])
            require(block["claim"] <= block["subsidy"] + fees, "excess coinbase claim")
            self.blocks.append(block)
            self.by_hash[block["hash"]] = block
        require(bool(self.blocks), "empty chain evidence")

    @property
    def tip(self):
        return self.blocks[-1]

    def fee(self, tx):
        values = []
        for inp in tx["inputs"]:
            if inp["outpoint"] not in self.outputs:
                raise Inconclusive("raw prevout unavailable")
            values.append(self.outputs[inp["outpoint"]]["value"])
        fee = money_sum(values) - money_sum(o["value"] for o in tx["outputs"])
        require(fee >= 0, "negative fee")
        return fee

    def prefix(self, height):
        return ChainPrefix(self, integer(height, 0, self.tip["height"]))


class _FilteredMap(Mapping):
    """Read-only projection; avoids copying a large UTXO set per old tip."""
    def __init__(self, source, predicate):
        self.source, self.predicate = source, predicate

    def __getitem__(self, key):
        value = self.source[key]
        if not self.predicate(key, value):
            raise KeyError(key)
        return value

    def __iter__(self):
        return (key for key, value in self.source.items() if self.predicate(key, value))

    def __len__(self):
        return sum(1 for _ in self)


class ChainPrefix:
    """A validated raw-chain prefix, with independent height-filtered state."""
    def __init__(self, chain, height):
        self.master = chain.master if isinstance(chain, ChainPrefix) else chain
        chain = self.master
        self.policy = chain.policy
        self.blocks = chain.blocks[:height + 1]
        self.by_hash = _FilteredMap(chain.by_hash, lambda _, b: b["height"] <= height)
        self.transactions = _FilteredMap(chain.transactions, lambda _, pair: pair[0]["height"] <= height)
        self.outputs = _FilteredMap(chain.outputs, lambda _, output: output["height"] <= height)
        self.utxos = _FilteredMap(chain.outputs, lambda point, output:
                                 0 < output["height"] <= height and not output["script"].startswith(b"\x6a")
                                 and chain.spent_at.get(point, height + 1) > height)

    @property
    def tip(self):
        return self.blocks[-1]

    def fee(self, tx):
        return Chain.fee(self, tx)

    def prefix(self, height):
        return ChainPrefix(self, integer(height, 0, self.tip["height"]))


def rollback_proof(proof, old, new):
    require(set(proof) == {"version", "old_anchor", "new_anchor", "common_ancestor",
                          "old_headers", "new_headers", "displaced_block_hashes"}, "rollback fields")
    require(proof["version"] == 1, "rollback version")
    common = proof["common_ancestor"]
    require(set(common) == {"block_hash", "height"}, "ancestor fields")
    integer(common["height"], 0, min(old.tip["height"], new.tip["height"]))
    for name, chain in (("old", old), ("new", new)):
        require(proof[f"{name}_anchor"] == {"block_hash": chain.tip["hash"], "height": chain.tip["height"]},
                "rollback anchor")
        raws = proof[f"{name}_headers"]
        require(1 <= len(raws) <= 4096, "rollback header cap")
        expected = chain.blocks[common["height"]:][::-1]
        require([unhex(x, 80) for x in raws] == [b["raw"] for b in expected], "rollback branch bytes")
        require(expected[-1]["hash"] == common["block_hash"], "rollback ancestor")
    require(proof["displaced_block_hashes"] ==
            [b["hash"] for b in old.blocks[proof["common_ancestor"]["height"] + 1:][::-1]],
            "displaced hashes/order")


def ladder(owed):
    integer(owed, 0, MAX_MONEY)
    pieces = []
    for rung in (5_000_000, 3_000_000, 1_000_000, 500_000):
        count, owed = divmod(owed, rung)
        require(len(pieces) + count <= 100_000, "ladder resource bound")
        pieces.extend([rung] * count)
    return pieces, owed


def raw_payouts(slices, subsidy, fees, fee_bps):
    """SPEC 4.4 from independently attributed shares, not pool totals.

    Each slice is a list of {identity, work, fees} derived from verified
    immutable job/header/share evidence by the caller.
    """
    integer(fee_bps, 0, 10_000)
    require(bool(slices) and all(slices), "empty payout window")
    total_work = sum(integer(s["work"], 1) for row in slices for s in row)
    net_subsidy = subsidy - subsidy * fee_bps // 10_000
    net_fees = fees - fees * fee_bps // 10_000
    identities = {s["identity"] for row in slices for s in row}
    result = {}
    for identity in identities:
        work = sum(s["work"] for row in slices for s in row if s["identity"] == identity)
        require(net_subsidy * work < 1 << 128, "payout u128 overflow")
        amount = net_subsidy * work // total_work
        for row in slices:
            den = sum(s["work"] * integer(s["fees"]) for s in row)
            num = sum(s["work"] * s["fees"] for s in row if s["identity"] == identity)
            require(net_fees * num < 1 << 128, "fee payout u128 overflow")
            if den:
                amount += (net_fees // len(slices)) * num // den
        result[identity] = amount
    return result, subsidy + fees - sum(result.values())
