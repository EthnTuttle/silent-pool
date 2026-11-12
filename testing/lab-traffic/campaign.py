#!/usr/bin/env python3
"""Serial, fresh-state T3/T4b campaign. Reuses sealed binaries, never chain state."""
import argparse
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import stat
import tempfile
import time
from types import SimpleNamespace
import uuid

from case_contract import ORDER, required, semantic_scenario, verdict, canonical_json
from oracle import Inconclusive, Invalid, integer, require, strict_json

HERE = Path(__file__).resolve().parent
PROTECTED = (HERE.parent.parent,)
LOCK = os.environ.get('SPC_LAB_CAMPAIGN_LOCK',str(Path(os.environ.get('XDG_RUNTIME_DIR',tempfile.gettempdir()))/f'silent-pool-{os.getuid()}-acceptance-campaign.lock'))
CLEANUP_SECONDS = 120


class Deadline(BaseException):
    """Bypass per-check Exception recovery so the campaign always enters cleanup."""


def encode(value):
    return canonical_json(value) + b"\n"


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def save(path, value):
    raw = value if isinstance(value, bytes) else encode(value)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(raw); stream.flush(); os.fsync(stream.fileno())
    return {"path": str(path), "sha256": sha(raw), "bytes": len(raw)}


def fresh_root(path):
    path = Path(path).absolute()
    resolved = path.resolve()
    require(all(not (resolved == p or resolved.is_relative_to(p) or p.is_relative_to(resolved)) for p in PROTECTED),
            "campaign output overlaps a protected inspection run")
    require(path == resolved and not path.exists() and not path.is_symlink(), "campaign root must be a fresh nonsymlink path")
    path.mkdir(mode=0o700, parents=False, exist_ok=False)
    return path


def source_seals():
    paths = sorted(HERE.glob("*.py")) + sorted((HERE.parent / "lab-witness").glob("*.py"))
    return {str(p): sha(p.read_bytes()) for p in paths}


def load_controller():
    spec = importlib.util.spec_from_file_location("campaign_controller", HERE / "regtest-traffic-e2e.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def assess_case(receipt, *, case, root, run_id, bindings, binary_hashes):
    require(receipt["schema"] == "spc-lab-funded-acceptance-receipt/2"
            and receipt["scope"] == "campaign-case-only" and receipt["case"] == case,
            "case receipt schema/scope mismatch")
    require(receipt["run_root"] == str(root) and receipt["run_id"] == run_id, "case fresh root/run binding mismatch")
    require(receipt["bindings"] == bindings and receipt["binary_hashes"] == binary_hashes, "case source/input/config binding mismatch")
    obligations = required(case, witness=case == "funded")
    require(receipt["required_cases"] == sorted(obligations), "case required obligations omitted/changed")
    status = verdict(receipt["cases"], obligations)
    require(receipt["status"] == status, "case status contradicts required results")
    return status


def aggregate(cases, ap5):
    statuses = {name: cases.get(name, {"status": "INCONCLUSIVE"}) for name in ORDER}
    t3 = verdict(statuses, set(ORDER))
    whole = verdict({"T3_T4b": {"status": t3}, "AP5": {"status": ap5["status"]}}, {"T3_T4b", "AP5"})
    return {"T3_T4b_status": t3, "AP5_status": ap5["status"], "whole_lab_status": whole,
            "T3_T4b_pass": t3 == "PASSED", "whole_lab_pass": whole == "PASSED"}


def harvest(controller, expected, exit_code):
    path = controller.root / "acceptance.json"
    raw = path.read_bytes()
    receipt = strict_json(raw)
    status = assess_case(receipt, **expected)
    require(exit_code == {"PASSED": 0, "FAILED": 1, "INCONCLUSIVE": 2}[status], "case exit/result mismatch")
    observations = controller.priv / "observations.jsonl"
    require(sha(observations.read_bytes()) == receipt["observations_sha256"], "case observation seal changed")
    # Verify the portable raw canonical evidence independently after actors stop.
    if receipt["cases"].get("chain_evidence_export", {}).get("status") == "PASSED":
        from canonical_capture import validate_export
        ref = receipt["chain_evidence"]
        require(ref["path"] == str(controller.priv / "chain-evidence.json"), "case export escaped fresh root")
        validate_export(ref["path"], controller.run_id, expected["binary_hashes"],
            expected_export_sha256=ref["sha256"], expected_source_snapshot_sha256=controller.binding["source_snapshot_sha256"],
            expected_scenario_sha256=expected["bindings"]["scenario_sha256"], expected_policy=controller.policy,
            expected_node_ids=[n["id"] for n in controller.scenario["nodes"]])
    if receipt["cases"].get("config_binding", {}).get("status") == "PASSED":
        configs = receipt["cases"]["config_binding"]["detail"]["configs"]
        for ref in configs:
            p = Path(ref["path"])
            require(p.is_relative_to(controller.run / "private") and ".." not in p.parts
                    and p.resolve() == p and not p.is_symlink(), "case config path binding")
            require(sha(p.read_bytes()) == ref["sha256"], "rendered case config changed after collection")
    return {"status": status, "receipt": {"path": str(path), "sha256": sha(raw)},
            "required_cases": receipt["required_cases"], "cases": receipt["cases"],
            "cleanup_passed": receipt["cases"].get("cleanup", {}).get("status") == "PASSED"}


def run_case(module, args, record, deadline):
    controller = module.Controller(args)
    controller.capture.add("campaign_case_binding", record)
    def expired(_signum, _frame):
        raise Deadline()
    old = signal.signal(signal.SIGALRM, expired)
    signal.setitimer(signal.ITIMER_REAL, max(0.001, deadline - time.monotonic()))
    cleanup_complete = False
    try:
        try:
            controller.prerequisites()
            controller.start()
            if args.case == "bootstrap-stop":
                controller.stop_bootstrap_case()
            elif args.case == "bootstrap-maturity":
                controller.bootstrap_maturity_case()
            else:
                controller.bootstrap()
                controller.measured()
                controller.reorg()
                controller.audit()
        except Invalid as error:
            controller.result("execution", "FAILED", str(error))
        except (Exception, Deadline) as error:
            controller.capture.add("campaign_case_incomplete", {"class": type(error).__name__})
            controller.result("execution", "INCONCLUSIVE", type(error).__name__)
        finally:
            # Cleanup has its own reserved wall budget, never a new hash budget.
            signal.setitimer(signal.ITIMER_REAL, CLEANUP_SECONDS)
            try:
                code = module.finalize_controller(controller)
                cleanup_complete = True
            except Deadline:
                # No next case can start. Retain this root for its owner/reaper.
                controller.capture.stream.close()
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, old)
    if not cleanup_complete:
        return controller, None
    return controller, code


def execute(args):
    root = fresh_root(args.out)
    cases, ap5 = {}, {"status": "INCONCLUSIVE", "missing_components": ["AP5 actual funded evidence"]}
    report = {"schema": "spc-lab-acceptance-campaign/1", "campaign_id": "campaign-" + uuid.uuid4().hex,
              "root": str(root), "order": list(ORDER), "cases": cases, "AP5": ap5,
              "budget": {"parallel_cases": 1, "hashes_per_second": 40, "ordinary_tx_per_second": "0.2",
                         "case_seconds": args.case_timeout, "cleanup_seconds_per_case": CLEANUP_SECONDS,
                         "total_seconds": args.timeout}, "consumer_sources": source_seals()}
    lock = None
    started = time.monotonic()
    try:
        fd=os.open(LOCK,os.O_RDWR|os.O_CREAT|os.O_NOFOLLOW,0o600)
        lock=os.fdopen(fd,'a+')
        info=os.fstat(fd)
        require(stat.S_ISREG(info.st_mode) and info.st_uid==os.getuid() and info.st_nlink==1
            and stat.S_IMODE(info.st_mode)==0o600,'campaign lock ownership/type/mode')
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise Inconclusive("another serial acceptance campaign owns the execution slot") from error
        inputs_raw, scenario_raw = Path(args.inputs).read_bytes(), Path(args.scenario).read_bytes()
        inputs, scenario = strict_json(inputs_raw), strict_json(scenario_raw)
        cap = integer(scenario["traffic"]["driver"]["bootstrap"]["total_attempts"], 1)
        report["budget"]["bootstrap_attempt_cap_per_case"] = cap
        report["budget"]["sum_of_fresh_case_bootstrap_attempt_caps"] = cap * len(ORDER)
        save(root / "inputs.json", inputs_raw); save(root / "base-scenario.json", scenario_raw)
        report["common_bindings"] = {"inputs_sha256": sha(inputs_raw), "base_scenario_sha256": sha(scenario_raw),
                                     "scenario_semantic_sha256": semantic_scenario(scenario),
                                     "source_snapshot_sha256": inputs["source_snapshot_sha256"],
                                     "policy": inputs["policy"],
                                     "binary_source_manifests": {k: v["source_manifest_sha256"] for k, v in inputs["binaries"].items()},
                                     "binary_hashes": {k: v["sha256"] for k, v in inputs["binaries"].items()}}
        module = load_controller()
        for index, case in enumerate(ORDER):
            if time.monotonic() + args.case_timeout + CLEANUP_SECONDS > started + args.timeout:
                raise Inconclusive("campaign deadline cannot accommodate another bounded case and cleanup")
            require(source_seals() == report["consumer_sources"], "consumer code changed during campaign")
            require((root / "inputs.json").read_bytes() == inputs_raw, "sealed campaign inputs changed")
            spec = strict_json(scenario_raw)
            original_run_id = spec["run"]["id"]
            spec["run"]["id"] = report["campaign_id"] + "-" + case
            for phase in spec["phases"]:
                for command in phase["commands"]:
                    if command.get("run_id") == original_run_id:
                        command["run_id"] = spec["run"]["id"]
            scenario_path = root / (case + "-scenario.json")
            scenario_ref = save(scenario_path, spec)
            case_root = root / (str(index + 1) + "-" + case)
            options = SimpleNamespace(inputs=str(root / "inputs.json"), scenario=str(scenario_path), run_root=str(case_root),
                case=case, timeout=args.case_timeout, start_timeout=args.start_timeout, measure_seconds=args.measure_seconds,
                pause_seconds=args.pause_seconds, max_blocks=args.max_blocks, destination=args.destination,
                amount_sats=args.amount_sats, allow_reorg=True, witness=case == "funded")
            bindings = {"inputs_sha256": sha(inputs_raw), "scenario_sha256": scenario_ref["sha256"],
                        "scenario_semantic_sha256": semantic_scenario(spec), "source_snapshot_sha256": inputs["source_snapshot_sha256"]}
            record = {"campaign_id": report["campaign_id"], "case": case, "root": str(case_root), "run_id": spec["run"]["id"],
                      "bindings": bindings, "options": vars(options), "started_monotonic_ns": time.monotonic_ns()}
            save(root / (case + "-intent.json"), record)
            controller, code = run_case(module, options, record, time.monotonic() + args.case_timeout)
            if code is None:
                cases[case] = {"status": "INCONCLUSIVE", "reason": "cleanup watchdog expired; next cases withheld", "cleanup_passed": False}
                break
            try:
                cases[case] = harvest(controller, {"case": case, "root": case_root, "run_id": spec["run"]["id"],
                    "bindings": bindings, "binary_hashes": report["common_bindings"]["binary_hashes"]}, code)
            except (Exception, Deadline) as error:
                cases[case] = {"status": "FAILED" if isinstance(error, Invalid) else "INCONCLUSIVE",
                               "reason": type(error).__name__, "cleanup_passed": False}
            save(root / (case + "-completion.json"), {**cases[case], "finished_monotonic_ns": time.monotonic_ns()})
            if not cases[case]["cleanup_passed"]:
                break
        require(source_seals() == report["consumer_sources"], "consumer code changed during campaign")
        if getattr(args,"ap5_seal",None):
            from ap5_v2 import inspect
            try:
                ap5.update(inspect(args.ap5_seal,args.ap5_seal_sha256,report["common_bindings"]))
            except Exception as error:
                ap5.update({"status":"FAILED" if isinstance(error,Invalid) else "INCONCLUSIVE","reason":str(error)})
        elif args.ap5_evidence:
            from ap5_observation import inspect
            try:
                ap5.update(inspect(Path(args.ap5_evidence), report["common_bindings"]))
            except Exception as error:
                ap5.update({"status": "FAILED" if isinstance(error, Invalid) else "INCONCLUSIVE", "reason": str(error)})
    except (Exception, Deadline) as error:
        report["execution"] = {"status": "FAILED" if isinstance(error, Invalid) else "INCONCLUSIVE", "reason": str(error)}
    finally:
        for case in ORDER:
            cases.setdefault(case, {"status": "INCONCLUSIVE", "reason": "required case not executed"})
        report.update(aggregate(cases, ap5))
        if report.get("execution"):
            state = verdict({"campaign": {"status": report["T3_T4b_status"]}, "execution": report["execution"]}, {"campaign", "execution"})
            report["T3_T4b_status"], report["T3_T4b_pass"] = state, state == "PASSED"
            whole = verdict({"T3_T4b": {"status": state}, "AP5": ap5}, {"T3_T4b", "AP5"})
            report["whole_lab_status"], report["whole_lab_pass"] = whole, whole == "PASSED"
        report["elapsed_seconds"] = time.monotonic() - started
        save(root / "campaign.json", report)
        if lock is not None:
            lock.close()
    print(json.dumps({"receipt": str(root / "campaign.json"), "T3_T4b_status": report["T3_T4b_status"],
                      "AP5_status": report["AP5_status"], "whole_lab_status": report["whole_lab_status"]}))
    return {"PASSED": 0, "FAILED": 1, "INCONCLUSIVE": 2}[report["whole_lab_status"]]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inputs", required=True); parser.add_argument("--scenario", required=True)
    parser.add_argument("--out", required=True); parser.add_argument("--destination", required=True)
    parser.add_argument("--case-timeout", type=int, default=900); parser.add_argument("--timeout", type=int, default=3300)
    parser.add_argument("--start-timeout", type=int, default=180); parser.add_argument("--measure-seconds", type=int, default=90)
    parser.add_argument("--pause-seconds", type=int, default=5); parser.add_argument("--max-blocks", type=int, default=4096)
    parser.add_argument("--amount-sats", type=int, default=10000)
    parser.add_argument("--ap5-evidence", help="parent-sealed descriptor of the actual AP5 event archive; read-only")
    parser.add_argument("--ap5-seal", help="actual AP5 v2 seal.json; read-only")
    parser.add_argument("--ap5-seal-sha256", help="externally pinned AP5 v2 seal hash")
    args = parser.parse_args()
    require(bool(args.ap5_seal)==bool(args.ap5_seal_sha256) and not(args.ap5_seal and args.ap5_evidence),
            "AP5 v2 requires its external seal hash and cannot mix legacy descriptors")
    for key in ("case_timeout", "timeout", "start_timeout", "measure_seconds", "pause_seconds", "max_blocks", "amount_sats"):
        integer(getattr(args, key), 1)
    os.umask(0o077)
    return execute(args)


if __name__ == "__main__":
    raise SystemExit(main())
