"""Source-applicable admission for the explicit native-funding diagnostic."""
import hashlib
import re
from pathlib import Path

from common import artifact, encoded, load, require, sha, verify_source, validate_pool_control_admission
from scenario import scenario

APPS = ("lab", "txd", "pool", "agent", "miner", "scanner")
CONTROL_PATHS = ("bins/spc-pool/src/ark/wallet_control.rs", "bins/spc-pool/src/ark/wallet_control_io.rs",
                 "bins/spc-pool/src/ark/wallet_control_proof.rs", "bins/spc-pool/src/bin/pool-wallet-executor.rs")
POOL_BASE = {"bins/spc-pool/Cargo.toml", "bins/spc-pool/src/main.rs", "bins/spc-pool/src/lib.rs",
             "bins/spc-pool/src/ark/mod.rs", "bins/spc-pool/src/ark/board.rs",
             "bins/spc-pool/src/ark/bark_stock.rs", "bins/spc-pool/src/ark/bark_transport.rs"}
NATIVE_COMPONENTS = {"fund-server-alice", "fund-server-bob", "fund-pool-a", "fund-pool-b"}
LIMITS = {"controller_commands": ["bootstrap_fund", "send_user_payment", "stop_run"],
          "components": sorted(NATIVE_COMPONENTS), "ark_lifecycle_acceptance": False,
          "aggregate_acceptance": False, "whole_lab_complete": False}
POLICY_FIELDS = {"schema", "scope", "source_class", "source_root", "source_manifest_sha256",
                 "joined_validation_sha256", "app_binaries", "pool_files", "absent_control_paths",
                 "absent_control_executables", "joined_gate", "legacy_maintenance", "limits", "runtime_bindings"}


def scenario_template_sha256(spec=None):
    import copy
    value = copy.deepcopy(spec) if spec is not None else scenario("ap5-policy-template", 0)
    value["run"]["id"], value["run"]["seed"] = "ap5-policy-template", 0
    return hashlib.sha256(encoded(value)).hexdigest()


def pool_inventory(root, manifest):
    pool = root / "bins/spc-pool"
    require(pool.is_dir() and not pool.is_symlink(), "pool source tree missing/indirect")
    files = {}
    for path in pool.rglob("*"):
        require(not path.is_symlink(), "indirect pool source cannot establish control absence")
        if path.is_file():
            files[str(path.relative_to(root))] = sha(path)
    require(POOL_BASE <= set(files), "pool source class lacks required implementation files")
    indexed = {r["path"]: r["sha256"] for r in manifest["files"] if r["path"].startswith("bins/spc-pool/")}
    require(indexed == files, "actual pool source census differs from joined manifest")
    return files


def source_class(root, files, paths, bundle):
    present = any((root / name).exists() for name in CONTROL_PATHS)
    markers = re.compile(r"wallet[_-]control|pool-wallet-executor|board_proof_v1|prepare_datadir")
    for name in files:
        if name.endswith((".rs", ".toml")) and markers.search((root / name).read_text()):
            present = True
    # A client installed alongside an admitted app also prevents a no-control
    # classification, even if its source was omitted from a purported manifest.
    installed = any((path.parent / "pool-wallet-executor").exists() for path in paths.values())
    if installed and not present:
        require(False, "installed control client is unbound to actual pool source")
    if "pool_wallet_executor" in bundle and not present:
        require(False, "declared control executable does not match no-control pool source")
    return "wallet-control" if present else "legacy-no-wallet-control"


def runtime_bindings(bundle):
    return {"node_sha256": bundle["binaries"]["node"]["sha256"],
            "server_sha256": bundle["binaries"]["captaind"]["sha256"],
            "probe_sha256": bundle["binaries"]["probe"]["sha256"],
            "scenario_template_sha256": scenario_template_sha256(), "history_version": 3,
            "global_hashes_per_second": 40, "fresh_registration_before_workers": True,
            "host_peering_before_profile_binding": True, "bootstrap_and_workers_single_budget": True}


def check_source_scope(bundle, validation, manifest, root, source_identity, paths, full):
    files = pool_inventory(root, manifest)
    kind = source_class(root, files, paths, bundle)
    if kind == "wallet-control":
        require("pool_control_source_policy" in bundle, "control code present: corrected bound source policy required in every scope")
        control = load(artifact(bundle["pool_control_source_policy"]))
        require(control.get("source_root") == source_identity
                and control.get("source_manifest_sha256") == bundle["joined_source_manifest"]["sha256"], "control policy covers a different actual source/root")
        require(all(control.get("files", {}).get(name) == value for name, value in files.items()), "control policy omits/mismatches actual pool source rows")
        verify_source(root, manifest, control["files"])
        require(control.get("compiled") is True and control.get("tests_passed") is True, "control source is not tested/compiled")
        validate_pool_control_admission(control)
        require(control.get("checks") and all(type(c.get("exit")) is int and c["exit"] == 0 for c in control["checks"]),
                "corrected control source requires actual passing check receipts in every scope")
        for check in control["checks"]:
            artifact(check["log"])
    else:
        require("pool_control_source_policy" not in bundle, "unrelated control policy attached to actual no-control source")
        require(not full, "full AP5 requires the corrected new wallet-control implementation")
    if full:
        return kind
    from received_profile import supports, check
    if supports(bundle):
        require(kind == "legacy-no-wallet-control", "received no-control profile contains control code")
        check(bundle, validation, manifest, root, source_identity, files, paths)
        return "received-legacy-no-wallet-control"
    require(bundle.get("scope") == "confirmed-funding", "diagnostic scope must be explicit in the executable bundle")
    require("diagnostic_source_policy" in bundle, "exact tested diagnostic source policy required")
    policy = load(artifact(bundle["diagnostic_source_policy"]))
    require(type(policy) is dict and set(policy) == POLICY_FIELDS, "closed diagnostic source policy shape")
    require(policy["schema"] == "spc.lab.ap5.confirmed-funding-source-policy/v1"
            and policy["scope"] == "confirmed-funding" and policy["source_class"] == kind, "diagnostic policy/source class mismatch")
    require(policy["source_root"] == source_identity
            and policy["source_manifest_sha256"] == bundle["joined_source_manifest"]["sha256"]
            and policy["joined_validation_sha256"] == bundle["joined_validation_receipt"]["sha256"], "diagnostic policy binds a different source/root/gate")
    require(policy["app_binaries"] == {name: bundle["binaries"][name]["sha256"] for name in APPS}, "diagnostic six-app binary binding mismatch")
    require(policy["pool_files"] == files, "diagnostic policy does not bind exact actual pool source census")
    absent = list(CONTROL_PATHS) if kind == "legacy-no-wallet-control" else []
    require(policy["absent_control_paths"] == absent
            and policy["absent_control_executables"] == (["pool-wallet-executor"] if absent else []), "diagnostic control absence declaration mismatch")
    require(encoded(policy["limits"]) == encoded(LIMITS), "diagnostic may accept only native confirmed funding")
    require(encoded(policy["runtime_bindings"]) == encoded(runtime_bindings(bundle)), "diagnostic node/probe/budget/registration/profile binding mismatch")
    gate = load(artifact(policy["joined_gate"]))
    if gate.get("schema") == "spc.lab.ap5.joined-350-gate/v1":
        from gate_inventory import check_qualified_350
        check_qualified_350(gate, validation, bundle, root, manifest, source_identity)
    else:
        from gate_inventory import check_historical_332_source
        check_historical_332_source(gate, source_identity)
        require(set(gate) == {"schema", "source_manifest_sha256", "checks_sha256", "passed", "failed", "evidence"}, "joined 332 gate shape")
        require(gate["schema"] == "spc.lab.ap5.joined-332-gate/v1" and type(gate["passed"]) is int and gate["passed"] == 332
                and type(gate["failed"]) is int and gate["failed"] == 0
                and gate["source_manifest_sha256"] == policy["source_manifest_sha256"]
                and gate["checks_sha256"] == hashlib.sha256(encoded(validation["checks"])).hexdigest(), "diagnostic requires the exact successful joined 332 gate")
    require(gate["evidence"], "joined 332 gate lacks raw evidence")
    for value in gate["evidence"]:
        artifact(value)
    maintenance = policy["legacy_maintenance"]
    require(set(maintenance) == {"mode", "max_runtime_secs", "native_transfer_boarding", "automatic_coinbase_refill", "coverage"}, "diagnostic maintenance policy shape")
    require(maintenance["mode"] == "enabled-source-tested-observation-only"
            and maintenance["native_transfer_boarding"] == "not-in-legacy-height-scan"
            and maintenance["automatic_coinbase_refill"] == "may-run-no-ark-acceptance"
            and type(maintenance["max_runtime_secs"]) is int and 0 < maintenance["max_runtime_secs"] <= 7200,
            "legacy maintenance must be explicit and runtime-bounded")
    coverage = load(artifact(maintenance["coverage"]))
    require(set(coverage) == {"schema", "source_manifest_sha256", "pool_files", "covered_paths", "check_purpose", "log", "conclusion"}, "legacy maintenance coverage shape")
    require(coverage["schema"] == "spc.lab.ap5.legacy-maintenance-coverage/v1"
            and coverage["source_manifest_sha256"] == policy["source_manifest_sha256"]
            and coverage["pool_files"] == files
            and coverage["covered_paths"] == ["initial-sync", "periodic-maintenance", "matured-coinbase-refill", "failure-retry"]
            and coverage["conclusion"] == "supported-for-bounded-native-funding-observation", "actual reachable legacy maintenance paths lack diagnostic coverage")
    check = next((c for c in validation["checks"] if c.get("purpose") == coverage["check_purpose"] == "pool-ark-tests"), None)
    require(check is not None and coverage["log"] == check["log"], "legacy maintenance coverage is not bound to the actual joined Ark tests")
    text = artifact(coverage["log"]).read_text()
    require(any(line.startswith("test ") and "ark::board::" in line and line.rstrip().endswith(" ... ok") for line in text.splitlines()),
            "reachable legacy refill path has no executed bound test evidence")
    return kind
