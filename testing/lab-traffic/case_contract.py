"""Source-owned required cases. No caller can remove a failing obligation."""
import hashlib
import json
from copy import deepcopy
from decimal import Decimal

from oracle import require

COMMON = {"fresh_chain", "config_binding", "hash_budget", "cleanup", "chain_evidence_export", "runtime_entrypoint_binding", "runtime_bootstrap_ownership"}
FUNDED = {"raw_chain", "fee_blocks_by_pool", "history3", "pause_interval", "manual_payment",
          "bootstrap_boundary", "rpc_funded_outcomes", "payout_ladder", "funded_reorg",
          "reorg_original_restore", "reorg_displaced_retry", "forward_completeness", "hold_after_sign",
          "response_loss", "all_sent_raw_observations", "funded_v3_sighash_all", "v3_prior_unknown_retention",
          "native_header_observations", "database_capture", "source_publication", "premining_proof_feed",
          "full_window_coverage", "native_source_applicability", "economic_final_drain", "final_sealed_prefix"}
WITNESS = {"witness_spends", "witness_mixed_block", "witness_job_republication"}
ORDER = ("bootstrap-stop", "bootstrap-maturity", "funded")
STATUSES = {"PASSED", "FAILED", "INCONCLUSIVE"}


def required(case, witness=False):
    require(case in ORDER, "unknown campaign case")
    extra = FUNDED if case == "funded" else {"bootstrap_stop" if case == "bootstrap-stop" else "retained_maturity_rollback"}
    return COMMON | extra | (WITNESS if witness and case == "funded" else set())


def verdict(results, obligations):
    require(isinstance(results, dict), "case results must be an object")
    states = [value.get("status") for value in results.values()]
    require(all(state in STATUSES for state in states), "unknown result status")
    # Supplemental failed/unknown checks cannot be averaged away either.
    if "FAILED" in states:
        return "FAILED"
    if not obligations <= set(results) or "INCONCLUSIVE" in states:
        return "INCONCLUSIVE"
    return "PASSED"


def semantic_scenario(scenario):
    value = deepcopy(scenario)
    for phase in value.get("phases", []):
        for command in phase["commands"]:
            if command.get("run_id") == value["run"]["id"]:
                command["run_id"] = "<per-case-fresh-id>"
    value["run"]["id"] = "<per-case-fresh-id>"
    return hashlib.sha256(canonical_json(value)).hexdigest()


def canonical_json(value):
    """Keep exact JSON decimal values as numbers, never float-round or quote."""
    def text(item):
        if isinstance(item, Decimal):
            require(item.is_finite(), "nonfinite JSON decimal")
            return str(item).lower()
        if isinstance(item, dict):
            require(all(isinstance(key, str) for key in item), "JSON object key type")
            return "{" + ",".join(json.dumps(key) + ":" + text(item[key]) for key in sorted(item)) + "}"
        if isinstance(item, (list, tuple)):
            return "[" + ",".join(text(v) for v in item) + "]"
        return json.dumps(item, allow_nan=False, separators=(",", ":"))
    return text(value).encode()
