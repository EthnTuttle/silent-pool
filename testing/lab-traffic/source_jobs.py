"""Actual source-evidence/2 -> raw job/witness/publication component checks.

No production verification helper, signer, or header search. JobProof output
uses the frozen Rust source_audit.rs byte-array fields. Complete SourceBundle
and TrustedSource are withheld until their missing history/feed joins exist.
"""
from oracle import (Inconclusive, decode_transaction, expected_bits, integer, merkle,
                    require, sha256d, unhex)
from pool_source import decimal_uint


class MissingComponent(Inconclusive):
    def __init__(self, component):
        self.component = component
        super().__init__(component)


def parent_view(payload, views):
    previous = unhex(payload["prev_hash"], 32)[::-1].hex()
    for view in views.values():
        if previous in view.by_hash:
            return view.prefix(view.by_hash[previous]["height"])
    raise MissingComponent("raw_parent_ancestry")


def branch(transactions):
    level = [bytes(32)] + [t["txid_bytes"] for t in transactions]
    result = []
    while len(level) > 1:
        require(all(level[i] != level[i + 1] for i in range(0, len(level) - 1, 2)), "job mutated transaction Merkle set")
        result.append(level[1])
        if len(level) % 2:
            level.append(level[-1])
        level = [sha256d(level[i] + level[i + 1]) for i in range(0, len(level), 2)]
    return result


def verify_job(source, key, views, policy):
    p = source.jobs[key]["payload"]
    parent = parent_view(p, views)
    height = integer(p["height"], 1)
    require(height == parent.tip["height"] + 1, "frozen job height/parent mismatch")
    require(p["nbits"] == expected_bits(policy, parent.blocks, integer(p["ntime"])), "frozen job contextual target mismatch")
    extra = integer(p["extranonce_len"], 0, 63)
    prefix, suffix = unhex(p["coinbase_prefix"]), unhex(p["coinbase_suffix"])
    cb = decode_transaction(prefix + bytes(extra) + suffix)
    require(cb["coinbase"], "issued mining transaction must be coinbase")
    script = cb["inputs"][0]["script"]
    # Exact minimal BIP34 encoding and pool-template carrier, without guessing
    # alternative offsets or accepting a donation fallback.
    if 1 <= height <= 16:
        height_push = bytes([0x50 + height])
    else:
        encoded = height.to_bytes((height.bit_length() + 7) // 8, "little")
        if encoded[-1] & 128:
            encoded += b"\0"
        height_push = bytes([len(encoded)]) + encoded
    require(script == height_push + b"\x20" + unhex(p["a_send_x"], 32) + bytes(extra), "job BIP34/carrier/extranonce binding")
    if extra:
        script_start=44 if cb["witness"] else 42
        require(len(prefix)>=script_start+len(height_push)+33 and len(prefix)+extra<=script_start+len(script),
                "extranonce overlaps immutable carrier, outputs or witness")
    txs = [decode_transaction(unhex(raw)) for raw in p["transactions"]]
    require(all(not tx["coinbase"] for tx in txs) and len({t["txid"] for t in txs}) == len(txs), "job transaction census")
    require([x.hex() for x in branch(txs)] == p["merkle_branch"], "job transaction bytes/Merkle path mismatch")
    spent, outputs, prevouts, fees = set(), {}, {}, 0
    for tx in txs:
        ins = 0
        for inp in tx["inputs"]:
            point = inp["outpoint"]
            require(point not in spent, "job spends an input twice")
            spent.add(point)
            item = outputs.get(point) or parent.utxos.get(point)
            if item is None:
                raise MissingComponent("job_fee_prevout_unspent_ancestry")
            require(not item["coinbase"] or height - item["height"] >= policy.coinbase_maturity, "job spends immature coinbase")
            ins += item["value"]
            txid = point.split(":")[0]
            if txid in parent.transactions:
                prevouts[txid] = parent.transactions[txid][1]["raw"]
        out = sum(o["value"] for o in tx["outputs"])
        require(ins >= out, "job negative transaction fee")
        fees += ins - out
        for index, output in enumerate(tx["outputs"]):
            outputs[f"{tx['txid']}:{index}"] = {**output, "coinbase": False, "height": height}
        # A preceding in-template parent is also an actual full prevout tx.
        prevouts[tx["txid"]] = tx["raw"]
    require(fees == decimal_uint(p["fees"]) and policy.subsidy(height) == decimal_uint(p["subsidy"]), "job raw fees/subsidy mismatch")
    require(sum(o["value"] for o in cb["outputs"]) == policy.subsidy(height) + fees, "job coinbase revenue mismatch")
    indices = [i for i, o in enumerate(cb["outputs"]) if o["script"].startswith(bytes.fromhex("6a24aa21a9ed"))]
    witness = bool(cb["witness"]) or any(t["witness"] for t in txs) or bool(indices) or p["witness_plan"] is not None
    plan = p["witness_plan"]
    if witness:
        if plan is None:
            raise MissingComponent("actual_witness_plan")
        require(set(plan) == {"wtxid_root", "reserved", "commitment"}, "unknown witness plan fields")
        root = merkle([bytes(32)] + [t["wtxid_bytes"] for t in txs])
        commitment = sha256d(root + unhex(plan["reserved"], 32))
        require(root.hex() == plan["wtxid_root"] and commitment.hex() == plan["commitment"], "job witness plan differs from raw transaction set")
        require(not cb["witness"] or cb["witness"]==[[unhex(plan["reserved"],32)]],"full coinbase witness differs from immutable reserved value")
        require(indices == [len(cb["outputs"]) - 1], "pool witness commitment must be unique and last")
        output = cb["outputs"][indices[0]]
        require(output["value"] == 0 and output["script"] == bytes.fromhex("6a24aa21a9ed") + commitment, "job commitment output mismatch")
    else:
        require(plan is None and not indices, "witness-free job has unexpected plan/commitment")
    job = {"job_id": (height << 32) | int(p["job_id"], 16), "generation": key[3], "height": height,
           "prev_hash": list(unhex(p["prev_hash"], 32)), "version": p["version"], "ntime": p["ntime"], "nbits": p["nbits"],
           "coinbase_prefix": list(prefix), "coinbase_suffix": list(suffix), "extranonce_len": extra,
           "merkle_path": [list(unhex(h, 32)) for h in p["merkle_branch"]],
           "share_target_be": list(unhex(p["share_target"], 32)), "credited_work": p["m_issue"],
           "transactions": [list(t["raw"]) for t in txs], "prevout_transactions": [list(raw) for _, raw in sorted(prevouts.items())]}
    return {"job": job, "coinbase": cb, "fees": fees, "witness": witness,
            "txids": [t["txid"] for t in txs], "wtxids": [t["wtxid"] for t in txs]}


def matches_coinbase_template(payload, coinbase):
    prefix,suffix=unhex(payload["coinbase_prefix"]),unhex(payload["coinbase_suffix"])
    sample=decode_transaction(prefix+bytes(payload["extranonce_len"])+suffix)
    raw=coinbase["raw"] if sample["witness"] else coinbase["stripped"]
    return len(raw)==len(prefix)+payload["extranonce_len"]+len(suffix) and raw.startswith(prefix) and raw.endswith(suffix)


def publication_job(source,wire):
    candidates = [(key, e) for key, e in source.jobs.items() if e["payload"]["fingerprint"] == wire["coinbase_fingerprint"]
                  and e["payload"]["height"] == wire["height"] and key[1] == wire["front_id"]]
    # Identical coinbase bytes can survive a new economic revision/generation.
    # SPCB/3 carries the exact FrozenJob (including full ID/generation/revision);
    # use that immutable evidence, never newest-job or current-balance fallback.
    if wire.get("economic") is not None:
        candidates=[item for item in candidates if item[1]["payload"].get("economic")==wire["economic"]]
    require(len(candidates)==1,"SPCB/frozen job join missing or ambiguous")
    return candidates[0]


def verify_publication(source, block_hash, views, policy):
    publications = source.block_publications.get(block_hash, [])
    if not publications:
        raise MissingComponent("actual_SPCB2_publication")
    require(len({e["payload"]["raw_d_hex"] for e, _ in publications}) == 1, "block republished with different SPCB bytes")
    event, wire = publications[0]
    key, frozen = publication_job(source,wire)
    if wire.get("economic") is not None:
        require(wire["economic"] == frozen["payload"].get("economic"), "SPCB/frozen economic provenance differs")
    require(frozen["source"]["session"] == event["source"]["session"]
            and decimal_uint(frozen["source"]["seq"]) < decimal_uint(event["source"]["seq"]), "publication precedes frozen job")
    checked = verify_job(source, key, views, policy)
    source.normalized_job(key, {decode_transaction(bytes(raw))["txid"]: bytes(raw).hex() for raw in checked["job"]["prevout_transactions"]})
    blocks = [v.by_hash[block_hash] for v in views.values() if block_hash in v.by_hash]
    if not blocks:
        raise MissingComponent("raw_published_block_ancestry")
    block, p = blocks[0], frozen["payload"]
    coinbase = block["transactions"][0]
    raw = coinbase["raw"] if checked["coinbase"]["witness"] else coinbase["stripped"]
    prefix, suffix = unhex(p["coinbase_prefix"]), unhex(p["coinbase_suffix"])
    require(matches_coinbase_template(p,coinbase),
            "published raw coinbase differs from immutable issued job")
    require([t["raw"].hex() for t in block["transactions"][1:]] == p["transactions"], "published block transaction/witness bytes differ from frozen job")
    require(block["height"] == p["height"] and block["prev"] == unhex(p["prev_hash"], 32)[::-1].hex(), "published block height/parent differs")
    winners = [a for records in source.accounts.values() for a in records
               if a["payload"]["share_hash"] == unhex(block_hash, 32)[::-1].hex() and source.job_key(a) == key]
    if len(winners) != 1:
        raise MissingComponent("unique_actual_winning_share_attribution")
    winner = winners[0]; a = winner["payload"]
    require(decimal_uint(frozen["source"]["seq"]) < decimal_uint(winner["source"]["seq"]) < decimal_uint(event["source"]["seq"]),
            "winning share outside frozen/publication boundary")
    header = (integer(a["version"], 0, 0xffffffff).to_bytes(4, "little") + unhex(p["prev_hash"], 32)
              + block["raw"][36:68] + integer(a["ntime"], 0, 0xffffffff).to_bytes(4, "little")
              + p["nbits"].to_bytes(4, "little") + integer(a["nonce"], 0, 0xffffffff).to_bytes(4, "little"))
    require(header == block["raw"] and unhex(a["coinbase"]) == raw
            and prefix + unhex(a["extranonce"]) + suffix == raw, "winning share/raw block bytes mismatch")
    require(any(i["payload"]["channel_id"] == a["channel_id"]
                and decimal_uint(i["source"]["seq"]) < decimal_uint(winner["source"]["seq"]) for i in source.issued[key]),
            "winning share lacks prior channel job issuance")
    require(wire["a_send_x"] == p["a_send_x"] and {str(k): v for k, v in wire["high_water_raw"].items()} == p["high_water"], "SPCB frozen carrier/high-water mismatch")
    require(wire["work_unit_version"] == p["work_unit_version"] and wire["work_unit_fingerprint"] == p["work_unit_fingerprint"]
            and wire["window_rule"] == p["window_rule"] and wire["window_target"] == decimal_uint(p["window_target"], 128),
            "SPCB frozen work/window policy mismatch")
    require(len(wire["entries"]) == len(p["entries"]), "SPCB/frozen logical entry census")
    for actual, claimed in zip(wire["entries"], p["entries"]):
        require(actual["sp_id"] == claimed["sp_id"] and actual["p_x"] == claimed["p_x"]
                and actual["sats"] == decimal_uint(claimed["sats"]) and actual["owed_sats"] == decimal_uint(claimed["owed_sats"]),
                "SPCB entry differs from frozen job")
    if checked["witness"]:
        require(coinbase["witness"] == [[unhex(p["witness_plan"]["reserved"], 32)]], "mined coinbase witness differs from frozen reserved value")
    return {"block_hash": block_hash, "job_key": list(key), "snapshot_key": wire["snapshot_key"],
            "raw_fee_sats": checked["fees"], "witness": checked["witness"], "owed_verified": False}


def witness_republication(source, views, policy):
    verified = []
    for key, event in source.jobs.items():
        p = event["payload"]
        if p.get("witness_plan") is None:
            continue
        old_id = p.get("superseded_job_id")
        old = [(k, e) for k, e in source.jobs.items() if k[:2] == key[:2] and k[2] == old_id
               and decimal_uint(e["source"]["seq"]) < decimal_uint(event["source"]["seq"])]
        if len(old) != 1:
            continue
        previous = old[0][1]["payload"]
        # Require an actual tx-set-only refresh on the same parent, not merely
        # a normal height change containing a witness transaction.
        if previous["prev_hash"] != p["prev_hash"] or previous["transactions"] == p["transactions"]:
            continue
        if old[0][0] not in source.issued:
            continue
        old_material = verify_job(source, old[0][0], views, policy)
        source.normalized_job(old[0][0], {decode_transaction(bytes(raw))["txid"]: bytes(raw).hex()
                                         for raw in old_material["job"]["prevout_transactions"]})
        material = verify_job(source, key, views, policy)
        source.normalized_job(key, {decode_transaction(bytes(raw))["txid"]: bytes(raw).hex() for raw in material["job"]["prevout_transactions"]})
        issued = source.issued[key]
        rejections = [e for e in source.events if e["kind"] == "share_rejected" and e["source"]["session"] == key[0]
                      and e["payload"].get("front_id") == key[1] and e["payload"].get("job_id") == old_id
                      and e["payload"].get("active_job_id") == key[2] and e["payload"].get("generation") == str(key[3])
                      and e["payload"].get("reason") == "stale-share"
                      and any(i["payload"]["channel_id"] == e["payload"].get("channel_id")
                              and decimal_uint(i["source"]["seq"]) < decimal_uint(event["source"]["seq"])
                              for i in source.issued[old[0][0]])
                      and any(i["payload"]["channel_id"] == e["payload"].get("channel_id")
                              and decimal_uint(i["source"]["seq"]) < decimal_uint(e["source"]["seq"]) for i in issued)]
        if rejections:
            verified.append({"job_key": list(key), "superseded_job_id": old_id, "txids": material["txids"],
                             "stale_source_sequences": [e["source"]["seq"] for e in rejections]})
    if not verified:
        raise MissingComponent("same_parent_witness_refresh_issued_job_and_actual_old_job_stale_rejection")
    return verified
