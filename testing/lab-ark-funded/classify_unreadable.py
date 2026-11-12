#!/usr/bin/env python3
"""Public PID metadata diagnostics only; never changes cleanup acceptance.

Reads stat/comm/status and namespace symlink identities. Does not read environ,
cmdline, cwd, descriptors, memory, wallet data or keys, and never signals a PID.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path

PROTECTED = ("/tmp/opencode/ark-inspection-20261109T025651Z/run/run-manifest.json",
             "/tmp/opencode/live-lab-20261108T231027Z/run/run-manifest.json")
ROLES = {"(sd-pam)": "PAM user-session helper (comm-based)", "gpg-agent": "OpenPGP user agent (comm-based)",
         "ssh-agent": "SSH user agent (comm-based)", "sd_espeak-ng-mb": "speech backend (comm-based)"}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def public_process(pid):
    p = Path("/proc") / str(pid)
    try:
        fields = (p / "stat").read_text().rsplit(")", 1)[1].split()
        result = {"pid": pid, "start_ticks": int(fields[19]), "state": fields[0],
                  "ppid": int(fields[1]), "pgid": int(fields[2]), "comm": (p / "comm").read_text().strip(),
                  "proc_uid": p.stat().st_uid}
        status = (p / "status").read_text().splitlines()
        result["uids"] = next(([int(n) for n in line.split()[1:]] for line in status if line.startswith("Uid:")), None)
        result["namespaces"] = {}
        for ns in ("pid", "user", "net"):
            try:
                result["namespaces"][ns] = {"identity": os.readlink(p / "ns" / ns)}
            except OSError as error:
                result["namespaces"][ns] = {"unreadable_errno": error.errno}
        after = (p / "stat").read_text().rsplit(")", 1)[1].split()
        result["identity_stable_during_read"] = int(after[19]) == result["start_ticks"]
        return result
    except OSError as error:
        return {"pid": pid, "unreadable_errno": error.errno, "identity_stable_during_read": False}


def classify(roots):
    hz = os.sysconf("SC_CLK_TCK")
    boot = next(int(line.split()[1]) for line in Path("/proc/stat").read_text().splitlines() if line.startswith("btime "))
    baselines, recorded, denied = [], [], set()
    for root in roots:
        cleanup_files = list(root.glob("*-cleanup-inventory.json"))
        assert len(cleanup_files) == 1
        cleanup = json.loads(cleanup_files[0].read_text())
        inv = cleanup["inventory"]
        denied.update(inv["unreadable_same_uid_processes"])
        controller = next(p for p in inv["processes"] if p["role"] == "controller")
        events = [json.loads(line) for line in (root / "events.jsonl").read_text().splitlines()]
        launch = next(e for e in events if e["file"].split("-", 1)[1] == "launch.json")
        baseline = {"run": root.name, "cleanup_sha256": digest(cleanup_files[0]),
                    "controller_pid": controller["pid"], "controller_start_ticks": controller["start_time_ticks"],
                    "launch_unix_ms": launch["unix_ms"],
                    "current_boot_consistent_with_recorded_clock": abs(boot + controller["start_time_ticks"] / hz - launch["unix_ms"] / 1000) <= 60,
                    "unreadable_pids": inv["unreadable_same_uid_processes"],
                    "recorded_owned_processes_all_gone": all(p["status"] == "gone_or_reused" for p in inv["processes"]),
                    "recorded_listeners_all_released": all(p["released"] is True for p in inv["listeners"])}
        baselines.append(baseline)
        recorded.extend({"pid": p["pid"], "start_ticks": p["start_time_ticks"], "role": p["role"], "census": root.name} for p in inv["processes"])
    protected = []
    for name in PROTECTED:
        path = Path(name)
        try:
            data = json.loads(path.read_text())
            protected.extend({"pid": p["pid"], "start_ticks": p["start_time_ticks"], "role": p["role"], "census": path.parent.parent.name} for p in data["processes"])
        except (OSError, ValueError, KeyError) as error:
            protected.append({"census": path.parent.parent.name, "unavailable": type(error).__name__})
    collector = public_process(os.getpid())
    output = []
    for pid in sorted(denied):
        row = public_process(pid)
        row["role_hint"] = ROLES.get(row.get("comm"), "unknown")
        row["ownership"] = "unknown beyond public UID and recorded census comparisons"
        row["run_baseline_comparisons"] = []
        for baseline in baselines:
            predates = None
            if row.get("identity_stable_during_read") and baseline["current_boot_consistent_with_recorded_clock"]:
                predates = row["start_ticks"] < baseline["controller_start_ticks"]
            row["run_baseline_comparisons"].append({"run": baseline["run"], "pid_recorded_unreadable": pid in baseline["unreadable_pids"],
                "current_identity_predates_run": predates, "historical_unreadable_start_ticks_recorded": False})
        row["recorded_owned_matches"] = [p for p in recorded if p["pid"] == pid and p["start_ticks"] == row.get("start_ticks")]
        row["protected_census_matches"] = [p for p in protected if p.get("pid") == pid and p.get("start_ticks") == row.get("start_ticks")]
        ancestry, seen, parent = [], {pid}, row.get("ppid", 0)
        while parent and parent not in seen and len(ancestry) < 8:
            seen.add(parent)
            ancestor = public_process(parent)
            ancestry.append({k: ancestor[k] for k in ("pid", "start_ticks", "ppid", "comm", "proc_uid", "identity_stable_during_read") if k in ancestor})
            parent = ancestor.get("ppid", 0)
        row["current_ancestry_only"] = ancestry
        row["namespace_comparison"] = {ns: "same-as-current-collector" if value.get("identity") and value == collector["namespaces"].get(ns)
                                       else "different" if value.get("identity") else "unknown" for ns, value in row.get("namespaces", {}).items()}
        output.append(row)
    return {"schema": "spc.lab.ap5.public-unreadable-pid-classification/v1", "clock_ticks_per_second": hz,
            "baselines": baselines, "processes": output,
            "protected_census_read_errors": [p for p in protected if "unavailable" in p],
            "global_unreadable_coverage": "INCONCLUSIVE", "cleanup_acceptance_changed": False,
            "limits": "Current stable start times can show an incarnation predates a run on a consistent boot. Original denial records stored PIDs but not start times. Current ancestry/comm is not a historical ownership or descriptor proof."}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--run", type=Path, action="append", required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    result = classify(args.run)
    with args.out.open("x") as out:
        os.chmod(args.out, 0o600)
        json.dump(result, out, indent=2, sort_keys=True)
        out.write("\n")
