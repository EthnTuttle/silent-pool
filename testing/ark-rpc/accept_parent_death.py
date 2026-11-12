"""Bounded J2 acceptance driver. Run only in the parent's owned-process window.

Consumes a declared controller command (already-built spc-lab), exact env,
and the probe's lifecycle receipt paths. Never builds, discovers or replaces
a binary. Each invocation is ONE case against a fresh owned run.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import time

from arkrpc.nodeproc import start_ticks, sha256_file, private_json


def alive(pid, ticks):
    try:
        text = Path("/proc/%d/stat" % pid).read_text()
        return start_ticks(pid) == ticks and text[text.rfind(")") + 2:].split()[0] != "Z"
    except (OSError, ValueError):
        return False


def signal_owned(pid, ticks):
    if alive(pid, ticks):
        os.kill(pid, signal.SIGKILL)


def wait_file(path, deadline):
    while time.monotonic() < deadline:
        try:
            return json.loads(Path(path).read_bytes())
        except (OSError, ValueError):
            time.sleep(0.005)
    raise RuntimeError("lifecycle receipt deadline")


def owned_processes(token):
    """Read-only survivor enumeration by exact private run token, never name."""
    expected = b"SPC_LAB_RUN_TOKEN=" + token.encode()
    found = []
    for path in Path("/proc").iterdir():
        if not path.name.isdigit():
            continue
        try:
            if expected in (path / "environ").read_bytes().split(b"\0"):
                pid = int(path.name)
                ticks = start_ticks(pid)
                if alive(pid, ticks):
                    found.append((pid, ticks))
        except (OSError, ValueError):
            pass
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case", choices=["controller-kill", "probe-kill", "early-startup"], required=True)
    parser.add_argument("--launch", required=True, help="private JSON: argv, env, helper_sha256")
    parser.add_argument("--intent", required=True)
    parser.add_argument("--observed", required=True)
    parser.add_argument("--receipt", required=True)
    args = parser.parse_args()
    config = json.loads(Path(args.launch).read_bytes())
    assert set(config) == {"argv", "env", "helper_sha256"}
    assert os.path.isabs(config["argv"][0])
    assert sha256_file(config["argv"][0]) == config["helper_sha256"]
    token = config["env"]["SPC_LAB_RUN_TOKEN"]
    assert len(token) >= 16 and not owned_processes(token), "run token already in use"
    assert not Path(args.intent).exists() and not Path(args.observed).exists()
    source_root = Path(__file__).resolve().parent
    sources = {str(p.relative_to(source_root)): sha256_file(str(p))
               for p in sorted(source_root.glob("arkrpc/*.py"))}
    sources[Path(__file__).name] = sha256_file(__file__)
    receipt = {"case": args.case, "status": "failed", "helper_sha256": config["helper_sha256"],
               "sources": sources, "launch_input_sha256": hashlib.sha256(Path(args.launch).read_bytes()).hexdigest()}
    known = []
    controller = None
    try:
        controller = subprocess.Popen(config["argv"], env=config["env"], start_new_session=True,
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        controller_id = (controller.pid, start_ticks(controller.pid))
        known.append(controller_id)
        intent = wait_file(args.intent, time.monotonic() + 30)
        assert (intent["controller_pid"], intent["controller_start_ticks"]) == controller_id
        probe_id = (intent["probe_pid"], intent["probe_start_ticks"])
        assert alive(*probe_id)
        known.append(probe_id)
        if args.case != "early-startup":
            node = wait_file(args.observed, time.monotonic() + 10)
            known.append((node["pid"], node["start_ticks"]))
        else:
            assert not Path(args.observed).exists(), "missed recording-gap race; rerun with a fresh run"
        signal_owned(*(controller_id if args.case == "controller-kill" else probe_id))
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            survivors = owned_processes(token)
            # The surviving controller in probe-kill cases is expected until
            # it handles the failed preflight. It must not retain probe/node.
            survivors = [p for p in survivors if p != controller_id]
            if not survivors and not alive(*probe_id) and all(not alive(*p) for p in known[2:]):
                break
            time.sleep(0.01)
        else:
            raise RuntimeError("owned probe/node survived parent death")
        receipt.update(status="passed", observed_identities=[list(p) for p in known])
    finally:
        # Exact observed identities only. No name-based or unbound PID kills.
        for identity in reversed(known):
            signal_owned(*identity)
        if controller is not None:
            controller.wait(timeout=5)
        private_json(args.receipt, receipt)


if __name__ == "__main__":
    main()
