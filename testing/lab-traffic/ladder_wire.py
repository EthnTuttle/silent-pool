"""Independent SPCB/2 decoder from ladder owner's WIRE-FORMATS 3.1.

This decodes actual stream bytes, not a renamed normalized fixture. Source
publication/window ancestry must still be joined before payout acceptance.
"""
import hashlib

from oracle import Inconclusive, Reader, require


def xonly(raw):
    require(len(raw) == 32, "SPCB x-only key length")
    p = 2**256 - 2**32 - 977
    x = int.from_bytes(raw, "big")
    require(x < p, "SPCB x-only key range")
    square = (x * x * x + 7) % p
    require(pow(square, (p - 1) // 2, p) == 1, "SPCB x-only key is not on secp256k1")
    return raw.hex()


def decode_spcb2(raw):
    return _decode(raw)


def decode_spcb_working(raw, *, api_sha256):
    from ruleb_source import working_pin
    working_pin(api_sha256)
    return _decode(raw, working_ruleb=True)


def _decode(raw, working_ruleb=False):
    if len(raw) > 16 * 1024 * 1024:
        raise Inconclusive("SPCB observation exceeds oracle resource bound")
    reader = Reader(raw)
    require(reader.take(4) == b"SPCB", "explicit SPCB/2 envelope required")
    wire_version = reader.uint(1)
    if working_ruleb and wire_version not in {2, 3}:
        raise Inconclusive("unsupported SPCB economic envelope version")
    require(wire_version in ({2, 3} if working_ruleb else {2}), "explicit SPCB/2 envelope required")
    block_hash_le = reader.take(32)
    height, front = reader.uint(4), reader.uint(1)
    require(front < 128, "SPCB front id range")
    fingerprint = reader.take(32).hex()
    a_send_x = xonly(reader.take(32))
    high_water = {}
    for _ in range(reader.uint(2)):
        front_id = reader.uint(1)
        require(front_id < 128 and front_id not in high_water, "SPCB duplicate/invalid high-water front")
        mark = reader.take(reader.uint(1))
        require(mark and all(32 <= c < 127 for c in mark), "SPCB high-water string")
        # Preserve the source bytes; the publication adapter decides whether
        # this producer's mark is a slice id or a Redis stream id.
        high_water[front_id] = mark.decode("ascii")
    window_target = reader.uint(16)
    entries, identities, keys = [], set(), set()
    for _ in range(reader.uint(2)):
        identity, rail, count = reader.take(32).hex(), reader.uint(1), reader.uint(2)
        require(identity not in identities and rail <= 3 and count <= 2048, "SPCB entry identity/rail/key-count")
        identities.add(identity)
        require((rail == 0) == (count > 0), "SPCB keys must be nonempty iff coinbase rail")
        public_keys = []
        for _ in range(count):
            key = xonly(reader.take(32))
            require(key not in keys, "SPCB repeated payout key")
            keys.add(key)
            require(len(keys) <= 2048, "SPCB block key-count cap")
            public_keys.append(key)
        destination = reader.take(reader.uint(2)).hex()
        sats, owed = reader.uint(8), reader.uint(8)
        require(sats <= owed, "SPCB payout exceeds frozen full owed")
        entries.append({"format_version": 2, "sp_id": identity, "rail": rail,
                        "p_x": public_keys, "dest_id": destination, "sats": sats, "owed_sats": owed})
    work_version = reader.uint(1)
    fingerprint_len = reader.uint(1)
    require(fingerprint_len == 32, "SPCB definite work fingerprint length")
    work_fingerprint = reader.take(fingerprint_len)
    rule = reader.uint(1)
    require((work_version == 1 and rule == 0 and work_fingerprint == bytes(32))
            or (work_version == 2 and rule == 1 and work_fingerprint != bytes(32)), "SPCB work-version/fingerprint/window-rule")
    economic = None
    if wire_version == 3:
        from ruleb_source import frozen_frame
        require(list(high_water) == sorted(high_water), "noncanonical SPCB/3 high-water map order")
        economic = frozen_frame(reader, height=height, identities=identities)
    reader.end()
    return {"format_version": 2, "block_hash": block_hash_le[::-1].hex(), "height": height,
            "wire_version": wire_version, "economic": economic,
            "front_id": front, "coinbase_fingerprint": fingerprint, "a_send_x": a_send_x,
            "high_water_raw": high_water, "window_target": window_target, "entries": entries,
            "work_unit_version": work_version, "work_unit_fingerprint": work_fingerprint.hex(), "window_rule": rule,
            "snapshot_key": hashlib.sha256(b"SPc/DistributionSnapshotV2" + raw).hexdigest(),
            "wire_sha256": hashlib.sha256(raw).hexdigest()}
