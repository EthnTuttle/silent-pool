"""Offline decoder for the explicitly UNVERIFIED working owner API r2.

Schema/field checks and independent integer arithmetic are usable now. This
pin is not a tested producer binding and cannot authorize live acceptance.
"""
from copy import deepcopy
import hashlib
import json
from pathlib import Path

from net_debits import block_economics, available
from oracle import Inconclusive, Reader, integer, require, strict_json

API_SHA256 = "bad6ff5e1a8602e8b0e25fabac23020af4862691db26e925363b8102fbf0f78c"
MAX_JSON = 4 * 1024 * 1024
MAX_ACTIVE = 4096
FROZEN_FIELDS = "version revision parent_hash job_id generation balances slice_ids".split()
BALANCE_FIELDS = "sp_id booked reserved available".split()
CONSUMPTION_FIELDS = "sp_id prior error_pending delta consumed".split()
RESERVATION_FIELDS = "block_hash parent_hash height payload_digest job_id generation lifecycle transition revision rows".split()
VIEW_FIELDS = "version revision anchor_height anchor_hash complete booked reservations".split()


def working_pin(digest):
    require(digest == API_SHA256, "working RULEB API pin mismatch")


def inspect_api(path):
    raw = Path(path).read_bytes()
    working_pin(hashlib.sha256(raw).hexdigest())
    value = strict_json(raw)
    require(value["schema"] == "spc.ladder-owner-handoff/2" and value["status"] == "working-source-unverified-disk-blocked"
            and value["verification"]["current_tests_passed"] is False
            and value["verification"]["current_tested_freeze"] is None, "working API is not the pinned unverified contract")
    return {"api_sha256": API_SHA256, "live_acceptance_eligible": False, "status": value["status"]}


def fields(value, names, label):
    if not isinstance(value, dict) or set(value) != set(names):
        raise Inconclusive("missing/unsupported " + label + " fields")


def u64(value):
    return integer(value, 0, (1 << 64) - 1)


def hash_array(value):
    require(isinstance(value, list) and len(value) == 32, "economic hash must be a 32-byte integer array")
    for byte in value:
        integer(byte, 0, 255)
    return bytes(value).hex()


def rows(value, limit, label):
    if not isinstance(value, list) or len(value) > limit:
        raise Inconclusive(label + " missing or exceeds capture bound")
    return value


def version_one(value):
    if type(value) is not int or value != 1:
        raise Inconclusive("unsupported economic schema version")


def frozen_job(value, *, height=None, wire_job_id=None, generation=None, parent_hash=None, slice_ids=None):
    if value is None:
        raise Inconclusive("historical/missing frozen economics is not P=0")
    fields(value, FROZEN_FIELDS, "FrozenJob")
    version_one(value["version"])
    u64(value["revision"]); u64(value["job_id"]); u64(value["generation"])
    parent = hash_array(value["parent_hash"])
    seen = set()
    for balance in rows(value["balances"], 65535, "FrozenJob balances"):
        fields(balance, BALANCE_FIELDS, "Balance")
        identity = hash_array(balance["sp_id"])
        require(identity not in seen, "duplicate frozen balance identity")
        seen.add(identity)
        b, r, a = (u64(balance[k]) for k in ("booked", "reserved", "available"))
        require(r <= b and b - r == a, "frozen B-R differs from claimed A")
    slices = [u64(sid) for sid in rows(value["slice_ids"], MAX_ACTIVE, "frozen slice ids")]
    require(all(a > b for a, b in zip(slices, slices[1:])), "frozen slice ids must be strictly decreasing")
    if height is not None:
        integer(height, 0, 0xffffffff)
        require(value["job_id"] >> 32 == height, "FrozenJob packed height mismatch")
    if wire_job_id is not None:
        integer(wire_job_id, 0, 0xffffffff)
        require(value["job_id"] & 0xffffffff == wire_job_id, "FrozenJob wire job sequence mismatch")
    if generation is not None:
        require(value["generation"] == u64(generation), "FrozenJob generation mismatch")
    if parent_hash is not None:
        require(parent == parent_hash, "FrozenJob parent mismatch")
    if slice_ids is not None:
        require(slices == slice_ids, "FrozenJob differs from actual frozen ordered slices")
    return deepcopy(value)


def canonical_frozen(value):
    ordered = {name: value[name] for name in FROZEN_FIELDS}
    ordered["balances"] = [{name: row[name] for name in BALANCE_FIELDS} for row in value["balances"]]
    return json.dumps(ordered, separators=(",", ":"), ensure_ascii=False).encode()


def frozen_frame(reader, *, height, identities):
    if len(reader.data) - reader.pos < 4:
        raise Inconclusive("truncated economic frame length")
    length = reader.uint(4)
    if length > MAX_JSON or len(reader.data) - reader.pos < length:
        raise Inconclusive("economic frame missing/truncated/oversized")
    raw = reader.take(length)
    try:
        value = frozen_job(strict_json(raw), height=height)
    except (UnicodeError, ValueError) as error:
        raise Inconclusive("economic JSON unavailable") from error
    require(canonical_frozen(value) == raw, "noncanonical FrozenJob JSON cannot define snapshot identity")
    require({hash_array(row["sp_id"]) for row in value["balances"]} == set(identities), "SPCB economic prior identity coverage")
    return value


def consumption(value, frozen=None):
    parsed, seen = [], set()
    prior = {hash_array(b["sp_id"]): b["available"] for b in frozen["balances"]} if frozen is not None else None
    for row in rows(value, 65535, "Consumption rows"):
        fields(row, CONSUMPTION_FIELDS, "Consumption")
        identity = hash_array(row["sp_id"])
        require(identity not in seen, "duplicate consumption identity")
        seen.add(identity)
        p, e, c = u64(row["prior"]), u64(row["error_pending"]), u64(row["consumed"])
        delta = integer(row["delta"], -(1 << 63), (1 << 63) - 1)
        own = block_economics(p, e)
        require(delta == own["delta"] and c == own["net_debit"], "consumption differs from independently derived E-P/c")
        if prior is not None:
            require(identity in prior and p == prior[identity], "consumption P differs from immutable frozen job")
        parsed.append(deepcopy(row))
    if prior is not None:
        require(seen == set(prior), "consumption/frozen identity census")
    return parsed


def reservation(value):
    fields(value, RESERVATION_FIELDS, "Reservation")
    for key in ("block_hash", "parent_hash", "payload_digest"):
        hash_array(value[key])
    integer(value["height"], 0, 0xffffffff)
    for key in ("job_id", "generation", "transition", "revision"):
        u64(value[key])
    require(value["job_id"] >> 32 == value["height"], "reservation packed job height mismatch")
    require(value["transition"] > 0, "economic transition must be positive")
    if value["lifecycle"] not in {"unbooked", "booked", "cancelled", "loss_held"}:
        raise Inconclusive("unsupported economic lifecycle")
    consumption(value["rows"])
    return deepcopy(value)


def bootstrap_view(value):
    fields(value, VIEW_FIELDS, "economic View")
    version_one(value["version"])
    u64(value["revision"]); integer(value["anchor_height"], 0, 0xffffffff); hash_array(value["anchor_hash"])
    if value["complete"] is not True:
        raise Inconclusive("incomplete parent economic checkpoint")
    booked, reserved, blocks = {}, {}, set()
    for row in rows(value["booked"], 65536, "booked rows"):
        require(isinstance(row, list) and len(row) == 2, "booked tuple encoding")
        identity = hash_array(row[0]); amount = u64(row[1])
        require(identity not in booked, "duplicate booked identity")
        booked[identity] = amount
    for raw in rows(value["reservations"], MAX_ACTIVE, "active reservations"):
        item = reservation(raw)
        block = hash_array(item["block_hash"])
        require(block not in blocks, "duplicate active economic block")
        blocks.add(block)
        if item["revision"] > value["revision"]:
            raise Inconclusive("reservation revision is ahead of checkpoint")
        require(item["lifecycle"] in {"unbooked", "loss_held"}, "bootstrap active set contains inactive lifecycle")
        for row in item["rows"]:
            identity = hash_array(row["sp_id"])
            if item["lifecycle"] == "unbooked":
                reserved[identity] = u64(reserved.get(identity, 0) + row["consumed"])
    balances = {}
    for identity in set(booked) | set(reserved):
        b, r = booked.get(identity, 0), reserved.get(identity, 0)
        if r > b:
            raise Inconclusive("checkpoint reserved exceeds booked")
        balances[identity] = {"booked": b, "reserved": r, "available": available(b, [r])}
    return {"view": deepcopy(value), "balances": balances, "authority": "unverified_checkpoint_claim"}


def economic_changed(raw):
    if len(raw) != 49:
        raise Inconclusive("EconomicChanged hint length")
    r = Reader(raw)
    if r.uint(1) != 5:
        raise Inconclusive("unsupported economic front-event discriminator")
    return {"rev": r.uint(8), "economic_revision": r.uint(8), "anchor_hash": r.take(32).hex(),
            "authority": "reload_hint_only", "checkpoint_verified": False}


def require_live_binding():
    raise Inconclusive("working RULEB API r2 is not a tested producer/source/binary acceptance binding")
