"""Independent consumer of T1's actual traffic_audit/1 CommandLog schema.

Authority remains in the producer. This reader checks full sequence coverage,
joins effect hashes to complete history-3 signed bytes, and verifies closed
intervals, watermarks and real acceptance-hook outcomes. No fixture schema is
accepted as an alternate producer format.
"""
import hashlib
import json

from oracle import Inconclusive, Invalid, decode_transaction, integer, require, strict_json, unhex
from send_proofs import SendOutcomeProofs

EVENT_FIELDS = {
    "session": "binding",
    "forward_attempt": "attempt",
    "forward_result": "attempt_seq outcome response_sha256 response_suppressed",
    "watermark": "incarnation last_attempt_seq completed_count permit_revision closed command_id",
    "measured_boundary": "command_id tip_hash tip_height",
    "miner_attempt": "run_id miner_id incarnation request_id payload_sha256 controller_sha256 actor_sha256",
    "miner_result": "attempt_seq succeeded",
    "hook": "key detail",
}
HOOK_FIELDS = {"armed": "mode deadline_ms boot_id", "signed": "raw_sha256 journal_seq",
               "released": "raw_sha256", "cancelled": "", "response_discarded": "attempt_seq"}
FORWARD_FIELDS = "incarnation rpc_request_id node_id method send_sequence payload_sha256 context permit_revision"
CONTEXT_FIELDS = "command_id journal_seq chain_revision intent_seq raw_sha256"
SESSION_FIELDS = "run_id driver_id incarnation controller_sha256 actor_sha256 acceptance_enabled"
EFFECTS = {"sendrawtransaction", "submitblock"}
METHODS = EFFECTS | {"getrawmempool", "getrawtransaction", "getmempoolinfo", "getblock", "getbestblockhash",
                     "getblockhash", "getblockheader", "getblockchaininfo", "getblocktemplate"}
LOG_FIELDS = {
    "begin": "record seq command payload_hash",
    "finish": "record seq command_id outcome",
    "conflict": "record seq command_id existing_hash incoming_hash",
    "hash_start": "record seq operation",
    "hash_reserve": "record seq operation_id driver_incarnation batch_id reserved_attempts cumulative_reserved",
    "hash_reconcile": "record seq operation_id driver_incarnation batch_id measured_attempts",
}
DEFINITIVE_ROWS = (
    (-26, "version", "rpc_tx_version_nonstandard"),
    (-8, "Unspendable output exceeds maximum configured by user (maxburnamount)", "burn_exceeds_max"),
    (-25, "Fee exceeds maximum configured by user (e.g. -maxtxfee, maxfeerate)", "rpc_tx_fee_exceeds_max"),
)


def fields(value, expected, name):
    if not isinstance(value, dict) or set(value) != set(expected.split()):
        raise Inconclusive("unknown/missing " + name + " schema fields")


def boolean(value):
    require(type(value) is bool, "audit boolean type")
    return value


def digest(data):
    return hashlib.sha256(data).hexdigest()


def compact(value, sorted_keys=False):
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False, sort_keys=sorted_keys).encode()


def read_commands(raw):
    if not raw or not raw.endswith(b"\n"):
        raise Inconclusive("missing/truncated strict CommandLog")
    rows = []
    for seq, line in enumerate(raw.splitlines(), 1):
        if len(line) > 16 * 1024 * 1024:
            raise Inconclusive("CommandLog record bound")
        try:
            row = strict_json(line)
        except (ValueError, Invalid) as error:
            raise Inconclusive("malformed complete CommandLog record") from error
        if not isinstance(row, dict):
            raise Inconclusive("CommandLog record must be an object")
        if row.get("seq") != seq or type(row.get("seq")) is not int:
            raise Inconclusive("CommandLog sequence gap")
        if row.get("record") not in {"begin", "finish", "conflict", "hash_start", "hash_reserve", "hash_reconcile", "traffic_audit"}:
            raise Inconclusive("unknown CommandLog record")
        if row["record"] in LOG_FIELDS:
            actual = set(row)
            if row["record"] == "begin":
                actual -= {"hash_scheme", "principal"}
                if ("hash_scheme" in row) != ("principal" in row):
                    raise Inconclusive("incomplete expanded command binding")
            if actual != set(LOG_FIELDS[row["record"]].split()):
                raise Inconclusive("unknown/missing CommandLog variant fields")
            if row["record"] == "begin":
                fields(row["command"], "command_id run_id issued_at_wall actor kind", "durable command")
        if row["record"] == "traffic_audit":
            fields(row, "record seq version boot_id monotonic_ms event", "traffic_audit envelope")
            if type(row["version"]) is not int or row["version"] != 1 or not isinstance(row["boot_id"], str) or not row["boot_id"]:
                raise Inconclusive("unsupported traffic audit version/boot schema")
            integer(row["monotonic_ms"])
            event = row["event"]
            if not isinstance(event, dict):
                raise Inconclusive("audit event must be an object")
            if event.get("event") not in EVENT_FIELDS:
                raise Inconclusive("unknown traffic audit event")
            fields(event, "event " + EVENT_FIELDS[event["event"]], "traffic audit event")
        rows.append(row)
    return rows


class TrafficAudit:
    def __init__(self, rows, histories, *, run_id, controller_sha256, txd_sha256,
                 miner_sha256, node_ids, acceptance_enabled, rejection_bindings=None, expected_rejection_binding=None):
        self.rows, self.histories, self.run_id = rows, histories, run_id
        self.controller_sha256, self.txd_sha256, self.miner_sha256 = controller_sha256, txd_sha256, miner_sha256
        self.nodes, self.acceptance_enabled = set(node_ids), acceptance_enabled
        self.sessions, self.attempts, self.results, self.watermarks = {}, {}, {}, []
        self.miner_attempts, self.miner_results, self.hooks = {}, {}, {}
        self.boundary, self.boot_id = None, None
        self.send_authorizations, self.send_sequences = {}, {}
        self.rejection_bindings = rejection_bindings or {}
        self.expected_rejection_binding = expected_rejection_binding
        self._validate()
        self.outcome_proofs = self._reconcile_send_outcomes()

    def history(self, incarnation):
        driver = self.sessions[incarnation]["binding"]["driver_id"]
        if driver not in self.histories:
            raise Inconclusive("audit driver history missing")
        rows = self.histories[driver]
        require(rows and rows[0]["run_id"] == self.run_id and rows[0]["driver_id"] == driver,
                "audit/history run-driver binding")
        return rows

    def _validate(self):
        previous_time = None
        for row in self.rows:
            if row["record"] != "traffic_audit":
                continue
            if self.boot_id is None:
                self.boot_id = row["boot_id"]
            require(row["boot_id"] == self.boot_id, "audit boot id changed")
            require(previous_time is None or row["monotonic_ms"] >= previous_time, "audit clock regression")
            previous_time = row["monotonic_ms"]
            event, seq = row["event"], row["seq"]
            kind = event["event"]
            if kind == "session":
                binding = event["binding"]
                fields(binding, SESSION_FIELDS, "audit session")
                require(binding["run_id"] == self.run_id and binding["controller_sha256"] == self.controller_sha256
                        and binding["actor_sha256"] == self.txd_sha256
                        and boolean(binding["acceptance_enabled"]) == self.acceptance_enabled, "audit source/session binding")
                incarnation = binding["incarnation"]
                require(incarnation and incarnation not in self.sessions, "audit duplicate incarnation")
                self.sessions[incarnation] = {"binding": binding, "request": 0, "attempts": [], "completed": set(), "revision": 0, "closed": False}
                self.history(incarnation)
            elif kind == "forward_attempt":
                attempt = event["attempt"]
                fields(attempt, FORWARD_FIELDS, "forward attempt")
                fields(attempt["context"], CONTEXT_FIELDS, "forward context")
                integer(attempt["rpc_request_id"], 1)
                integer(attempt["permit_revision"])
                require(attempt["method"] in METHODS, "unknown/forbidden audited RPC method")
                if attempt["send_sequence"] is not None:
                    integer(attempt["send_sequence"], 1)
                if attempt["context"]["chain_revision"] is not None:
                    integer(attempt["context"]["chain_revision"])
                require(self.boundary is None or attempt["method"] not in {"getblocktemplate", "submitblock"},
                        "native bootstrap RPC after measured boundary")
                incarnation = attempt["incarnation"]
                if incarnation not in self.sessions:
                    raise Inconclusive("forward attempt has no session")
                stream = self.sessions[incarnation]
                require(attempt["node_id"] in self.nodes and attempt["rpc_request_id"] > stream["request"]
                        and attempt["permit_revision"] == stream["revision"], "forward request/permit correlation")
                unhex(attempt["payload_sha256"], 32)
                if attempt["method"] in EFFECTS:
                    require(not stream["closed"], "effect forwarded while gate closed")
                if attempt["method"] == "sendrawtransaction":
                    self._send_preimage(attempt)
                    driver = stream["binding"]["driver_id"]
                    authorization = self.run_id, driver, attempt["context"]["intent_seq"]
                    send_number = self.run_id, driver, attempt["send_sequence"]
                    # Incarnation is deliberately NOT part of either key: a
                    # reopened actor cannot redeem the old journal intent.
                    require(authorization not in self.send_authorizations and send_number not in self.send_sequences,
                            "durable send authorization forwarded more than once")
                    self.send_authorizations[authorization] = seq
                    self.send_sequences[send_number] = seq
                stream["request"] = attempt["rpc_request_id"]
                stream["attempts"].append(seq)
                self.attempts[seq] = row
            elif kind == "forward_result":
                target = event["attempt_seq"]
                if target not in self.attempts:
                    raise Inconclusive("orphan forward result")
                require(target not in self.results and target < seq, "duplicate/out-of-order forward result")
                require(event["outcome"] in {"ok", "node_rejected", "unknown", "gate_closed"}, "forward outcome class")
                unhex(event["response_sha256"], 32)
                boolean(event["response_suppressed"])
                self.results[target] = row
                incarnation = self.attempts[target]["event"]["attempt"]["incarnation"]
                self.sessions[incarnation]["completed"].add(target)
            elif kind == "watermark":
                integer(event["last_attempt_seq"])
                integer(event["completed_count"])
                integer(event["permit_revision"], 1)
                incarnation = event["incarnation"]
                if incarnation not in self.sessions:
                    raise Inconclusive("watermark has no session")
                stream = self.sessions[incarnation]
                if set(stream["attempts"]) != stream["completed"]:
                    raise Inconclusive("watermark crosses unresolved attempt")
                require(event["completed_count"] == len(stream["completed"])
                        and event["last_attempt_seq"] == (stream["attempts"][-1] if stream["attempts"] else 0)
                        and event["permit_revision"] == stream["revision"] + 1, "watermark counters/revision")
                stream["revision"], stream["closed"] = event["permit_revision"], boolean(event["closed"])
                self.watermarks.append(row)
            elif kind == "measured_boundary":
                require(self.boundary is None, "duplicate measured boundary")
                unhex(event["tip_hash"], 32); integer(event["tip_height"])
                self.boundary = row
            elif kind == "miner_attempt":
                require(self.boundary is not None and self.boundary["seq"] < seq, "AddConnection precedes measured boundary")
                require(event["run_id"] == self.run_id and event["controller_sha256"] == self.controller_sha256
                        and event["actor_sha256"] == self.miner_sha256, "miner actual source identity")
                unhex(event["payload_sha256"], 32)
                require(event["request_id"] and event["incarnation"], "miner wire identity missing")
                self.miner_attempts[seq] = row
            elif kind == "miner_result":
                target = event["attempt_seq"]
                if target not in self.miner_attempts:
                    raise Inconclusive("orphan miner result")
                require(target not in self.miner_results and target < seq, "duplicate/out-of-order miner result")
                boolean(event["succeeded"])
                self.miner_results[target] = row
            elif kind == "hook":
                self._hook(row)
        if not self.sessions:
            raise Inconclusive("no source-bound traffic audit sessions")
        if set(self.results) != set(self.attempts) or set(self.miner_results) != set(self.miner_attempts):
            raise Inconclusive("audit snapshot ends with unresolved forward/miner attempts")
        for incarnation, stream in self.sessions.items():
            watermarks = [w for w in self.watermarks if w["event"]["incarnation"] == incarnation]
            if not watermarks or (stream["attempts"] and watermarks[-1]["seq"] < self.results[stream["attempts"][-1]]["seq"]):
                raise Inconclusive("audit stream lacks a final complete watermark")
        for (driver, command), hook in self.hooks.items():
            events = hook["events"]
            signed = [r for r in events if r["event"]["detail"]["transition"] == "signed"]
            released = [r for r in events if r["event"]["detail"]["transition"] == "released"]
            cancelled = [r for r in events if r["event"]["detail"]["transition"] == "cancelled"]
            hold = hook["arm"]["event"]["detail"]["mode"] == "hold_before_intent"
            for attempt in self.attempts.values():
                a = attempt["event"]["attempt"]
                if a["method"] != "sendrawtransaction" or a["context"]["command_id"] != command:
                    continue
                if self.sessions[a["incarnation"]]["binding"]["driver_id"] != driver:
                    continue
                require(a["incarnation"] == hook["key"]["incarnation"], "hook target forwarded by a new incarnation")
                require(attempt["monotonic_ms"] <= hook["arm"]["event"]["detail"]["deadline_ms"], "hook target forwarded after expiry")
                require(signed and signed[0]["seq"] < attempt["seq"], "hook target sent before signed checkpoint")
                require(not cancelled or attempt["seq"] < cancelled[0]["seq"], "hook target forwarded after cancellation")
                if hold:
                    require(released and released[0]["seq"] < attempt["seq"], "held payment forwarded before release")

    def _send_preimage(self, attempt):
        rows = self.history(attempt["incarnation"])
        context = attempt["context"]
        journal_seq = integer(context["journal_seq"])
        if journal_seq >= len(rows):
            raise Inconclusive("history snapshot precedes forward context")
        intent_seq = integer(context["intent_seq"], 1, journal_seq)
        intent = rows[intent_seq]
        require(intent["seq"] == intent_seq and intent["record"] in {"broadcast_sent", "reorg_resubmit"}
                and intent["command_id"] == context["command_id"] and intent["send_sequence"] == attempt["send_sequence"],
                "forward not bound to exact durable send intent")
        signed = [r for r in rows[:intent_seq] if r["record"] == "signed_tx" and r["command_id"] == context["command_id"]]
        require(len(signed) == 1 and signed[0]["txid"] == intent["txid"], "forward signed transaction binding")
        raw = unhex(signed[0]["raw_hex"])
        require(decode_transaction(raw)["txid"] == signed[0]["txid"], "forward raw transaction txid")
        require(digest(raw) == context["raw_sha256"]
                and digest(compact([signed[0]["raw_hex"]])) == attempt["payload_sha256"], "forward full raw/params hash mismatch")
        revisions = [r["seq"] for r in rows[:journal_seq + 1] if r["record"] in {"chain_observed", "chain_rollback"}]
        require(revisions and context["chain_revision"] == revisions[-1], "forward canonical revision binding")
        return signed[0]

    def _reconcile_send_outcomes(self):
        """Join every RPC classification to the exact supporting send intent.

        The first local outcome may be a restart/Unknown despite an actual
        success, since unsuppressed does not prove pipe delivery. Every later
        AcceptedByRpc must still have an actual unsuppressed success for this
        same intent/node. The v1 stream permits one result per attempt; an
        actual Unknown cannot be upgraded without new actual evidence.
        Later raw-chain confirmations remain an independent recovery path.
        """
        reconciled, rejections, acceptances = [], {}, {}
        uncertain_commands = set()
        response_kinds = {"accepted_by_rpc", "unknown_outcome", "rejected"}
        for driver, history in self.histories.items():
            intents = [r for r in history if r["record"] in {"broadcast_sent", "reorg_resubmit"}]
            for intent in intents:
                cid, start = intent["command_id"], intent["seq"]
                later = [r["seq"] for r in intents if r["command_id"] == cid and r["seq"] > start]
                stop = min(later) if later else len(history)
                segment = history[start + 1:stop]
                local = next((r for r in segment if r["record"] == "session_started"
                              or r.get("command_id") == cid and r["record"] in response_kinds), None)
                attempt_seq = self.send_authorizations.get((self.run_id, driver, start))
                rejected = [r for r in segment if r.get("command_id") == cid and r["record"] == "rejected"]
                accepted = [r for r in segment if r.get("command_id") == cid and r["record"] == "accepted_by_rpc"]
                if attempt_seq is None:
                    if rejected or accepted:
                        raise Inconclusive("journal RPC classification lacks actual forward evidence")
                    if local is None:
                        raise Inconclusive("unforwarded intent has no durable uncertainty outcome")
                    require(local["record"] in {"unknown_outcome", "session_started"}, "unforwarded intent classified as definite")
                    uncertain_commands.add((driver, cid))
                    reconciled.append({"driver_id": driver, "intent_seq": start, "forward_attempt_seq": None,
                                       "local_outcome_seq": local["seq"], "retention_required": True})
                    continue
                attempt = self.attempts[attempt_seq]["event"]["attempt"]
                if attempt_seq not in self.results:
                    raise Inconclusive("forward lacks completed response evidence")
                result = self.results[attempt_seq]["event"]
                if local is None:
                    raise Inconclusive("forward result has no corresponding durable local outcome")
                require(local["seq"] > attempt["context"]["journal_seq"], "local outcome predates actual forward context")
                if local["record"] != "session_started":
                    require(local["txid"] == intent["txid"], "RPC outcome names a different transaction")
                unknown = result["response_suppressed"] or result["outcome"] in {"unknown", "gate_closed"}
                if result["outcome"] == "ok":
                    require(result["response_sha256"] == digest(compact({"outcome": "ok", "result": intent["txid"]})),
                            "actual send success does not hash to signed txid")
                if result["outcome"] == "unknown":
                    require(result["response_sha256"] in {digest(compact({"outcome": "unknown", "class": k}))
                                                         for k in ("transport", "broker", "timeout", "protocol", "shape")},
                            "unknown RPC outcome digest/class mismatch")
                if result["outcome"] == "gate_closed":
                    require(result["response_sha256"] == digest(compact({"outcome": "gate_closed"})), "gate-closed outcome digest mismatch")
                for record in accepted:
                    require(result["outcome"] == "ok" and not result["response_suppressed"],
                            "RPC acceptance lacks actual unsuppressed success for its correlated intent")
                    require(record["seq"] > attempt["context"]["journal_seq"] and record["txid"] == intent["txid"],
                            "accepted outcome transaction/forward context mismatch")
                    require(record["node_id"] == attempt["node_id"], "accepted outcome node mismatch")
                    acceptances[(driver, record["seq"])] = {
                        "command_id": cid, "txid": intent["txid"], "node_id": attempt["node_id"],
                        "intent_seq": start, "send_sequence": intent["send_sequence"],
                        "forward_attempt_seq": attempt_seq, "forward_result_seq": self.results[attempt_seq]["seq"],
                        "incarnation": attempt["incarnation"],
                        "history_prefix_sha256": digest(compact(history[:record["seq"] + 1], sorted_keys=True))}
                if unknown:
                    require(local["record"] in {"unknown_outcome", "session_started"} and not rejected,
                            "unknown/suppressed actual response classified as definite rejection or acceptance")
                elif result["outcome"] == "ok":
                    require(local["record"] in {"accepted_by_rpc", "unknown_outcome", "session_started"} and not rejected,
                            "actual RPC success classified as definitive rejection")
                elif result["outcome"] == "node_rejected":
                    require(local["record"] in {"rejected", "unknown_outcome", "session_started"}, "node rejection classified as RPC acceptance")
                if rejected:
                    require(len(rejected) == 1 and local is rejected[0] and not result["response_suppressed"]
                            and result["outcome"] == "node_rejected", "late rejection borrowed evidence from another intent/outcome")
                    matches = [v for v in DEFINITIVE_ROWS if result["response_sha256"] ==
                               digest(compact({"outcome": "node_rejected", "code": v[0], "message": v[1]}))]
                    require(len(matches) == 1 and rejected[0]["code"] == matches[0][2], "unlisted or mismatched definitive rejection")
                    expected = self.expected_rejection_binding
                    if expected is None or self.rejection_bindings.get(driver) is None:
                        raise Inconclusive("definitive rejection lacks reviewed source/binary/profile binding")
                    fields(expected, "source_sha256 binary_sha256 consensus_fingerprint", "reviewed rejection binding")
                    for value in expected.values():
                        unhex(value, 32)
                    require(self.rejection_bindings[driver] == expected, "actual rejection profile differs from reviewed profile")
                    retention = (driver, cid) in uncertain_commands or intent["record"] == "reorg_resubmit"
                    rejections[(driver, rejected[0]["seq"])] = {"command_id": cid, "txid": intent["txid"], "code": rejected[0]["code"],
                        "intent_seq": start, "forward_attempt_seq": attempt_seq, "forward_result_seq": self.results[attempt_seq]["seq"],
                        "incarnation": attempt["incarnation"], "retention_required": retention,
                        "history_prefix_sha256": digest(compact(history[:rejected[0]["seq"] + 1], sorted_keys=True))}
                if unknown or result["outcome"] == "ok" or local["record"] in {"unknown_outcome", "session_started"} or intent["record"] == "reorg_resubmit":
                    uncertain_commands.add((driver, cid))
                reconciled.append({"driver_id": driver, "incarnation": attempt["incarnation"], "intent_seq": start,
                                   "send_sequence": intent["send_sequence"], "forward_attempt_seq": attempt_seq,
                                   "forward_result_seq": self.results[attempt_seq]["seq"],
                                   "local_outcome_seq": local["seq"], "retention_required": (driver, cid) in uncertain_commands})
            # Reject orphan classifications that were not in any intent range.
            for record in history:
                if record["record"] == "rejected" and (driver, record["seq"]) not in rejections:
                    raise Inconclusive("journal rejection has no exact reconciled forward outcome")
                if record["record"] == "accepted_by_rpc" and (driver, record["seq"]) not in acceptances:
                    raise Inconclusive("journal acceptance has no exact reconciled forward outcome")
        return SendOutcomeProofs(self.run_id, rejections, reconciled, acceptances)

    def _hook(self, row):
        event, now = row["event"], row["monotonic_ms"]
        key, detail = event["key"], event["detail"]
        fields(key, "driver_id incarnation command_id", "hook key")
        transition = detail.get("transition")
        if transition not in HOOK_FIELDS:
            raise Inconclusive("unknown hook transition")
        fields(detail, "transition " + HOOK_FIELDS[transition], "hook detail")
        if key["incarnation"] not in self.sessions:
            raise Inconclusive("hook session missing")
        binding = self.sessions[key["incarnation"]]["binding"]
        require(binding["acceptance_enabled"] and key["driver_id"] == binding["driver_id"], "hook opt-in/driver binding")
        slot = key["driver_id"], key["command_id"]
        if transition == "armed":
            require(slot not in self.hooks and len(self.hooks) < 64 and detail["mode"] in {"hold_before_intent", "lose_one_response"}, "hook arm/rearm bounds")
            require(detail["boot_id"] == self.boot_id and now < detail["deadline_ms"] <= now + 120000, "hook absolute deadline")
            self.hooks[slot] = {"key": key, "arm": row, "events": [row]}
            return
        require(slot in self.hooks and self.hooks[slot]["key"] == key, "hook stale incarnation/target")
        hook = self.hooks[slot]
        kinds = [r["event"]["detail"]["transition"] for r in hook["events"]]
        require("cancelled" not in kinds and "response_discarded" not in kinds, "hook transition after terminal")
        if transition == "signed":
            require("signed" not in kinds, "duplicate signed hook")
            history = self.history(key["incarnation"])
            revision = integer(detail["journal_seq"], 1)
            if revision >= len(history):
                raise Inconclusive("signed hook checkpoint beyond captured journal")
            rows = [r for r in history[:revision + 1] if r["record"] == "signed_tx" and r["command_id"] == key["command_id"]]
            require(len(rows) == 1 and digest(unhex(rows[0]["raw_hex"])) == detail["raw_sha256"], "hook signed raw checkpoint")
        elif transition == "released":
            require(hook["arm"]["event"]["detail"]["mode"] == "hold_before_intent" and "signed" in kinds
                    and "released" not in kinds, "hook release sequence")
            checkpoint = next(r for r in hook["events"] if r["event"]["detail"]["transition"] == "signed")
            require(checkpoint["event"]["detail"]["raw_sha256"] == detail["raw_sha256"], "released different signed bytes")
        elif transition == "response_discarded":
            target = detail["attempt_seq"]
            require(hook["arm"]["event"]["detail"]["mode"] == "lose_one_response" and target in self.results,
                    "response loss without actual completed attempt")
            attempt = self.attempts[target]["event"]["attempt"]
            result = self.results[target]["event"]
            require(result["response_suppressed"] and attempt["incarnation"] == key["incarnation"]
                    and attempt["context"]["command_id"] == key["command_id"], "response loss attempt correlation")
        hook["events"].append(row)

    def pause_intervals(self):
        proofs = []
        for driver, history in self.histories.items():
            for barrier in (r for r in history if r["record"] == "send_barrier"):
                closes = [w for w in self.watermarks if w["event"]["command_id"] == barrier["command_id"] and w["event"]["closed"]
                          and self.sessions[w["event"]["incarnation"]]["binding"]["driver_id"] == driver]
                if len(closes) < 2:
                    raise Inconclusive("pause lacks pre/post drain watermarks")
                require(closes[0]["monotonic_ms"] <= barrier["monotonic_ms"] <= closes[-1]["monotonic_ms"], "barrier clock outside gate closure")
                resumes = [r for r in history if r["seq"] > barrier["seq"] and r["record"] == "send_resumed"]
                resume = resumes[0] if resumes else None
                finish_seq = resume["seq"] if resume else len(history)
                require(not any(r["record"] in {"broadcast_sent", "reorg_resubmit"} for r in history[barrier["seq"] + 1:finish_seq]),
                        "durable send intent inside paused interval")
                opens = [w for w in self.watermarks if resume and w["event"]["command_id"] == resume["command_id"] and not w["event"]["closed"]
                         and self.sessions[w["event"]["incarnation"]]["binding"]["driver_id"] == driver]
                if resume and not opens:
                    raise Inconclusive("resume lacks reopening watermark")
                end = opens[0]["seq"] if opens else self.rows[-1]["seq"] + 1
                attempts = [r for seq, r in self.attempts.items() if closes[0]["seq"] < seq < end
                            and r["event"]["attempt"]["method"] in EFFECTS
                            and self.sessions[r["event"]["attempt"]["incarnation"]]["binding"]["driver_id"] == driver]
                require(not attempts, "actual forward attempt during pause")
                proofs.append({"driver_id": driver, "barrier_command": barrier["command_id"],
                               "last_used_send_sequence": barrier["send_sequence"], "start_audit_seq": closes[0]["seq"],
                               "end_audit_seq": end, "forward_attempts": 0})
        if not proofs:
            raise Inconclusive("no actual committed pause interval")
        return proofs

    def response_loss(self, driver, command_id):
        hook = self.hooks.get((driver, command_id))
        if not hook:
            raise Inconclusive("response-loss hook missing")
        discarded = [r for r in hook["events"] if r["event"]["detail"]["transition"] == "response_discarded"]
        require(len(discarded) == 1, "response loss must occur exactly once")
        seq = discarded[0]["event"]["detail"]["attempt_seq"]
        attempt = self.attempts[seq]["event"]["attempt"]
        signed = self._send_preimage(attempt)
        outcome = self.results[seq]["event"]
        require(outcome["outcome"] == "ok" and outcome["response_sha256"] == digest(compact({"outcome": "ok", "result": signed["txid"]})),
                "lost response did not hash to actual successful node txid")
        joined = [p for p in self.outcome_proofs.reconciled if p["driver_id"] == driver and p["forward_attempt_seq"] == seq]
        if len(joined) != 1:
            raise Inconclusive("actor has not durably recorded loss as Unknown")
        local = self.histories[driver][joined[0]["local_outcome_seq"]]
        require(local["record"] in {"unknown_outcome", "session_started"}, "loss did not produce durable uncertainty")
        return {"command_id": command_id, "txid": signed["txid"], "actual_attempt_seq": seq,
                "real_success_response_suppressed_once": True, "actor_unknown_seq": local["seq"]}

    def hold_proof(self, driver, command_id):
        hook = self.hooks.get((driver, command_id))
        if not hook:
            raise Inconclusive("actual hold hook missing")
        signed = [r for r in hook["events"] if r["event"]["detail"]["transition"] == "signed"]
        released = [r for r in hook["events"] if r["event"]["detail"]["transition"] == "released"]
        if len(signed) != 1 or len(released) != 1:
            raise Inconclusive("hold has not completed signed/released checkpoints")
        forwards = [r for r in self.attempts.values() if r["event"]["attempt"]["method"] == "sendrawtransaction"
                    and r["event"]["attempt"]["context"]["command_id"] == command_id
                    and self.sessions[r["event"]["attempt"]["incarnation"]]["binding"]["driver_id"] == driver]
        require(all(r["seq"] > released[0]["seq"] for r in forwards), "held transaction forwarded before release")
        return {"command_id": command_id, "signed_audit_seq": signed[0]["seq"], "released_audit_seq": released[0]["seq"],
                "held_milliseconds": released[0]["monotonic_ms"] - signed[0]["monotonic_ms"],
                "forward_attempts_while_held": 0, "forward_attempts_after_release": len(forwards)}

    def definitive_rejections(self, bindings, expected_binding):
        if expected_binding is None:
            raise Inconclusive("reviewed definitive-rejection source/binary/profile binding unavailable")
        fields(expected_binding, "source_sha256 binary_sha256 consensus_fingerprint", "reviewed node profile")
        for value in expected_binding.values():
            unhex(value, 32)
        require(expected_binding == self.expected_rejection_binding, "rejection binding changed after outcome reconciliation")
        proofs = []
        for (driver, _), proof in self.outcome_proofs.rejections.items():
            require(bindings.get(driver) == expected_binding, "actual actor rejection profile binding differs")
            node_code = next(item[0] for item in DEFINITIVE_ROWS if item[2] == proof["code"])
            proofs.append({"attempt_seq": proof["forward_attempt_seq"], "command_id": proof["command_id"], "txid": proof["txid"],
                           "code": proof["code"], "node_error_code": node_code, "retention_required": proof["retention_required"]})
        if not proofs:
            raise Inconclusive("no actual funded source-bound definitive rejection captured")
        return proofs

    def journal_anchor_views(self, captured_views, authority_nodes):
        """Recover missed polling tips only from actual audited RPC preimages.

        A final chain prefix by itself is not a historical canonical witness.
        The prior-journal-revision interval must contain this driver's actual
        authority-node tip response hashing to the claimed header hash.
        """
        views, proofs = dict(captured_views), []
        for driver, history in self.histories.items():
            previous_revision = None
            for record in history:
                if record["record"] == "chain_observed":
                    tip, height = record["tip_block_hash"], record["tip_height"]
                elif record["record"] == "chain_rollback":
                    anchor = record["evidence"]["new_anchor"]
                    tip, height = anchor["block_hash"], anchor["height"]
                else:
                    continue
                matches = []
                for seq, row in self.attempts.items():
                    attempt = row["event"]["attempt"]
                    context = attempt["context"]
                    if (self.sessions[attempt["incarnation"]]["binding"]["driver_id"] != driver
                            or attempt["node_id"] != authority_nodes[driver]
                            or attempt["method"] != "getbestblockhash"
                            or context["chain_revision"] != previous_revision
                            or context["journal_seq"] >= record["seq"]):
                        continue
                    result = self.results[seq]["event"]
                    if (attempt["payload_sha256"] == digest(compact([])) and not result["response_suppressed"]
                            and result["response_sha256"] == digest(compact({"outcome": "ok", "result": tip}))):
                        matches.append(seq)
                if not matches:
                    raise Inconclusive("journal anchor lacks actual authority-tip response in its revision interval")
                source = next((v for v in captured_views.values() if tip in v.by_hash), None)
                if source is None:
                    raise Inconclusive("audited canonical tip has no independently decoded raw ancestry")
                require(source.by_hash[tip]["height"] == height, "journal anchor height differs from raw ancestry")
                views[tip] = source.prefix(height)
                proofs.append({"driver_id": driver, "journal_seq": record["seq"], "tip_hash": tip,
                               "node_id": authority_nodes[driver], "actual_tip_attempt_seq": matches[-1]})
                previous_revision = record["seq"]
        return views, proofs

    def activation_proof(self, manifest_boundary, chain, raw_tip, distribution_txids, miner_payloads):
        if self.boundary is None:
            raise Inconclusive("actual measured boundary event missing")
        boundary = self.boundary["event"]
        require(boundary["tip_hash"] == manifest_boundary["tip_hash"] and boundary["tip_height"] == manifest_boundary["tip_height"],
                "measured audit/manifest boundary mismatch")
        require(boundary["tip_hash"] in chain.by_hash and chain.by_hash[boundary["tip_hash"]]["height"] == boundary["tip_height"],
                "measured boundary raw chain membership")
        require(distribution_txids and all(txid in chain.transactions and not chain.transactions[txid][1]["coinbase"]
                                          and chain.transactions[txid][1]["fee"] > 0 and chain.transactions[txid][0]["height"] <= boundary["tip_height"]
                                          for txid in distribution_txids), "distribution not confirmed before activation")
        anchored_nodes = []
        for node in self.nodes:
            # Known exact response preimages make digest-only producer records
            # independently checkable without publishing full RPC transcripts.
            tip_calls = [r for seq, r in self.attempts.items() if seq < self.boundary["seq"]
                         and r["event"]["attempt"]["node_id"] == node
                         and r["event"]["attempt"]["method"] == "getbestblockhash"]
            if not tip_calls:
                raise Inconclusive("no actual pre-boundary tip call for node")
            last = tip_calls[-1]
            result = self.results[last["seq"]]["event"]
            require(last["event"]["attempt"]["payload_sha256"] == digest(compact([]))
                    and result["response_sha256"] == digest(compact({"outcome": "ok", "result": boundary["tip_hash"]}))
                    and not result["response_suppressed"], "actual node tip response does not bind common boundary")
            body_calls = [r for seq, r in self.attempts.items() if seq < self.boundary["seq"]
                          and r["event"]["attempt"]["node_id"] == node and r["event"]["attempt"]["method"] == "getblock"
                          and r["event"]["attempt"]["payload_sha256"] == digest(compact([boundary["tip_hash"], 0]))]
            require(body_calls and any(self.results[r["seq"]]["event"]["response_sha256"] ==
                                       digest(compact({"outcome": "ok", "result": raw_tip.hex()})) for r in body_calls),
                    "actual node raw boundary block response mismatch")
            anchored_nodes.append(node)
        observed = set()
        for seq, row in self.miner_attempts.items():
            event = row["event"]
            candidates = miner_payloads.get(event["miner_id"], [])
            matches = [p for p in candidates if digest(miner_wire_bytes(event["request_id"], p)) == event["payload_sha256"]]
            require(len(matches) == 1, "AddConnection bytes do not match actual recorded scenario/config")
            identity = event["miner_id"], matches[0]["connection"]["id"]
            require(identity not in observed and self.miner_results[seq]["event"]["succeeded"], "duplicate/failed initial AddConnection")
            observed.add(identity)
        expected = {(miner, p["connection"]["id"]) for miner, payloads in miner_payloads.items() for p in payloads}
        require(observed == expected, "initial AddConnection coverage incomplete")
        require(not any(seq > self.boundary["seq"] and r["event"]["attempt"]["method"] == "submitblock"
                        for seq, r in self.attempts.items()), "native funder submitted a block after measured boundary")
        return {"boundary_audit_seq": self.boundary["seq"], "tip_height": boundary["tip_height"],
                "anchored_nodes": sorted(anchored_nodes), "actual_connections": len(observed),
                "distribution_transactions_confirmed_before_activation": len(distribution_txids)}


def miner_wire_bytes(request_id, connection_payload):
    # The outer value is the declared MinerWireCommand serde struct order;
    # payload is a serde_json::Value map (sorted keys in this bound source).
    prefix = compact({"protocol": "spc-cpu-miner-control", "version": 1,
                      "command_id": request_id, "operation": "add_connection"})[:-1]
    return prefix + b',"payload":' + compact(connection_payload, sorted_keys=True) + b"}\n"
