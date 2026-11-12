#!/usr/bin/env python3
"""Read-only actual SPCB/2 archive audit; no full T3/rbitcoin/Ark claim."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import sys

from oracle import (Inconclusive, Invalid, Policy, decode_block, expected_bits, header,
                    integer, ladder, raw_payouts, require, sha256d, strict_json, unhex)
from pool_source import PoolSource
from public_derivation import _N, _GX, _GY, _point_add
from signature_oracle import _multiply, public_point


def tagged(name, data):
    tag = hashlib.sha256(name.encode()).digest()
    return hashlib.sha256(tag + tag + data).digest()


def scalar(raw):
    value = int.from_bytes(raw, "big")
    require(0 < value < _N, "SPc hash-to-scalar is invalid; no modular reduction")
    return value


def compressed(point):
    require(point is not None, "SPc derived point at infinity")
    x, y = point
    return bytes([2 + y % 2]) + x.to_bytes(32, "big")


def silent_address_keys(address):
    require(address == address.lower(), "silent address case")
    hrp, encoded = address.rsplit("1", 1)
    require(hrp == "tsp", "component address must be test-network silent payment")
    alphabet = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
    require(all(c in alphabet for c in encoded), "silent address alphabet")
    values = [alphabet.index(c) for c in encoded]
    checksum = 1
    for value in [ord(c) >> 5 for c in hrp] + [0] + [ord(c) & 31 for c in hrp] + values:
        top = checksum >> 25
        checksum = ((checksum & 0x1ffffff) << 5) ^ value
        for i, generator in enumerate((0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3)):
            if top >> i & 1:
                checksum ^= generator
    require(checksum == 0x2bc830a3 and values[0] == 0, "silent address version/checksum")
    acc, bits, decoded = 0, 0, bytearray()
    for value in values[1:-6]:
        acc = (acc << 5) | value; bits += 5
        while bits >= 8:
            bits -= 8; decoded.append((acc >> bits) & 255)
    require(bits < 5 and acc & ((1 << bits) - 1) == 0 and len(decoded) == 66, "silent address program/padding")
    return bytes(decoded[:33]), bytes(decoded[33:])


def carrier(coinbase, height):
    script = coinbase["inputs"][0]["script"]
    first = script[0]
    if 0x51 <= first <= 0x60:
        actual, offset = first - 0x50, 1
    else:
        require(1 <= first <= 5 and len(script) > first, "coinbase height push")
        encoded = script[1:first + 1]
        require(not encoded[-1] & 128, "negative coinbase height")
        actual, offset = int.from_bytes(encoded, "little"), first + 1
    require(actual == height and script[offset] == 32, "SPc coinbase height/carrier binding")
    return script[offset + 1:offset + 33]


def audit(case, output):
    case, output = Path(case), Path(output)
    output.mkdir(mode=0o700, exist_ok=False)
    inputs = {}
    def read(path):
        data = path.read_bytes()
        inputs[str(path)] = {"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}
        return data
    receipt = strict_json(read(case / "receipt.json"))
    require(receipt["scope"] == "native_sv2_ladder_component", "wrong archive scope")
    trace_raw = read(case / "private" / "pool.log")
    trace = PoolSource(trace_raw, independently_captured_log_sha256=inputs[str(case / "private" / "pool.log")]["sha256"])
    raw_record = strict_json(read(case / "raw-block.json"))
    raw = unhex(raw_record["raw_hex"])
    require(raw == read(case / "private" / "block-proof" / "block.bin"), "independent raw-block copies disagree")
    policy = Policy("0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206", 0x207fffff,
                    0x207fffff, None, True, True, 600, 1209600, 2016, False, None, 5000000000, 150, 100)
    block = decode_block(raw, policy, production=True)
    height = integer(raw_record["height"], 1)
    require(block["hash"] == raw_record["hash"] == receipt["block_hash"] and height == receipt["height"], "actual block receipt mismatch")
    require(len(block["transactions"]) == 1, "component archive needs raw prevouts for nonempty transaction set")
    require(sum(o["value"] for o in block["transactions"][0]["outputs"]) == policy.subsidy(height), "actual block subsidy/revenue")
    publications = trace.block_publications.get(block["hash"], [])
    require(len(publications) == 1, "actual SPCB/2 block publication missing/ambiguous")
    published, decoded = publications[0]
    candidates = [(key, event) for key, event in trace.jobs.items() if event["payload"]["height"] == height
                  and event["payload"]["fingerprint"] == decoded["coinbase_fingerprint"]]
    require(len(candidates) == 1, "actual frozen distribution join ambiguous")
    key, frozen = candidates[0]
    p = frozen["payload"]
    require(decoded["height"] == height and decoded["a_send_x"] == p["a_send_x"] and
            {str(k): v for k, v in decoded["high_water_raw"].items()} == p["high_water"], "SPCB/frozen window binding")
    # The actual component's public pool config declares R=2^255-1 and N=8.
    config = read(case / "private" / "pool.toml").decode()
    import re
    def config_scalar(name):
        values = re.findall(r"^" + name + r"\s*=\s*(.+)$", config, re.MULTILINE)
        require(len(values) == 1, "component config scalar missing/ambiguous")
        return strict_json(values[0])
    reference = int(config_scalar("work_reference_target"), 16)
    n = config_scalar("window_n")
    require(config_scalar("fee_bps") == p["fee_bps"], "frozen fee policy differs from actual config")
    window = trace.verified_window(key, reference_target=reference, window_n=n, prevout_transactions={})
    fresh, pool = raw_payouts(window["selected"], policy.subsidy(height), 0, p["fee_bps"])
    require(len(p["entries"]) == len(decoded["entries"]) == 1, "component expected one logical identity")
    entry, binary_entry = p["entries"][0], decoded["entries"][0]
    require(entry["sp_id"] == binary_entry["sp_id"] and entry["p_x"] == binary_entry["p_x"]
            and int(entry["sats"]) == binary_entry["sats"] and int(entry["owed_sats"]) == binary_entry["owed_sats"], "logical SPCB entry differs from frozen source")
    identity = strict_json(read(case / "private" / "identity.json"))
    scan, spend = silent_address_keys(identity["address"])
    require(scan.hex() == identity["b_scan"] and spend.hex() == identity["b_spend"], "silent address/public keys mismatch")
    scan_secret = scalar(unhex(identity["scan_key"], 32))
    require(compressed(_multiply(scan_secret, (_GX, _GY))) == scan, "scan key/address mismatch")
    require(tagged("SPc/Id", scan + spend).hex() == entry["sp_id"], "source identity/address attribution mismatch")
    a_send_x = carrier(block["transactions"][0], height)
    require(a_send_x.hex() == p["a_send_x"], "raw SPc carrier differs from frozen source")
    send = public_point(b"\2" + a_send_x)
    ih = scalar(tagged("SPc/Inputs", height.to_bytes(4, "big") + compressed(send)))
    shared = compressed(_multiply(ih * scan_secret, send))
    spend_point = public_point(spend)
    outputs = block["transactions"][0]["outputs"]
    by_script = {}
    for index, item in enumerate(outputs):
        by_script.setdefault(item["script"], []).append((index, item["value"]))
    tweaks = strict_json(read(case / "private" / "tweaks.json"))
    exported = {item["outpoint"]: item for item in tweaks}
    require(len(exported) == len(tweaks), "duplicate exported tweak outpoint")
    pieces, remainder = ladder(int(entry["owed_sats"]))
    require(len(pieces) == len(entry["p_x"]) <= 2048, "claimed ladder shape/count")
    found, total = [], 0
    coinbase_id = block["transactions"][0]["txid"]
    for k, expected_amount in enumerate(pieces):
        tweak_bytes = tagged("SPc/TierSpc", shared + k.to_bytes(4, "big"))
        point = _point_add(spend_point, _multiply(scalar(tweak_bytes), (_GX, _GY)))
        require(point is not None and point[0].to_bytes(32, "big").hex() == entry["p_x"][k], "independently derived SPc key mismatch")
        script = b"\x51\x20" + point[0].to_bytes(32, "big")
        matches = by_script.get(script, [])
        require(len(matches) == 1 and matches[0][1] == expected_amount and matches[0][0] != 0, "raw k-associated output amount/location")
        index, amount = matches[0]
        outpoint = f"{coinbase_id}:{index}"
        require(outpoint in exported and exported[outpoint]["tweak"] == tweak_bytes.hex()
                and exported[outpoint]["amount"] == amount, "exported tweak differs from independent derivation")
        found.append(index); total += amount
    require(len(set(found)) == len(found) == receipt["owned_outputs"] and total == receipt["owned_sats"], "actual recovery count/value")
    result = {"schema": "spc-lab-independent-ladder-archive/1", "status": "INCONCLUSIVE_FULL_ENTITLEMENT",
              "scope": "stock-Bitcoin native-SV2 ladder component, not rbitcoin T3 or Ark",
              "actual_node_sha256": receipt["node_sha256"], "input_artifacts": inputs,
              "source_trace": {"events": len(trace.events), "heads": trace.heads, "external_log_seal": trace.log_sha256,
                               "capture_scope": "read-only operator archive capture, not a live pre-start collection claim"},
              "verified": {"block_hash": block["hash"], "height": height, "raw_pow_merkle_subsidy": True,
                           "actual_SPCB2_publication": True, "snapshot_key": decoded["snapshot_key"],
                           "Q": window["window_target"], "selected_work": window["window_work"],
                           "source_slice_publication_mappings": window["mappings"], "fresh_raw_rewards": fresh,
                           "independent_owned_outputs": len(found), "independent_owned_sats": total,
                           "independent_tweaks_matched": len(found), "claimed_ladder_remainder_sats": remainder},
              "pending_provisional_ancestry": "INCONCLUSIVE: r1 has no complete RULEB reservation/economic watermark producer",
              "full_owed_verified": False, "full_T3_acceptance": False, "Ark_acceptance": False,
              "private_scan_or_spend_material_published": False}
    (output / "receipt.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    (output / "REPORT.md").write_text("# Independent ladder component archive audit\n\n"
        f"Verified actual source trace ({len(trace.events)} events), SPCB/2 publication, raw block {height}, Q={window['window_target']}, "
        f"and {len(found)} independently derived outputs/tweaks totaling {total} sats.\n\n"
        "Full entitlement remains INCONCLUSIVE pending complete RULEB booked-pending/reservation history. "
        "This stock-Bitcoin component is not rbitcoin T3 or Ark acceptance. No node RPC, producer helper or archive mutation was performed.\n")
    for path in output.iterdir():
        path.chmod(0o600)
    print(json.dumps({"status": result["status"], "receipt": str(output / "receipt.json")}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case", required=True); parser.add_argument("--output", required=True)
    args = parser.parse_args()
    output = Path(args.output)
    require(not output.exists(), "archive oracle output must be fresh")
    try:
        audit(args.case, args.output)
        return 2
    except (Inconclusive, Invalid, OSError, ValueError, KeyError, TypeError, IndexError) as error:
        status = "FAILED" if isinstance(error, Invalid) else "INCONCLUSIVE"
        if output.is_dir():
            receipt = {"schema": "spc-lab-independent-ladder-archive/1", "status": status,
                       "scope": "stock-Bitcoin component archive only", "error_class": type(error).__name__,
                       "code": str(error) if isinstance(error, (Invalid, Inconclusive)) else "source_capture_unavailable",
                       "full_T3_acceptance": False, "Ark_acceptance": False}
            (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
            (output / "receipt.json").chmod(0o600)
        print(json.dumps({"status": status, "receipt": str(output / "receipt.json")}))
        return 1 if status == "FAILED" else 2


if __name__ == "__main__":
    sys.exit(main())
