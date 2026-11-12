"""History-3 observer: opens O_RDONLY, never repairs, locks or appends.

Production records are claims. Funding, confirmation, fees, rollback and
send eligibility are checked against independently captured raw chain views.
Unknown evidence/schema/tails yield INCONCLUSIVE rather than guessed state.
"""
import hashlib
import os
import stat

from oracle import (Inconclusive, Invalid, decode_block, decode_transaction,
                    expected_bits, integer, require, rollback_proof, sha256d,
                    strict_json, target, unhex)


FIELDS = {
    "header": "format_magic history_version protocol network key_mode derivation_version run_id driver_id run_seed_fingerprint allocation_digest caps created_unix_ms",
    "session_started": "restarted_unix_ms",
    "funding_observed": "outpoint amount_sats script_hex chain index owner_role source context observed_unix_ms",
    "command_begin": "command_id request",
    "command_conflict": "command_id existing_digest incoming_digest",
    "reserved": "command_id outpoints change_index",
    "signed_tx": "command_id txid raw_hex vbytes weight_wu fee_sats fee_rate_sats_vb category payer_role spent_outpoints destinations change size_class",
    "command_finished": "command_id outcome txid",
    "inputs_released": "command_id reason",
    "broadcast_sent": "command_id txid send_sequence",
    "accepted_by_rpc": "command_id txid node_id",
    "observed_in_mempool": "command_id txid node_id",
    "confirmed": "command_id txid block_hash height",
    "rejected": "command_id txid code",
    "unknown_outcome": "command_id txid reason attempt",
    "conflicted": "command_id txid reason",
    "reorged": "command_id txid from_block_hash height",
    "reorg_resubmit": "command_id txid proven_unspent send_sequence",
    "chain_observed": "tip_block_hash tip_height displaced_block_hashes observed_unix_ms",
    "chain_rollback": "evidence observed_unix_ms",
    "send_barrier": "command_id wall_unix_ms monotonic_ms last_txid send_sequence",
    "send_resumed": "command_id",
}


def read_records(path, max_bytes=128 * 1024 * 1024):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        before = os.fstat(fd)
        require(stat.S_ISREG(before.st_mode), "journal is not regular")
        require(before.st_uid == os.getuid() and stat.S_IMODE(before.st_mode) == 0o600,
                "journal private ownership/mode")
        if before.st_size > max_bytes:
            raise Inconclusive("journal resource bound")
        with os.fdopen(fd, "rb", closefd=False) as stream:
            data = stream.read(max_bytes + 1)
        after = os.fstat(fd)
        if (before.st_size, before.st_mtime_ns) != (after.st_size, after.st_mtime_ns):
            raise Inconclusive("journal changed during read; retry a quiescent snapshot")
    finally:
        os.close(fd)
    return parse_records(data), hashlib.sha256(data).hexdigest()


def parse_records(data):
    if not data or not data.endswith(b"\n"):
        raise Inconclusive("missing/truncated journal tail; read-only observer cannot repair")
    result = []
    for seq, line in enumerate(data.splitlines()):
        if len(line) > 2 * 1024 * 1024:
            raise Inconclusive("journal record exceeds proof bound")
        try:
            row = strict_json(line)
        except (ValueError, Invalid) as error:
            raise Inconclusive("malformed journal record") from error
        if not isinstance(row, dict) or row.get("record") not in FIELDS:
            raise Inconclusive("unknown journal record")
        if set(row) != set(FIELDS[row["record"]].split()) | {"record", "seq"}:
            raise Inconclusive("unknown/missing history-3 fields")
        if type(row["seq"]) is not int or row["seq"] != seq:
            raise Inconclusive("journal sequence gap")
        result.append(row)
    return result


class Journal:
    def __init__(self, records, binding, views, outcome_proofs=None):
        self.records, self.views = records, views
        self.funding, self.reserved, self.commands, self.txs = {}, {}, {}, {}
        self.chain = None
        self.paused, self.send_sequence, self.last_txid = False, 0, None
        self.sends, self.barriers, self.reorgs = [], [], []
        self.gaps = []
        self.outcome_proofs = outcome_proofs
        require(records[0]["record"] == "header", "journal header first")
        h = records[0]
        require(h["history_version"] == 3 and h["protocol"] == "spc-lab-tx-journal/3"
                and h["format_magic"] == "SPC-LAB-TRAFFIC-JOURNAL"
                and h["network"] == "regtest" and h["derivation_version"] == 2
                and h["key_mode"] == "lab_deterministic_run_seed_v1",
                "history binding")
        for key, value in binding.items():
            require(h[key] == value, "journal run/allocation binding")
        self.caps = h["caps"]
        require(set(self.caps) == {"unknown_retry_sends", "reorg_resubmits"}, "retry cap fields")
        integer(self.caps["unknown_retry_sends"], 0, 3)
        integer(self.caps["reorg_resubmits"], 1, 3)
        unhex(h["allocation_digest"], 32)
        for row in records[1:]:
            self.apply(row)

    def view(self, tip):
        if tip not in self.views:
            raise Inconclusive("missing independently captured journal anchor " + tip)
        return self.views[tip]

    def apply(self, row):
        kind = row["record"]
        cid = row.get("command_id")
        if kind == "header":
            raise Invalid("second journal header")
        if kind == "chain_observed":
            new = self.view(row["tip_block_hash"])
            require(new.tip["height"] == row["tip_height"], "chain height claim")
            if self.chain:
                require(new.tip["height"] >= self.chain.tip["height"], "lower tip without rollback proof")
                displaced = {b["hash"] for b in self.chain.blocks} - set(new.by_hash)
                require(set(row["displaced_block_hashes"]) == displaced, "chain displacement claim")
            self.chain = new
            self.cascade()
        elif kind == "chain_rollback":
            require(self.chain is not None, "rollback without prior chain")
            new = self.view(row["evidence"]["new_anchor"]["block_hash"])
            rollback_proof(row["evidence"], self.chain, new)
            self.chain = new
            self.cascade()
        elif kind == "funding_observed":
            point = row["outpoint"]
            funding_view = self.chain
            if isinstance(row["context"], dict) and "canonical_block" in row["context"]:
                block_hash = row["context"]["canonical_block"]["block_hash"]
                # Funding proof can be recorded before the next committed tip
                # observation. It does not advance maturity/spend authority.
                funding_view = next((v for v in self.views.values() if block_hash in v.by_hash), None)
            if funding_view is None or point not in funding_view.outputs:
                raise Inconclusive("funding raw outpoint unavailable at observed anchor")
            actual = funding_view.outputs[point]
            require(actual["value"] == row["amount_sats"] and actual["script"] == unhex(row["script_hex"]),
                    "funding amount/script mismatch")
            require(row["source"] in {"bootstrap_coinbase", "canonical_block", "own_transaction"},
                    "synthetic funding cannot satisfy acceptance")
            if row["source"] == "own_transaction":
                creator = point.split(":")[0]
                require(creator in self.txs and self.txs[creator]["state"] == "confirmed",
                        "own funding without journaled canonical creator")
                require(row["context"] == "own_transaction", "own funding context")
            else:
                require(row["context"] == {"canonical_block": {
                    "block_hash": actual["block_hash"], "height": actual["height"],
                    "coinbase": actual["coinbase"]}}, "funding canonical context")
                if row["source"] == "bootstrap_coinbase":
                    require(actual["coinbase"], "bootstrap funding not coinbase")
            if point in self.funding:
                old = self.funding[point]
                require(all(old[k] == row[k] for k in ("amount_sats", "script_hex", "chain", "index", "owner_role", "source")),
                        "funding identity changed")
            self.funding[point] = row
        elif kind == "command_begin":
            require(cid not in self.commands, "duplicate command begin")
            request = row["request"]
            required = {"payer_role", "category", "destinations", "fee_rate_sats_vb"}
            require(isinstance(request, dict) and set(request) in (required, required | {"construction_effect"}),
                    "unknown/missing journal request fields")
            if "construction_effect" in request:
                require(request["construction_effect"] == {"schema": "spc-lab-version-rejection/v1"},
                        "unknown construction effect; no arbitrary transaction version")
            self.commands[cid] = {"request": row["request"], "points": None, "txid": None}
        elif kind == "reserved":
            require(cid in self.commands and self.commands[cid]["points"] is None, "reservation command")
            require(row["outpoints"] and len(set(row["outpoints"])) == len(row["outpoints"]), "reservation set")
            for point in row["outpoints"]:
                require(point in self.funding and point not in self.reserved, "double/unknown reservation")
                self.eligible(point)
                self.reserved[point] = cid
            self.commands[cid]["points"] = set(row["outpoints"])
        elif kind == "signed_tx":
            require(cid in self.commands, "signed without command")
            command = self.commands[cid]
            tx = decode_transaction(unhex(row["raw_hex"]))
            require(not tx["coinbase"] and not tx["witness"], "production T1 signer is legacy only")
            version = int.from_bytes(tx["stripped"][:4], "little", signed=True)
            selected = "construction_effect" in command["request"]
            require(version == (3 if selected else 2), "raw version differs from immutable before-sign construction effect")
            require(command["txid"] is None and tx["txid"] not in self.txs, "duplicate signed payment")
            require(tx["txid"] == row["txid"], "signed txid")
            require({i["outpoint"] for i in tx["inputs"]} == set(row["spent_outpoints"]) == command["points"],
                    "signed input/reservation set")
            require(tx["weight"] == row["weight_wu"] and tx["vbytes"] == row["vbytes"], "signed size")
            fee = sum(self.funding[i["outpoint"]]["amount_sats"] for i in tx["inputs"]) - sum(o["value"] for o in tx["outputs"])
            require(0 < fee == row["fee_sats"], "signed exact fee")
            request = command["request"]
            require(request["destinations"] == row["destinations"] and request["payer_role"] == row["payer_role"]
                    and request["category"] == row["category"] and request["fee_rate_sats_vb"] == row["fee_rate_sats_vb"],
                    "signed request binding")
            expected = [(d["amount_sats"], unhex(d["script_hex"])) for d in row["destinations"]]
            if row["change"] is not None:
                expected.append((row["change"]["amount_sats"], unhex(row["change"]["script_hex"])))
            require([(o["value"], o["script"]) for o in tx["outputs"]] == expected, "destination/change outputs")
            require(fee >= tx["vbytes"] * row["fee_rate_sats_vb"], "signed fee floor")
            command["txid"] = tx["txid"]
            self.txs[tx["txid"]] = {"tx": tx, "row": row, "state": "signed", "ever_uncertain": False,
                                     "retries": 0, "resubmits": 0, "unknowns": 0}
        elif kind in {"broadcast_sent", "reorg_resubmit"}:
            tx = self.get_tx(row)
            require(not self.paused, "send inside committed pause interval")
            require(row["send_sequence"] == self.send_sequence + 1, "send sequence must start at one and be contiguous")
            for inp in tx["tx"]["inputs"]:
                self.eligible(inp["outpoint"])
            if kind == "reorg_resubmit":
                require(tx["state"] == "reorged", "resubmit without displacement")
                require(set(row["proven_unspent"]) == {i["outpoint"] for i in tx["tx"]["inputs"]}, "resubmit proof coverage")
                tx["resubmits"] += 1
                require(tx["resubmits"] <= self.caps["reorg_resubmits"], "reorg retry cap")
                tx["ever_uncertain"] = True
            else:
                require(tx["state"] in {"signed", "unknown"}, "illegal normal send transition")
                if tx["state"] == "unknown":
                    tx["retries"] += 1
                    require(tx["retries"] <= self.caps["unknown_retry_sends"], "unknown retry cap")
            tx["state"] = "sent"
            self.send_sequence = row["send_sequence"]
            self.last_txid = row["txid"]
            self.sends.append(row)
        elif kind == "send_barrier":
            require(not self.paused and row["send_sequence"] == self.send_sequence
                    and row["last_txid"] == self.last_txid, "pause last-used boundary")
            require(all(t["state"] != "sent" for t in self.txs.values()), "pause before local send classification")
            self.paused = True
            self.barriers.append({"start": row["seq"], "end": None, "command_id": cid})
        elif kind == "send_resumed":
            require(self.paused, "resume without barrier")
            self.paused = False
            self.barriers[-1]["end"] = row["seq"]
        elif kind == "session_started":
            for tx in self.txs.values():
                if tx["state"] == "sent":
                    tx["state"], tx["ever_uncertain"] = "unknown", True
                    tx["unknowns"] += 1
        elif kind in {"accepted_by_rpc", "observed_in_mempool", "confirmed", "rejected", "unknown_outcome", "conflicted", "reorged"}:
            tx = self.get_tx(row)
            rejection_proof = None
            if kind == "accepted_by_rpc" and self.outcome_proofs is not None:
                from send_proofs import SendOutcomeProofs
                if not isinstance(self.outcome_proofs, SendOutcomeProofs):
                    raise Inconclusive("RPC acceptance requires verified cross-stream outcome proof")
                self.outcome_proofs.acceptance(self.records[0], row, self.records)
            if kind == "rejected":
                from send_proofs import SendOutcomeProofs
                if not isinstance(self.outcome_proofs, SendOutcomeProofs):
                    raise Inconclusive("standalone journal cannot prove a node NoEffect rejection")
                rejection_proof = self.outcome_proofs.rejection(self.records[0], row, self.records)
                tx["ever_uncertain"] = tx["ever_uncertain"] or rejection_proof["retention_required"]
            allowed = {
                "accepted_by_rpc": {"sent", "unknown"},
                "observed_in_mempool": {"accepted", "mempool"},
                "confirmed": {"accepted", "mempool", "unknown", "confirmed", "reorged"},
                "rejected": {"sent", "accepted", "unknown"},
                "unknown_outcome": {"sent", "accepted", "mempool", "unknown"},
                "conflicted": {"mempool", "unknown", "reorged"},
                "reorged": {"confirmed", "reorged"},
            }
            require(tx["state"] in allowed[kind], "illegal transaction outcome transition")
            if kind == "confirmed":
                require(self.chain is not None, "confirmation without chain anchor")
                found = self.chain.transactions.get(row["txid"])
                require(found is not None and found[0]["hash"] == row["block_hash"]
                        and found[0]["height"] == row["height"] and found[1]["raw"] == tx["tx"]["raw"],
                        "confirmation requires actual canonical bytes")
                tx["state"] = "confirmed"
                tx["confirmed_block"] = row["block_hash"]
            elif kind == "reorged":
                require(tx.get("confirmed_block") == row["from_block_hash"]
                        and row["from_block_hash"] not in self.chain.by_hash, "displacement evidence")
                tx["state"], tx["ever_uncertain"] = "reorged", True
                self.reorgs.append(row)
            elif kind == "unknown_outcome":
                tx["unknowns"] += 1
                require(row["attempt"] == tx["unknowns"], "unknown attempt counter")
                tx["state"], tx["ever_uncertain"] = "unknown", True
            elif kind in {"accepted_by_rpc", "observed_in_mempool"}:
                tx["state"] = "accepted" if kind == "accepted_by_rpc" else "mempool"
                tx["ever_uncertain"] = True
            else:
                tx["state"] = "rejected" if kind == "rejected" else "conflicted"
                if kind == "rejected" and not tx["ever_uncertain"]:
                    # Release only after the cross-stream oracle proved the
                    # exact actual unsuppressed, reviewed NoEffect response.
                    for inp in tx["tx"]["inputs"]:
                        if self.reserved.get(inp["outpoint"]) == cid:
                            del self.reserved[inp["outpoint"]]
        elif kind == "inputs_released":
            require(cid in self.commands, "release without command")
            txid = self.commands[cid]["txid"]
            if txid:
                tx = self.txs[txid]
                require(not tx["ever_uncertain"] and tx["state"] == "rejected", "released possibly-spent inputs")
            for point in self.commands[cid]["points"] or []:
                require(self.reserved.get(point) == cid, "release owner")
                del self.reserved[point]
        elif kind in {"command_finished", "command_conflict"}:
            require(cid in self.commands, "command completion/conflict without begin")

    def get_tx(self, row):
        require(row["txid"] in self.txs, "unknown transaction")
        require(self.commands[row["command_id"]]["txid"] == row["txid"], "send command binding")
        return self.txs[row["txid"]]

    def cascade(self):
        """Derive displacement from raw-chain membership, never journal credit.

        A restored funding observation cannot itself confirm its spender.
        Previously confirmed spends return to the conservative reorged path.
        """
        for tx in self.txs.values():
            if tx.get("confirmed_block") not in self.chain.by_hash and tx["state"] == "confirmed":
                tx["state"], tx["ever_uncertain"] = "reorged", True
            missing = any(i["outpoint"] not in self.chain.outputs for i in tx["tx"]["inputs"])
            if missing and tx["state"] != "invalidated":
                tx["prior_state"] = tx["state"]
                tx["state"], tx["ever_uncertain"] = "invalidated", True
            elif not missing and tx["state"] == "invalidated":
                tx["state"] = tx.pop("prior_state")
                if tx["state"] == "confirmed":
                    tx["state"] = "reorged"

    def eligible(self, point):
        if self.chain is None:
            raise Inconclusive("no canonical anchor for spend")
        require(point in self.chain.utxos, "send of displaced/spent funding")
        prev = self.chain.utxos[point]
        # T1 conservatively requires observed tip >= coinbase height + 100.
        require(not prev["coinbase"] or self.chain.tip["height"] - prev["height"] >= self.chain.policy.coinbase_maturity,
                "retained coinbase immature after lower-tip rollback")

    def balances(self):
        if self.chain is None:
            raise Inconclusive("balance without anchor")
        result = {"canonical_unspent": 0, "reserved_unspent": 0, "canonical_spent": 0, "invalidated": 0}
        for point, row in self.funding.items():
            if point not in self.chain.outputs:
                key = "invalidated"
            elif point not in self.chain.utxos:
                key = "canonical_spent"
            elif point in self.reserved:
                key = "reserved_unspent"
            else:
                key = "canonical_unspent"
            result[key] += row["amount_sats"]
        return result


def hash_budget(records, total_cap, require_start=True):
    """Controller CommandLog reservations are authority; native log is not.

    This input is the *unfiltered* command log. Unknown budget record names
    reject; unrelated known command records may be passed through unchanged.
    """
    batches, cumulative, measured, operations = {}, 0, 0, {}
    previous_seq = 0
    for row in records:
        integer(row["seq"])
        if previous_seq is not None and row["seq"] != previous_seq + 1:
            raise Inconclusive("controller command-log sequence gap")
        previous_seq = row["seq"]
        kind = row.get("record", row.get("type"))
        if kind == "traffic_audit":
            if set(row) != {"record", "seq", "version", "boot_id", "monotonic_ms", "event"} or row["version"] != 1:
                raise Inconclusive("unknown traffic audit envelope in command budget log")
            # These are observations, never budget authority. The independent
            # traffic_audit reader separately verifies their complete stream.
            continue
        if kind == "hash_start":
            operation = row["operation"]
            require(operation["operation_id"] not in operations, "duplicate budget operation")
            require(operation["hashes_per_second"] == 40 and operation["total_attempts"] == total_cap,
                    "bound bootstrap rate/finite attempt cap")
            require(operation["boot_id"] and operation["deadline_monotonic_ms"] > 0
                    and operation["cancel_grace_secs"] > 0, "persistent bootstrap deadline")
            operations[operation["operation_id"]] = operation
            continue
        if kind not in {"hash_reserve", "hash_reconcile"}:
            if kind not in {"begin", "finish", "conflict"}:
                raise Inconclusive("unknown controller budget-log schema")
            continue
        key = (row["operation_id"], row["driver_incarnation"], row["batch_id"])
        if require_start and key[0] not in operations:
            raise Inconclusive("missing persistent bootstrap operation")
        if kind == "hash_reserve":
            require(key not in batches, "duplicate hash authorization")
            require(key[2] == 1 + sum(k[0] == key[0] for k in batches), "batch id reused/skipped across incarnations")
            require(not any(not b["reconciled"] and k[0] == key[0] for k, b in batches.items()), "multiple outstanding batches")
            count = integer(row["reserved_attempts"], 1)
            cumulative += count
            require(cumulative == row["cumulative_reserved"] and cumulative <= total_cap, "hash reservation cap/arithmetic")
            batches[key] = {"reserved": count, "reconciled": False}
        else:
            require(key in batches, "reconcile without reservation")
            batch = batches[key]
            count = row["measured_attempts"]
            if count is not None:
                integer(count, 0, batch["reserved"])
            if batch["reconciled"]:
                require(batch["measured"] == count, "non-idempotent reconcile")
            else:
                measured += count if count is not None else 0
                batch.update(reconciled=True, measured=count)
    if not batches:
        raise Inconclusive("no authoritative controller hash reservations")
    return {"charged_reserved": cumulative, "measured_executed": measured,
            "charged_unresolved": sum(b["reserved"] for b in batches.values()
                                      if not b["reconciled"] or b.get("measured") is None)}


def native_observations(records, command_records, views, policy, earliest_header_time):
    """Cross-check T2 diagnostics; never redeem permits or replay any money.

    Raw attempted headers include failed PoW. Only actual solutions submitted
    as full blocks must satisfy PoW; every attempt must use contextual bits.
    """
    reserves, reconciles, deadlines = {}, {}, {}
    for record in command_records:
        kind = record["record"]
        if kind == "hash_start":
            op = record["operation"]
            deadlines[op["operation_id"]] = op["deadline_monotonic_ms"]
        if kind in {"hash_reserve", "hash_reconcile"}:
            key = record["operation_id"], record["driver_incarnation"], record["batch_id"]
            (reserves if kind == "hash_reserve" else reconciles)[key] = record
    measured, seen, headers_seen, previous_time, submitted = 0, set(), set(), None, 0
    allowed = {"begin", "terminal", "distribution_plan", "distribution_result", "distribution_canonical", "template_refused",
               "template", "hash_batch", "submitblock", "submission_not_canonical", "coinbase_canonical"}
    for record in records:
        if record.get("event") not in allowed:
            raise Inconclusive("unknown native observation record")
        require(record["source"] == "bootstrap" and record["exclude_from_statistics"] is True, "native observation scope")
        when = integer(record["monotonic_ms"])
        require(previous_time is None or when >= previous_time, "native observation time regression")
        previous_time = when
        if record["event"] == "hash_batch":
            auth = record["authorization"]
            key = auth["operation_id"], auth["driver_incarnation"], auth["batch_id"]
            require(key in reserves and key not in seen, "native batch without unique controller reservation")
            reserve = {k: v for k, v in reserves[key].items() if k not in {"seq", "record"}}
            require(auth == reserve, "native diagnostic authorization differs from controller authority")
            require(key[0] in deadlines and when <= deadlines[key[0]], "native batch past persistent deadline")
            raws = record["raw_headers"]
            require(len(raws) == record["measured_attempts"] <= auth["reserved_attempts"], "native actual/charged batch counts")
            if key in reconciles and reconciles[key]["measured_attempts"] is not None:
                require(reconciles[key]["measured_attempts"] == len(raws), "native/header measured reconcile mismatch")
            for value in raws:
                raw = unhex(value, 80)
                require(raw not in headers_seen, "native header attempt reused")
                headers_seen.add(raw)
                parent_hash = raw[4:36][::-1].hex()
                ancestors = None
                for view in views.values():
                    if parent_hash in view.by_hash:
                        parent = view.by_hash[parent_hash]
                        ancestors = view.blocks[:parent["height"] + 1]
                        break
                if ancestors is None:
                    raise Inconclusive("native attempt raw parent ancestry unavailable")
                time = int.from_bytes(raw[68:72], "little")
                bits = int.from_bytes(raw[72:76], "little")
                require(time >= earliest_header_time, "fabricated historical native timestamp")
                require(len(ancestors) == record["height"] and bits == expected_bits(policy, ancestors, time), "native contextual target")
                require(target(bits) == int.from_bytes(unhex(record["target"], 32), "big"), "native separately encoded target")
            measured += len(raws)
            seen.add(key)
        elif record["event"] == "submitblock":
            block = decode_block(unhex(record["raw_block"]), policy)
            require(block["raw"] in headers_seen and block["hash"] == record["block_hash"], "native submitted header not accounted")
            submitted += 1
    if not seen:
        raise Inconclusive("no native header observations")
    if any(key not in seen and value["measured_attempts"] not in {None, 0}
           for key, value in reconciles.items()):
        raise Inconclusive("measured controller batch lacks raw-header diagnostics")
    return {"observation_only": True, "raw_attempt_headers": measured, "submitted_blocks": submitted,
            "authoritative_reservations_without_diagnostic": len(set(reserves) - seen)}
