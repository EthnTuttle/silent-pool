"""Integer payout proof from immutable windows and source-attributed shares.

This module consumes observations, not pool-computed payout totals. A source
adapter must retain the actual publication/attribution records referenced by
each bundle. Missing retention or pending ancestry is inconclusive.
"""
from oracle import (Inconclusive, decode_transaction, integer, ladder, merkle,
                    raw_payouts, require, sha256d, target, unhex)


def share_proof(share, job, reference):
    cb = decode_transaction(unhex(job["coinbase_prefix"]) + unhex(share["extranonce"])
                            + unhex(job["coinbase_suffix"]))
    root = cb["txid_bytes"]
    for sibling in job["merkle_path"]:
        root = sha256d(root + unhex(sibling, 32))
    raw = (integer(share["version"], 0, 0xffffffff).to_bytes(4, "little")
           + unhex(job["prev_hash_le"], 32) + root
           + integer(share["ntime"], 0, 0xffffffff).to_bytes(4, "little")
           + integer(job["nbits"], 0, 0xffffffff).to_bytes(4, "little")
           + integer(share["nonce"], 0, 0xffffffff).to_bytes(4, "little"))
    digest = sha256d(raw)
    require(digest == unhex(share["share_hash_le"], 32), "attributed share reconstructed hash")
    threshold = int.from_bytes(unhex(job["share_target_be"], 32), "big")
    require(int.from_bytes(digest, "little") <= threshold, "share target proof")
    require((reference + 1) % (threshold + 1) == 0, "nonintegral reference work")
    work = (reference + 1) // (threshold + 1)
    require(work == job["m_issue"] == share["credited_work"] and work > 0 and work & (work - 1) == 0,
            "share credit versus issued target")
    require(share["generation"] == job["generation"] and share["job_id"] == job["job_id"],
            "immutable job attribution")
    # The adapter supplies raw job transactions, including their real prevouts.
    # Claimed job_fees are never used as the fee attribution authority.
    fees, txids = 0, [cb["txid_bytes"]]
    for item in job["transactions"]:
        tx = decode_transaction(unhex(item["raw_hex"]))
        txids.append(tx["txid_bytes"])
        prevouts = item["prevout_transactions"]
        total = 0
        for inp in tx["inputs"]:
            txid, index = inp["outpoint"].split(":")
            if txid not in prevouts:
                raise Inconclusive("job fee prevout evidence missing")
            prev = decode_transaction(unhex(prevouts[txid]))
            require(prev["txid"] == txid, "job fee prevout txid")
            index = integer(int(index), 0, len(prev["outputs"]) - 1)
            total += prev["outputs"][index]["value"]
        fee = total - sum(o["value"] for o in tx["outputs"])
        require(fee >= 0, "job negative fee")
        fees += fee
    require(merkle(txids) == root, "job transaction set versus actual coinbase Merkle path")
    return {"identity": share["identity"], "work": work, "fees": fees}


def checked_numeric_window(snapshot, candidates):
    """Validate the ENTIRE declared vector before any Q-based early stop.

    This primitive handles source-normalized numeric IDs for a single front.
    Across fronts, numeric IDs are not a global publication order. Redis marks
    and multi-front inputs need the authenticated immutable stream mapping;
    the pending producer adapter may not silently reinterpret them here.
    """
    high = snapshot["high_water"]
    require(isinstance(high, dict) and high, "missing immutable high-water vector")
    marks = {}
    for key, mark in high.items():
        require(isinstance(key, str) and key.isdecimal() and str(int(key)) == key, "high-water front id encoding")
        front = integer(int(key), 0, 127)
        if type(mark) is not int:
            raise Inconclusive("Redis high-water marks require authenticated stream-to-slice mapping")
        integer(mark, 0, (1 << 63) - 1)
        require(mark == 0 or mark >> 56 == front, "high-water numeric id belongs to another front")
        marks[front] = mark
    latest, seen, front_ids = {}, set(), set()
    for row in candidates:
        sid, front = integer(row["slice_id"], 1, (1 << 63) - 1), integer(row["front_id"], 0, 127)
        require(sid >> 56 == front, "slice id/front identity mismatch")
        require(sid not in seen, "duplicate retained slice id")
        require(row["work_unit_fingerprint"] == snapshot["work_unit_fingerprint"], "slice profile")
        seen.add(sid); front_ids.add(front)
        latest[front] = max(latest.get(front, 0), sid)
        if front in marks and marks[front]:
            require(sid <= marks[front], "retained candidate newer than published high-water")
    for front, mark in marks.items():
        if mark and (front not in latest or latest[front] != mark):
            raise Inconclusive("nonzero high-water front/latest slice proof missing")
    if len(front_ids | {front for front, mark in marks.items() if mark}) > 1:
        raise Inconclusive("multi-front window requires authenticated global publication order, not numeric slice ordering")
    require(all(candidates[i]["slice_id"] > candidates[i + 1]["slice_id"] for i in range(len(candidates) - 1)),
            "retained window order changed")
    return marks


def verify_distribution(bundle, chain, pending_before):
    """Verify one published, settled pool-template distribution.

    pending_before is caller-maintained state from the complete prior settled
    history (fresh run initializes explicit identities at zero). It must not
    be filled in from a current pool balance or an omitted-row default.
    """
    snapshot = bundle["snapshot"]
    if bundle["block_hash"] not in chain.by_hash:
        raise Inconclusive("payout block no longer canonical at audit anchor")
    block = chain.by_hash[bundle["block_hash"]]
    require(snapshot["height"] == block["height"], "distribution height")
    require(snapshot["work_unit_version"] == 2 and snapshot["window_rule"] == 1, "payout v2 window rule")
    reference = int.from_bytes(unhex(bundle["reference_target_be"], 32), "big")
    q = (bundle["window_n"] * (reference + 1) + target(block["bits"])) // (target(block["bits"]) + 1)
    require(snapshot["window_target"] == q, "frozen contextual window Q")
    candidates = bundle["slices_newest_first"]
    if not candidates:
        raise Inconclusive("published immutable slice window missing")
    require(len(candidates) <= 4096, "window retention bound")
    declared_marks = checked_numeric_window(snapshot, candidates)
    selected, work, previous, seen_shares = [], 0, None, set()
    first_per_front = {}
    for row in candidates:
        require(row["work_unit_fingerprint"] == snapshot["work_unit_fingerprint"], "slice profile")
        require(row["slice_id"] <= snapshot["high_water"][str(row["front_id"])] and
                (previous is None or row["slice_id"] < previous), "sealed window ordering")
        require(row["slice_id"] != bundle["finding_slice_id"], "finding slice included")
        if row["front_id"] not in first_per_front:
            require(row["slice_id"] == snapshot["high_water"][str(row["front_id"])], "missing high-water slice")
            first_per_front[row["front_id"]] = row["slice_id"]
        previous = row["slice_id"]
        for share in row["shares"]:
            require(share["share_hash_le"] not in seen_shares, "duplicate attributed share")
            seen_shares.add(share["share_hash_le"])
        proofs = [share_proof(s, bundle["jobs"][s["job_key"]], reference) for s in row["shares"]]
        actual = sum(s["work"] for s in proofs)
        require(actual == row["difficulty"] and len(proofs) == row["n_shares"], "sealed share attribution total")
        work += actual
        selected.append(proofs)
        if work >= q:
            break
    if work < q:
        raise Inconclusive("underfilled or retained window gap")
    require(first_per_front == {front: mark for front, mark in declared_marks.items() if mark},
            "selected-window membership differs from complete published high-water vector")
    raw, pool = raw_payouts(selected, block["subsidy"], block["fees"], bundle["fee_bps"])
    identities = set(raw) | set(pending_before)
    require(set(bundle["ledger"]) == identities, "ledger identity coverage")
    outputs = block["transactions"][0]["outputs"]
    require(outputs[0]["script"].hex() == bundle["pool_script_hex"], "pool output binding")
    paid_total, after, mapping = 0, {}, {}
    for identity in identities:
        if identity not in pending_before:
            raise Inconclusive("pending ancestry missing for identity")
        owed = raw.get(identity, 0) + pending_before[identity]
        pieces, remainder = ladder(owed)
        entry = bundle["entries"][identity]
        require(entry["format_version"] == 2 and entry["owed_sats"] == owed
                and entry["sats"] == sum(pieces), "SPCB/2 full owed/shaped amount binding")
        require(len(entry["p_x"]) == len(pieces), "ladder output count")
        amounts = []
        for key in entry["p_x"]:
            script = b"\x51\x20" + unhex(key, 32)
            require(script not in mapping, "duplicate attribution script")
            mapping[script] = identity
            matches = [o["value"] for o in outputs[1:] if o["script"] == script]
            require(len(matches) == 1, "missing/duplicate actual miner output")
            amounts += matches
        require(amounts == pieces, "fixed ladder k-associated denomination mismatch")
        paid = sum(amounts)
        row = bundle["ledger"][identity]
        for field in ("paid_sats", "ark_due_sats", "forfeited_sats"):
            integer(row[field])
        require(row["paid_sats"] == paid, "ledger paid differs from raw coinbase")
        next_pending = pending_before[identity] + row["pending_delta"]
        require(next_pending >= 0 and paid + row["ark_due_sats"] + row["forfeited_sats"] + next_pending == owed,
                "per-identity paid/owed/remainder conservation")
        require(not pieces or row["forfeited_sats"] == 0, "rung remainder forfeited")
        if entry["delivery"] == "registered_derived":
            require(row["ark_due_sats"] == remainder and next_pending == 0 and row["forfeited_sats"] == 0,
                    "registered remainder routing")
        else:
            raise Inconclusive("delivery state requires source registration history adapter")
        paid_total += paid
        after[identity] = next_pending
    for output in outputs[1:]:
        require(output["script"] in mapping or output["value"] == 0 and output["script"].startswith(bytes.fromhex("6a24aa21a9ed")),
                "unattributed coinbase output")
    require(paid_total + outputs[0]["value"] == block["subsidy"] + block["fees"], "pool-template revenue conservation")
    require(outputs[0]["value"] == pool + sum(bundle["ledger"][i]["ark_due_sats"] + after[i] - pending_before[i] for i in identities),
            "pool backing conservation")
    return {"height": block["height"], "fees": block["fees"], "window_work": work,
            "selected_slice_ids": [r["slice_id"] for r in candidates[:len(selected)]],
            "retained_tail_rows_outside_window": len(candidates) - len(selected),
            "window_target": q, "overshoot": work - q, "paid": paid_total,
            "pending_after": after}
