#!/usr/bin/env python3
"""Independent artifact predicates; observations are never promoted silently."""

from __future__ import annotations

# hash helpers are provided by release_lib
import json
from pathlib import Path
import re
from typing import Any

from namespace_replay import collect_namespace_evidence, prove_namespace_chain
from release_lib import load_json, require, sha256_file


def compare_profile(actual: dict, expected: dict, context: str) -> list[dict]:
    require(isinstance(actual, dict), "profile_shape", f"{context} profile must be an object")
    differences = []
    for key, wanted in expected.items():
        observed = actual.get(key, "<missing>")
        if observed != wanted:
            differences.append({"field": key, "expected": wanted, "observed": observed})
    require(not differences, "profile_mismatch", f"{context} effective profile differs: {differences}")
    return differences


def expected_profile(contract: dict, profile_name: str, overlay: str | None = None,
                     include_consensus: bool = True) -> dict:
    profile = dict(contract["profiles"][profile_name])
    if overlay is not None:
        profile["regtest_pow_bits_overlay"] = overlay
    if not include_consensus:
        return {}
    return profile


def audit_node_api(path: Path) -> dict:
    require(path.is_file() and not path.is_symlink(), "artifact_missing", f"node API log is missing: {path}")
    methods = []
    malformed = 0
    for line in path.read_text(encoding="utf-8", errors="strict").splitlines():
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            malformed += 1
            continue
        if isinstance(row, dict) and isinstance(row.get("method"), str):
            methods.append(row["method"])
    forbidden = [method for method in methods if method == "setmocktime" or method.startswith("generate")]
    require(malformed == 0, "node_api_malformed", f"{path} has {malformed} malformed JSONL records")
    require(not forbidden, "forbidden_mining_control", f"forbidden measured-mining RPC methods in {path}: {sorted(set(forbidden))}")
    return {"path": str(path), "record_count": len(methods), "forbidden_methods": forbidden,
            "method_counts": {name: methods.count(name) for name in sorted(set(methods))}}


def load_and_check_lp_summary(run_dir: Path, node_sha256: str, retarget: bool) -> dict:
    summary_path = run_dir / "public" / "summary.json"
    summary = load_json(summary_path)
    require(summary.get("status") == "passed", "lp_live_status", f"approved driver did not pass: {summary.get('error')}")
    # The outer release gate owns candidate approval. LP-live necessarily runs
    # provisionally for a new node: its internal retarget pin is the old L2.
    require(summary.get("source_gate") == "provisional", "lp_live_scope", "LP-live result was not explicitly provisional")
    require(summary.get("retarget_claim") is retarget, "lp_live_scenario", "LP-live scenario/retarget role changed")
    require(summary.get("binaries", {}).get("node", {}).get("sha256") == node_sha256,
            "lp_live_node_binding", "LP-live used a different node binary")
    assertions = summary.get("assertions")
    require(isinstance(assertions, dict) and assertions, "lp_live_assertions", "missing LP-live assertions")
    require(all(value is True for value in assertions.values()), "lp_live_assertions",
            f"LP-live non-passing assertions: {assertions}")
    api = audit_node_api(run_dir / "private" / "node-api.jsonl")
    observed = {
        "summary_path": str(summary_path), "summary_sha256": sha256_file(summary_path),
        "scenario": summary.get("scenario"), "source_gate": summary.get("source_gate"),
        "assertions": assertions, "node_api": api,
        "status": "passed", "scope": "LP-live behavior only; outer candidate gate remains separate",
    }
    if retarget:
        evidence = summary.get("retarget_evidence", {})
        boundaries = evidence.get("boundaries")
        require(isinstance(boundaries, list) and len(boundaries) == 3,
                "retarget_evidence", "expected three effective-profile retarget boundaries")
        require(all(row.get("actual_bits") == row.get("independently_expected_bits") for row in boundaries),
                "retarget_oracle", "boundary vector and independently expected bits differ")
        require(evidence.get("duty_cycle", {}).get("causal_attribution") == "not established by this evidence block",
                "retarget_noncausal", "retarget evidence lacks its non-causal disclosure")
        observed["retarget_boundaries"] = [
            {key: row.get(key) for key in ["height", "parent_bits", "actual_bits", "independently_expected_bits",
                                            "actual_elapsed_seconds", "clamped_elapsed_seconds"]}
            for row in boundaries
        ]
    else:
        require(summary.get("candidate", {}).get("conservation") is not None,
                "settlement_conservation", "baseline report omitted exact conservation evidence")
        observed["canonical_candidate"] = {
            "block_hash": summary.get("candidate", {}).get("block_hash"),
            "scanner_output_count": summary.get("candidate", {}).get("scanner_output_count"),
            "conservation": summary.get("candidate", {}).get("conservation"),
        }
    return observed


def _atomic_view(view: Any, field: str) -> dict:
    require(isinstance(view, dict) and view.get("atomic") is True,
            "namespace_view", f"{field} is not an atomic paused view")
    require(type(view.get("height")) is int and view["height"] >= 0 and
            isinstance(view.get("hash"), str) and re.fullmatch(r"[0-9a-f]{64}", view["hash"]),
            "namespace_view", f"{field} has invalid height/hash")
    return {"node": view.get("node"), "height": view["height"], "hash": view["hash"], "atomic": True}


def _phase_min_height(scenario: dict, phase_id: str, node_id: str) -> int:
    phase = next((row for row in scenario["phases"] if row.get("id") == phase_id), None)
    require(phase is not None, "namespace_scenario", f"missing scenario phase {phase_id}")
    heights = [command["kind"]["height"] for command in phase["commands"]
               if command.get("kind", {}).get("type") == "wait_height" and command["kind"].get("node_id") == node_id]
    require(heights, "namespace_scenario", f"{phase_id} has no wait_height for {node_id}")
    return max(heights)


def check_namespace_artifacts(run_dir: Path, scenario_path: Path, expected: dict) -> dict:
    manifest_path = run_dir / "run-manifest.json"
    result_path = run_dir / "shareable" / "result.json"
    manifest = load_json(manifest_path)
    result = load_json(result_path)
    scenario = load_json(scenario_path)
    require(manifest.get("state") == result.get("state") == "stopped", "namespace_state", "namespace run is not stopped")
    require(manifest.get("base_revision") == expected["base_revision"] and
            manifest.get("dirty_tree_hash") == expected["code_snapshot_sha256"],
            "namespace_source_binding", "runtime manifest does not bind the frozen runtime source")
    private_scenario_path = run_dir / "private" / "scenario.json"
    private_scenario = load_json(private_scenario_path)
    require(manifest.get("scenario_sha256") == sha256_file(private_scenario_path),
            "namespace_scenario_binding", "runtime manifest does not match its actual rendered scenario")
    # The runtime adds default/null fields, generated addresses and passwords.
    # Require all behavior-bearing topology, phase and work declarations to be
    # those in the source file independently bound by the outer manifest gate.
    for key in ["run", "links", "phases", "assertions", "statistics"]:
        require(private_scenario.get(key) == scenario.get(key),
                "namespace_scenario_semantics", f"rendered scenario changed {key}")
    for plural, singular in [("nodes", "node"), ("pools", "pool"), ("miners", "miner"), ("fronts", "front")]:
        require([row.get("id") for row in private_scenario.get(plural, [])] ==
                [row.get("id") for row in scenario.get(plural, [])],
                "namespace_scenario_semantics", f"rendered scenario changed {plural} IDs")
        require(len(private_scenario.get(plural, [])) == len(scenario.get(plural, [])),
                "namespace_scenario_semantics", f"rendered scenario changed {plural} count")
    require([row.get("work_profile") for row in private_scenario["pools"]] ==
            [{**row["work_profile"], "legacy_share_difficulty": None} for row in scenario["pools"]],
            "namespace_scenario_semantics", "rendered scenario changed pool work profiles")
    require(result.get("gaps") == 0, "namespace_gaps", f"runtime reported evidence gaps: {result.get('gaps')}")
    assertion_ids = {row["id"] for row in scenario["assertions"]}
    observed_assertions = result.get("assertions", [])
    require({row.get("id") for row in observed_assertions} == assertion_ids and
            all(row.get("state") == "passed" for row in observed_assertions),
            "namespace_assertions", f"namespace assertions are not all passing: {observed_assertions}")

    phases = result.get("topology", {}).get("phases")
    require(isinstance(phases, list) and phases, "namespace_topology", "topology phases are missing")
    bypass_checks = 0
    for phase in phases:
        broker = phase.get("broker_evidence", {})
        no_bypass = broker.get("no_bypass")
        if phase.get("phase") in {"partition-p2p"}:
            require(isinstance(no_bypass, dict), "namespace_no_bypass_missing",
                    f"required no-bypass observation missing in {phase.get('phase')}")
        if no_bypass is None:
            # Baseline phases before the bounded negative probe are unknown,
            # not fabricated passes or failures.
            continue
        require(isinstance(no_bypass, dict) and no_bypass.get("ok") is True and
                no_bypass.get("violations") == [] and no_bypass.get("backend") == "netns" and
                no_bypass.get("isolation") == "netns",
                "namespace_no_bypass", f"no-bypass check failed in {phase.get('phase')}")
        require(phase.get("controller", {}).get("reachable") is True and
                all(front.get("reachable") is True for front in phase.get("fronts", [])),
                "namespace_controller", f"controller/front health failed in {phase.get('phase')}")
        bypass_checks += len(no_bypass.get("checks", []))
    # Re-check health also for baseline rows where no negative probe was run.
    for phase in phases:
        require(phase.get("controller", {}).get("reachable") is True and
                all(front.get("reachable") is True for front in phase.get("fronts", [])),
                "namespace_controller", f"controller/front health failed in {phase.get('phase')}")
    require(bypass_checks > 0, "namespace_no_bypass_empty", "no actual no-bypass probes were recorded")

    partition_index = next((i for i, row in enumerate(phases) if row.get("phase") == "partition-p2p"), None)
    heal_index = next((i for i, row in enumerate(phases) if row.get("phase") == "heal-p2p"), None)
    require(partition_index is not None and heal_index is not None and partition_index < heal_index,
            "namespace_phase_order", "partition/heal phases are missing or out of order")
    fork_phase = phases[heal_index - 1]
    require(fork_phase.get("phase") == "quiesce-before-heal" and fork_phase.get("outcome", {}).get("detail") == "workers paused",
            "namespace_fork_quiesce", "pre-heal fork was not captured in a paused phase")
    fork_views = [_atomic_view(row, "pre-heal") for row in fork_phase.get("nodes", [])]
    require({row["node"] for row in fork_views} == {"node-a", "node-b"} and len(fork_views) == 2,
            "namespace_fork_peers", "pre-heal views are not exactly node-a/node-b")
    for view in fork_views:
        minimum = _phase_min_height(scenario, "partition-real-work", view["node"])
        require(view["height"] >= minimum, "namespace_fork_work", f"{view['node']} did not perform real partition work")
    require(phases[partition_index].get("outcome", {}).get("detail") == "link link-p2p-nodes -> partition" and
            phases[heal_index].get("outcome", {}).get("detail") == "link link-p2p-nodes -> normal",
            "namespace_fault_commands", "real link partition/heal commands were not recorded")

    convergence = result.get("topology", {}).get("convergence", {})
    require(convergence.get("status") == "converged", "namespace_convergence", "runtime did not report converged")
    final_views = [_atomic_view(row, "final") for row in convergence.get("views", [])]
    require(len(final_views) == 2 and {row["node"] for row in final_views} == {"node-a", "node-b"},
            "namespace_convergence_peers", "final views are not exactly node-a/node-b")
    require(final_views[0]["height"] == final_views[1]["height"] and final_views[0]["hash"] == final_views[1]["hash"],
            "namespace_convergence", "paused final canonical tips differ")
    for view in final_views:
        minimum = _phase_min_height(scenario, "work-after-heal", view["node"])
        require(view["height"] >= minimum, "namespace_heal_work", f"{view['node']} did not reach post-heal real-work bound")
    controls = convergence.get("controls", [])
    require(controls and all(row.get("resume") is False and row.get("outcome", {}).get("state") == "succeeded" and
                             row.get("outcome", {}).get("detail") == "workers paused" for row in controls),
            "namespace_final_pause", "convergence is not anchored in successful paused controls")
    chain_evidence = collect_namespace_evidence(run_dir)
    chain_proof = prove_namespace_chain(chain_evidence)

    processes = manifest.get("processes", [])
    expected_roles = {"node": 2, "front": 2, "miner": 2, "postgres": 2, "redis": 2}
    require(processes and all(row.get("state") in {"stopped", "exited"} for row in processes),
            "namespace_process_cleanup", "one or more namespace actors is still running or unknown")
    require({role: sum(row.get("role") == role for row in processes) for role in expected_roles} == expected_roles,
            "namespace_process_roles", "namespace actor role counts changed")
    for row in processes:
        wanted_role = "node" if row["role"] == "node" else {"front": "spc-pool", "miner": "spc-cpu-miner"}.get(row["role"])
        if wanted_role is not None:
            expected_hash = expected["binary_hashes"][wanted_role]
            require(row.get("executable_sha256") == expected_hash,
                    "namespace_process_binary", f"{row.get('entity')} used an unexpected executable")
    node_api = [audit_node_api(run_dir / "private" / f"node-api-node-{suffix}.jsonl") for suffix in ["a", "b"]]
    return {
        "status": "passed", "manifest_sha256": sha256_file(manifest_path), "result_sha256": sha256_file(result_path),
        "assertions": sorted(assertion_ids), "fork_views": fork_views, "final_views": final_views,
        "no_bypass_checks": bypass_checks, "network_tooling": "host tools; separate from unavailable pinned Nix network tools",
        "node_api": node_api, "process_terminal_records": len(processes),
        "chain_proof": chain_proof,
        "scope": "observed partition/heal and scoped accounting; not full header, payout, settlement or causal-rate proof",
    }


def check_static_reference(audit_dir: Path, contract: dict) -> dict:
    static = contract["static_reference"]
    compare_path = audit_dir / "compare.json"
    patch_path = audit_dir / "l2-port.patch"
    require(sha256_file(compare_path) == static["compare_sha256"], "static_input_drift", "stock compare.json changed")
    require(sha256_file(patch_path) == static["l2_patch_sha256"], "static_input_drift", "old L2 patch reference changed")
    compare = load_json(compare_path)
    require(compare["commits"][-1]["sha"] == contract["release"]["official_commit"],
            "stock_release_binding", "comparison does not end at official v0.9.0")
    patch = patch_path.read_text(encoding="utf-8")
    for needle in ["--retarget-regtest", "info[\"consensus\"] = consensus_info(ctx);",
                   "REGTEST_RETARGET_SPACING_SECS: u64 = 10", "REGTEST_RETARGET_TIMESPAN_SECS: u64 = 240",
                   "REGTEST_RETARGET_FLOOR_BITS: u32 = 0x1e7f_ffff"]:
        require(needle in patch, "l2_reference", f"old approved L2 reference lacks {needle}")
    stock_hashes = {}
    stock_contains_retarget = []
    for relative, expected_hash in static["stock_files"].items():
        path = audit_dir / "v090" / relative
        require(path.is_file() and not path.is_symlink(), "static_input_missing", f"missing stock excerpt {relative}")
        actual_hash = sha256_file(path)
        require(actual_hash == expected_hash, "static_input_drift", f"stock excerpt changed: {relative}")
        stock_hashes[relative] = actual_hash
        text = path.read_text(encoding="utf-8", errors="strict")
        if "retarget-regtest" in text or "retarget_regtest" in text:
            stock_contains_retarget.append(relative)
    require(not stock_contains_retarget, "stock_reference_assumption",
            f"stock source unexpectedly contains the lab flag: {stock_contains_retarget}")
    chain = (audit_dir / "v090" / "crates/rbitcoin-rpc/src/methods/chain.rs").read_text(encoding="utf-8")
    require("pub fn consensus_info" not in chain and 'info["consensus"] = consensus_info(ctx);' not in chain,
            "stock_reference_assumption", "stock v0.9 unexpectedly has getblockchaininfo.consensus")
    primitives = (audit_dir / "v090" / "prim-lib.rs").read_text(encoding="utf-8")
    require("pub const SCHEMA_VERSION: u16 = 26;" in primitives,
            "stock_schema", "source-only stock schema reference is not 26")
    return {
        "status": "passed", "official_commit": contract["release"]["official_commit"],
        "stock_lab_retarget_flag": "absent in the pinned source excerpts; live stock CLI refusal is a separate check",
        "stock_consensus_rpc": "absent in pinned chain.rs excerpt",
        "schema_26": "source excerpt only; no datadir migration/opening claim",
        "l2_patch_sha256": static["l2_patch_sha256"], "stock_file_count": len(stock_hashes),
        "stock_files_sha256": stock_hashes,
        "candidate_port_proven": False,
        "reason": "static stock and old-patch references cannot validate the moving/new candidate",
    }
