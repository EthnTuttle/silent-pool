"""Exhaustive, explicitly scoped owned-resource census. Never kills by PID."""
import errno
import hashlib
import os
from pathlib import Path
import socket
import stat
import time
from collections import Counter

from common import encoded, sha, require


REQUIRED_ROLES = {"node": 2, "front": 2, "wallet": 4, "miner": 4, "pool-ark": 2,
                  "ark": 1, "ark-postgres": 1, "postgres": 2, "redis": 2, "tx_driver": 2}
REQUIRED_LISTENERS = {"ark.local-ark", "ark.local-ark.admin", "ark.local-ark.postgres"}
REQUIRED_LISTENERS |= {f"node.node-{n}.{kind}" for n in ("a", "b") for kind in ("p2p", "rpc", "tdp")}
REQUIRED_LISTENERS |= {f"front.front-{n}1.{kind}" for n in ("a", "b") for kind in ("api", "jd", "operator", "sv2")}
REQUIRED_LISTENERS |= {f"wallet.{u}.status" for u in ("alice", "bob", "carol", "dave")}


def coverage_errors(manifest):
    counts = Counter(r["role"] for r in manifest.get("processes", []))
    errors = ["role:" + role for role, n in REQUIRED_ROLES.items() if counts[role] < n]
    resources = manifest.get("resources", {})
    errors += ["listener:" + key for key in sorted(REQUIRED_LISTENERS - resources.get("listeners", {}).keys())]
    for group in ("postgres_databases", "redis_instances"):
        if len(resources.get(group, [])) != 2:
            errors.append(group)
    if not manifest.get("processes") or any(not r.get("run_token") for r in manifest.get("processes", [])):
        errors.append("run_tokens")
    return errors


def validate_cleanup(receipt):
    """Offline fail-closed validation; labels alone never establish release."""
    require(receipt.get("schema") == "spc.lab.ap5.cleanup/v2", "cleanup schema")
    inv = receipt["inventory"]
    require(hashlib.sha256(encoded(inv)).hexdigest() == receipt["inventory_sha256"], "cleanup inventory digest mismatch")
    coverage = inv["active_manifest"]
    errors = coverage_errors(coverage)
    require(inv["coverage_errors"] == errors, "cleanup coverage errors misreported")
    ids = {(r["pid"], r["start_time_ticks"]) for r in inv["processes"]}
    require(len(ids) == len(inv["processes"]), "duplicate cleanup process identity")
    require(len({(r["pid"], r["start_time_ticks"]) for r in coverage.get("processes", [])}) == len(coverage.get("processes", [])), "duplicate active actor identity")
    require(all((r["pid"], r["start_time_ticks"]) in ids for r in coverage.get("processes", [])), "cleanup omitted an active actor")
    planned = {(r["host"], r["port"]) for r in inv["listeners"] if r["kind"] == "planned_tcp_endpoint"}
    endpoints = list(coverage.get("resources", {}).get("listeners", {}).values())
    endpoints += [{"host": "127.0.0.1", "port": r["port"]} for g in ("postgres_databases", "redis_instances") for r in coverage.get("resources", {}).get(g, [])]
    require(all((r["host"], r["port"]) in planned for r in endpoints), "cleanup omitted a planned listener")
    require(set(inv["observed_socket_inodes"]) == {r["inode"] for r in inv["listeners"] if r["kind"] == "owned_socket_inode"}, "cleanup socket inode census mismatch")
    require(set(inv["observed_unix_paths"]) == {r["path"] for r in inv["listeners"] if r["kind"] == "run_unix_socket"}, "cleanup Unix socket census mismatch")
    require({r["run_token"] for r in coverage.get("processes", [])} <= set(inv["run_tokens"]), "cleanup run token census mismatch")
    complete = not (errors or inv["capture_errors"] or inv["unreadable_same_uid_processes"])
    released = complete and not (inv["file_references"] or inv["locks"]) and all(r["status"] == "gone_or_reused" for r in inv["processes"]) and all(r["released"] is True for r in inv["listeners"])
    require(receipt["exhaustive"] is complete and receipt["all_resources_released"] is released, "cleanup success contradicts raw census")
    require(receipt["status"] == ("passed" if released else "inconclusive" if not complete else "failed"), "cleanup status contradiction")
    return released


def process(pid):
    root = Path(f"/proc/{pid}")
    fields = (root / "stat").read_text().rsplit(")", 1)[1].split()
    return {"pid": pid, "ticks": int(fields[19]), "state": fields[0], "ppid": int(fields[1]), "pgid": int(fields[2])}


def inside(path, root):
    # /proc appends this marker for unlinked open files.
    path = path.removesuffix(" (deleted)")
    return path.startswith(str(root) + "/") or path == str(root)


def tcp_inodes():
    values = set()
    for path in (Path("/proc/net/tcp"), Path("/proc/net/tcp6"), Path("/proc/net/udp"), Path("/proc/net/udp6")):
        for line in path.read_text().splitlines()[1:]:
            fields = line.split()
            if fields[9] != "0":
                values.add(fields[9])
    for line in Path("/proc/net/unix").read_text().splitlines()[1:]:
        values.add(line.split()[6])
    return values


class Census:
    def __init__(self, run):
        self.run = Path(run).resolve()
        self.known = {}
        self.tokens = set()
        self.ports = set()
        self.unix = set()
        self.socket_inodes = set()
        self.file_inodes = set()
        self.capture_errors = []
        self.active_manifest = {}

    def add(self, row, role):
        key = (row["pid"], row.get("ticks", row.get("start_time_ticks")))
        self.known.setdefault(key, {"pid": key[0], "start_time_ticks": key[1], "role": role,
                                    "executable_path": row.get("executable_path"),
                                    "executable_sha256": row.get("executable_sha256")})

    def capture(self, manifest, controller=None):
        if not coverage_errors(manifest):
            # Public resource census only: omit credentials, argv and wallet data.
            resources = manifest["resources"]
            self.active_manifest = {
                "processes": [{k: r[k] for k in ("pid", "start_time_ticks", "role", "run_token")} for r in manifest["processes"]],
                "resources": {"listeners": resources["listeners"], **{
                    g: [{"port": r["port"]} for r in resources[g]] for g in ("postgres_databases", "redis_instances")}}}
        for row in manifest.get("processes", []):
            self.add(row, row["role"])
            if row.get("run_token"):
                self.tokens.add(row["run_token"])
        if controller:
            self.add(controller, "controller")
        resources = manifest.get("resources", {})
        for endpoint in resources.get("listeners", {}).values():
            self.ports.add((endpoint["host"], endpoint["port"]))
        for group in ("postgres_databases", "redis_instances"):
            for row in resources.get(group, []):
                self.ports.add(("127.0.0.1", row["port"]))
        # The preflight probe writes identities for temporary nodes that may
        # have exited before the normal process manifest was saved.
        for path in (self.run / "private/traffic/probe").glob("**/node-launch-observed.json"):
            import json
            row = json.loads(path.read_text())
            self.add({"pid": row["pid"], "ticks": row["start_ticks"]}, "probe-node")
        visible = {}
        for path in Path("/proc").glob("[0-9]*"):
            try:
                if path.stat().st_uid == os.getuid():
                    row = process(int(path.name))
                    visible[row["pid"]] = row
            except (FileNotFoundError, ProcessLookupError):
                continue
            except PermissionError:
                self.capture_errors.append({"kind": "proc-stat-denied", "pid": int(path.name)})
        roots = {pid for (pid, ticks) in self.known if pid in visible and visible[pid]["ticks"] == ticks}
        while True:
            children = [row for row in visible.values() if row["ppid"] in roots and row["pid"] not in roots]
            if not children:
                break
            for row in children:
                roots.add(row["pid"])
                self.add(row, "os-child")
        for pid in roots:
            row = visible[pid]
            saved = self.known[(pid, row["ticks"])]
            try:
                exe = Path(f"/proc/{pid}/exe")
                saved["executable_path"] = os.readlink(exe)
                if not saved.get("executable_sha256"):
                    saved["executable_sha256"] = sha(exe)
                for fd in Path(f"/proc/{pid}/fd").iterdir():
                    try:
                        target = os.readlink(fd)
                        if target.startswith("socket:["):
                            self.socket_inodes.add(target[8:-1])
                        elif inside(target, self.run):
                            meta = fd.stat()
                            self.file_inodes.add((os.major(meta.st_dev), os.minor(meta.st_dev), meta.st_ino))
                    except FileNotFoundError:
                        pass
            except (FileNotFoundError, ProcessLookupError):
                pass
            except PermissionError:
                self.capture_errors.append({"kind": "owned-proc-denied", "pid": pid})
        if self.run.exists():
            for path in self.run.rglob("*"):
                try:
                    meta = path.lstat()
                    if stat.S_ISSOCK(meta.st_mode):
                        self.unix.add(str(path))
                    if stat.S_ISREG(meta.st_mode):
                        self.file_inodes.add((os.major(meta.st_dev), os.minor(meta.st_dev), meta.st_ino))
                except FileNotFoundError:
                    pass

    def finish(self, grace_secs=15):
        deadline = time.monotonic() + grace_secs
        while True:
            remaining = []
            for pid, ticks in self.known:
                try:
                    row = process(pid)
                    if row["ticks"] == ticks:
                        remaining.append(row)  # Zombies are not silently counted as gone.
                except (FileNotFoundError, ProcessLookupError):
                    pass
                except PermissionError:
                    self.capture_errors.append({"kind": "final-proc-denied", "pid": pid})
            if not remaining or time.monotonic() >= deadline:
                break
            time.sleep(0.1)
        records = []
        survivors = {(r["pid"], r["ticks"]): r for r in remaining}
        for key, value in sorted(self.known.items()):
            records.append({**value, "status": "gone_or_reused" if key not in survivors else
                            "zombie_unreaped" if survivors[key]["state"] == "Z" else "survived"})
        listener_results = []
        live_sockets = tcp_inodes()
        for inode in sorted(self.socket_inodes):
            listener_results.append({"kind": "owned_socket_inode", "inode": inode, "released": inode not in live_sockets})
        for host, port in sorted(self.ports):
            family = socket.AF_INET6 if ":" in host else socket.AF_INET
            with socket.socket(family) as probe:
                probe.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                try:
                    probe.bind((host, port))
                    free = True
                except OSError:
                    free = False  # Foreign reuse stays ambiguous, not a fabricated release.
                listener_results.append({"kind": "planned_tcp_endpoint", "host": host, "port": port, "released": free})
        for path in sorted(self.unix):
            with socket.socket(socket.AF_UNIX) as probe:
                probe.settimeout(0.2)
                error = probe.connect_ex(path)
                listener_results.append({"kind": "run_unix_socket", "path": path,
                                         "released": error in (errno.ENOENT, errno.ECONNREFUSED),
                                         "path_retained": Path(path).exists()})
        refs, denied = [], []
        # Every readable same-uid process is examined for run-contained file
        # references or an exact recorded run token, including reparented ones.
        for path in Path("/proc").glob("[0-9]*"):
            try:
                if path.stat().st_uid != os.getuid():
                    continue
                identity = process(int(path.name))
                env = (path / "environ").read_bytes().split(b"\0")
                matched = any(e == b"SPC_LAB_RUN_TOKEN=" + t.encode() for e in env for t in self.tokens)
                if matched:
                    refs.append({"pid": identity["pid"], "ticks": identity["ticks"], "kind": "run_token"})
                links = [path / "cwd", *list((path / "fd").iterdir())]
                for link in links:
                    try:
                        target = os.readlink(link)
                        if inside(target, self.run):
                            refs.append({"pid": identity["pid"], "ticks": identity["ticks"], "kind": "file_reference", "target": target})
                        elif target.startswith("socket:[") and target[8:-1] in self.socket_inodes:
                            refs.append({"pid": identity["pid"], "ticks": identity["ticks"], "kind": "owned_socket_reference", "target": target})
                    except FileNotFoundError:
                        pass
                for line in (path / "maps").read_text().splitlines():
                    fields = line.split(maxsplit=5)
                    if len(fields) == 6 and inside(fields[5], self.run):
                        refs.append({"pid": identity["pid"], "ticks": identity["ticks"], "kind": "mapped_run_file", "target": fields[5]})
            except (FileNotFoundError, ProcessLookupError):
                continue
            except PermissionError:
                denied.append(int(path.name))
        locks = []
        for line in Path("/proc/locks").read_text().splitlines():
            for word in line.split():
                if word.count(":") == 2:
                    try:
                        a, b, c = word.split(":")
                        key = (int(a, 16), int(b, 16), int(c))
                        if key in self.file_inodes:
                            locks.append(line)
                    except ValueError:
                        pass
        errors = coverage_errors(self.active_manifest)
        inventory = {"active_manifest": self.active_manifest, "coverage_errors": errors,
                     "observed_socket_inodes": sorted(self.socket_inodes), "observed_unix_paths": sorted(self.unix),
                     "run_tokens": sorted(self.tokens), "observed_file_inodes": sorted(self.file_inodes),
                     "processes": records, "listeners": listener_results, "file_references": refs,
                     "locks": locks, "capture_errors": self.capture_errors, "unreadable_same_uid_processes": sorted(set(denied))}
        complete = not (errors or remaining or refs or locks or denied or self.capture_errors) and all(r["released"] for r in listener_results)
        return {"schema": "spc.lab.ap5.cleanup/v2", "scope": "manifest, controller, observed descendants, probe records, exact run tokens, planned listeners, run files and locks",
                "inventory": inventory, "inventory_sha256": hashlib.sha256(encoded(inventory)).hexdigest(),
                "exhaustive": not errors and not denied and not self.capture_errors, "all_resources_released": complete,
                "status": "passed" if complete else "inconclusive" if errors or denied or self.capture_errors else "failed"}
