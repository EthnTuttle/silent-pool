#!/usr/bin/env python3
"""Verify one built release closure, using the single canonical r4 probe."""
import argparse
import base64
import copy
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
import urllib.request

from infrastructure import alive, descendants, verify_clusters, verify_redis_boundary, wait_stopped


def load(path):
    return json.loads(path.read_text())


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def save(path, value):
    with path.open("x") as file:
        os.chmod(path, 0o600)
        json.dump(value, file, indent=2)
        file.write("\n")


def main():
    parser = argparse.ArgumentParser()
    for name in ["launcher", "probe", "node", "tools", "out", "binaries"]:
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--canonical-from", type=Path,
                        help="Reuse matching, completed canonical reports without relaunching their nodes")
    args = parser.parse_args()
    args.out.mkdir(mode=0o700)
    sys.path.insert(0, str(args.probe / "share/ark-rpc"))
    from arkrpc.contracts import contract_hash
    from arkrpc.schemadef import validate_report
    from arkrpc.launchprofile import normalize, digest, LaunchProfileError
    from arkrpc.trafficfacts import effective_consensus

    contract = "754e1e6131955f9e5bbce7ef81b6f9aabaa2f1379ec6409f1c3a7a9e88be91f9"
    assert contract_hash() == contract
    node = (args.node / "bin/rbitcoin-node").resolve()
    helper = (args.tools / "bin/spc-lab").resolve()
    node_hash, helper_hash = sha(node), sha(helper)
    assert node_hash == "bba4752298aff258adb6cce99c6db849b02f50736a8ec32766ed1710045051aa"
    binding = {"helper": {"path": str(helper), "sha256": helper_hash},
               "node": {"path": str(node), "sha256": node_hash,
                        "source": load(args.node / "share/rbitcoin-lab/source.json")},
               "probe_contract_sha256": contract, "probe_package": str(args.probe)}
    save(args.out / "binding.json", binding)
    cases = {}
    canonical_root = args.canonical_from or args.out
    for name, extra in [("default", []), ("bark-v31", ["--compat"])]:
        if not args.canonical_from:
            with (args.out / (name + ".log")).open("xb") as log:
                subprocess.run([str(args.probe / "bin/ark-lab-profile-check"), "--case", "cli-profile",
                                "--binding", str(args.out / "binding.json"), "--out", str(args.out / name), *extra],
                               stdout=log, stderr=subprocess.STDOUT, check=True, timeout=120)
        report = load(canonical_root / name / "cli/report.json")
        validate_report(report)
        receipt = load(canonical_root / name / "receipt.json")
        assert receipt["cli_probe_exit"] == 0
        assert report["contract_hash"] == contract
        assert report["node"]["binary_sha256"] == node_hash
        assert report["node"]["launch_profile"]["helper_sha256"] == helper_hash
        assert report["node"]["exit"] == {"returncode": 0, "reaped": True}
        for pid, ticks in receipt["identities"]:
            assert not alive({"pid": pid, "start_time_ticks": ticks})
        assert any("maxdatacarriersize: expected JSON integer, got null" in f["detail"]
                   for f in report["consumer_model_failures"])
        raw = load(canonical_root / name / "profile.json")
        normalized = digest(normalize(raw))
        assert normalized == report["node"]["launch_profile"]["comparison_sha256"]
        toggled = copy.deepcopy(raw)
        if name == "default":
            toggled["argv"].extend(["--rpc-compat-profile", "bark-v31"])
        else:
            del toggled["argv"][-2:]
        counterfactual = digest(normalize(toggled))
        assert counterfactual != normalized
        cases[name] = {"report_path": str(canonical_root / name / "cli/report.json"),
                       "report_sha256": sha(canonical_root / name / "cli/report.json"),
                       "consensus": report["node"]["effective_consensus"],
                       "comparison_sha256": normalized, "policy_only_counterfactual": counterfactual,
                       "verdict": report["verdict"], "all_processes_reaped": True}
    assert cases["default"]["consensus"] == cases["bark-v31"]["consensus"]
    save(args.out / "canonical-release-summary.json", {"binding": binding, "cases": cases, "funded_e2e": False})

    run = args.out / "run"
    command = [str(args.launcher / "bin/spc-lab-self-hosted-ark"), "--run-dir", str(run),
               "--base-revision", "b0ab4ca34fda07c61a6335d05f598eaacc69678c",
               "--dirty-tree-hash", sha(args.launcher / "bin/spc-lab-self-hosted-ark"),
               "--operator-ui", "--listen", "127.0.0.1:0", "--duration-secs", "20"]
    env = {"PATH": "/usr/bin:/bin", "HOME": str(args.out), "LC_ALL": "C",
           "PYTHONDONTWRITEBYTECODE": "1"}
    records, children = [], []
    with (args.out / "launcher.log").open("xb") as log:
        child = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            deadline = time.monotonic() + 100
            while time.monotonic() < deadline:
                if (run / "run-manifest.json").exists():
                    manifest = load(run / "run-manifest.json")
                    if manifest["state"] == "running":
                        break
                assert child.poll() is None, "launcher exited before running; see launcher.log"
                time.sleep(0.1)
            else:
                raise AssertionError("launcher readiness deadline")
            records = manifest["processes"]
            children = descendants(records)
            assert len(records) == 11
            assert all(Path(r["executable_path"]).resolve().is_relative_to("/nix/store") for r in records)
            node_record = next(r for r in records if r["role"] == "node")
            assert alive(node_record) and sha(Path(f"/proc/{node_record['pid']}/exe")) == node_hash
            argv = Path(f"/proc/{node_record['pid']}/cmdline").read_bytes().rstrip(b"\0").decode().split("\0")[1:]
            actual_env = dict(x.split("=", 1) for x in Path(f"/proc/{node_record['pid']}/environ").read_bytes().rstrip(b"\0").decode().split("\0"))
            assert argv == node_record["args"]
            assert actual_env["SPC_LAB_RUN_TOKEN"] == node_record["run_token"]
            listeners = manifest["resources"]["listeners"]
            rpc = listeners["node.node-a.rpc"]
            cookie = (run / "private/node-rpc-node-a.cookie").read_bytes().strip()
            def call(method):
                request = urllib.request.Request(f"http://{rpc['host']}:{rpc['port']}",
                    data=json.dumps({"jsonrpc": "1.0", "id": 1, "method": method, "params": []}).encode(),
                    headers={"Authorization": "Basic " + base64.b64encode(cookie).decode(), "Content-Type": "application/json"})
                with urllib.request.urlopen(request, timeout=5) as response:
                    result = json.load(response)
                assert result.get("error") is None
                return result["result"]
            chain, network = call("getblockchaininfo"), call("getnetworkinfo")
            facts = effective_consensus(chain)
            assert facts == cases["bark-v31"]["consensus"]
            assert network["version"] == 310000 and "rbitcoin" in network["subversion"].lower()
            assert chain["blocks"] == 0 and call("getrawmempool") == []
            # Construct the one known runtime argv shape for the canonical
            # normalizer. No parser/normalization logic is reimplemented here.
            profile = {"profile_version": 1, "binary_sha256": node_hash, "argv": argv,
                       "env": actual_env, "config_files": [], "substitutions": [
                           {"class": "datadir", "argv_index": argv.index("--datadir") + 1},
                           {"class": "rpc-listen", "argv_index": argv.index("--rpc-listen") + 1},
                           {"class": "cookie-file", "argv_index": argv.index("--rpc-cookie-file") + 1},
                           {"class": "api-log", "span": [argv.index("--api-log"), argv.index("--log-level") + 2]},
                           {"class": "p2p-isolation", "span": [argv.index("--no-listen"), argv.index("--max-outbound") + 2]}]}
            # This is an observation of the existing engine, not a J2 gate.
            # Preserve canonical rejection (notably the legacy ':' ownership
            # token) rather than rewriting ENV or weakening the normalizer.
            comparison, canonical_error = None, None
            try:
                comparison = digest(normalize(profile))
            except LaunchProfileError as error:
                canonical_error = str(error)
                assert canonical_error.endswith("missing or invalid ownership token")
                assert ":" in actual_env["SPC_LAB_RUN_TOKEN"]
            save(args.out / "actual-runtime-node-profile.json", profile)
            info = load(run / "private/ark-info.json")
            assert info["state"] == "initialized_unfunded"
            bins = load(args.binaries)
            clusters = verify_clusters(bins, run)
            for pool, front in [("pool-a", "front-a1"), ("pool-b", "front-b1")]:
                import tomllib
                cfg = tomllib.loads((run / f"private/pool-{front}.toml").read_text())
                verify_redis_boundary(bins, cfg)
                status = load(run / f"private/pool-ark-{pool}/wallet/provision.json")
                assert status["float_spendable_sats"] == 0 and not status["funded_e2e"]
                assert cfg["ark"]["ark_id"] == info["bound"]["ark_id"]
            assert child.wait(timeout=30) == 0
            wait_stopped(records + children)
            save(args.out / "full-launcher-receipt.json", {"launcher": str(args.launcher),
                "command": command, "node_sha256": node_hash, "canonical_contract": contract,
                "actual_node_comparison_sha256": comparison, "actual_consensus": facts,
                "actual_runtime_canonical_error": canonical_error,
                "observed_node_profile_file_sha256": sha(args.out / "actual-runtime-node-profile.json"),
                "ark_state": info["state"], "ark_info_sha256": sha(run / "private/ark-info.json"),
                "postgres_system_identifiers": clusters, "actors": len(records), "os_children": len(children),
                "all_stopped": True, "full_nix_launcher_verified": True, "engine_j2_gate_implemented": False,
                "funded_e2e": False})
        finally:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait(timeout=15)
            if (run / "run-manifest.json").exists():
                subprocess.run([str(helper), "reap", "--run-dir", str(run)], stdout=log, stderr=subprocess.STDOUT, timeout=20, check=True)


if __name__ == "__main__":
    main()
