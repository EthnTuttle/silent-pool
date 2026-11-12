"""Cleanup requires successful owned reaping AND complete resource evidence."""
import hashlib
import ipaddress
import json
import os
from pathlib import Path

from oracle import Inconclusive, Invalid, integer, require, strict_json


def identity_observation(record):
    if not all(k in record for k in ("role", "entity", "pid", "start_time_ticks", "pgid", "run_token", "executable_path", "executable_sha256")):
        return {"status": "unknown", "reason": "ownership_fields_missing"}
    pid, expected = record["pid"], record["start_time_ticks"]
    try:
        integer(pid, 1); integer(expected, 1)
        raw = Path(f"/proc/{pid}/stat").read_text()
    except FileNotFoundError:
        return {"status": "gone", "pid": pid, "start_time_ticks": expected}
    except (OSError, ValueError, Invalid):
        return {"status": "unknown", "reason": "process_identity_unreadable"}
    fields = raw[raw.rfind(")") + 2:].split()
    if len(fields) < 20:
        return {"status": "unknown", "reason": "process_stat_incomplete"}
    if int(fields[19]) != expected:
        return {"status": "unknown", "reason": "pid_reused", "pid": pid}
    if fields[0] == "Z":
        return {"status": "surviving_zombie", "pid": pid, "start_time_ticks": expected}
    try:
        from storage import sha_file
        env = Path(f"/proc/{pid}/environ").read_bytes().split(b"\0")
        tokens = [v.split(b"=", 1)[1] for v in env if v.startswith(b"SPC_LAB_RUN_TOKEN=")]
        if tokens != [record["run_token"].encode()] or sha_file(f"/proc/{pid}/exe") != record["executable_sha256"]:
            return {"status": "unknown", "reason": "live_process_ownership_changed", "pid": pid}
    except FileNotFoundError:
        return {"status": "gone", "pid": pid, "start_time_ticks": expected}
    except (OSError, ValueError, TypeError):
        return {"status": "unknown", "reason": "live_process_ownership_unreadable", "pid": pid}
    return {"status": "surviving", "pid": pid, "start_time_ticks": expected}


def namespace_observation(holder):
    try:
        pid, expected = integer(holder["holder_pid"], 1), integer(int(holder["start_time_token"]), 1)
        raw = Path(f"/proc/{pid}/stat").read_text()
        values = raw[raw.rfind(")") + 2:].split()
        if int(values[19]) != expected:
            return {"status": "unknown", "reason": "namespace_holder_pid_reused"}
        return {"status": "surviving", "pid": pid, "start_time_ticks": expected}
    except FileNotFoundError:
        return {"status": "gone", "pid": pid, "start_time_ticks": expected}
    except (OSError, ValueError, KeyError, TypeError, IndexError, Invalid):
        return {"status": "unknown", "reason": "namespace_holder_identity_missing"}


def collect_resource_receipt(run_dir, manifest):
    root = Path(run_dir)
    resources = manifest.get("resources")
    if "placement" not in manifest or not isinstance(resources, dict) or any(k not in resources for k in ("listeners", "postgres_databases", "redis_instances")):
        raise Inconclusive("applicable resource inventory missing")
    groups, tokens = set(), set()
    for p in manifest["processes"]:
        if not all(k in p for k in ("pgid", "run_token", "pid", "start_time_ticks")):
            raise Inconclusive("resource owner identity incomplete")
        groups.add(integer(p["pgid"], 1))
        require(isinstance(p["run_token"], str) and p["run_token"], "resource owner token missing")
        tokens.add(p["run_token"].encode())
    owned_survivors, unknown_group_members = [], []
    for path in Path("/proc").iterdir():
        if not path.name.isdecimal():
            continue
        fields = None
        try:
            raw = (path / "stat").read_text()
            fields = raw[raw.rfind(")") + 2:].split()
            if int(fields[2]) not in groups:
                continue
            env = (path / "environ").read_bytes().split(b"\0")
            marked = [v.split(b"=", 1)[1] for v in env if v.startswith(b"SPC_LAB_RUN_TOKEN=")]
            if any(token in tokens for token in marked):
                owned_survivors.append({"pid": int(path.name), "start_time_ticks": int(fields[19]), "state": fields[0]})
            else:
                unknown_group_members.append({"pid": int(path.name), "reason": "owned_group_member_without_matching_token"})
        except FileNotFoundError:
            continue
        except (OSError, ValueError, IndexError):
            # If the stat was readable and placed this process in an owned
            # group, unavailable ownership data is not evidence of absence.
            if fields is not None and len(fields) > 2 and fields[2].isdecimal() and int(fields[2]) in groups:
                unknown_group_members.append({"pid": int(path.name), "reason": "group_identity_unreadable"})
    live_unix = []
    for line in Path("/proc/net/unix").read_text().splitlines()[1:]:
        parts = line.split()
        if len(parts) >= 8 and parts[3] == "00010000":
            socket_path = Path(parts[7])
            if socket_path.is_relative_to(root):
                live_unix.append({"path": str(socket_path.relative_to(root)), "socket_inode": parts[6]})
    namespace_checks = []
    if manifest.get("placement") is not None:
        broker_path = root / "broker" / "manifest.json"
        if not broker_path.is_file():
            raise Inconclusive("namespace cleanup publication missing")
        broker = strict_json(broker_path.read_bytes())
        endpoints = broker.get("endpoints")
        if not isinstance(endpoints, list) or not endpoints:
            raise Inconclusive("namespace holder inventory incomplete")
        for endpoint in endpoints:
            holder = endpoint["namespace"]
            check = namespace_observation(holder)
            namespace_checks.append({"endpoint": endpoint["name"], **check})
    ports = set()
    if manifest.get("placement") is None:
        for endpoint in resources["listeners"].values():
            ports.add((endpoint["host"], endpoint["port"]))
        for db in resources["postgres_databases"]:
            ports.add(("127.0.0.1", db["port"]))
        for db in resources["redis_instances"]:
            ports.add(("127.0.0.1", db["port"]))
    live_tcp = []
    entries = Path("/proc/net/tcp").read_text().splitlines()[1:]
    for host, port in ports:
        ip = ipaddress.ip_address(host)
        if ip.version != 4:
            raise Inconclusive("cleanup TCP address family lacks a bound reader")
        address = int.from_bytes(ip.packed, "little")
        for line in entries:
            values = line.split()
            h, p = values[1].split(":")
            if values[3] == "0A" and int(h, 16) == address and int(p, 16) == port:
                live_tcp.append({"host": host, "port": port, "socket_inode": values[9]})
    status = "PASSED"
    if owned_survivors or live_unix or any(c["status"].startswith("surviving") for c in namespace_checks):
        status = "FAILED"
    elif unknown_group_members or live_tcp or any(c["status"] != "gone" for c in namespace_checks):
        status = "INCONCLUSIVE"
    return {"schema": "spc-lab-owned-resource-cleanup/1", "run_id": manifest["run_id"], "status": status,
            "process_groups_checked": sorted(groups), "owned_group_survivors": owned_survivors,
            "unknown_group_members": unknown_group_members, "remaining_unix_listeners": live_unix,
            "remaining_unattributed_host_listeners": live_tcp, "namespace_holders": namespace_checks,
            "retained_datadirs_are_evidence_not_cleanup_failures": True}


def assess_cleanup(reap_exit, reap_stdout, before, after, identities, resources):
    if reap_exit != 0:
        return {"status": "FAILED", "code": "reaper_nonzero", "reaper_exit": reap_exit}
    if before is None or after is None:
        return {"status": "INCONCLUSIVE", "code": "cleanup_manifest_missing"}
    try:
        require(before["run_id"] == after["run_id"], "cleanup run identity changed")
        require("resources" in before and "placement" in before and before["resources"] == after["resources"]
                and before["placement"] == after["placement"], "cleanup resource/placement inventory missing or changed")
        expected = {(p["role"], p["entity"]) for p in before["processes"]}
        actual = {(p["role"], p["entity"]) for p in after["processes"]}
        require(len(expected) == len(before["processes"]) and actual == expected, "cleanup process inventory changed")
        for p in before["processes"]:
            require(all(key in p for key in ("pid", "start_time_ticks", "pgid", "run_token", "executable_path", "executable_sha256")),
                    "cleanup ownership record incomplete")
        rows = strict_json(reap_stdout)
        require(isinstance(rows, list), "reaper result must be the source-defined tuple array")
        reported = {}
        for row in rows:
            require(isinstance(row, list) and len(row) == 3 and (row[0], row[1]) not in reported, "reaper result shape/duplicate")
            reported[(row[0], row[1])] = row[2]
        require(set(reported) == expected, "reaper omitted owned identities")
        if any(value not in {"stopped", "exited"} for value in reported.values()):
            return {"status": "INCONCLUSIVE", "code": "reaper_ownership_unverified"}
        require(set(identities) == expected, "independent identity census incomplete")
        if any(value["status"].startswith("surviving") for value in identities.values()):
            return {"status": "FAILED", "code": "owned_process_survived", "survivors": [list(k) for k, v in identities.items() if v["status"].startswith("surviving")]}
        if any(value["status"] != "gone" for value in identities.values()):
            return {"status": "INCONCLUSIVE", "code": "process_ownership_incomplete"}
        if resources is None or resources.get("schema") != "spc-lab-owned-resource-cleanup/1" or resources.get("run_id") != before["run_id"]:
            return {"status": "INCONCLUSIVE", "code": "resource_cleanup_receipt_missing"}
        list_fields = ("process_groups_checked", "owned_group_survivors", "unknown_group_members", "remaining_unix_listeners",
                       "remaining_unattributed_host_listeners", "namespace_holders")
        if any(not isinstance(resources.get(key), list) for key in list_fields):
            return {"status": "INCONCLUSIVE", "code": "resource_cleanup_evidence_missing"}
        require(set(resources["process_groups_checked"]) == {p["pgid"] for p in before["processes"]}, "resource group inventory incomplete")
        leaked = resources["owned_group_survivors"] or resources["remaining_unix_listeners"] or any(
            item["status"].startswith("surviving") for item in resources["namespace_holders"])
        unknown = resources["unknown_group_members"] or resources["remaining_unattributed_host_listeners"] or any(
            item["status"] != "gone" for item in resources["namespace_holders"])
        status = "FAILED" if leaked else ("INCONCLUSIVE" if unknown else "PASSED")
        if status != "PASSED" or resources["status"] != status:
            return {"status": status if status != "PASSED" else "INCONCLUSIVE", "code": "resource_cleanup_incomplete", "resources": resources}
        return {"status": "PASSED", "reaper_exit": 0, "reaper_results": rows, "owned_identities_checked": len(expected), "resources": resources}
    except (KeyError, TypeError, ValueError, Invalid, Inconclusive) as error:
        return {"status": "INCONCLUSIVE", "code": "cleanup_evidence_incomplete", "error_class": type(error).__name__}
