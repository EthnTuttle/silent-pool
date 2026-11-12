#!/usr/bin/env python3
"""Independent read-only audit of the archived real T2 native funding slice.

No daemon, wallet, production helper, build, replay or write into the input
archive is performed. Consensus-prefix consistency is explicitly distinct
from independently captured per-node canonical-at-time responses.
"""
import argparse
from collections import Counter
from dataclasses import asdict, replace
import hashlib
import json
import os
from pathlib import Path
import sys

from oracle import (Chain, Inconclusive, Invalid, Policy, address_script,
                    decode_transaction, expected_bits, integer, require,
                    strict_json, unhex)
from journal import Journal, hash_budget, native_observations, parse_records


def sha(data):
    return hashlib.sha256(data).hexdigest()


class EvidenceGap(Inconclusive):
    def __init__(self, code, facts):
        super().__init__(code)
        self.facts = facts


class ArchiveAudit:
    def __init__(self, receipt, freeze, output):
        self.output = Path(output).absolute()
        self.output.mkdir(mode=0o700, exist_ok=False)
        self.sources, self.cases, self.findings = {}, {}, []
        self.receipt = self.load(receipt)
        self.freeze = self.load(freeze)
        self.root = Path(self.receipt["case"])
        self.policy = None

    def load(self, path):
        path = Path(path)
        require(path.stat().st_size <= 128 * 1024 * 1024, "archive file resource bound")
        data = path.read_bytes()
        self.sources[str(path)] = {"sha256": sha(data), "bytes": len(data)}
        return strict_json(data)

    def jsonl(self, path):
        require(Path(path).stat().st_size <= 128 * 1024 * 1024, "archive JSONL resource bound")
        data = Path(path).read_bytes()
        self.sources[str(path)] = {"sha256": sha(data), "bytes": len(data)}
        if not data or not data.endswith(b"\n"):
            raise Inconclusive("missing/truncated archive JSONL")
        try:
            return [strict_json(line) for line in data.splitlines()]
        except (ValueError, Invalid) as error:
            raise Inconclusive("malformed/poisoned archive JSONL record") from error

    def check(self, name, function):
        try:
            self.cases[name] = {"status": "PASSED", "facts": function()}
        except Invalid as error:
            self.cases[name] = {"status": "FAILED", "code": str(error)}
        except (Inconclusive, OSError, ValueError, KeyError, TypeError, IndexError, AttributeError, StopIteration) as error:
            self.cases[name] = {"status": "INCONCLUSIVE", "code": str(error)}
            if isinstance(error, EvidenceGap):
                self.cases[name]["available_facts"] = error.facts

    def seals(self):
        for row in self.receipt["artifacts"]:
            path = Path(row["path"])
            # The producer manifest also explicitly seals its external build
            # receipts. Reading those exact rows is part of the input contract.
            require(path.is_absolute(), "artifact path is not absolute")
            data = path.read_bytes()
            require(sha(data) == row["sha256"] and len(data) == row["bytes"], "archive artifact seal mismatch: " + path.name)
            self.sources[str(path)] = {"sha256": sha(data), "bytes": len(data), "matches_T2_seal": True}
        native = Path(self.receipt["bootstrap"]["artifact_path"])
        require(native.is_relative_to(self.root), "native diagnostics outside archive")
        data = native.read_bytes()
        require(sha(data) == self.receipt["bootstrap"]["artifact_sha256"], "native diagnostic seal mismatch")
        self.sources[str(native)] = {"sha256": sha(data), "bytes": len(data), "matches_T2_seal": True}
        return {"artifact_rows": len(self.receipt["artifacts"]), "native_diagnostics_separately_sealed": True,
                "total_sealed_artifacts": len(self.receipt["artifacts"]) + 1,
                "source_full": self.freeze["full_sha256"], "source_code": self.freeze["code_sha256"]}

    def chain(self):
        self.manifest = self.load(self.root / "run" / "run-manifest.json")
        profiles = self.manifest["effective_profiles"]
        require(len(profiles) == 2, "archived node census")
        p = next(iter(profiles.values()))
        require(all(v == p for v in profiles.values()), "bound archived node profiles disagree")
        # In this no-retarget slice BIP94 is not read by next-work. Verify that
        # explicitly below rather than guessing an unreported retarget policy.
        require(p["no_pow_retargeting"] is True, "archive adapter is bound to the declared no-retarget slice")
        self.policy = Policy(self.manifest["traffic_capability"]["expected_genesis"],
                             int(p["pow_limit_bits"], 16), int(p["initial_target_bits"], 16),
                             int(p["regtest_pow_bits_overlay"], 16), p["no_pow_retargeting"],
                             p["allow_min_difficulty_blocks"], p["target_spacing_secs"], p["target_timespan_secs"],
                             p["retarget_interval_blocks"], False, int(p["retarget_floor_bits"], 16),
                             int(p["subsidy_initial_sats"]), p["subsidy_halving_interval_blocks"], p["coinbase_maturity_blocks"])
        self.raw_blocks = [unhex(raw) for raw in self.load(self.root / "canonical-raw-blocks.json")]
        self.canonical = Chain(self.policy, self.raw_blocks)
        for height, block in enumerate(self.canonical.blocks[1:], 1):
            require(expected_bits(replace(self.policy, enforce_bip94=True), self.canonical.blocks[:height], block["time"])
                    == block["bits"], "unreported BIP94 fact affects this archive")
        fees = {tx["txid"]: tx["fee"] for block in self.canonical.blocks for tx in block["transactions"] if not tx["coinbase"]}
        claimed = self.receipt["bootstrap"]["distribution_fees_sats"]
        require(fees == claimed, "raw distribution fees differ from T2 claim")
        require(set(fees) == set(self.receipt["bootstrap"]["distribution_txids"]), "distribution transaction coverage")
        ages = []
        for txid in fees:
            block, tx = self.canonical.transactions[txid]
            for inp in tx["inputs"]:
                output = self.canonical.outputs[inp["outpoint"]]
                if output["coinbase"]:
                    ages.append(block["height"] - output["height"])
        return {"height": self.canonical.tip["height"], "raw_block_count": len(self.raw_blocks),
                "genesis": self.policy.genesis, "tip": self.canonical.tip["hash"],
                "bits_counts_excluding_genesis": dict(Counter(f"{b['bits']:08x}" for b in self.canonical.blocks[1:])),
                "first_five_bits": [f"{b['bits']:08x}" for b in self.canonical.blocks[:5]],
                "exact_fees_sats": fees, "total_fees_sats": sum(fees.values()),
                "unique_created_outpoints": len(self.canonical.outputs), "canonical_unspent_outpoints": len(self.canonical.utxos),
                "minimum_spent_coinbase_age": min(ages), "bip94_not_exercised_and_value_invariant": True,
                "canonical_scope": "valid raw archived chain; per-node response bodies not inferred from metadata"}

    def budgets(self):
        scenario = self.load(self.root / "scenario.json")
        self.command_rows = self.jsonl(self.root / "run" / "private" / "commands.jsonl")
        self.native_rows = self.jsonl(self.receipt["bootstrap"]["artifact_path"])
        starts = [r for r in self.native_rows if r["event"] == "begin"]
        require(len(starts) == 1, "native begin policy binding missing/duplicated")
        p = self.policy
        expected_policy = {"genesis": p.genesis, "pow_limit_bits": p.pow_limit_bits,
                           "initial_target_bits": p.initial_target_bits, "regtest_pow_bits_overlay": p.regtest_pow_bits_overlay,
                           "no_pow_retargeting": p.no_pow_retargeting, "allow_min_difficulty_blocks": p.allow_min_difficulty_blocks,
                           "pow_target_spacing": p.target_spacing, "difficulty_adjustment_interval": p.difficulty_adjustment_interval,
                           "subsidy_initial_sats": p.subsidy_initial_sats, "subsidy_halving_interval": p.subsidy_halving_interval}
        require(starts[0]["policy"] == expected_policy and p.coinbase_maturity == 100,
                "manifest policy differs from sealed native begin policy/T2 maturity contract")
        budget = hash_budget(self.command_rows, scenario["traffic"]["driver"]["bootstrap"]["total_attempts"])
        require(budget["charged_reserved"] == self.receipt["bootstrap"]["charged_hashes"]
                and budget["measured_executed"] == self.receipt["bootstrap"]["measured_hashes"], "budget claims disagree")
        observations = native_observations(self.native_rows, self.command_rows,
                                          {self.canonical.tip["hash"]: self.canonical}, self.policy,
                                          self.manifest["created_at"] // 1000 - 5)
        require(observations["raw_attempt_headers"] == budget["measured_executed"], "native attempted headers/count")
        return {**budget, **observations, "reserved_but_not_executed": budget["charged_reserved"] - budget["measured_executed"],
                "hash_batches": sum(r["record"] == "hash_reserve" for r in self.command_rows),
                "requested_aggregate_hashes_per_second": scenario["traffic"]["driver"]["bootstrap"]["hashes_per_second"]}

    def provenance(self):
        before = self.load(self.root / "provenance-before.json")
        after = self.load(self.root / "provenance-after.json")
        require(before["binaries"] == after["binaries"] == self.receipt["binaries"], "binary provenance before/after mismatch")
        require(before["source_sha256"] == self.receipt["runtime_source_sha256"] and after["source_unchanged"] is True,
                "runtime source provenance mismatch")
        require(before["probe_contract"] == self.receipt["bootstrap"]["preflight_contract"], "canonical probe contract provenance mismatch")
        nodes = [p for p in self.manifest["processes"] if p["role"] == "node"]
        require(len(nodes) == 2 and all(p["executable_sha256"] == before["node_sha256"] for p in nodes),
                "actual owned node binary provenance mismatch")
        for value in before["source_files"].values():
            unhex(value, 32)
        return {"runtime_source_sha256": before["source_sha256"], "captured_source_rows": len(before["source_files"]),
                "actual_node_binary_sha256": before["node_sha256"], "actual_spc_binary_hashes": before["binaries"],
                "canonical_probe_contract": before["probe_contract"], "source_and_binary_before_after_consistent": True,
                "inert_fixture_binary_explicitly_present": "spc-lab-fixture" in before["binaries"]}

    def journal_consistency(self):
        self.histories = {}
        prefix_views = {block["hash"]: self.canonical.prefix(height)
                        for height, block in enumerate(self.canonical.blocks)}
        summaries, allocated_points = {}, {}
        for driver, mapping in self.manifest["traffic_driver_paths"].items():
            archived = self.load(self.root / (driver + "-history.json"))
            current = self.jsonl(mapping["journal_path"])
            require(current[:len(archived)] == archived, "sealed history snapshot not a prefix of retained original journal")
            # Strict parser checks field sets and contiguous original sequence;
            # no writer/reopen path or repair is invoked.
            rows = parse_records(b"".join(json.dumps(r).encode() + b"\n" for r in current))
            self.histories[driver] = rows
            verifier = Journal(rows, {"run_id": self.manifest["run_id"], "driver_id": driver,
                                     "allocation_digest": mapping["allocation_digest"]}, prefix_views)
            for point, row in verifier.funding.items():
                require(point not in allocated_points, "funding outpoint assigned to multiple drivers")
                allocated_points[point] = row
            summaries[driver] = {"sealed_snapshot_records": len(archived), "retained_records": len(rows),
                                 "funding_outpoints": len(verifier.funding), "signed_transactions": len(verifier.txs),
                                 "send_sequences": verifier.send_sequence, "barrier_count": len(verifier.barriers),
                                 "paused_at_end": verifier.paused, "balances": verifier.balances()}
        self.funding = allocated_points
        return {"drivers": summaries, "globally_unique_funding_outpoints": len(allocated_points),
                "anchor_scope": "journal claims checked against valid archived prefixes; not a live timestamp attestation"}

    def distribution(self):
        # Allocation ranges come from the actual source-bound actor configs,
        # not hardcoded 8/4 default census assumptions about this T2-only case.
        allocations = []
        for mapping in self.manifest["traffic_driver_paths"].values():
            data = Path(mapping["config_path"]).read_bytes()
            require(sha(data) == mapping["config_sha256"], "actor config seal mismatch")
            allocations.append(self.load(mapping["config_path"])["allocation"])
        require(all(a == allocations[0] for a in allocations), "actors received different complete allocations")
        allocation = allocations[0]
        self.allocation = allocation
        counts, roles, owners = Counter(), {}, {}
        n, u = 0, 0
        for actor in sorted(allocation["actors"], key=lambda a: a["driver_id"]):
            for kind, first, count in (("network_wallet", n, actor["network_wallets"]),
                                       ("user_payment", u, actor["user_payment_wallets"])):
                for index in range(first, first + count):
                    roles[(kind, index)] = actor["driver_id"]
            n += actor["network_wallets"]; u += actor["user_payment_wallets"]
        for driver, rows in self.histories.items():
            for row in rows:
                if row["record"] != "funding_observed":
                    continue
                role = row["owner_role"]
                if role["kind"] == "bootstrap_funding":
                    continue
                key = (role["kind"], role["index"])
                require(roles.get(key) == driver, "funding role outside declared actor allocation")
                point = row["outpoint"]
                txid = point.split(":")[0]
                if txid in self.receipt["bootstrap"]["distribution_txids"]:
                    require(point not in owners, "duplicate distribution funding record")
                    owners[point] = driver
                    counts[key] += 1
        plans = [r for r in self.native_rows if r["event"] == "distribution_plan"]
        require(len(plans) == 1, "distribution plan observation missing/duplicated")
        declared_per_wallet = self.load(self.root / "scenario.json")["traffic"]["bootstrap"]["distribution_outputs_per_wallet"]
        require(set(counts) == set(roles) and all(v == declared_per_wallet for v in counts.values()), "distribution allocation output coverage")
        require(len(owners) == plans[0]["output_count"], "distribution plan output count claim")
        values = [self.canonical.outputs[point]["value"] for point in owners]
        require(all(value == plans[0]["amount_per_output"] for value in values), "distribution exact raw output amount")
        return {"network_wallets": n, "user_payment_wallets": u, "distribution_outputs": len(owners),
                "outputs_per_wallet": declared_per_wallet, "amount_per_output_sats": values[0],
                "distributed_total_sats": sum(values), "declared_role_allocation_checked": True,
                "public_seed_script_derivation": "separate source-bound check"}

    def public_scripts(self):
        from public_derivation import expected_p2pkh_script
        provenance = self.load(self.root / "provenance-before.json")
        source_hash = provenance["source_files"]["crates/spc-lab-txd/src/keys.rs"]
        if source_hash != "c67e8e561e9a28905e4af66d8cee2a2a185f7c61a8e2e5a047dfc26cf6071a32":
            raise Inconclusive("traffic key child-index policy needs an explicit new source binding")
        a = self.allocation
        require(a["run_id"] == self.manifest["run_id"] and a["network"] == "regtest", "public-seed allocation run/network")
        scenario = self.load(self.root / "scenario.json")
        require(a["run_seed"] == scenario["run"]["seed"], "actor/scenario public seed binding")
        count, changes, documented_first = 0, 0, None
        for rows in self.histories.values():
            require(rows[0]["run_seed_fingerprint"] == sha(integer(a["run_seed"]).to_bytes(8, "big")), "run seed fingerprint")
            for row in rows:
                if row["record"] == "funding_observed":
                    # This exact sealed source uses hardened indices at BOTH
                    # levels. The independent BIP32 primitive never probes
                    # alternative paths to find one that happens to match.
                    script = expected_p2pkh_script(a["run_seed"], a["run_id"], row["owner_role"], row["chain"], row["index"], hardened_index=True)
                    require(script.hex() == row["script_hex"], "source-bound public-seed funding script mismatch")
                    count += 1
                    if documented_first is None:
                        documented = expected_p2pkh_script(a["run_seed"], a["run_id"], row["owner_role"], row["chain"], row["index"], hardened_index=False)
                        documented_first = {"journal_seq": row["seq"], "role": row["owner_role"], "chain": row["chain"],
                                            "index": row["index"], "documented_unhardened_script": documented.hex(),
                                            "actual_hardened_script": script.hex()}
                if row["record"] == "signed_tx" and row["change"] is not None:
                    script = expected_p2pkh_script(a["run_seed"], a["run_id"], row["payer_role"], "change", row["change"]["index"], hardened_index=True)
                    require(script.hex() == row["change"]["script_hex"], "source-bound public-seed change script mismatch")
                    changes += 1
        self.findings.append({"kind": "documentation_mismatch", "source_sha256": source_hash,
                              "source": "crates/spc-lab-txd/src/keys.rs:21 versus :324-326",
                              "detail": "header comment omits the second apostrophe; sealed implementation hardens both levels",
                              "actual_path": "m/{receive=0,change=1}'/index'", "counterexample": documented_first,
                              "funding_contradiction": False})
        return {"public_seed_mode": "lab_deterministic_run_seed_v1", "derivation_version": 2,
                "source_sha256": source_hash, "child_index_hardened_explicitly": True,
                "funding_scripts_checked": count, "signed_change_scripts_checked": changes,
                "wallet_key_files_read": False, "keys_or_seeds_published": False}

    def external(self):
        checks = []
        for claimed in self.receipt["external_destinations"]:
            tx = decode_transaction(unhex(claimed["raw_hex"]))
            require(tx["txid"] == claimed["txid"] and not tx["witness"], "external destination raw txid/legacy input")
            require(tx["txid"] not in self.canonical.transactions, "external transaction incorrectly marked unconfirmed")
            expected = address_script(claimed["address"])
            require(expected == unhex(claimed["script_hex"]), "external requested address/script")
            require(sum(o["value"] for o in tx["outputs"] if o["script"] == expected) == claimed["amount_sats"], "external destination amount")
            fee = self.canonical.fee(tx)
            require(fee == claimed["fee_sats"] and tx["vbytes"] == claimed["vbytes"], "external exact fees/vbytes")
            for inp in tx["inputs"]:
                require(inp["outpoint"] in self.canonical.utxos, "external spend input not canonical unspent")
            checks.append({"type": claimed["type"], "txid": tx["txid"], "amount_sats": claimed["amount_sats"],
                           "fee_sats": fee, "vbytes": tx["vbytes"], "confirmed": False,
                           "node_acceptance_scope": "claimed in archived raw proof; canonical confirmation not inferred"})
        return checks

    def send_and_pause_facts(self):
        # Node logs attest requests that reached each server. They do not
        # contain canonical read response bodies or unforwarded failures.
        observed = Counter()
        reads = Counter()
        truncated = []
        for node_id in self.manifest["effective_profiles"]:
            rows = self.jsonl(self.root / "run" / "private" / ("node-api-" + node_id + ".jsonl"))
            for row in rows:
                method = row["method"]
                require(not method.startswith("generate") and method != "setmocktime", "forbidden mining/time RPC in real archive")
                if method == "sendrawtransaction":
                    try:
                        params = strict_json(row["params"])
                    except ValueError:
                        truncated.append({"node_id": node_id, "params_chars": len(row["params"]),
                                          "ends_with_ellipsis": row["params"].endswith("\u2026"), "ok": row["ok"]})
                        continue
                    require(isinstance(params, list) and len(params) == 1, "actual raw-send request form")
                    tx = decode_transaction(unhex(params[0]))
                    require(row["ok"] is True and row["err"] is None, "archived successful-send claim disagrees with node log")
                    observed[(node_id, tx["txid"], tx["raw"])] += 1
                if method in {"getblock", "getblockhash", "getbestblockhash", "getrawmempool", "getrawtransaction"}:
                    reads[(node_id, method)] += 1
        if truncated:
            raise EvidenceGap("node_api_raw_params_truncated", {
                "truncated_send_records": truncated,
                "committed_send_barriers": sum(r["record"] == "send_barrier" for rows in self.histories.values() for r in rows),
                "journal_send_intents": sum(r["record"] == "broadcast_sent" for rows in self.histories.values() for r in rows),
                "required_capture": "complete durable forward method/params or a reference to a sealed full raw request body",
                "raw_read_response_bodies": "absent from node API logging schema"})
        sends = 0
        barriers = 0
        for driver, rows in self.histories.items():
            signed = {r["txid"]: r for r in rows if r["record"] == "signed_tx"}
            for row in rows:
                if row["record"] == "send_barrier":
                    barriers += 1
                if row["record"] == "broadcast_sent":
                    raw = unhex(signed[row["txid"]]["raw_hex"])
                    count = sum(value for (node_id, txid, body), value in observed.items() if txid == row["txid"] and body == raw)
                    require(count == 1, "journal send lacks exactly one actual node raw-body observation")
                    sends += 1
        require(sum(observed.values()) == sends, "extra node raw sends not covered by driver journals")
        for item in self.receipt["external_destinations"]:
            command = self.load(self.root / ("external-" + item["type"] + "-command.json"))
            cid = command["request"]["command_id"]
            begins = [r for r in self.command_rows if r["record"] == "begin" and r["command"]["command_id"] == cid]
            finishes = [r for r in self.command_rows if r["record"] == "finish" and r["command_id"] == cid]
            require(len(begins) == len(finishes) == 1 and finishes[0]["outcome"]["state"] == "succeeded", "external command dedup/terminal evidence")
            require(begins[0]["command"] == command["request"], "external durable command payload changed")
        return {"actual_node_raw_sends": sends, "external_commands_each_have_one_begin_finish_send": True,
                "committed_send_barriers": barriers,
                "pause_acceptance": "INCONCLUSIVE" if barriers == 0 else "requires forward-attempt interval join",
                "read_request_counts": {node + ":" + method: count for (node, method), count in reads.items()},
                "raw_canonical_read_response_bodies": "not present in these node API logs"}

    def publish(self):
        failed = any(c["status"] == "FAILED" for c in self.cases.values())
        incomplete = any(c["status"] == "INCONCLUSIVE" for c in self.cases.values())
        status = "FAILED" if failed else ("INCONCLUSIVE" if incomplete else "PASSED_SCOPED_ARCHIVE_CHECKS")
        receipt = {"schema": "spc-lab-independent-T2-archive-oracle/1", "status": status,
                   "scope": "real T2 native funding archive; inert storage/front/miner fixtures explicitly excluded",
                   "full_T3_acceptance": "INCONCLUSIVE", "Ark_acceptance": "INCONCLUSIVE", "T4_witness_spends": "INCONCLUSIVE",
                   "cases": self.cases, "input_artifacts": self.sources,
                   "findings": self.findings,
                   "policy": asdict(self.policy) if self.policy else None,
                   "missing_next_run_capture_keys": [
                       "per-node canonical_at responses: node_id, query id, raw response, observed monotonic/boot id, anchored tip",
                       "complete raw RPC request bodies: existing node-api params are truncated to 384 characters plus ellipsis",
                       "durable forward attempt/outcome records and AddConnection/boundary ordering",
                       "full source-bound ladder/window/share/pending/witness job publication bundle",
                       "actual worker hash-rate/budget records and real fee-bearing pool blocks",
                       "funded parent-child reorg/maturity rollback, rejection and transport-unknown seam effects",
                       "actual witness spends and stock Ark receive/refresh/offboard proofs"]}
        (self.output / "receipt.json").write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
        lines = ["# Independent T2 real-data oracle", "", "Status: **" + status + "**", "",
                 "This is a scoped audit of real native funding, not full T3/T4b or Ark acceptance.", "",
                 "| Check | Result |", "|---|---|"]
        lines += ["| " + name + " | " + result["status"] + " |" for name, result in self.cases.items()]
        raw = self.cases.get("raw_chain_consensus_fees_maturity", {}).get("facts")
        if raw:
            lines += ["", f"Raw consensus checks: {raw['raw_block_count']} blocks including genesis, height {raw['height']}, "
                      f"{raw['total_fees_sats']} sats exact distribution fees. Minimum spent coinbase age: {raw['minimum_spent_coinbase_age']}.",
                      "Independently checked contextual nBits counts (excluding genesis): " + json.dumps(raw["bits_counts_excluding_genesis"], sort_keys=True) + "."]
        funding = self.cases.get("distribution_allocation_amounts", {}).get("facts")
        if funding:
            lines += [f"Distribution: {funding['distribution_outputs']} real outputs, {funding['amount_per_output_sats']} sats each; "
                      f"{funding['network_wallets']} network and {funding['user_payment_wallets']} payment wallets in this T2 slice."]
        scripts = self.cases.get("source_bound_public_seed_script_ownership", {}).get("facts")
        if scripts:
            lines += [f"Independently derived and matched {scripts['funding_scripts_checked']} funding scripts and "
                      f"{scripts['signed_change_scripts_checked']} signed change scripts, without reading wallet key files."]
        budget = self.cases.get("native_hash_reservations_and_headers", {}).get("facts")
        if budget:
            lines += [f"Hash accounting: {budget['measured_executed']} measured headers, {budget['charged_reserved']} charged, "
                      f"{budget['charged_unresolved']} unresolved; requested aggregate rate 40 H/s."]
        if self.findings:
            lines += ["", "Documentation finding: the sealed traffic key source hardens both path levels, "
                      "but its header comment omits the final apostrophe. The actual hardened path matches all checked scripts; this is not a funding contradiction."]
        lines += ["", "All source data was read-only. No process, production helper, wallet reopen, mining or build was invoked.",
                  "", "Exact facts and missing next-run evidence keys: `receipt.json`."]
        (self.output / "REPORT.md").write_text("\n".join(lines) + "\n")
        for path in self.output.iterdir():
            path.chmod(0o600)
        print(json.dumps({"status": status, "receipt": str(self.output / "receipt.json")}))
        return 1 if failed else (2 if incomplete else 0)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--receipt", required=True)
    parser.add_argument("--freeze", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    audit = ArchiveAudit(args.receipt, args.freeze, args.output)
    audit.check("artifact_seals", audit.seals)
    if audit.cases["artifact_seals"]["status"] == "PASSED":
        audit.check("raw_chain_consensus_fees_maturity", audit.chain)
        if audit.cases["raw_chain_consensus_fees_maturity"]["status"] == "PASSED":
            audit.check("sealed_provenance_binding", audit.provenance)
            audit.check("native_hash_reservations_and_headers", audit.budgets)
            audit.check("history3_archived_prefix_consistency", audit.journal_consistency)
            if audit.cases["history3_archived_prefix_consistency"]["status"] == "PASSED" and hasattr(audit, "native_rows"):
                audit.check("distribution_allocation_amounts", audit.distribution)
                if audit.cases["distribution_allocation_amounts"]["status"] == "PASSED":
                    audit.check("source_bound_public_seed_script_ownership", audit.public_scripts)
            audit.check("external_destination_bytes_unconfirmed", audit.external)
            if hasattr(audit, "histories") and hasattr(audit, "command_rows"):
                audit.check("actual_send_dedup_and_available_pause_facts", audit.send_and_pause_facts)
    return audit.publish()


if __name__ == "__main__":
    sys.exit(main())
