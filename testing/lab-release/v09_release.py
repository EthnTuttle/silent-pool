#!/usr/bin/env python3
"""Independent v0.9+L2 release diagnostics, reusing the approved LP-live gate."""

from __future__ import annotations

import argparse
import base64
import http.client
import json
import os
from pathlib import Path
import secrets
import signal
import socket
import subprocess
import sys
import time

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))
from independent_checks import check_namespace_artifacts, check_static_reference, compare_profile, load_and_check_lp_summary
from release_lib import (
    OwnedProcess, RunDirectory, ValidationError, absolute_dir, absolute_file,
    atomic_write, full_hash, load_json, require, secure_tree, sha256_bytes,
    sha256_file, stop_owned_token, token_survivors, utc_now, verify_approval, verify_binary,
    verify_nix_provenance, verify_source_manifest,
)

BASE = "b0ab4ca34fda07c61a6335d05f598eaacc69678c"
NIX_NAMES = {"spc-lab", "spc-pool", "spc-cpu-miner", "spc-scan"}


def checked_json_object(path: Path, allowed: set[str]) -> dict:
    value = load_json(path)
    require(isinstance(value, dict), "input_shape", f"{path} must contain a JSON object")
    unknown = sorted(set(value) - allowed)
    require(not unknown, "input_unknown_fields", f"unknown fields in {path}: {unknown}")
    return value


class InputGate:
    def __init__(self, config_path: Path, contract: dict, release_check: bool):
        self.config_path = config_path.resolve()
        self.contract = contract
        self.release_check = release_check
        self.config = checked_json_object(config_path, {
            "schema", "handoff_id", "node", "stock_node", "l2_reference", "lp_live",
            "pool", "miner", "scanner", "runtime", "storage",
        })
        require(self.config.get("schema") == "spc.lab.v09_release_inputs.v1", "input_schema", "unsupported candidate input schema")
        require(isinstance(self.config.get("handoff_id"), str) and self.config["handoff_id"],
                "handoff_required", "an explicit parent candidate handoff_id is required", "not_run")
        self.records = {}
        self.binaries = {}
        self.nix = {}

    def _component(self, name: str, needs_binary: bool = True, approval_required: bool = True) -> dict:
        row = self.config[name]
        require(isinstance(row, dict), "input_shape", f"{name} must be an object")
        source_dir = absolute_dir(row.get("source_dir"), f"{name}.source_dir")
        source_manifest = absolute_file(row.get("source_manifest"), f"{name}.source_manifest")
        snapshot = full_hash(row.get("snapshot_sha256"), f"{name}.snapshot_sha256")
        source = verify_source_manifest(source_dir, source_manifest, snapshot)
        record = {"source": source}
        if needs_binary:
            binary = absolute_file(row.get("binary"), f"{name}.binary", executable=True)
            record["binary"] = verify_binary(binary, full_hash(row.get("binary_sha256"), f"{name}.binary_sha256"))
            self.binaries[name] = record["binary"]
        approval_path = row.get("approval_record")
        if approval_required or approval_path:
            approval = verify_approval(absolute_file(approval_path, f"{name}.approval_record"), snapshot, name, approval_required)
            record["approval"] = approval
        else:
            record["approval"] = {"required": False, "status": "not_run"}
        self.records[name] = record
        return record

    def _verify_handoff(self, node_source: dict, node_binary: dict) -> dict:
        row = self.config["node"]
        path = absolute_file(row.get("build_provenance"), "node.build_provenance")
        handoff = checked_json_object(path, {
            "schema", "status", "official_commit", "old_l2_snapshot", "source_dir", "source_manifest",
            "source_snapshot_sha256", "binary", "binary_sha256", "build_log", "build_log_sha256",
            "port_report", "port_report_sha256", "porter_test_note",
        })
        expected = {
            "schema": "spc.lab.v09_node_handoff.v1", "status": "ready_for_independent_validation",
            "official_commit": self.contract["release"]["official_commit"],
            "old_l2_snapshot": self.contract["release"]["old_l2_snapshot"],
            "source_dir": node_source["source_dir"], "source_manifest": node_source["manifest_path"],
            "source_snapshot_sha256": node_source["snapshot_sha256"], "binary": node_binary["path"],
            "binary_sha256": node_binary["sha256"], "build_log_sha256": sha256_file(absolute_file(handoff.get("build_log"), "handoff.build_log")),
            "port_report_sha256": sha256_file(absolute_file(handoff.get("port_report"), "handoff.port_report")),
        }
        for key, wanted in expected.items():
            require(handoff.get(key) == wanted, "handoff_binding", f"handoff {key} differs: {handoff.get(key)!r} != {wanted!r}")
        return {"path": str(path), "sha256": sha256_file(path), "verified": True,
                "scope": "parent/porter build association; not independently rebuilt or stability-approved"}

    def prepare(self) -> dict:
        for name in ["pool", "miner", "scanner", "l2_reference", "lp_live"]:
            self._component(name, needs_binary=name in {"pool", "miner", "scanner"}, approval_required=name != "scanner")
        lp_snapshot = self.contract["lp_live"]["snapshot_sha256"]
        l2_snapshot = self.contract["release"]["old_l2_snapshot"]
        require(self.records["lp_live"]["source"]["snapshot_sha256"] == lp_snapshot,
                "lp_live_snapshot", "LP-live input is not the approved AA3 snapshot")
        require(self.records["l2_reference"]["source"]["snapshot_sha256"] == l2_snapshot,
                "l2_reference_snapshot", "old L2 reference changed")
        node = self.config["node"]
        node_snapshot = full_hash(node.get("snapshot_sha256"), "node.snapshot_sha256")
        require(node_snapshot != l2_snapshot, "candidate_identity", "old L2 snapshot cannot identify the v0.9 candidate")
        node_source_dir = absolute_dir(node.get("source_dir"), "node.source_dir")
        node_manifest_path = absolute_file(node.get("source_manifest"), "node.source_manifest")
        node_manifest = load_json(node_manifest_path)
        require(node_manifest.get("base") == self.contract["release"]["official_commit"],
                "candidate_base", "candidate manifest base is not the official v0.9.0 commit")
        node_source = verify_source_manifest(node_source_dir, node_manifest_path, node_snapshot)
        node_binary = absolute_file(node.get("binary"), "node.binary", executable=True)
        binary_record = verify_binary(node_binary, full_hash(node.get("binary_sha256"), "node.binary_sha256"))
        node_approval_path = node.get("approval_record")
        node_record = {
            "source": node_source, "binary": binary_record,
            "handoff": self._verify_handoff(node_source, binary_record),
            "approval": verify_approval(absolute_file(node_approval_path, "node.approval_record"), node_snapshot, "node", self.release_check)
            if self.release_check or node_approval_path else {"required": self.release_check, "status": "not_run", "reason": "candidate Fable review pending"},
        }
        if node_record["approval"].get("status") == "approved":
            review_text = Path(node_record["approval"]["approval_record"]).read_text(encoding="utf-8")
            require(binary_record["sha256"][:8] in review_text, "node_approval_binary_scope",
                    "node approval does not name the handed-off binary hash prefix")
        self.records["node"] = node_record
        self.binaries["node"] = binary_record
        self._runtime()
        stock = self.config.get("stock_node") or {}
        if stock.get("binary"):
            binary = absolute_file(stock["binary"], "stock_node.binary", executable=True)
            record = verify_binary(binary, full_hash(stock.get("binary_sha256"), "stock_node.binary_sha256"))
            self.records["stock_node"] = {"binary": record, "status": "bound"}
            self.binaries["stock_node"] = record
        else:
            self.records["stock_node"] = {"status": "not_run", "reason": "no separate frozen official-stock binary supplied; static source absence remains available"}
        storage = self.config["storage"]
        self.storage = {
            "postgres_bin_dir": absolute_dir(storage.get("postgres_bin_dir"), "storage.postgres_bin_dir"),
            "redis_server": absolute_file(storage.get("redis_server"), "storage.redis_server", executable=True),
            "redis_cli": absolute_file(storage.get("redis_cli"), "storage.redis_cli", executable=True),
            "nix_develop": storage.get("nix_develop") is True,
        }
        for tool in ["initdb", "postgres", "createdb", "psql", "pg_isready"]:
            absolute_file(str(self.storage["postgres_bin_dir"] / tool), f"storage.{tool}", executable=True)
        return {"mode": "release-check" if self.release_check else "diagnostic", "inputs": self.records,
                "storage": {key: str(value) for key, value in self.storage.items()},
                "input_gate_complete": self.release_check or all(self.records[name]["approval"].get("status") == "approved"
                                                                  for name in ["node", "runtime"])}

    def _runtime(self) -> None:
        row = self.config["runtime"]
        source_dir = absolute_dir(row.get("source_dir"), "runtime.source_dir")
        manifest_path = absolute_file(row.get("source_manifest"), "runtime.source_manifest")
        full = verify_source_manifest(source_dir, manifest_path, self.contract["namespace"]["full_snapshot_sha256"], "runtime_full")
        code = verify_source_manifest(source_dir, manifest_path, self.contract["namespace"]["code_snapshot_sha256"], "runtime_code")
        provenance_path = absolute_file(row.get("nix_binary_provenance"), "runtime.nix_binary_provenance")
        self.nix = verify_nix_provenance(provenance_path, NIX_NAMES)["records"]
        for name, record in self.nix.items():
            require(record["sha256"] == self.contract["namespace"]["binary_hashes"][name],
                    "runtime_binary_hash", f"{name} hash differs from frozen runtime candidate")
        approval_path = row.get("approval_record")
        approval = verify_approval(absolute_file(approval_path, "runtime.approval_record"),
                                   self.contract["namespace"]["full_snapshot_sha256"], "runtime", self.release_check) \
            if self.release_check or approval_path else {"required": self.release_check, "status": "not_run", "reason": "runtime Fable review pending"}
        if approval.get("status") == "approved":
            review_text = Path(approval["approval_record"]).read_text(encoding="utf-8")
            for record in self.nix.values():
                require(record["sha256"][:8] in review_text, "runtime_approval_binary_scope",
                        f"runtime approval does not name Nix binary hash prefix {record['sha256'][:8]}")
            require(self.contract["namespace"]["code_snapshot_sha256"] in review_text,
                    "runtime_approval_code_scope", "runtime approval does not name the full code aggregate")
        self.records["runtime"] = {"source_full": full, "source_code": code,
                                   "nix_binary_provenance": self.nix, "approval": approval,
                                   "tooling_scope": "host-tool namespace evidence; immutable Nix project binaries and store paths verified"}

    def reverify_after(self) -> None:
        # Re-hash every bound file and executable after the run. Approval verdicts
        # are rechecked too, so edits or revocation during a run fail closed.
        before = json.dumps(self.records, sort_keys=True, separators=(",", ":"))
        self.records = {}
        self.binaries = {}
        self.nix = {}
        self.prepare()
        after = json.dumps(self.records, sort_keys=True, separators=(",", ":"))
        require(before == after, "post_run_provenance", "a source, binary, handoff, or approval record changed during validation")


class RpcClient:
    def __init__(self, port: int, cookie: Path):
        user, password = cookie.read_text().rstrip("\n").split(":", 1)
        self.auth = base64.b64encode(f"{user}:{password}".encode()).decode()
        self.port = port
        self.sequence = 0

    def call(self, method: str, params: list | None = None):
        self.sequence += 1
        body = json.dumps({"jsonrpc": "2.0", "id": self.sequence, "method": method, "params": params or []})
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=8)
        try:
            connection.request("POST", "/", body, {"Authorization": f"Basic {self.auth}", "Content-Type": "application/json"})
            response = connection.getresponse()
            payload = response.read()
        finally:
            connection.close()
        require(response.status == 200, "rpc_http", f"{method}: HTTP {response.status}: {payload[:300]!r}")
        value = json.loads(payload)
        require(value.get("error") is None, "rpc_error", f"{method}: {value.get('error')}")
        return value.get("result")


def reserve_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def run_owned(name: str, argv: list[str], cwd: Path, log_path: Path, token: str,
              timeout: int, extra_env: dict[str, str] | None = None) -> dict:
    process = OwnedProcess(name, argv, cwd, log_path, token, extra_env)
    timed_out = False
    try:
        deadline = time.monotonic() + timeout
        while process.popen.poll() is None:
            if time.monotonic() >= deadline:
                timed_out = True
                process.signal(signal.SIGTERM)
                grace = time.monotonic() + 30
                while process.popen.poll() is None and time.monotonic() < grace:
                    time.sleep(0.1)
                if process.popen.poll() is None:
                    process.signal(signal.SIGKILL)
                break
            time.sleep(0.1)
        returncode = process.popen.wait(timeout=10)
    finally:
        process.close_log()
        survivors = token_survivors(token)
        if survivors:
            stop_owned_token(token)
    require(not timed_out, "stage_timeout", f"{name} exceeded {timeout}s; owned-token cleanup completed", "unknown")
    return {"argv": argv, "cwd": str(cwd), "returncode": returncode, "log_path": str(log_path),
            "log_sha256": sha256_file(log_path), "process_identity": process.identity.as_dict()}


def negative_profile_cases(binary: Path, contract: dict, stage: Path, token: str) -> list[dict]:
    results = []
    for case in contract["negative_profile_cases"]:
        case_dir = stage / "negative" / case["id"]
        data_dir = case_dir / "data"
        data_dir.mkdir(parents=True, mode=0o700)
        cookie = case_dir / "rpc.cookie"
        atomic_write(cookie, f"spc:{secrets.token_hex(24)}")
        api_log = case_dir / "api.jsonl"
        argv = [str(binary), *case["args"], "--datadir", str(data_dir), "--no-listen", "--no-seeds",
                "--no-discover", "--max-outbound", "1", "--rpc", "--rpc-listen", "127.0.0.1:0",
                "--rpc-cookie-file", str(cookie), "--api-log", str(api_log)]
        process = OwnedProcess(case["id"], argv, stage, case_dir / "process.log", token)
        timed_out = False
        try:
            try:
                returncode = process.popen.wait(timeout=10)
            except subprocess.TimeoutExpired:
                timed_out = True
                process.signal(signal.SIGTERM)
                try:
                    process.popen.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.signal(signal.SIGKILL)
                    process.popen.wait(timeout=5)
        finally:
            process.close_log()
            if token_survivors(token):
                stop_owned_token(token)
        output = (case_dir / "process.log").read_text(errors="replace")
        require(not timed_out and returncode != 0, "negative_case_started",
                f"{case['id']} was not refused before bounded timeout")
        require(case["diagnostic"].lower() in output.lower(), "negative_diagnostic",
                f"{case['id']} failed for an unclear/wrong reason; wanted {case['diagnostic']!r}, got {output[-800:]!r}")
        results.append({"id": case["id"], "status": "passed", "returncode": returncode,
                        "diagnostic": case["diagnostic"], "log_sha256": sha256_file(case_dir / "process.log")})
    return results


def positive_profile_probe(binary: Path, profile_name: str, overlay: str | None,
                           contract: dict, stage: Path, token: str, consensus_present: bool = True) -> dict:
    probe_dir = stage / "positive" / profile_name / ("stock" if not consensus_present else "candidate")
    data_dir = probe_dir / "data"
    data_dir.mkdir(parents=True, mode=0o700)
    cookie = probe_dir / "rpc.cookie"
    atomic_write(cookie, f"spc:{secrets.token_hex(24)}")
    require(stat_mode(cookie) == 0o600, "cookie_mode", "RPC cookie is not 0600")
    port = reserve_port()
    argv = [str(binary), "--network", "regtest", "--datadir", str(data_dir), "--rpc", "--rpc-listen",
            f"127.0.0.1:{port}", "--rpc-cookie-file", str(cookie)]
    if profile_name == "retarget_regtest":
        argv.append("--retarget-regtest")
    elif overlay:
        argv.extend(["--regtest-pow-bits", overlay])
    argv.extend(["--no-listen", "--no-seeds", "--no-discover", "--max-outbound", "1",
                 "--api-log", str(probe_dir / "api.jsonl"), "--log-level", "info"])
    process = OwnedProcess(f"profile-{profile_name}", argv, probe_dir, probe_dir / "node.log", token)
    rpc = RpcClient(port, cookie)
    try:
        deadline = time.monotonic() + 45
        last_error = None
        while time.monotonic() < deadline:
            require(process.popen.poll() is None, "node_start", f"node exited during RPC readiness; see {probe_dir / 'node.log'}")
            try:
                info = rpc.call("getblockchaininfo")
                break
            except (OSError, ValueError, ValidationError, http.client.HTTPException) as error:
                last_error = error
                time.sleep(0.1)
        else:
            raise ValidationError("rpc_readiness", f"RPC was not ready in 45s: {last_error}", "unknown")
        require(info.get("chain") == "regtest" and info.get("blocks") == 0,
                "fresh_regtest", f"probe was not a fresh regtest: {info}")
        if consensus_present:
            compare_profile(info.get("consensus"), dict(contract["profiles"][profile_name], **({"regtest_pow_bits_overlay": overlay} if overlay else {})), profile_name)
        else:
            require("consensus" not in info, "stock_unexpected_consensus", "stock official node unexpectedly exposes consensus")
        genesis = rpc.call("getblockhash", [0])
        template = rpc.call("getblocktemplate", [{"rules": ["segwit"]}])
        expected_bits = "1e7fffff" if profile_name == "retarget_regtest" else overlay
        allowed_template_bits = {expected_bits}
        if profile_name == "default_regtest":
            # The approved old semantics explicitly allow pow-limit bits when
            # the real-clock gap exceeds two spacings (normally height 1).
            allowed_template_bits.add(contract["profiles"][profile_name]["pow_limit_bits"])
        require(template.get("height") == 1 and template.get("bits") in allowed_template_bits,
                "initial_template", f"initial template violates overlay/min-gap semantics: {template.get('height')}, {template.get('bits')}")
        atomic_write(probe_dir / "observed-profile.json", {"getblockchaininfo": info, "genesis_rpc": genesis,
                                                           "initial_template": {"height": template.get("height"), "bits": template.get("bits"),
                                                                                "allowed_by_profile": sorted(allowed_template_bits)}})
        try:
            rpc.call("stop")
        except (OSError, ValidationError):
            pass
        try:
            returncode = process.popen.wait(timeout=25)
        except subprocess.TimeoutExpired:
            process.signal(signal.SIGTERM)
            returncode = process.popen.wait(timeout=10)
        require(returncode == 0, "node_shutdown", f"profile node did not stop cleanly: {returncode}")
    finally:
        if process.popen.poll() is None:
            process.signal(signal.SIGTERM)
            try:
                process.popen.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.signal(signal.SIGKILL)
        process.close_log()
        if token_survivors(token):
            stop_owned_token(token)
    return {"profile": profile_name, "consensus_present": consensus_present, "status": "passed",
            "fresh_datadir": True, "initial_template_bits": template.get("bits"),
            "allowed_initial_template_bits": sorted(allowed_template_bits),
            "observed_profile_sha256": sha256_file(probe_dir / "observed-profile.json"),
            "node_log_sha256": sha256_file(probe_dir / "node.log")}


def stat_mode(path: Path) -> int:
    return path.stat().st_mode & 0o777


class ReleaseRunner:
    def __init__(self, gate: InputGate, root: RunDirectory):
        self.gate = gate
        self.contract = gate.contract
        self.root = root
        self.results = {}

    def _stage_dir(self, name: str) -> Path:
        path = self.root.private / "stages" / name
        path.mkdir(parents=True, exist_ok=False, mode=0o700)
        return path

    def _node_version(self, stage: Path) -> dict:
        path = stage / "version.log"
        with path.open("xb") as output:
            os.chmod(path, 0o600)
            result = subprocess.run([self.gate.binaries["node"]["path"], "--version"], stdout=output,
                                    stderr=subprocess.STDOUT, timeout=20, check=False)
        text = path.read_text(errors="replace")
        require(result.returncode == 0 and self.contract["release"]["required_version_text"] in text,
                "candidate_version", f"binary does not identify official release 0.9.0: {text!r}")
        return {"text": text.strip(), "log_sha256": sha256_file(path)}

    def profile(self) -> dict:
        stage = self._stage_dir("profile")
        token = self.root.token
        version = self._node_version(stage)
        help_path = stage / "help.log"
        with help_path.open("xb") as output:
            os.chmod(help_path, 0o600)
            result = subprocess.run([self.gate.binaries["node"]["path"], "--help"], stdout=output,
                                    stderr=subprocess.STDOUT, timeout=20, check=False)
        help_text = help_path.read_text(errors="replace")
        require(result.returncode == 0 and "--retarget-regtest" in help_text and "--regtest-pow-bits" in help_text,
                "candidate_cli", "candidate does not advertise both lab profile interfaces")
        negatives = negative_profile_cases(Path(self.gate.binaries["node"]["path"]), self.contract, stage, token)
        baseline = positive_profile_probe(Path(self.gate.binaries["node"]["path"]), "default_regtest", "200fffff",
                                          self.contract, stage, token)
        retarget = positive_profile_probe(Path(self.gate.binaries["node"]["path"]), "retarget_regtest", None,
                                          self.contract, stage, token)
        stock_result = self.gate.records["stock_node"]
        if stock_result.get("status") == "bound":
            stock_binary = Path(stock_result["binary"]["path"])
            stock_probe = positive_profile_probe(stock_binary, "default_regtest", "200fffff",
                                                 self.contract, stage, token, consensus_present=False)
            stock_probe["stock_lab_flag_refusal"] = self._stock_refusal(stock_binary, stage, token)
            stock_result = stock_probe
        return {"version": version, "help_sha256": sha256_file(help_path), "negative_cases": negatives,
                "default_rpc": baseline, "retarget_rpc": retarget, "stock_probe": stock_result,
                "status": "passed" if stock_result.get("status") == "passed" else "incomplete"}

    def _stock_refusal(self, binary: Path, stage: Path, token: str) -> dict:
        # Nonzero exit plus the echoed argument is not proof of refusal. Bind
        # the unsupported flag to both stock help absence and a real parser
        # diagnostic, exactly as the official v0.9 binary implements it.
        help_path = stage / "stock-help.log"
        stock_help = subprocess.run([str(binary), "--help"], stdout=subprocess.PIPE,
                                    stderr=subprocess.STDOUT, timeout=20, check=False)
        atomic_write(help_path, stock_help.stdout)
        help_text = stock_help.stdout.decode(errors="replace")
        require(stock_help.returncode == 0 and "--regtest-pow-bits" in help_text and
                "--retarget-regtest" not in help_text,
                "stock_help_profile", "stock help must have regtest pow bits and no retarget-regtest flag")
        case = {"id": "stock-missing-lab-profile", "args": ["--network", "regtest", "--retarget-regtest"],
                "diagnostic": "retarget"}
        original = self.contract["negative_profile_cases"]
        self.contract["negative_profile_cases"] = [case]
        try:
            record = negative_profile_cases(binary, self.contract, stage / "stock-cli", token)[0]
        finally:
            self.contract["negative_profile_cases"] = original
        refusal_log = stage / "stock-cli" / "negative" / case["id"] / "process.log"
        refusal = refusal_log.read_text(errors="replace").lower()
        reasons = ["unknown argument", "unrecognized argument", "unexpected argument",
                   "unknown option", "requires a value"]
        reason = next((text for text in reasons if text in refusal), None)
        require(reason is not None, "stock_refusal_reason",
                f"stock lab refusal has no recognized parser reason: {refusal[-800:]!r}")
        record.update({"help_retarget_absent": True, "help_sha256": sha256_file(help_path),
                       "refusal_reason": reason, "not_mere_argument_echo": True})
        return record

    def _lp_argv(self, name: str, destination: Path) -> list[str]:
        cfg = self.gate.config
        lp_root = Path(self.gate.records["lp_live"]["source"]["source_dir"])
        driver = lp_root / self.contract["lp_live"]["driver"]
        scenario = lp_root / self.contract["lp_live"][name]
        command = [str(driver), "--provisional", "--scenario", str(scenario), "--run-dir", str(destination),
                   "--pool-binary", self.gate.binaries["pool"]["path"], "--miner-binary", self.gate.binaries["miner"]["path"],
                   "--scanner-binary", self.gate.binaries["scanner"]["path"], "--node-binary", self.gate.binaries["node"]["path"],
                   "--pool-source-dir", cfg["pool"]["source_dir"], "--pool-source-manifest", cfg["pool"]["source_manifest"],
                   "--pool-approval-record", cfg["pool"]["approval_record"], "--miner-source-dir", cfg["miner"]["source_dir"],
                   "--miner-source-manifest", cfg["miner"]["source_manifest"], "--miner-approval-record", cfg["miner"]["approval_record"],
                   "--scanner-source-dir", cfg["scanner"]["source_dir"], "--scanner-source-manifest", cfg["scanner"]["source_manifest"],
                   "--node-source-dir", cfg["node"]["source_dir"], "--postgres-bin-dir", str(self.gate.storage["postgres_bin_dir"]),
                   "--redis-server", str(self.gate.storage["redis_server"]), "--redis-cli", str(self.gate.storage["redis_cli"])]
        # LP-live's internal acceptance pin is intentionally the OLD L2 hash.
        # Do not alter/bypass it: every new candidate run is provisional and the
        # outer gate binds the new manifest before/after. Baseline has no old-L2
        # special case, so its manifest can also be passed to LP-live directly.
        if name == "baseline":
            command.extend(["--node-source-manifest", cfg["node"]["source_manifest"]])
        if self.gate.storage["nix_develop"]:
            command = ["nix", "develop", "-c", "python3", *command]
        else:
            command = [sys.executable, *command]
        return command

    def lp_stage(self, name: str) -> dict:
        stage = self._stage_dir(name)
        lp_root = Path(self.gate.records["lp_live"]["source"]["source_dir"])
        # PostgreSQL's Unix socket path limit is 107 bytes. Keep the approved
        # driver's run root short instead of nesting it under private/stages.
        destination = Path(str(self.root.root) + "-lp")
        require(not destination.exists(), "live_destination_exists", f"LP destination already exists: {destination}")
        argv = self._lp_argv(name, destination)
        scenario = load_json(lp_root / self.contract["lp_live"][name])
        # LP-live gives each height wait and the final settlement phase its own
        # scenario_timeout_secs. Retarget has three height waits plus settlement;
        # baseline has one production wait plus settlement. Bound their sum
        # instead of aborting a valid run during its last height wait.
        bounded_phases = 4 if name == "retarget" else 2
        timeout = (bounded_phases * scenario["scenario_timeout_secs"] +
                   scenario["startup_timeout_secs"] + scenario["shutdown_timeout_secs"] + 180)
        result = run_owned(f"lp-{name}", argv, lp_root, stage / "driver.log", self.root.token, timeout,
                           {"NIX_BUILD_CORES": "2", "CARGO_BUILD_JOBS": "2"})
        summary_path = destination / "public" / "summary.json"
        summary = load_json(summary_path) if summary_path.exists() else {}
        require(result["returncode"] == 0 and summary.get("status") == "passed",
                "lp_live_failed", f"approved LP-live {name} failed: rc={result['returncode']} error={summary.get('error')}; private log={stage / 'driver.log'}")
        observed = load_and_check_lp_summary(destination, self.gate.binaries["node"]["sha256"], name == "retarget")
        derived = None
        if name == "retarget":
            derived_dir = stage / "independent-replay"
            derive = [str(lp_root / self.contract["lp_live"]["driver"]), "--derive-retarget-from", str(destination),
                      "--derived-output-dir", str(derived_dir)]
            if self.gate.storage["nix_develop"]:
                derive = ["nix", "develop", "-c", "python3", *derive]
            else:
                derive = [sys.executable, *derive]
            replay = run_owned("retarget-replay", derive, lp_root, stage / "replay.log", self.root.token, 180)
            require(replay["returncode"] == 0, "retarget_replay", "approved independent historical replay failed")
            derived = {"output_dir": str(derived_dir), "replay_log_sha256": replay["log_sha256"],
                       "artifacts": {path.name: sha256_file(path) for path in sorted(derived_dir.glob("*.json"))}}
        secure_tree(destination)
        return {"status": "passed", "driver_execution": result, "observations": observed, "independent_replay": derived,
                "candidate_scope": "provisional v0.9+L2 validation; LP-live's internal old-L2 acceptance was not bypassed or inherited"}

    def namespace(self) -> dict:
        stage = self._stage_dir("namespace")
        source_root = Path(self.gate.records["runtime"]["source_full"]["source_dir"])
        scenario = source_root / self.contract["namespace"]["scenario"]
        require(sha256_file(scenario) == next(row["sha256"] for row in load_json(Path(self.gate.config["runtime"]["source_manifest"]))["files"]
                                               if row["path"] == self.contract["namespace"]["scenario"]),
                "namespace_scenario_source", "scenario drift from frozen runtime source")
        lab = Path(self.gate.nix["spc-lab"]["path"])
        validate = subprocess.run([str(lab), "validate", "--scenario", str(scenario)], cwd=source_root,
                                  stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60, check=False)
        atomic_write(stage / "validate.log", validate.stdout)
        require(validate.returncode == 0, "namespace_validate", f"runtime scenario validation failed: {validate.stdout[-1000:]!r}")
        destination = Path(str(self.root.root) + "-rt")
        require(not destination.exists(), "live_destination_exists", f"runtime destination already exists: {destination}")
        argv = [str(lab), "runtime", "--scenario", str(scenario), "--run-dir", str(destination),
                "--node", self.gate.binaries["node"]["path"], "--pool", self.gate.nix["spc-pool"]["path"],
                "--miner", self.gate.nix["spc-cpu-miner"]["path"], "--scanner", self.gate.nix["spc-scan"]["path"],
                "--postgres-bin-dir", str(self.gate.storage["postgres_bin_dir"]), "--redis-server", str(self.gate.storage["redis_server"]),
                "--redis-cli", str(self.gate.storage["redis_cli"]), "--pow-bits", self.contract["namespace"]["pow_bits"],
                "--base-revision", self.contract["namespace"]["base_revision"],
                "--dirty-tree-hash", self.contract["namespace"]["code_snapshot_sha256"], "--topology",
                "--broker-script", str(source_root / self.contract["namespace"]["broker"]),
                "--listen", "127.0.0.1:0", "--duration-secs", "3"]
        execution = run_owned("namespace-runtime", argv, source_root, stage / "runtime.log", self.root.token, 900)
        reap = subprocess.run([str(lab), "reap", "--run-dir", str(destination)], cwd=source_root,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60, check=False)
        atomic_write(stage / "reap.log", reap.stdout)
        require(reap.returncode == 0, "namespace_reap", f"owned-runtime reap failed: {reap.stdout[-1000:]!r}")
        require(execution["returncode"] == 0, "namespace_failed", f"namespace runtime exited {execution['returncode']}; private log={stage / 'runtime.log'}")
        expected = dict(self.contract["namespace"])
        expected["binary_hashes"] = dict(expected["binary_hashes"], node=self.gate.binaries["node"]["sha256"])
        observed = check_namespace_artifacts(destination, scenario, expected)
        secure_tree(destination)
        return {"status": "passed", "execution": execution, "reap_log_sha256": sha256_file(stage / "reap.log"),
                "observations": observed, "approval_scope": "provisional host-tool diagnostic until runtime exact-snapshot Fable approval"}

    def run(self, stages: list[str]) -> dict:
        for name in stages:
            require(name in self.contract["stage_order"], "stage_dependency", f"unknown stage {name}")
            if name == "profile":
                result = self.profile()
            elif name in {"baseline", "retarget"}:
                # Stages can be independently re-run, but all mode documents its
                # ordered whole-candidate sequence in this same fresh run.
                result = self.lp_stage(name)
            elif name == "namespace":
                result = self.namespace()
            else:
                raise ValidationError("stage", f"unsupported stage {name}")
            self.results[name] = result
            self._write_summary("running")
        return self.results

    def _write_summary(self, status: str, error: ValidationError | None = None) -> dict:
        statuses = [row.get("status") for row in self.results.values()]
        if error is not None:
            overall = "unknown" if error.outcome == "unknown" else "failed"
        elif status == "running":
            overall = "running"
        elif any(value == "failed" for value in statuses):
            overall = "failed"
        elif any(value == "incomplete" for value in statuses):
            overall = "incomplete"
        else:
            overall = "release_checks_passed" if self.gate.release_check else "provisional_pass"
        summary = {
            "schema": "spc.lab.v09_release_result.v1", "observed_at": utc_now(), "base_revision": BASE,
            "candidate_source_snapshot": self.gate.records["node"]["source"]["snapshot_sha256"],
            "candidate_binary_sha256": self.gate.binaries["node"]["sha256"], "official_commit": self.contract["release"]["official_commit"],
            "mode": "release-check" if self.gate.release_check else "diagnostic",
            "status": overall, "requested_stages": list(self.results), "stages": self.results,
            "input_provenance": self.gate.records, "non_claims": self.contract["non_claims"],
            "approval_disclosure": "No deployment or production approval is asserted. Candidate and runtime remain subject to their own Fable reviews.",
            "namespace_tool_disclosure": self.contract["namespace"]["network_tools"],
            "node_stability_disclosure": "Porter-reported shared-load failures are outside these functional observations and are not declared flake-fixed.",
            "artifacts": {"root": str(self.root.root), "private": str(self.root.private), "file_modes": "0600", "directory_modes": "0700"},
            "driver_script_sha256": sha256_file(Path(__file__).resolve()), "contract_sha256": sha256_file(SCRIPT_DIR / "contract.json"),
            "input_file_sha256": sha256_file(self.gate.config_path),
        }
        if error is not None:
            summary["error"] = error.as_dict()
        atomic_write(self.root.public / "summary.json", summary)
        return summary


def check_contract(path: Path) -> dict:
    contract = checked_json_object(path, {"schema", "release", "profiles", "lp_live", "namespace", "static_reference",
                                           "negative_profile_cases", "stage_order", "forbidden_measured_methods", "artifact_policy", "non_claims"})
    require(contract["schema"] == "spc.lab.v09_release_contract.v1", "contract_schema", "unsupported contract schema")
    require(contract["release"]["base_revision"] == BASE, "contract_base", "unexpected base revision")
    require(contract["stage_order"] == ["profile", "baseline", "retarget", "namespace"], "stage_order", "stage order changed")
    for profile_name, profile in contract["profiles"].items():
        require(profile["network"] == "regtest" and profile["difficulty_adjustment_interval"] > 0 and
                profile["pow_target_timespan"] > 0, "contract_profile", f"invalid profile {profile_name}")
    return contract


def freeze_manifest(output: Path) -> dict:
    paths = ["docs/reviews/2026-11-06-lab-v09-validation.md",
             "testing/lab-release/candidate.example.json", "testing/lab-release/contract.json",
             "testing/lab-release/fixtures/namespace-v09-structure.json",
             "testing/lab-release/independent_checks.py", "testing/lab-release/namespace_replay.py",
             "testing/lab-release/release_lib.py", "testing/lab-release/review_contract.md",
             "testing/lab-release/test_release.py", "testing/lab-release/v09_release.py"]
    files = [{"path": name, "sha256": sha256_file(Path(name).resolve())} for name in sorted(paths)]
    snapshot = sha256_bytes(json.dumps({"base": BASE, "files": files}, sort_keys=True, separators=(",", ":")).encode())
    manifest = {"base": BASE, "files": files, "snapshot_sha256": snapshot,
                "aggregate_recipe": "sha256(canonical compact JSON of base plus path/sha256-sorted files)"}
    atomic_write(output, manifest)
    return {"output": str(output), "snapshot_sha256": snapshot, "manifest_sha256": sha256_file(output)}


def make_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--contract", type=Path, default=SCRIPT_DIR / "contract.json")
    parser.add_argument("--inputs", type=Path)
    parser.add_argument("--release-check", action="store_true", help="require new node+runtime exact-snapshot approvals; never imply deployment approval")
    parser.add_argument("--check-contract", action="store_true")
    parser.add_argument("--check-input", action="store_true")
    parser.add_argument("--static-audit", type=Path, metavar="AUDIT_DIR")
    parser.add_argument("--stage", choices=["profile", "baseline", "retarget", "namespace", "all"])
    parser.add_argument("--run-dir", type=Path)
    parser.add_argument("--freeze-manifest", type=Path)
    return parser


def main(argv=None) -> int:
    os.umask(0o077)
    args = make_parser().parse_args(argv)
    try:
        contract = check_contract(args.contract.resolve()) if args.check_contract or args.static_audit or args.stage or args.check_input else load_json(args.contract)
        if args.check_contract:
            print(json.dumps({"status": "passed", "contract_sha256": sha256_file(args.contract)}, sort_keys=True))
            return 0
        if args.static_audit:
            result = check_static_reference(args.static_audit.resolve(), contract)
            print(json.dumps(result, sort_keys=True))
            return 0
        if args.freeze_manifest:
            result = freeze_manifest(args.freeze_manifest.resolve())
            print(json.dumps(result, sort_keys=True))
            return 0
        require(args.inputs is not None, "input_required", "--inputs is required for input/live checks", "not_run")
        gate = InputGate(args.inputs, contract, args.release_check)
        prepared = gate.prepare()
        if args.check_input:
            print(json.dumps(prepared, sort_keys=True))
            return 0
        require(args.stage is not None and args.run_dir is not None, "live_arguments", "--stage and --run-dir are required for live validation", "not_run")
        root = RunDirectory(args.run_dir.resolve())
        atomic_write(root.private / "release-input.json", args.inputs.read_bytes())
        runner = ReleaseRunner(gate, root)
        failure = None
        summary = None
        try:
            stages = contract["stage_order"] if args.stage == "all" else [args.stage]
            runner.run(stages)
        except ValidationError as error:
            failure = error
        finally:
            try:
                gate.reverify_after()
            except Exception as post_error:
                if failure is None:
                    failure = post_error if isinstance(post_error, ValidationError) else ValidationError("post_run_provenance", str(post_error))
            secure_tree(root.root)
            summary = runner._write_summary("complete", failure)
        print(json.dumps({"status": summary["status"], "summary": str(root.public / "summary.json"),
                          "summary_sha256": sha256_file(root.public / "summary.json"), "error": summary.get("error")}, sort_keys=True))
        if summary["status"] in {"provisional_pass", "release_checks_passed"}:
            return 0
        return 2 if summary["status"] in {"incomplete", "unknown"} else 1
    except ValidationError as error:
        print(json.dumps({"status": "not_run" if error.outcome == "not_run" else error.outcome,
                          "error": error.as_dict()}, sort_keys=True), file=sys.stderr)
        return 2 if error.outcome in {"not_run", "unknown"} else 1


if __name__ == "__main__":
    sys.exit(main())
