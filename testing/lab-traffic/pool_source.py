"""Consumer of the actual ladder r1 spc.source-evidence/2 producer.

Input is the independently sealed process log, not a bundle-supplied head.
This verifies concrete source_publication/distribution_frozen/job/share bytes.
Pending/provisional completeness is intentionally a separate required proof.
"""
from collections import Counter
import hashlib
import json
from pathlib import Path
import re

from oracle import (Inconclusive, Reader, decode_transaction, integer, ladder,
                    merkle, raw_payouts, require, sha256d, strict_json, unhex)
from ladder_wire import decode_spcb2
from payout import share_proof

KINDS = {"bootstrap_applied", "distribution_frozen", "job_issued", "accounted", "source_publication",
         "share_rejected", "pending_applied", "proof_unavailable", "multiplier_stepup", "multiplier_stepdown", "share_outcome_unknown",
         "economic_change_observed"}
SOURCE_FIELDS = {"schema", "session", "seq", "previous", "digest", "complete_prefix_required"}
ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")


def decimal_uint(value, bits=64):
    require(isinstance(value, str) and value.isascii() and value.isdecimal()
            and str(int(value)) == value, "noncanonical source integer string")
    return integer(int(value), 0, (1 << bits) - 1)


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def trace_digest(kind, payload, source):
    name = kind.encode("utf-8")
    return hashlib.sha256(b"SPc/SourceEvidenceV2" + unhex(source["session"], 16)
                          + decimal_uint(source["seq"]).to_bytes(8, "little") + unhex(source["previous"], 32)
                          + len(name).to_bytes(4, "little") + name + canonical(payload)).hexdigest()


def leaf_key(leaf):
    extra = leaf["extranonce"]
    require(isinstance(extra, list) and all(type(v) is int and 0 <= v <= 255 for v in extra), "source share extranonce bytes")
    return (integer(leaf["nonce"], 0, 0xffffffff), integer(leaf["ntime"], 0, 0xffffffff),
            integer(leaf["version"], 0, 0xffffffff), bytes(extra), integer(leaf["job_id"]),
            integer(leaf["share_index"], 0, 0xffffffff))


def leaf_hash(leaf):
    nonce, ntime, version, extra, job, index = leaf_key(leaf)
    return sha256d(nonce.to_bytes(4, "little") + ntime.to_bytes(4, "little") + version.to_bytes(4, "little")
                   + extra + job.to_bytes(8, "little") + index.to_bytes(4, "little"))


def decode_slice(raw):
    if len(raw) > 16 * 1024 * 1024:
        raise Inconclusive("source slice observation bound")
    r = Reader(raw)
    sid, height, previous = r.uint(8), r.uint(4), r.take(32).hex()
    ref_job, ref_fees, count, work, root = r.uint(8), r.uint(8), r.uint(4), r.uint(16), r.take(32)
    leaves = strict_json(r.take(r.uint(4)))
    require(isinstance(leaves, list) and len(leaves) == count and count > 0, "source slice share count/blob")
    keys = [leaf_key(leaf) for leaf in leaves]
    require(len(set(keys)) == len(keys), "duplicate source slice leaf")
    require(merkle([leaf_hash(leaf) for leaf in leaves]) == root, "source slice Merkle root")
    credits = {}
    for _ in range(r.uint(2)):
        identity, d, df = r.take(32).hex(), r.uint(16), r.uint(16)
        require(identity not in credits, "duplicate source credit identity")
        credits[identity] = (d, df)
    version, size = r.uint(1), r.uint(1)
    require(size == 32, "source slice definite work fingerprint")
    fingerprint = r.take(size).hex()
    r.end()
    require(version == 2 and fingerprint != "00" * 32, "source slice must be declared v2")
    return {"slice_id": sid, "height": height, "front_id": sid >> 56, "prev_hash": previous,
            "ref_job_id": ref_job, "ref_fees": ref_fees, "n_shares": count, "difficulty": work,
            "root": root.hex(), "shares": leaves, "credits": credits,
            "work_unit_version": version, "work_unit_fingerprint": fingerprint}


class PoolSource:
    def __init__(self, raw_log, *, independently_captured_log_sha256, working_ruleb_api_sha256=None,
                 source_contract=None, pool_pubkey=None, decision_contract=None):
        self.source_contract, self.pool_pubkey = source_contract, pool_pubkey
        self.decision_contract=decision_contract
        if decision_contract is not None:
            from decision_observations import CONTRACT as DECISIONS
            require(decision_contract==DECISIONS and source_contract is not None,'decision source contract requires signed source')
        if source_contract is not None:
            from source_feed_codec import READ_COMPATIBLE_CONTRACTS
            require(source_contract in READ_COMPATIBLE_CONTRACTS and pool_pubkey is not None, "signed source contract/key binding")
            # SPCB/3 economics are unchanged in source-r3; its transport/seed
            # authority is separately pinned and not inferred from this alias.
            from ruleb_source import API_SHA256
            working_ruleb_api_sha256 = API_SHA256
        self.working_ruleb_api_sha256 = working_ruleb_api_sha256
        if working_ruleb_api_sha256 is not None:
            from ruleb_source import working_pin
            working_pin(working_ruleb_api_sha256)
        require(hashlib.sha256(raw_log).hexdigest() == independently_captured_log_sha256, "source log differs from external collector seal")
        if not raw_log.endswith(b"\n"):
            raise Inconclusive("source log ends with an incomplete record")
        self.log_sha256 = independently_captured_log_sha256
        self.events, self.heads, self.jobs, self.issued, self.accounts = [], {}, {}, {}, {}
        self.slice_publications, self.block_publications = {}, {}
        for line_number, line in enumerate(raw_log.decode().splitlines(), 1):
            text = ANSI.sub("", line)
            if "spc_evidence" not in text:
                continue
            match = re.search(r'event="([a-z_]+)"\s+evidence=', text)
            if not match:
                raise Inconclusive("source trace line shape unsupported")
            kind = match.group(1)
            allowed = KINDS | ({"provisional_candidate"} if working_ruleb_api_sha256 is not None else set())
            if decision_contract is not None:allowed|={'derivation_prepared','derivation_observed'}
            if kind not in allowed:
                raise Inconclusive("unrecognized source producer event: " + kind)
            payload = strict_json(text[match.end():])
            source = payload.pop("source", None)
            envelope = SOURCE_FIELDS | ({"pool_seal"} if source_contract is not None else set())
            if not isinstance(source, dict) or set(source) != envelope:
                raise Inconclusive("source envelope fields missing/unknown")
            require(source["schema"] == "spc.source-evidence/2" and source["complete_prefix_required"] is True, "source version/completeness contract")
            session, sequence = source["session"], decimal_uint(source["seq"])
            last, previous = self.heads.get(session, (0, "00" * 32))
            if sequence != last + 1 or source["previous"] != previous:
                raise Inconclusive("source session prefix/sequence/hash-link gap")
            require(trace_digest(kind, payload, source) == source["digest"], "source event hash mismatch")
            if source_contract is not None:
                from source_feed_codec import trace_digest as seal_digest, verify_signature, array
                seal=source["pool_seal"]
                require(array(seal["session"],16).hex()==session and seal["seq"]==sequence
                        and array(seal["digest"],32).hex()==source["digest"], "pool seal differs from captured source head")
                verify_signature(seal_digest(seal),seal["signature"],pool_pubkey)
            if kind == "economic_change_observed":
                # front.rs EconomicChanged is only a coherent-reload hint. Its
                # fields never initialize or change economic/pending balances.
                require(set(payload)=={"economic_revision","anchor_hash","coherent_snapshot_reload_required"},"economic reload hint fields")
                decimal_uint(payload["economic_revision"],64)
                unhex(payload["anchor_hash"],32)
                require(payload["coherent_snapshot_reload_required"] is True,"economic reload hint must require coherent snapshot")
            if kind in {'derivation_prepared','derivation_observed'}:
                from decision_observations import front_event
                front_event(kind,payload,session)
            self.heads[session] = sequence, source["digest"]
            event = {"kind": kind, "payload": payload, "source": source, "line": line_number}
            self.events.append(event)
            if kind == "distribution_frozen":
                key = self.job_key(event)
                if working_ruleb_api_sha256 is not None and payload.get("economic") is not None:
                    from ruleb_source import frozen_job
                    frozen_job(payload["economic"], height=payload["height"], wire_job_id=int(payload["job_id"], 16),
                               generation=key[3], parent_hash=payload["prev_hash"],
                               slice_ids=[decimal_uint(row["slice_id"]) for row in payload["slices"]])
                if source_contract is not None:
                    if "finding_slice_id" not in payload or payload.get("finding_slice_excluded") is not True:
                        raise Inconclusive("actual frozen finding-slice exclusion missing")
                    finding=decimal_uint(payload["finding_slice_id"])
                    require(finding >> 56 == key[1] and finding not in [decimal_uint(r["slice_id"]) for r in payload["slices"]],
                            "finding slice/front or exclusion mismatch")
                require(key not in self.jobs, "duplicate frozen source generation")
                self.jobs[key] = event
            elif kind == "job_issued":
                self.issued.setdefault(self.job_key(event), []).append(event)
            elif kind == "accounted":
                sid = decimal_uint(payload["slice_id"])
                self.accounts.setdefault(sid, []).append(event)
            elif kind == "source_publication":
                self._publication(event)
        if not self.events:
            raise Inconclusive("no actual source trace retained")

    @staticmethod
    def job_key(event):
        p = event["payload"]
        return event["source"]["session"], integer(p["front_id"], 0, 127), p["job_id"], decimal_uint(p["generation"])

    def _publication(self, event):
        p = event["payload"]
        require(type(p["publication_complete"]) is bool, "publication completeness type")
        if not p["publication_complete"]:
            return
        require(isinstance(p["stream_id"], str) and re.fullmatch(r"[0-9]+-[0-9]+", p["stream_id"]), "actual Redis stream id missing")
        raw = unhex(p["raw_d_hex"])
        if p["stream"] == "spc:slices":
            decoded = decode_slice(raw)
            require(str(decoded["slice_id"]) == p["slice_id"], "slice publication id/bytes mismatch")
            self.slice_publications.setdefault(decoded["slice_id"], []).append((event, decoded))
        elif p["stream"] == "spc:blocks":
            if self.working_ruleb_api_sha256 is None:
                decoded = decode_spcb2(raw)
            else:
                from ladder_wire import decode_spcb_working
                decoded = decode_spcb_working(raw, api_sha256=self.working_ruleb_api_sha256)
            require(decoded["block_hash"] == unhex(p["block_hash"], 32)[::-1].hex()
                    and decoded["coinbase_fingerprint"] == p["coinbase_fingerprint"] and decoded["snapshot_key"] == p["sql_snapshot_key"],
                    "SPCB source publication/snapshot identity mismatch")
            self.block_publications.setdefault(decoded["block_hash"], []).append((event, decoded))
        else:
            raise Inconclusive("unknown source publication stream")

    def normalized_job(self, key, prevout_transactions):
        if key not in self.jobs or key not in self.issued:
            raise Inconclusive("actual frozen/issued job join missing")
        frozen = self.jobs[key]
        p = frozen["payload"]
        for event in self.issued[key]:
            require(decimal_uint(event["source"]["seq"]) > decimal_uint(frozen["source"]["seq"]), "job issuance precedes frozen generation")
            issued = event["payload"]
            for name in ("height", "version", "ntime", "prev_hash", "nbits", "coinbase_prefix", "coinbase_suffix",
                         "merkle_branch", "share_target", "m_issue", "work_unit_version", "work_unit_fingerprint"):
                require(issued[name] == p[name], "actual issued job differs from frozen generation: " + name)
        require(p["share_target_byte_order"] == "big-endian unsigned 256-bit", "issued target byte order")
        txs = []
        for raw in p["transactions"]:
            tx = decode_transaction(unhex(raw))
            needed = {}
            for inp in tx["inputs"]:
                parent = inp["outpoint"].split(":")[0]
                if parent not in prevout_transactions:
                    raise Inconclusive("actual job fee prevout unavailable")
                needed[parent] = prevout_transactions[parent]
            txs.append({"raw_hex": raw, "prevout_transactions": needed})
        return {"job_id": p["job_id"], "generation": key[3], "coinbase_prefix": p["coinbase_prefix"],
                "coinbase_suffix": p["coinbase_suffix"], "merkle_path": p["merkle_branch"], "prev_hash_le": p["prev_hash"],
                "nbits": p["nbits"], "share_target_be": p["share_target"], "m_issue": p["m_issue"], "transactions": txs}

    def verified_window(self, frozen_key, *, reference_target, window_n, prevout_transactions, eligible_history=None):
        if frozen_key not in self.jobs:
            raise Inconclusive("frozen window generation absent")
        event = self.jobs[frozen_key]
        p, source = event["payload"], event["source"]
        if eligible_history is not None:
            from audited_window import EligibleHistory
            require(isinstance(eligible_history,EligibleHistory) and eligible_history.job_key==frozen_key
                    and eligible_history.source_log_sha256==self.log_sha256
                    and eligible_history.selected_ids==tuple(decimal_uint(r["slice_id"]) for r in p["slices"]),
                    "eligible-history certificate differs from captured job")
        if any(e["kind"] == "proof_unavailable" and e["source"]["session"] == source["session"]
               and decimal_uint(e["source"]["seq"]) <= decimal_uint(source["seq"]) for e in self.events):
            raise Inconclusive("source prefix explicitly reports unavailable proof")
        from oracle import target
        q = (window_n * (reference_target + 1) + target(p["nbits"])) // (target(p["nbits"]) + 1)
        require(decimal_uint(p["window_target"], 128) == q and p["window_rule"] == 1 and p["work_unit_version"] == 2,
                "frozen source work/Q rule mismatch")
        if eligible_history is None and (p["window_underfilled"] or not p["slices"]):
            raise Inconclusive("frozen source window is underfilled")
        selected, marks, seen, global_shares, mappings, total = [], {}, set(), set(), [], 0
        for position, row in enumerate(p["slices"]):
            sid = decimal_uint(row["slice_id"])
            front = integer(row["front_id"], 0, 127)
            require(sid >> 56 == front and sid not in seen, "frozen slice identity/front mismatch")
            seen.add(sid)
            if sid not in self.slice_publications:
                raise Inconclusive("selected source slice publication missing")
            publications = self.slice_publications[sid]
            require(all(item[0]["payload"]["raw_d_hex"] == publications[0][0]["payload"]["raw_d_hex"] for item in publications), "same slice id republished with different bytes")
            publication, decoded = publications[0]
            require(decoded["root"] == row["root"] and decoded["height"] == row["height"]
                    and decoded["difficulty"] == decimal_uint(row["difficulty"], 128)
                    and decoded["work_unit_fingerprint"] == p["work_unit_fingerprint"], "frozen slice differs from actual publication")
            credits = {c["sp_id"]: (decimal_uint(c["diff_sum"], 128), decimal_uint(c["diff_fee_sum"], 128)) for c in row["credits"]}
            require(len(credits) == len(row["credits"]) and credits == decoded["credits"], "frozen/source credits mismatch")
            accounts = self.accounts.get(sid, [])
            require(len(accounts) == len(decoded["shares"]), "source slice/accounted share census mismatch")
            verified, sums = [], {}
            for leaf in decoded["shares"]:
                key = leaf_key(leaf)
                matches = []
                for item in accounts:
                    a = item["payload"]
                    job_key = self.job_key(item)
                    if job_key not in self.jobs:
                        raise Inconclusive("accounted share missing actual frozen job")
                    packed_id = (self.jobs[job_key]["payload"]["height"] << 32) | int(a["job_id"], 16)
                    candidate = (a["nonce"], a["ntime"], a["version"], unhex(a["extranonce"]), packed_id, a["share_index"])
                    if candidate == key:
                        matches.append(item)
                require(len(matches) == 1, "share leaf lacks unique actual ingress attribution")
                account = matches[0]; a = account["payload"]; job_key = self.job_key(account)
                require(account["source"]["session"] == source["session"]
                        and decimal_uint(account["source"]["seq"]) < decimal_uint(source["seq"]), "post-freeze share included in selected window")
                require(a["share_hash"] not in global_shares, "accepted share counted twice")
                global_shares.add(a["share_hash"])
                job = self.normalized_job(job_key, prevout_transactions)
                require(any(i["payload"]["channel_id"] == a["channel_id"]
                            and decimal_uint(i["source"]["seq"]) < decimal_uint(account["source"]["seq"]) for i in self.issued[job_key]),
                        "share has no prior actual channel job issuance")
                share = {"identity": a["sp_id"], "version": a["version"], "ntime": a["ntime"], "nonce": a["nonce"],
                         "extranonce": a["extranonce"], "share_hash_le": a["share_hash"], "generation": job_key[3],
                         "job_id": a["job_id"], "credited_work": a["d"]}
                proof = share_proof(share, job, reference_target)
                require(proof["fees"] == decimal_uint(a["credited_job_fees"]) and proof["work"] == a["m_issue"], "actual attributed fee/work claim mismatch")
                reconstructed = unhex(job["coinbase_prefix"]) + unhex(a["extranonce"]) + unhex(job["coinbase_suffix"])
                require(reconstructed == unhex(a["coinbase"]), "accounted raw coinbase differs from issued bytes")
                values = sums.setdefault(a["sp_id"], [0, 0]); values[0] += proof["work"]; values[1] += proof["work"] * proof["fees"]
                verified.append(proof)
            require({k: tuple(v) for k, v in sums.items()} == credits, "independent attributed shares differ from frozen credits")
            work = sum(s["work"] for s in verified)
            require(work == decoded["difficulty"], "independent selected work mismatch")
            require(total < q, "frozen selected window extends past Q crossing")
            total += work
            selected.append(verified)
            marks.setdefault(str(front), sid)
            mappings.append({"slice_id": sid, "redis_stream_id": publication["payload"]["stream_id"],
                             "publication_source_seq": decimal_uint(publication["source"]["seq"]), "selected_position": position})
        if eligible_history is None:require(total >= q, "frozen window does not reach Q")
        else:require(type(p["window_underfilled"]) is bool and p["window_underfilled"]==(total<q),"frozen underfill flag differs from actual selected sum")
        declared = {key: decimal_uint(value) for key, value in p["high_water"].items() if value != "0"}
        require(marks == declared, "complete selected high-water vector mismatch")
        # This r1 producer is single-front; a future multi-front contract needs
        # explicit local-RAM versus consumed-stream visibility ordering.
        if len(marks)>1 or (eligible_history is None and len(marks)!=1):
            raise Inconclusive("r1 multi-front visibility/order proof not provided")
        ids = [m["slice_id"] for m in mappings]
        require(ids == sorted(ids, reverse=True), "frozen single-front order changed")
        result={"selected": selected, "window_target": q, "window_work": total, "mappings": mappings,
                "source_watermark": decimal_uint(source["seq"]), "source_session": source["session"]}
        if eligible_history is not None:result["eligible_history"]=eligible_history
        return result
