#!/usr/bin/env python3
"""AP-1b r1 external-run harness (owned by this packet; NOT part of the
frozen AP-1a probe).

Launches the AP-1b candidate node directly with an EXPLICIT CLI argv
(private cookie + private scratch datadir, fresh port), then runs the
frozen, unmodified AP-1a probe against it in external mode
(`probe.py probe --rpc-url ... --cookie-file ...`), which is the approved
probe's own path for an already-running node.

Why this exists (r1 correction): the profile is CLI/conf-only (the
RBITCOIN_RPC_COMPAT_PROFILE env knob was removed - not ask-first
approved), and the probe's `node-probe` mode cannot pass extra argv, so
the profile-on node must be launched outside the probe. This harness
supplies the honest external binding the probe's external mode does not
carry (argv, pid, binary sha, source name, effective config, devcheck
receipt); it never fabricates the probe's internal OwnedNode fields.

Scratch (cookie credential, datadir) is destroyed at the end; the binding
records the cleanup.
"""
import argparse
import base64
import hashlib
import json
import os
import secrets
import shutil
import signal
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request

PROBE_DEFAULT_ROOT = (
    "/home/ethan/.local/share/opencode/worktree/ad8886/"
    "lab-ark-rpc-probe/testing/ark-rpc"
)


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def rpc_call(url, user, password, method, params=None, deadline=10.0):
    body = json.dumps(
        {"jsonrpc": "2.0", "id": 1, "method": method, "params": params or []}
    ).encode()
    req = urllib.request.Request(url, data=body, method="POST")
    token = base64.b64encode(f"{user}:{password}".encode()).decode()
    req.add_header("Authorization", "Basic %s" % token)
    req.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(req, timeout=deadline) as resp:
        return json.loads(resp.read().decode())


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--binary", required=True)
    ap.add_argument("--label", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--profile", choices=["none", "bark-v31"], default="none")
    ap.add_argument("--source-name", default="AP1b-bark-profile-r1")
    ap.add_argument("--probe-root", default=PROBE_DEFAULT_ROOT)
    ap.add_argument("--scratch-root", default="/tmp/opencode")
    ap.add_argument("--devcheck-receipt", default=None,
                    help="JSON receipt of the devcheck golden-vector gate "
                         "run before this harness (documented in the "
                         "binding; the external probe mode cannot embed it)")
    ap.add_argument("--max-run-secs", type=int, default=300)
    ap.add_argument("--ready-timeout", type=float, default=90.0)
    args = ap.parse_args()

    if "RBITCOIN_RPC_COMPAT_PROFILE" in os.environ:
        sys.exit(
            "refusing to run with RBITCOIN_RPC_COMPAT_PROFILE set: the "
            "profile is CLI/conf-only and this harness must not depend on "
            "an env knob that does not exist"
        )

    os.makedirs(args.out, exist_ok=True)
    token = secrets.token_hex(6)
    scratch = os.path.join(args.scratch_root, "AP1b-r1-%s-%s" % (
        args.label, token))
    datadir = os.path.join(scratch, "datadir")
    cookie_path = os.path.join(scratch, "rpc.cookie")
    log_path = os.path.join(scratch, "node.log")
    os.makedirs(datadir, exist_ok=True)

    cookie_user = "ap1b"
    cookie_pass = secrets.token_hex(24)
    with open(cookie_path, "w") as fh:
        fh.write("%s:%s" % (cookie_user, cookie_pass))
    os.chmod(cookie_path, 0o600)

    port = free_port()
    rpc_url = "http://127.0.0.1:%d" % port
    argv = [
        args.binary,
        "--network", "regtest",
        "--datadir", datadir,
        "--rpc",
        "--rpc-listen", "127.0.0.1:%d" % port,
        "--rpc-cookie-file", cookie_path,
        "--no-listen",
        "--no-seeds",
        "--no-discover",
        "--max-run-secs", str(args.max_run_secs),
    ]
    if args.profile != "none":
        argv += ["--rpc-compat-profile", args.profile]

    # Mirror the probe's OwnedNode dialect default (the approved probe
    # launches owned nodes with this env set); nothing else is injected.
    env = dict(os.environ)
    env["RBITCOIN_RPC_PACKAGE_DIALECT"] = "1"
    env_record = {
        "inherited_then_set": {"RBITCOIN_RPC_PACKAGE_DIALECT": "1"},
        "RBITCOIN_RPC_COMPAT_PROFILE": "verified unset (harness refuses to "
                                       "run if it is set; not a knob)",
    }

    binary_sha = sha256_file(args.binary)
    log_fh = open(log_path, "wb")
    started_wall = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    started_mono = time.monotonic()
    proc = subprocess.Popen(
        argv, env=env, stdout=log_fh, stderr=subprocess.STDOUT,
        stdin=subprocess.DEVNULL,
    )
    pid = proc.pid

    # Readiness: getnetworkinfo answers.
    deadline = started_mono + args.ready_timeout
    info = None
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            log_fh.close()
            sys.exit("node exited early with %r; see %s" % (proc.returncode, log_path))
        try:
            info = rpc_call(rpc_url, cookie_user, cookie_pass, "getnetworkinfo")
            break
        except (urllib.error.URLError, urllib.error.HTTPError, OSError, ValueError):
            time.sleep(0.25)
    if info is None:
        proc.terminate()
        log_fh.close()
        sys.exit("node did not become ready in %.0fs" % args.ready_timeout)
    ready_s = time.monotonic() - started_mono

    result = info.get("result", info)
    effective = {
        "getnetworkinfo.version": result.get("version"),
        "getnetworkinfo.subversion": result.get("subversion"),
        "getnetworkinfo.rbitcoin_compat": result.get("rbitcoin_compat"),
    }

    # Run the frozen, unmodified probe in external mode.
    probe_argv = [
        sys.executable, "probe.py", "probe",
        "--rpc-url", rpc_url,
        "--cookie-file", cookie_path,
        "--label", args.label,
        "--out", args.out,
        "--expected-network", "regtest",
    ]
    probe = subprocess.run(
        probe_argv, cwd=args.probe_root,
        capture_output=True, text=True,
    )
    print(probe.stdout, end="")
    if probe.returncode != 0:
        print(probe.stderr, end="", file=sys.stderr)

    # Stop the node, keep the log for the banner evidence.
    proc.send_signal(signal.SIGTERM)
    try:
        proc.wait(timeout=30)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()
    log_fh.close()
    stopped_wall = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    uptime_s = time.monotonic() - started_mono

    # Extract the banner line + copy node.log into the artifact dir.
    banner = None
    with open(log_path, "rb") as fh:
        for line in fh.read().decode("utf-8", "replace").splitlines():
            if "rpc_compat_profile=" in line:
                banner = line
    node_log_out = os.path.join(args.out, "node.log")
    shutil.copyfile(log_path, node_log_out)

    binding = {
        "harness": "AP-1b r1 external-run harness (packet-owned; probe "
                   "unchanged, external mode)",
        "date": started_wall,
        "label": args.label,
        "profile": None if args.profile == "none" else args.profile,
        "node": {
            "binary": args.binary,
            "binary_sha256": binary_sha,
            "source_name": args.source_name,
            "argv": argv,
            "pid": pid,
            "started_utc": started_wall,
            "stopped_utc": stopped_wall,
            "uptime_s": round(uptime_s, 3),
            "ready_after_s": round(ready_s, 3),
            "exit": {"returncode": proc.returncode, "signal": None},
            "env": env_record,
            "rpc_url": rpc_url,
            "cookie_file": "rpc.cookie (single-run credential, mode 0600, "
                           "generated in the private scratch; contents "
                           "never recorded; destroyed with the scratch)",
            "network": "regtest",
            "isolation": ["--no-listen", "--no-seeds", "--no-discover",
                          "loopback rpc only"],
            "launch_mode": "explicit CLI argv owned by this harness (the "
                           "probe's external mode; the probe's node-probe "
                           "mode cannot pass --rpc-compat-profile, and the "
                           "env knob does not exist)",
        },
        "effective_config": effective,
        "startup_banner": banner,
        "node_log": {"path": "node.log", "sha256": sha256_file(node_log_out)},
        "probe": {
            "root": args.probe_root,
            "mode": "external (probe.py probe --rpc-url ... --cookie-file ...)",
            "argv": probe_argv,
            "exit_code": probe.returncode,
            "note": "the external probe mode carries no OwnedNode fields by "
                    "design; this binding supplies them honestly",
        },
        "devcheck": None,
        "cleanup": None,
    }
    if args.devcheck_receipt:
        with open(args.devcheck_receipt) as fh:
            dev = json.load(fh)
        binding["devcheck"] = {
            "receipt": args.devcheck_receipt,
            "exit_code": dev.get("exit_code"),
            "vectors": dev.get("vectors"),
            "note": "golden-vector gate run before the probe runs, "
                    "separately documented here because the external probe "
                    "mode has no --devcheck-receipt option",
        }

    # Cleanup: destroy the scratch (cookie credential + datadir).
    shutil.rmtree(scratch)
    binding["cleanup"] = {
        "action": "rm -rf %s (cookie credential, rpc token, datadir store)" % scratch,
        "verified": not os.path.exists(scratch),
    }

    out_binding = os.path.join(args.out, "external-node-binding.json")
    with open(out_binding, "w") as fh:
        json.dump(binding, fh, indent=1)
    print("wrote %s" % out_binding)
    return probe.returncode


if __name__ == "__main__":
    sys.exit(main())
