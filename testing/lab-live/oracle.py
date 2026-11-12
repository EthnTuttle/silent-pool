"""Independent integer, Bitcoin-wire, and payout checks for LP-live.

This module deliberately imports no silent-pool code. Its inputs are the
miner's recorded header/job bytes, the node's canonical block, and database
rows. Expected work and payout arithmetic comes from the approved L3 work
unit/window vectors and SPEC section 4.4, not from the pool executable.
"""

from __future__ import annotations

import hashlib
from typing import Any, Iterable, Sequence

U256 = 1 << 256
BASE58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def sha256d(raw: bytes) -> bytes:
    return hashlib.sha256(hashlib.sha256(raw).digest()).digest()


def tagged_hash(tag: str, payload: bytes) -> bytes:
    tag_hash = hashlib.sha256(tag.encode("ascii")).digest()
    return hashlib.sha256(tag_hash + tag_hash + payload).digest()


def sp_id(b_scan: bytes, b_spend: bytes) -> bytes:
    if len(b_scan) != 33 or len(b_spend) != 33:
        raise ValueError("compressed scan/spend keys are required")
    return tagged_hash("SPc/Id", b_scan + b_spend)


def work_unit_fingerprint(reference_target: int) -> bytes:
    if not 0 <= reference_target < U256:
        raise ValueError("reference target outside uint256")
    return hashlib.sha256(
        b"spc-lab/work-unit/v2\x00" + reference_target.to_bytes(32, "big")
    ).digest()


def decode_compact_target(bits: int) -> int:
    """Decode with Bitcoin consensus SetCompact sign/overflow/zero rules."""
    if not 0 <= bits <= 0xFFFFFFFF:
        raise ValueError("compact target is not uint32")
    size = bits >> 24
    word = bits & 0x007FFFFF
    if word and bits & 0x00800000:
        raise ValueError("negative compact target")
    if word and (
        size > 34 or (word > 0xFF and size > 33) or (word > 0xFFFF and size > 32)
    ):
        raise ValueError("compact target overflow")
    target = word >> (8 * (3 - size)) if size <= 3 else word << (8 * (size - 3))
    if target == 0:
        raise ValueError("zero compact target")
    if target >= U256:
        raise ValueError("target outside uint256")
    return target


def reference_exponent(reference_target: int) -> int:
    if not 0 < reference_target < U256 - 1:
        raise ValueError("reference target outside the v2 family")
    k = (reference_target + 1).bit_length() - 1
    if not 1 <= k <= 255 or reference_target + 1 != 1 << k:
        raise ValueError("reference_not_power_of_two")
    return k


def issuance_multiplier(reference_target: int, configured_m: int, network_target: int) -> int:
    k = reference_exponent(reference_target)
    if configured_m <= 0 or configured_m & (configured_m - 1):
        raise ValueError("multiplier_not_in_family")
    j_cfg = configured_m.bit_length() - 1
    if j_cfg >= k:
        raise ValueError("multiplier_not_in_family")
    if not 0 < network_target < U256:
        raise ValueError("invalid network target")
    ceil_log2 = network_target.bit_length()
    j_issue = min(j_cfg, k - ceil_log2)
    if j_issue < 0:
        raise ValueError("target_profile_unplannable")
    m_issue = 1 << j_issue
    share = ((reference_target + 1) // m_issue) - 1
    if share < network_target or share < 1:
        raise ValueError("share target is harder than the network target")
    return m_issue


def share_target(reference_target: int, m_issue: int) -> int:
    reference_exponent(reference_target)
    if m_issue <= 0 or m_issue & (m_issue - 1):
        raise ValueError("multiplier_not_in_family")
    value = ((reference_target + 1) // m_issue) - 1
    if value < 1 or (reference_target + 1) % m_issue:
        raise ValueError("invalid family member")
    return value


def window_target(reference_target: int, window_n: int, network_target: int) -> int:
    reference_exponent(reference_target)
    if window_n <= 0:
        raise ValueError("window_n must be positive")
    if not 0 < network_target < U256:
        raise ValueError("invalid network target")
    numerator = window_n * (reference_target + 1)
    denominator = network_target + 1
    return (numerator + denominator - 1) // denominator


def display_hash(raw_digest: bytes) -> str:
    if len(raw_digest) != 32:
        raise ValueError("digest must be 32 bytes")
    return raw_digest[::-1].hex()


def target_from_le_hex(value: str) -> int:
    raw = bytes.fromhex(value)
    if len(raw) != 32:
        raise ValueError("target must be 32 bytes")
    return int.from_bytes(raw, "little")


def target_to_le_hex(target: int) -> str:
    if not 0 <= target < U256:
        raise ValueError("target outside uint256")
    return target.to_bytes(32, "little").hex()


def base58check_encode(payload: bytes) -> str:
    raw = payload + sha256d(payload)[:4]
    zeroes = len(raw) - len(raw.lstrip(b"\x00"))
    number = int.from_bytes(raw, "big")
    encoded = ""
    while number:
        number, digit = divmod(number, 58)
        encoded = BASE58_ALPHABET[digit] + encoded
    return "1" * zeroes + (encoded or ("" if zeroes else "1"))


def noise_authority_from_secp_material(compressed_public: bytes, secret: bytes):
    """Encode key-utils forms using scanner-generated secp keys.

    No elliptic-curve operation is reimplemented here: spc-scan generated the
    valid pair. Key-utils public payloads have the 0x01 0x00 discriminator;
    secret payload is the raw 32-byte scalar.
    """
    if len(compressed_public) != 33 or compressed_public[0] not in (2, 3):
        raise ValueError("invalid compressed secp public key")
    if len(secret) != 32 or not 0 < int.from_bytes(secret, "big"):
        raise ValueError("invalid secp secret")
    return (
        base58check_encode(b"\x01\x00" + compressed_public[1:]),
        base58check_encode(secret),
    )


def read_compact_size(raw: bytes, offset: int) -> tuple[int, int]:
    if offset >= len(raw):
        raise ValueError("truncated compact size")
    first = raw[offset]
    offset += 1
    if first <= 0xFC:
        return first, offset
    widths = {0xFD: 2, 0xFE: 4, 0xFF: 8}
    width = widths[first]
    end = offset + width
    if end > len(raw):
        raise ValueError("truncated compact size integer")
    value = int.from_bytes(raw[offset:end], "little")
    if (
        (first == 0xFD and value < 0xFD)
        or (first == 0xFE and value <= 0xFFFF)
        or (first == 0xFF and value <= 0xFFFFFFFF)
    ):
        raise ValueError("non-canonical compact size")
    return value, end


def _take(raw: bytes, offset: int, length: int) -> tuple[bytes, int]:
    end = offset + length
    if length < 0 or end > len(raw):
        raise ValueError("truncated wire value")
    return raw[offset:end], end


def _compact_size(value: int) -> bytes:
    if value <= 0xFC:
        return bytes([value])
    if value <= 0xFFFF:
        return b"\xfd" + value.to_bytes(2, "little")
    if value <= 0xFFFFFFFF:
        return b"\xfe" + value.to_bytes(4, "little")
    return b"\xff" + value.to_bytes(8, "little")


def _without_witness(raw: bytes) -> bytes:
    """Witness-stripping serializer used only for txid computation."""
    parsed, end = parse_transaction(raw, 0)
    if end != len(raw) or not parsed["has_witness"]:
        raise ValueError("witness stripping requires one complete witness transaction")
    out = bytearray(raw[:4])
    out.extend(_compact_size(len(parsed["inputs"])))
    for tx_input in parsed["inputs"]:
        out.extend(tx_input["outpoint"])
        out.extend(_compact_size(len(tx_input["script_sig"])))
        out.extend(tx_input["script_sig"])
        out.extend(tx_input["sequence"].to_bytes(4, "little"))
    out.extend(_compact_size(len(parsed["outputs"])))
    for tx_output in parsed["outputs"]:
        out.extend(tx_output["value"].to_bytes(8, "little"))
        out.extend(_compact_size(len(tx_output["script"])))
        out.extend(tx_output["script"])
    out.extend(raw[-4:])
    return bytes(out)


def parse_transaction(raw: bytes, offset: int = 0) -> tuple[dict[str, Any], int]:
    start = offset
    _, offset = _take(raw, offset, 4)
    has_witness = False
    if offset + 2 <= len(raw) and raw[offset] == 0 and raw[offset + 1] != 0:
        has_witness = True
        offset += 2
    n_inputs, offset = read_compact_size(raw, offset)
    inputs = []
    for _ in range(n_inputs):
        outpoint, offset = _take(raw, offset, 36)
        script_len, offset = read_compact_size(raw, offset)
        script_sig, offset = _take(raw, offset, script_len)
        sequence, offset = _take(raw, offset, 4)
        inputs.append(
            {
                "outpoint": outpoint,
                "script_sig": script_sig,
                "sequence": int.from_bytes(sequence, "little"),
            }
        )
    n_outputs, offset = read_compact_size(raw, offset)
    outputs = []
    for _ in range(n_outputs):
        value_raw, offset = _take(raw, offset, 8)
        script_len, offset = read_compact_size(raw, offset)
        script, offset = _take(raw, offset, script_len)
        outputs.append({"value": int.from_bytes(value_raw, "little"), "script": script})
    if has_witness:
        for tx_input in inputs:
            count, offset = read_compact_size(raw, offset)
            witness = []
            for _ in range(count):
                size, offset = read_compact_size(raw, offset)
                item, offset = _take(raw, offset, size)
                witness.append(item)
            tx_input["witness"] = witness
    _, offset = _take(raw, offset, 4)
    txid_source = raw[start:offset] if not has_witness else _without_witness(raw[start:offset])
    return {
        "raw": raw[start:offset],
        "txid": sha256d(txid_source),
        "inputs": inputs,
        "outputs": outputs,
        "has_witness": has_witness,
    }, offset


def parse_block(raw: bytes) -> dict[str, Any]:
    if len(raw) < 81:
        raise ValueError("truncated block")
    header = raw[:80]
    count, offset = read_compact_size(raw, 80)
    transactions = []
    for _ in range(count):
        transaction, offset = parse_transaction(raw, offset)
        transactions.append(transaction)
    if offset != len(raw) or not transactions:
        raise ValueError("trailing block bytes or empty block")
    return {
        "header": header,
        "hash": sha256d(header),
        "transactions": transactions,
        "bits": int.from_bytes(header[72:76], "little"),
    }


def merkle_root(txids: Sequence[bytes]) -> bytes:
    if not txids:
        raise ValueError("empty merkle tree")
    level = list(txids)
    while len(level) > 1:
        if len(level) % 2:
            level.append(level[-1])
        level = [sha256d(level[i] + level[i + 1]) for i in range(0, len(level), 2)]
    return level[0]


def merkle_branch_zero(txids: Sequence[bytes]) -> list[bytes]:
    if not txids:
        raise ValueError("empty merkle tree")
    branch = []
    level = list(txids)
    index = 0
    while len(level) > 1:
        if len(level) % 2:
            level.append(level[-1])
        branch.append(level[index ^ 1])
        level = [sha256d(level[i] + level[i + 1]) for i in range(0, len(level), 2)]
        index //= 2
    return branch


def light_proof(header: bytes, branch: Iterable[bytes], coinbase: bytes) -> bytes:
    if len(header) != 80 or not coinbase:
        raise ValueError("invalid light proof members")
    siblings = list(branch)
    if len(siblings) > 255 or any(len(sibling) != 32 for sibling in siblings):
        raise ValueError("invalid light proof branch")
    body = header + bytes([len(siblings)]) + b"".join(siblings)
    body += len(coinbase).to_bytes(4, "little") + coinbase
    return len(body).to_bytes(4, "little") + body


def select_window(rows: Sequence[dict[str, Any]], target_q: int) -> dict[str, Any]:
    """Select whole slices newest-first, independently checking their sums."""
    if target_q <= 0 or len(rows) > 4097:
        raise ValueError("invalid or ambiguous window walk")
    selected = []
    actual_sum = 0
    for row in rows:
        difficulty = int(row["difficulty"])
        credit_sum = sum(int(c["diff_sum"]) for c in row["credits"])
        fee_sum = sum(int(c["diff_fee_sum"]) for c in row["credits"])
        if difficulty != credit_sum:
            raise ValueError("stored slice difficulty differs from credit sum")
        if fee_sum != int(row["diff_fee_sum"]):
            raise ValueError("stored slice fee-work differs from credits")
        selected.append(row)
        actual_sum += difficulty
        if actual_sum >= target_q:
            break
    ambiguous = len(rows) == 4097 and actual_sum < target_q
    return {
        "rows": selected,
        "actual_sum": actual_sum,
        "underfill": max(0, target_q - actual_sum),
        "overshoot": max(0, actual_sum - target_q),
        "ambiguous": ambiguous,
    }


def expected_raw_payouts(
    selected_rows: Sequence[dict[str, Any]], subsidy: int, fees: int, fee_bps: int
) -> dict[str, Any]:
    """SPEC 4.4 exact rational products, with one floor at each stated term."""
    if not selected_rows or not 0 <= fee_bps <= 10_000:
        raise ValueError("empty payout window or invalid pool fee")
    if subsidy < 0 or fees < 0:
        raise ValueError("negative block economics")
    d_win = sum(int(row["difficulty"]) for row in selected_rows)
    if d_win <= 0:
        raise ValueError("zero-work payout window")
    r_net = subsidy - subsidy * fee_bps // 10_000
    f_net = fees - fees * fee_bps // 10_000
    t = len(selected_rows)
    d_by_id: dict[str, int] = {}
    raw: dict[str, int] = {}
    for row in selected_rows:
        for credit in row["credits"]:
            identity = credit["sp_id"].lower()
            d_by_id[identity] = d_by_id.get(identity, 0) + int(credit["diff_sum"])
            denominator = t * int(row["diff_fee_sum"])
            fee_term = (
                0
                if denominator == 0
                else f_net * int(credit["diff_fee_sum"]) // denominator
            )
            raw[identity] = raw.get(identity, 0) + fee_term
    for identity, work in d_by_id.items():
        raw[identity] = raw.get(identity, 0) + r_net * work // d_win
    total = subsidy + fees
    pool_sats = total - sum(raw.values())
    if pool_sats < 0:
        raise ValueError("raw payouts exceed actual block revenue")
    return {
        "raw": raw,
        "d_win": d_win,
        "r_net": r_net,
        "f_net": f_net,
        "pool_sats": pool_sats,
    }


def encode_compact_target(target: int) -> int:
    """Bitcoin GetCompact encoding used to compare actual header nBits."""
    if not 0 < target < U256:
        raise ValueError("target outside uint256")
    size = (target.bit_length() + 7) // 8
    word = target << (8 * (3 - size)) if size <= 3 else target >> (8 * (size - 3))
    if word & 0x800000:
        word >>= 8
        size += 1
    return (size << 24) | word


def require_timespan(target_span: int) -> None:
    # Harness input validation only: Core does not require divisibility by four.
    if target_span <= 0:
        raise ValueError("pow_target_timespan must be positive")


def expected_retarget_bits(headers: Sequence[dict[str, Any]], height: int, profile: dict[str, Any]) -> int:
    """Core next-work oracle over recorded headers and effective profile."""
    if height != len(headers) - 1 or height < 0:
        raise ValueError("header vector must be anchored at exactly the requested height")
    if height == 0:
        return int(profile["pow_limit_bits"], 16)
    if height == 1:
        return int(profile["regtest_pow_bits_overlay"], 16)
    interval = int(profile["difficulty_adjustment_interval"])
    if interval <= 0:
        raise ValueError("invalid retarget interval")
    if height % interval:
        return int(headers[height - 1]["bits"], 16)
    previous_target = decode_compact_target(int(headers[height - 1]["bits"], 16))
    first = headers[height - interval]
    elapsed = int(headers[height - 1]["time"]) - int(first["time"])
    target_span = int(profile["pow_target_timespan"])
    require_timespan(target_span)
    elapsed = min(target_span * 4, max(target_span // 4, elapsed))
    target = previous_target * elapsed // target_span
    floor_target = decode_compact_target(int(profile["retarget_floor_bits"], 16))
    transition_ceiling = min(previous_target << 2, floor_target)
    target = min(target, transition_ceiling)
    validation_ceiling = decode_compact_target(int(profile["pow_limit_bits"], 16))
    target = min(target, validation_ceiling)
    return encode_compact_target(target)


def validate_retarget_vector(headers: Sequence[dict[str, Any]], profile: dict[str, Any]) -> list[int]:
    if not headers or int(profile["no_pow_retargeting"]) != 0:
        raise ValueError("retarget profile is not enabled or vector is empty")
    if int(profile["allow_min_difficulty_blocks"]) != 0:
        raise ValueError("minimum-difficulty exception is enabled")
    mismatches = []
    for height, header in enumerate(headers):
        expected = expected_retarget_bits(headers[: height + 1], height, profile)
        if int(header["bits"], 16) != expected:
            mismatches.append(height)
    return mismatches


def parse_serialized_bytes(value: Any, length: int | None = 32) -> bytes:
    if isinstance(value, list):
        raw = bytes(value)
    elif isinstance(value, str):
        text = value[2:] if value.startswith("\\x") else value
        raw = bytes.fromhex(text)
    else:
        raise ValueError("unsupported serialized byte array")
    if length is not None and len(raw) != length:
        raise ValueError(f"expected {length} serialized bytes")
    return raw


def parse_entries(value: Any) -> list[dict[str, Any]]:
    rail_names = {0: "coinbase", 1: "ark", 2: "pending", 3: "forfeit"}
    entries = []
    for item in value:
        rail = item["rail"]
        rail = rail.lower() if isinstance(rail, str) else rail_names[rail]
        entries.append(
            {
                "sp_id": parse_serialized_bytes(item["sp_id"]).hex(),
                "rail": rail,
                "p_x": parse_serialized_bytes(item["p_x"], None).hex(),
                "dest_id": parse_serialized_bytes(item["dest_id"], None).hex(),
                "sats": int(item["sats"]),
            }
        )
    return entries
