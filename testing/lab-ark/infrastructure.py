#!/usr/bin/env python3
"""Exercise real local infrastructure, stop, partial startup failure and death.

No funding, block generation, fixture Ark endpoint, or hosted service is used.
Pass a JSON mapping of runtime CLI binary arguments to their built paths.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import time
import tomllib
import urllib.error
import urllib.request
import urllib.parse


def read(path):
    return json.loads(path.read_text())


def alive(record):
    try:
        stat = Path(f"/proc/{record['pid']}/stat").read_text().rsplit(")", 1)[1].split()
        return stat[0] != "Z" and stat[19] == str(record["start_time_ticks"])
    except FileNotFoundError:
        return False


def wait_stopped(records):
    deadline = time.monotonic() + 15
    while any(alive(r) for r in records) and time.monotonic() < deadline:
        time.sleep(0.1)
    assert not any(alive(r) for r in records), "owned actor survived stop/death"


def descendants(records):
    """Census OS children separately from logical actors, before shutdown."""
    processes = {}
    for path in Path("/proc").glob("[0-9]*/stat"):
        try:
            fields = path.read_text().rsplit(")", 1)[1].split()
            processes[int(path.parent.name)] = (int(fields[1]), fields[19])
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            continue
    owned = {r["pid"] for r in records}
    children = []
    while True:
        found = [pid for pid, (parent, _) in processes.items() if parent in owned and pid not in owned]
        if not found:
            return children
        for pid in found:
            owned.add(pid)
            children.append({"pid": pid, "start_time_ticks": processes[pid][1]})


def verify_redis_boundary(binaries, front_config):
    endpoint = urllib.parse.urlparse(front_config["storage"]["redis"])
    env = dict(os.environ, REDISCLI_AUTH=endpoint.password, LC_ALL="C")
    for allowed, operation in [
        (True, ["XREADGROUP", "GROUP", "ark", "probe", "STREAMS", "spc:ark:orders", ">"]),
        (True, ["XADD", "spc:ark:receipts", "*", "d", "unused-dry-run"]),
        (False, ["XADD", "spc:ark:orders", "*", "d", "unused-dry-run"]),
        (False, ["HGET", "spc:arkreg:ack", "unused-dry-run"]),
        (False, ["XREAD", "STREAMS", "spc:slices", "0"]),
    ]:
        output = subprocess.check_output([
            binaries["redis-cli"], "--no-auth-warning", "-h", endpoint.hostname,
            "-p", str(endpoint.port), "ACL", "DRYRUN", "ark", *operation,
        ], env=env, text=True, timeout=5).strip()
        assert (output == "OK") == allowed, (operation[0], allowed, output)
        if not allowed:
            assert "permission" in output.lower(), output


def verify_clusters(binaries, root):
    identities = set()
    for path in [root / "postgres/pg-pool-a", root / "postgres/pg-pool-b",
                 root / "private/ark-server-local-ark/postgres"]:
        output = subprocess.check_output([
            str(Path(binaries["postgres-bin-dir"]) / "pg_controldata"), str(path),
        ], env=dict(os.environ, LC_ALL="C"), text=True, timeout=5)
        match = re.search(r"Database system identifier:\s+(\d+)", output)
        assert match, "no actual PostgreSQL system identifier"
        assert match[1] not in identities, "provider/pool clusters share storage"
        identities.add(match[1])
    return sorted(identities)


def run_case(args, binaries, case):
    root = args.output / case
    log = args.output / f"{case}.log"
    command = [binaries["lab"], "runtime", "--scenario", str(args.scenario),
               "--run-dir", str(root), "--base-revision", args.base_revision,
               "--dirty-tree-hash", args.source_hash, "--listen", "127.0.0.1:0",
               "--operator-ui", "--duration-secs", "8" if case == "stop" else "300"]
    for name, path in binaries.items():
        if name == "lab":
            continue
        command.extend(["--" + name, "/bin/false" if name == "pool" and case == "partial-failure" else path])
    trace = None
    if args.trace_tool and case == "stop":
        # Observe actual syscalls of the real ark-only configs. A missing
        # signing-key path still causes an open syscall in the old code.
        assert not Path("/run/secrets/spc/ark-order-key").exists()
        trace = args.output / "signing-key-open.trace"
        trace.touch(mode=0o600, exist_ok=False)
        command = [str(args.trace_tool), "-f", "-qq", "-s", "1024", "-e",
                   "trace=open,openat,openat2", "-o", str(trace), "--", *command]
    with log.open("xb") as output:
        os.chmod(log, 0o600)
        child = subprocess.Popen(command, stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
        records = []
        children = []
        try:
            deadline = time.monotonic() + 100
            while time.monotonic() < deadline:
                text = log.read_text()
                match = re.search(r"spc-lab-runtime: http=(http://[^\s]+)", text)
                if match or child.poll() is not None:
                    break
                time.sleep(0.1)
            manifest = read(root / "run-manifest.json")
            records = manifest["processes"]
            children = descendants(records)
            if case == "partial-failure":
                assert child.wait(timeout=15) != 0, "negative startup unexpectedly succeeded"
                assert any(r["role"] == "ark" for r in records), "did not reach the partial failure boundary"
                assert manifest["state"] == "failed"
            else:
                assert match, f"runtime did not start; inspect {log}"
                info = read(root / "private/ark-info.json")
                assert info["state"] == "initialized_unfunded"
                assert info["ark_info"]["network"] == "regtest"
                assert info["source_revision"] == "c419159fa54085572f8ca72d2ba1716556a9d3eb"
                assert sum(r["role"] == "ark-postgres" for r in records) == 1
                assert sum(r["role"] == "pool-ark" for r in records) == 2
                cluster_ids = verify_clusters(binaries, root)
                seeds = []
                for pool in ["pool-a", "pool-b"]:
                    private = root / f"private/pool-ark-{pool}"
                    seeds.append((private / "seed.hex").read_bytes())
                    cfg = tomllib.loads((private / "pool.toml").read_text())
                    assert "postgres" not in cfg["storage"]
                    assert "authority_secret_key" not in cfg["pool"]
                    assert cfg["pool"]["roles"] == ["ark"]
                    assert cfg["ark"]["server"] == info["endpoint"]
                    status = read(private / "wallet/provision.json")
                    assert status["float_spendable_sats"] == 0
                    assert status["onchain_funding_address"].startswith("bcrt1")
                    assert status["funded_e2e"] is False
                    front = "front-a1" if pool == "pool-a" else "front-b1"
                    front_cfg = tomllib.loads((root / f"private/pool-{front}.toml").read_text())
                    assert front_cfg["ark"]["ark_id"] == info["bound"]["ark_id"]
                    assert front_cfg["ark"]["server_pubkey"] == info["bound"]["server_pubkey"]
                    assert front_cfg["ark"]["mailbox_pubkey"] == info["bound"]["mailbox_pubkey"]
                    verify_redis_boundary(binaries, front_cfg)
                assert seeds[0] != seeds[1]
                for path in (root / "private").rglob("*"):
                    if path.name.endswith((".hex", ".cookie", ".toml")) or path.name == "mnemonic":
                        assert path.stat().st_mode & 0o077 == 0, str(path)
                token = read(root / "private/access/operator.json")["token"]
                url = match[1].rstrip("/") + "/api/lab/v1/runs/lab-self-hosted-ark/inventory"
                try:
                    urllib.request.urlopen(url)
                    raise AssertionError("inventory accepted an unauthenticated reader")
                except urllib.error.HTTPError as error:
                    assert error.code in (401, 403)
                request = urllib.request.Request(url, headers={"Authorization": "Bearer " + token})
                with urllib.request.urlopen(request) as response:
                    inventory = json.load(response)
                encoded = json.dumps(inventory)
                assert "pool-ark-pool-a" in encoded and "pool-ark-pool-b" in encoded
                assert "ark-postgres-local-ark" in encoded
                assert all(s.decode() not in encoded for s in seeds)
                if case == "controller-death":
                    child.send_signal(signal.SIGKILL)
                    assert child.wait(timeout=15) == -signal.SIGKILL
                else:
                    assert child.wait(timeout=25) == 0
            wait_stopped(records + children)
            if trace:
                opened = trace.read_text()
                assert "/run/secrets/spc/ark-order-key" not in opened, "ark process attempted payout-key read"
                def read_by(pid, path):
                    return any(line.startswith(str(pid) + " ") and str(path) in line
                               and "O_RDONLY" in line for line in opened.splitlines())
                for pool in ["pool-a", "pool-b"]:
                    ark_pid = next(r["pid"] for r in records if r["role"] == "pool-ark" and r["entity"] == pool)
                    assert read_by(ark_pid, root / f"private/pool-ark-{pool}/pool.toml")
                    # Positive control: actual payout roles did read their own
                    # explicitly provisioned signing files under the same trace.
                    front = "front-a1" if pool == "pool-a" else "front-b1"
                    payout_pid = next(r["pid"] for r in records if r["role"] == "front" and r["entity"] == front)
                    assert read_by(payout_pid, root / f"private/order-{pool}.hex")
            subprocess.run([binaries["lab"], "reap", "--run-dir", str(root)], check=True,
                           stdout=output, stderr=subprocess.STDOUT, timeout=20)
            return {"case": case, "outcome": "passed", "recorded_actors": len(records),
                    "os_children": len(children),
                    "postgres_system_identifiers": cluster_ids if case != "partial-failure" else [],
                    "absent_signing_key_open_attempts": 0 if trace else None,
                    "trace_sha256": hashlib.sha256(trace.read_bytes()).hexdigest() if trace else None,
                    "run_dir": str(root), "log_sha256": hashlib.sha256(log.read_bytes()).hexdigest()}
        finally:
            if child.poll() is None:
                # The traced stop case also owns the tracer/controller process
                # group. Actors have their own supervised groups/PDEATHSIG.
                os.killpg(child.pid, signal.SIGKILL)
                child.wait(timeout=15)
            if (root / "run-manifest.json").exists():
                subprocess.run([binaries["lab"], "reap", "--run-dir", str(root)],
                               stdout=output, stderr=subprocess.STDOUT, timeout=20)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binaries", type=Path, required=True)
    parser.add_argument("--scenario", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--base-revision", required=True)
    parser.add_argument("--source-hash", required=True)
    parser.add_argument("--trace-tool", type=Path)
    args = parser.parse_args()
    args.output.mkdir(mode=0o700)
    binaries = read(args.binaries)
    receipt = {"funded_e2e": False, "tests": [], "binaries": binaries,
               "trace_tool_sha256": hashlib.sha256(args.trace_tool.read_bytes()).hexdigest() if args.trace_tool else None}
    try:
        for case in ["stop", "partial-failure", "controller-death"]:
            receipt["tests"].append(run_case(args, binaries, case))
    finally:
        (args.output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")


if __name__ == "__main__":
    main()
