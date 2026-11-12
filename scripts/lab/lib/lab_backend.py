#!/usr/bin/env python3
"""LP-4a link-fault backend: Linux network namespaces, veth pairs, nftables, tc.

This program is the supervisor of one lab topology.  It is started inside an
ephemeral user + network namespace created by `unshare -Urn --map-root-user`
and it is the only process that touches the namespaces, veth pairs, nft tables
and qdiscs it creates.

Namespace handles
-----------------
`/run/netns` is root owned on most hosts, so the backend never writes there.
Instead it creates one network namespace per endpoint with
`unshare(CLONE_NEWNET)` in a forked holder process and keeps the holder PID.
A namespace is entered with `setns("/proc/<holder-pid>/ns/net")`.

The controller process itself never changes its network namespace and never
forks from a request thread.  Every command that has to run inside an
endpoint namespace is executed as an external `nsenter --net` process, which
is a fresh single threaded process: the namespace change cannot leak between
threads and there is no fork-in-a-threaded-controller hazard.

Control plane
-------------
A Unix domain socket inside the run directory.  One JSON request per line in,
one JSON response per line out.  The socket lives on the filesystem, so it
works across the network namespace boundary.  Requests are served by bounded
worker threads, so a long `exec` never delays `apply`, `heal` or `down`.

Ownership
---------
Every resource the run creates carries the per-run token: interface names,
nft table names, holder argv, fixture argv, the socket path and the run
directory.  Cleanup only ever touches resources whose name contains the token
and whose PID still matches its recorded start time token.
"""

from __future__ import annotations

import argparse
import ctypes
import errno
import fcntl
import json
import os
import queue
import shutil
import signal
import socket
import subprocess
import sys
import threading
import time

CLONE_NEWNET = 0x40000000
PR_SET_PDEATHSIG = 1

_libc = ctypes.CDLL("libc.so.6", use_errno=True)

# Bound every wait so a broken host fails the run instead of hanging it.
DEFAULT_TIMEOUT_S = 30.0
MAX_TIMEOUT_S = 3600.0
# 256 KiB per stream.  JSON escaping can expand a byte into six characters,
# so the worst case response stays well below the socket client's cap.
MAX_CAPTURE_BYTES = 256 * 1024
MAX_REQUEST_BYTES = 1024 * 1024
MAX_CONCURRENT_REQUESTS = 8
MAX_CONTROL_REQUESTS = 4
MAX_ENDPOINTS = 32
MAX_LINKS = 64
PROBE_PORT = 9  # discard; nothing in the lab listens on it
LAB_PREFIX = "10.250."

# Errno classes.
UNREACHABLE_ERRNOS = {
    errno.ENETUNREACH,
    errno.EHOSTUNREACH,
    errno.EADDRNOTAVAIL,
    errno.ENETDOWN,
    errno.EHOSTDOWN,
}
# A timeout is "the packet went out and nothing answered".  For IPv4 that
# would mean a silent filter on a routable path (a bypass); for an IPv6
# link-local target it is the normal answer, because the neighbour
# discovery request is itself dropped.
IPV4_STRICT_UNREACHABLE = {errno.ENETUNREACH, errno.EHOSTUNREACH, errno.EADDRNOTAVAIL}


# --------------------------------------------------------------------- helpers
def now_ms() -> int:
    return int(time.time() * 1000)


def now_iso(ms: int | None = None) -> str:
    if ms is None:
        ms = now_ms()
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(ms / 1000.0))


def start_time_token(pid: int) -> str | None:
    """Field 22 of /proc/<pid>/stat: the process identity token."""
    try:
        with open("/proc/%d/stat" % pid, "rb") as fh:
            data = fh.read().decode("utf-8", "replace")
    except OSError:
        return None
    idx = data.rfind(")")
    if idx < 0:
        return None
    fields = data[idx + 2:].split()
    if len(fields) < 20:
        return None
    return fields[19]  # field 22


def process_state(pid: int) -> str | None:
    """The /proc/<pid>/stat state field, or None when the pid is gone."""
    try:
        with open("/proc/%d/stat" % pid, "rb") as fh:
            data = fh.read().decode("utf-8", "replace")
    except OSError:
        return None
    idx = data.rfind(")")
    if idx < 0:
        return None
    fields = data[idx + 2:].split()
    return fields[0] if fields else None


def pid_matches(pid: int | None, token: str | None) -> bool:
    """True when the pid is still the recorded process and it is not a zombie."""
    if not pid or not token:
        return False
    if start_time_token(pid) != token:
        return False
    return process_state(pid) != "Z"


def set_pdeathsig() -> None:
    try:
        _libc.prctl(PR_SET_PDEATHSIG, signal.SIGKILL, 0, 0, 0)
    except Exception:
        pass


def close_inherited_fds(keep: set[int]) -> None:
    try:
        import resource
        soft = resource.getrlimit(resource.RLIMIT_NOFILE)[0]
    except Exception:
        soft = 1024
    for fd in range(3, min(soft, 4096)):
        if fd in keep:
            continue
        try:
            os.close(fd)
        except OSError:
            pass


class CommandResult:
    def __init__(self, argv, rc, out, err, timed_out=False, truncated_out=False,
                 truncated_err=False):
        self.argv = list(argv)
        self.rc = rc
        self.stdout = out
        self.stderr = err
        self.timed_out = timed_out
        self.truncated_out = truncated_out
        self.truncated_err = truncated_err

    @property
    def ok(self):
        return self.rc == 0 and not self.timed_out

    def as_dict(self):
        return {
            "argv": self.argv,
            "exit_code": self.rc,
            "stdout": self.stdout,
            "stderr": self.stderr,
            "timed_out": self.timed_out,
            "stdout_truncated": self.truncated_out,
            "stderr_truncated": self.truncated_err,
        }


def run_capped(argv, timeout=DEFAULT_TIMEOUT_S, env=None, max_bytes=MAX_CAPTURE_BYTES) -> CommandResult:
    """Run a command, capture at most max_bytes of each stream, bounded in time."""
    argv = [str(a) for a in argv]
    try:
        proc = subprocess.Popen(
            argv,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            start_new_session=True,
        )
    except OSError as exc:
        return CommandResult(argv, 127, "", "cannot execute %s: %s" % (argv[0], exc))
    out_buf = bytearray()
    err_buf = bytearray()
    flags = {"out": False, "err": False}

    def reader(stream, buf, key):
        try:
            while True:
                chunk = stream.read(65536)
                if not chunk:
                    break
                room = max_bytes - len(buf)
                if room > 0:
                    buf.extend(chunk[:room])
                if len(chunk) > max(0, room):
                    flags[key] = True
        except (OSError, ValueError):
            pass
        finally:
            try:
                stream.close()
            except OSError:
                pass

    threads = [
        threading.Thread(target=reader, args=(proc.stdout, out_buf, "out"), daemon=True),
        threading.Thread(target=reader, args=(proc.stderr, err_buf, "err"), daemon=True),
    ]
    for thread in threads:
        thread.start()
    try:
        proc.wait(timeout=max(0.001, timeout))
        timed_out = False
    except subprocess.TimeoutExpired:
        timed_out = True
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except OSError:
            pass
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            pass
    for thread in threads:
        thread.join(timeout=2)
    rc = proc.returncode if proc.returncode is not None else -1
    return CommandResult(argv, rc, out_buf.decode("utf-8", "replace"),
                         err_buf.decode("utf-8", "replace"), timed_out,
                         flags["out"], flags["err"])


# ------------------------------------------------------------- namespace plumbing
class Namespace:
    """A network namespace owned by this supervisor, addressed by holder PID."""

    def __init__(self, name: str, role: str):
        self.name = name
        self.role = role
        self.pid: int | None = None
        self.start_token: str | None = None

    def create(self) -> "Namespace":
        r, w = os.pipe()
        pid = os.fork()
        if pid == 0:
            try:
                os.close(r)
                set_pdeathsig()
                # The holder must die on SIGTERM: the supervisor's handler would
                # otherwise leave the holder alive.
                signal.signal(signal.SIGTERM, signal.SIG_DFL)
                signal.signal(signal.SIGINT, signal.SIG_DFL)
                if _libc.unshare(CLONE_NEWNET) != 0:
                    err = ctypes.get_errno()
                    os.write(w, ("unshare(CLONE_NEWNET) failed: %s" % os.strerror(err)).encode())
                    os.close(w)
                    os._exit(1)
                os.write(w, b"OK")
                os.close(w)
                while True:
                    time.sleep(3600)
            except BaseException as exc:  # pragma: no cover - child path
                try:
                    os.write(w, ("error: %s" % exc).encode()[:400])
                    os.close(w)
                except Exception:
                    pass
                os._exit(1)
        os.close(w)
        msg = os.read(r, 4096)
        os.close(r)
        if not msg.startswith(b"OK"):
            raise RuntimeError("namespace %s: %s" % (self.name, msg.decode("utf-8", "replace")))
        self.pid = pid
        self.start_token = start_time_token(pid)
        return self

    @property
    def netns_path(self) -> str:
        return "/proc/%d/ns/net" % self.pid

    def alive(self) -> bool:
        return pid_matches(self.pid, self.start_token)


# ------------------------------------------------------------------- topology
class Spawner:
    """A dedicated, long lived thread that creates spawned processes.

    `PR_SET_PDEATHSIG` is delivered when the *thread* that created a child
    exits, not when the whole process exits.  A request worker thread exits
    after every request, so a fixture spawned from it would be killed with
    the worker.  Every spawn therefore happens in this one thread, which
    lives exactly as long as the supervisor: a dead backend still takes its
    fixtures with it, and a live backend keeps them alive.
    """

    def __init__(self):
        self._queue: queue.Queue = queue.Queue()
        self._thread = threading.Thread(target=self._loop, name="lab-spawner", daemon=True)
        self._thread.start()

    def _loop(self):
        while True:
            item = self._queue.get()
            if item is None:
                break
            fn, args, kwargs, result, done = item
            try:
                result["value"] = fn(*args, **kwargs)
            except BaseException as exc:  # noqa: BLE001 - forwarded to the caller
                result["error"] = exc
            finally:
                done.set()

    def submit(self, fn, *args, timeout=120.0, **kwargs):
        result: dict = {}
        done = threading.Event()
        self._queue.put((fn, args, kwargs, result, done))
        if not done.wait(timeout=timeout):
            raise BackendError("spawn_timeout", "the spawn did not complete in time")
        if "error" in result:
            raise result["error"]
        return result.get("value")

    def stop(self):
        self._queue.put(None)


class Endpoint:
    def __init__(self, name: str, role: str):
        self.name = name
        self.role = role
        self.ns = Namespace(name, role)
        self.addresses: list[dict] = []
        self.ipv6_link_local: list[str] = []
        self.forwarding = None

    def as_dict(self) -> dict:
        return {
            "name": self.name,
            "role": self.role,
            "namespace": {
                "holder_pid": self.ns.pid,
                "start_time_token": self.ns.start_token,
                "netns_path": self.ns.netns_path,
                "alive": self.ns.alive(),
            },
            "addresses": self.addresses,
            "ipv6_link_local": self.ipv6_link_local,
            "forwarding": self.forwarding,
        }


class Link:
    def __init__(self, link_id: str, a: str, b: str, index: int, token: str):
        self.id = link_id
        self.a = a
        self.b = b
        self.index = index
        third = index // 64
        fourth = (index % 64) * 4
        self.subnet = "10.250.%d.%d/30" % (third, fourth)
        self.a_addr = "10.250.%d.%d" % (third, fourth + 1)
        self.b_addr = "10.250.%d.%d" % (third, fourth + 2)
        self.a_iface = "l%s%02da" % (token, index)
        self.b_iface = "l%s%02db" % (token, index)
        self.a_table = "lab4a_%s_%02da" % (token, index)
        self.b_table = "lab4a_%s_%02db" % (token, index)
        self.policy = {"type": "normal"}
        self.applied_at_ms: int | None = None

    def as_dict(self) -> dict:
        return {
            "id": self.id,
            "from": self.a,
            "to": self.b,
            "subnet": self.subnet,
            "addresses": {self.a: self.a_addr, self.b: self.b_addr},
            "interfaces": {self.a: self.a_iface, self.b: self.b_iface},
            "nft_tables": {self.a: self.a_table, self.b: self.b_table},
            "policy": self.policy,
            "applied_at_unix_ms": self.applied_at_ms,
        }


# -------------------------------------------------------------------- backend
class BackendError(Exception):
    def __init__(self, code: str, detail: str):
        super().__init__(detail)
        self.code = code
        self.detail = detail


class Backend:
    def __init__(self, run_dir: str, spec: dict, run_token: str, host_addresses,
                 run_id: str = "", idle_timeout_s: float = 3600.0,
                 max_lifetime_s: float = 7200.0):
        self.run_dir = os.path.abspath(run_dir)
        self.spec = spec
        self.token = run_token
        self.run_id = run_id or os.path.basename(self.run_dir)
        self.host_addresses = list(host_addresses or [])
        self.sock_path = os.path.join(self.run_dir, "control.sock")
        self.lock_path = os.path.join(self.run_dir, ".backend.lock")
        self.manifest_path = os.path.join(self.run_dir, "manifest.json")
        self.error_path = os.path.join(self.run_dir, "up-error-%s.json" % run_token)
        self.endpoints: dict[str, Endpoint] = {}
        self.links: dict[str, Link] = {}
        self.children: list[dict] = []
        self._stop = False
        self._socket_created = False
        self._lock_fd: int | None = None
        self._workers: list[threading.Thread] = []
        # Two admission pools: long running commands (exec, spawn) must not
        # starve the control operations (apply, heal, status, counters).
        self._exec_semaphore = threading.BoundedSemaphore(MAX_CONCURRENT_REQUESTS)
        self._control_semaphore = threading.BoundedSemaphore(MAX_CONTROL_REQUESTS)
        self._links_lock = threading.Lock()
        self._link_locks: dict[str, threading.Lock] = {}
        self._state_lock = threading.Lock()
        self.idle_timeout_s = idle_timeout_s
        self.max_lifetime_s = max_lifetime_s
        self._started_at = time.monotonic()
        self._started_at_ms = now_ms()
        self._last_activity = time.monotonic()
        self.tools = self._resolve_tools()
        self._spawner = Spawner()
        self.probe_script = os.path.join(os.path.dirname(os.path.abspath(__file__)), "netns_probe.py")
        self.spawn_helper = os.path.join(os.path.dirname(os.path.abspath(__file__)), "ns_exec.py")
        self.nsenter = self.tools["nsenter"]

    # -- tools -------------------------------------------------------------
    @staticmethod
    def _resolve_tools() -> dict:
        wanted = ("nsenter", "ip", "nft", "tc", "sysctl", "cat")
        found = {}
        for name in wanted:
            path = shutil.which(name)
            if path is None:
                raise BackendError("tool_missing",
                                   "required tool not found: %s" % name)
            found[name] = path
        return found

    # -- setup -------------------------------------------------------------
    def _load_spec(self):
        endpoints = self.spec.get("endpoints") or []
        links = self.spec.get("links") or []
        if not endpoints:
            raise BackendError("invalid_spec", "the topology declares no endpoints")
        if len(endpoints) > MAX_ENDPOINTS:
            raise BackendError("invalid_spec", "too many endpoints (limit %d)" % MAX_ENDPOINTS)
        if len(links) > MAX_LINKS:
            raise BackendError("invalid_spec", "too many links (limit %d)" % MAX_LINKS)
        names = set()
        for ep in endpoints:
            name = ep.get("name")
            if not name or not isinstance(name, str):
                raise BackendError("invalid_spec", "an endpoint has no name")
            if name in names:
                raise BackendError("invalid_spec", "duplicate endpoint name %r" % name)
            names.add(name)
            self.endpoints[name] = Endpoint(name, ep.get("role", "unknown"))
        for idx, ln in enumerate(links):
            link_id = ln.get("id")
            a = ln.get("a")
            b = ln.get("b")
            if not link_id or not a or not b:
                raise BackendError("invalid_spec", "a link is missing id, a or b")
            if a not in self.endpoints or b not in self.endpoints:
                raise BackendError("invalid_spec", "link %r references an unknown endpoint" % link_id)
            if a == b:
                raise BackendError("invalid_spec", "link %r connects an endpoint to itself" % link_id)
            if link_id in self.links:
                raise BackendError("invalid_spec", "duplicate link id %r" % link_id)
            self.links[link_id] = Link(link_id, a, b, idx, self.token)

    def _take_run_lock(self):
        """Exclusive ownership of the run directory.

        The lock is taken before anything is created, so a second `up` can
        never overwrite a live run.  The lock is held for the whole lifetime
        of the backend.
        """
        fd = os.open(self.lock_path, os.O_RDWR | os.O_CREAT, 0o600)
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            os.close(fd)
            raise BackendError("run_dir_in_use",
                               "another backend owns %s" % self.run_dir)
        self._lock_fd = fd

    def _bind_socket(self):
        """Bind the control socket.

        The exclusive run lock is already held, so no live backend owns this
        directory.  A socket file that is still there is therefore stale: it
        is removed and replaced.  A socket that still answers would mean a
        live owner, which the lock rules out, so that case is refused.
        """
        if os.path.exists(self.sock_path):
            if self._socket_answers():
                raise BackendError("run_dir_in_use",
                                   "a live control socket already exists at %s" % self.sock_path)
            try:
                os.unlink(self.sock_path)
            except OSError as exc:
                raise BackendError("run_dir_in_use",
                                   "the stale control socket %s cannot be removed: %s"
                                   % (self.sock_path, exc))
        srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        srv.bind(self.sock_path)
        os.chmod(self.sock_path, 0o600)
        srv.listen(16)
        self._socket_created = True
        return srv

    def _socket_answers(self) -> bool:
        """True when something is still serving the control socket."""
        probe = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        probe.settimeout(1.0)
        try:
            probe.connect(self.sock_path)
            return True
        except OSError:
            return False
        finally:
            probe.close()

    def _create_namespaces(self):
        for ep in self.endpoints.values():
            ep.ns.create()
            self._harden_namespace(ep)

    def _harden_namespace(self, ep: Endpoint):
        """Netns scoped hardening.

        `net.ipv4.ip_forward` is inherited from the host (it is 1 on this
        machine).  Forwarding inside an owned namespace would let a namespace
        transit traffic for another namespace, so it is disabled here.  These
        sysctls are network namespace scoped: the host values are untouched.
        """
        wanted = (
            ("net.ipv4.ip_forward", "0"),
            ("net.ipv6.conf.all.forwarding", "0"),
            ("net.ipv6.conf.default.accept_dad", "0"),
            ("net.ipv6.conf.default.accept_ra", "0"),
        )
        for key, value in wanted:
            res = self._run_in(ep, [self.tools["sysctl"], "-q", "-w", "%s=%s" % (key, value)])
            if not res.ok:
                raise BackendError("namespace_setup_failed",
                                   "sysctl %s=%s in %s: %s" % (key, value, ep.name, res.stderr.strip()))
        ep.forwarding = self._read_forwarding(ep)

    def _read_forwarding(self, ep: Endpoint):
        v4 = self._run_in(ep, [self.tools["cat"], "/proc/sys/net/ipv4/ip_forward"])
        v6 = self._run_in(ep, [self.tools["cat"], "/proc/sys/net/ipv6/conf/all/forwarding"])
        if not (v4.ok and v6.ok):
            return None
        try:
            return {"ipv4": int(v4.stdout.strip()), "ipv6": int(v6.stdout.strip())}
        except ValueError:
            return None

    def _create_links(self):
        for link in self.links.values():
            res = self._run([self.tools["ip"], "link", "add", link.a_iface, "type", "veth",
                             "peer", "name", link.b_iface])
            if not res.ok:
                raise BackendError("link_setup_failed",
                                   "veth pair %s/%s: %s" % (link.a_iface, link.b_iface, res.stderr.strip()))
            for iface, ep_name in ((link.a_iface, link.a), (link.b_iface, link.b)):
                res = self._run([self.tools["ip"], "link", "set", iface, "netns",
                                 str(self.endpoints[ep_name].ns.pid)])
                if not res.ok:
                    raise BackendError("link_setup_failed",
                                       "moving %s into %s: %s" % (iface, ep_name, res.stderr.strip()))
            self._address_side(link, link.a, link.a_iface, link.a_addr)
            self._address_side(link, link.b, link.b_iface, link.b_addr)
        self._collect_ipv6()

    def _address_side(self, link: Link, ep_name: str, iface: str, local: str):
        ep = self.endpoints[ep_name]
        res = self._run_in(ep, [self.tools["ip"], "link", "set", "lo", "up"])
        if not res.ok:
            raise BackendError("link_setup_failed", "ip link set lo up: %s" % res.stderr.strip())
        # Per interface IPv6 knobs, still inside the owned namespace.  accept_dad
        # off makes the kernel generated link-local address immediately usable
        # instead of tentative, which keeps the tests deterministic.
        for key, value in (("accept_dad", "0"), ("accept_ra", "0")):
            res = self._run_in(ep, [self.tools["sysctl"], "-q", "-w",
                                    "net.ipv6.conf.%s.%s=%s" % (iface, key, value)])
            if not res.ok:
                raise BackendError("link_setup_failed",
                                   "sysctl net.ipv6.conf.%s.%s in %s: %s"
                                   % (iface, key, ep_name, res.stderr.strip()))
        res = self._run_in(ep, [self.tools["ip"], "addr", "add", "%s/30" % local, "dev", iface])
        if not res.ok:
            raise BackendError("link_setup_failed", "ip addr add in %s: %s" % (ep_name, res.stderr.strip()))
        res = self._run_in(ep, [self.tools["ip"], "link", "set", iface, "up"])
        if not res.ok:
            raise BackendError("link_setup_failed", "ip link set up in %s: %s" % (ep_name, res.stderr.strip()))
        ep.addresses.append({
            "link_id": link.id,
            "addr": local,
            "prefix": 30,
            "iface": iface,
            "family": "inet",
        })

    def _collect_ipv6(self):
        """Record the kernel generated IPv6 link-local addresses.

        Every veth gets a `fe80::/64` address and a matching route, so an
        IPv6 path across the link exists unless the fault covers it.  The
        backend records them so the no-bypass check can probe them.
        """
        for ep in self.endpoints.values():
            res = self._run_in(ep, [self.tools["ip"], "-j", "-6", "addr", "show"])
            if not res.ok:
                continue
            try:
                data = json.loads(res.stdout or "[]")
            except json.JSONDecodeError:
                continue
            for entry in data:
                iface = entry.get("ifname")
                link_id = None
                for candidate in self.links.values():
                    if iface in (candidate.a_iface, candidate.b_iface):
                        link_id = candidate.id
                        break
                for info in entry.get("addr_info", []) or []:
                    if info.get("family") == "inet6" and info.get("scope") == "link":
                        self.endpoints[ep.name].ipv6_link_local.append(
                            {"iface": iface, "addr": info.get("local"), "link_id": link_id})

    # -- command helpers ---------------------------------------------------
    def _run(self, argv, timeout=DEFAULT_TIMEOUT_S) -> CommandResult:
        return run_capped(argv, timeout=timeout)

    def _run_in(self, ep: Endpoint, argv, timeout=DEFAULT_TIMEOUT_S,
                env=None) -> CommandResult:
        """Run a command inside an endpoint namespace.

        The namespace change happens in a fresh `nsenter` process, so the
        controller thread and the supervisor process never change namespace.
        """
        if not ep.ns.alive():
            raise BackendError("namespace_gone",
                               "the namespace of endpoint %s is gone" % ep.name)
        cmd = [self.nsenter, "-t", str(ep.ns.pid), "-n", "--"] + [str(a) for a in argv]
        child_env = dict(os.environ)
        if env:
            child_env.update(env)
        return run_capped(cmd, timeout=timeout, env=child_env)

    def _link_lock(self, link_id: str) -> threading.Lock:
        with self._links_lock:
            return self._link_locks.setdefault(link_id, threading.Lock())

    # -- manifest ----------------------------------------------------------
    def manifest(self) -> dict:
        started = self._started_at_ms
        limits = {
            "max_endpoints": MAX_ENDPOINTS,
            "max_links": MAX_LINKS,
            "default_command_timeout_s": DEFAULT_TIMEOUT_S,
            "max_command_timeout_s": MAX_TIMEOUT_S,
            "max_capture_bytes": MAX_CAPTURE_BYTES,
            "max_request_bytes": MAX_REQUEST_BYTES,
            "max_concurrent_requests": MAX_CONCURRENT_REQUESTS,
            "idle_timeout_s": self.idle_timeout_s,
            "max_lifetime_s": self.max_lifetime_s,
            "zero_disables_the_budget": True,
            "kill_on_expiry": True,
            "leave_for_adoption": False,
            "adoption_model": "pdeathsig",
            "supports_survive_supervisor": False,
            "started_at_unix_ms": started,
            "idle_deadline_unix_ms": (int(started + self.idle_timeout_s * 1000)
                                      if self.idle_timeout_s else None),
            "lifetime_deadline_unix_ms": (int(started + self.max_lifetime_s * 1000)
                                          if self.max_lifetime_s else None),
            "address_space": "10.250.0.0/16",
        }
        return {
            "schema": "spc.lab.link_manifest.v1",
            "run_id": self.run_id,
            "run_token": self.token,
            "backend": "netns",
            "isolation": "netns",
            "created_at_unix_ms": started,
            "created_at_iso": now_iso(started),
            "control_socket": self.sock_path,
            "fixture_script": self.spec.get("fixture_script", ""),
            "supervisor": {
                "pid": os.getpid(),
                "start_time_token": start_time_token(os.getpid()),
                "argv_contains_token": self.token in " ".join(sys.argv),
            },
            "tools": self.tools,
            "user_namespace": {
                "note": "the supervisor runs inside `unshare -Urn`; the user namespace is "
                        "owned by the run and dies with it",
            },
            "lifetime_semantics": {
                "graceful_stop": "`down` terminates the fixtures and the namespace holders, "
                                 "reaps them and removes the control socket",
                "supervisor_death": "the namespace holders and the spawned fixtures carry "
                                    "PR_SET_PDEATHSIG=SIGKILL, so they die with the "
                                    "supervisor; nothing is left for adoption",
                "supports_survive_supervisor": False,
                "adoption_model": "pdeathsig",
                "note": "surviving the supervisor is not implemented and is not claimed; "
                        "reconnecting to a live backend is a different thing and is "
                        "supported by the control socket",
            },
            "endpoints": [ep.as_dict() for ep in self.endpoints.values()],
            "links": [ln.as_dict() for ln in self.links.values()],
            "children": self.children,
            "host_addresses": [addr for family, addr in self.host_addresses if family == "inet"],
            "host_addresses_v6": [addr for family, addr in self.host_addresses if family == "inet6"],
            "guarantees": self.guarantees(),
            "unsupported_requests": self.unsupported_requests(),
            "guarantee_scope": {
                "sustained_partition": "all traffic on the link interface, IPv4 and IPv6, "
                                       "both directions, until the fault is healed",
                "no_bypass_routing_enforced": "IPv4 and IPv6 routing of the lab namespaces; "
                                              "forwarding is disabled in every namespace",
                "heal_restores_delivery": "the fault is removed and the link carries traffic again",
            },
            "limits": limits,
        }

    @staticmethod
    def guarantees() -> list:
        return [
            "isolated_listeners",
            "sustained_partition",
            "link_scoped_fault",
            "heal_restores_delivery",
            "listener_stays_bound_during_fault",
            "no_bypass_routing_enforced",
            "reconnect_failures_observed",
            "netem_latency_and_loss",
            "ipv6_link_local_isolated",
            "namespace_forwarding_disabled",
        ]

    @staticmethod
    def unsupported_requests() -> list:
        return [
            {"request": "full_node_p2p_links", "reason": "LP-4 integration gate", "assertion_state": "not_applicable"},
            {"request": "ark_backend_links", "reason": "LP-4 integration gate", "assertion_state": "not_applicable"},
            {"request": "mining_and_reorg_scenarios", "reason": "LP-4 integration gate", "assertion_state": "not_applicable"},
        ]

    def write_manifest(self):
        """Write the manifest atomically.

        The temp file name is unique per writer, so two threads never share a
        temp path, and the state lock keeps the published snapshot consistent.
        """
        with self._state_lock:
            tmp = "%s.tmp.%d.%d" % (self.manifest_path, os.getpid(), threading.get_ident())
            try:
                with open(tmp, "w") as fh:
                    json.dump(self.manifest(), fh, indent=2, sort_keys=True)
                    fh.write("\n")
                os.chmod(tmp, 0o600)
                os.replace(tmp, self.manifest_path)
                os.chmod(self.manifest_path, 0o600)
            finally:
                if os.path.exists(tmp):
                    try:
                        os.unlink(tmp)
                    except OSError:
                        pass

    # -- faults ------------------------------------------------------------
    def _table_exists(self, ep: Endpoint, table: str):
        """(exists, error).  `error` is set when the query itself failed."""
        res = self._run_in(ep, [self.tools["nft"], "-j", "list", "table", "inet", table])
        if _truncated(res):
            return False, "the nft list table %s output was truncated" % table
        if res.ok:
            return True, None
        text = (res.stderr or "").lower()
        if "no such file or directory" in text:
            return False, None
        return False, "nft list table %s failed: %s" % (table, res.stderr.strip())

    def _nft_transaction(self, ep: Endpoint, table: str, iface: str, replace: bool):
        """Install the partition ruleset as one atomic nftables transaction.

        `nft -f` sends the whole file as a single netlink batch, so the kernel
        applies it atomically: a reapply replaces the rules without a window
        in which the link is open.
        """
        lines = []
        if replace:
            lines.append("delete table inet %s" % table)
        lines.append("add table inet %s" % table)
        lines.append("add chain inet %s out { type filter hook output priority 0; policy accept; }" % table)
        lines.append("add chain inet %s in { type filter hook input priority 0; policy accept; }" % table)
        lines.append('add rule inet %s out oifname "%s" counter drop comment "lab4a-partition-out"' % (table, iface))
        lines.append('add rule inet %s in iifname "%s" counter drop comment "lab4a-partition-in"' % (table, iface))
        payload = ("\n".join(lines) + "\n").encode()
        return self._run_nft_file(ep, payload)

    def _run_nft_file(self, ep: Endpoint, payload: bytes) -> CommandResult:
        """Feed a ruleset to `nft -f -` on stdin."""
        if not ep.ns.alive():
            raise BackendError("namespace_gone",
                               "the namespace of endpoint %s is gone" % ep.name)
        cmd = [self.nsenter, "-t", str(ep.ns.pid), "-n", "--", self.tools["nft"], "-f", "-"]
        try:
            proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                    stderr=subprocess.PIPE, start_new_session=True)
        except OSError as exc:
            return CommandResult(cmd, 127, "", "cannot execute nft: %s" % exc)
        out, err = proc.communicate(payload, timeout=DEFAULT_TIMEOUT_S)
        return CommandResult(cmd, proc.returncode, out.decode("utf-8", "replace"),
                             err.decode("utf-8", "replace"))

    def _tc_apply(self, ep: Endpoint, iface: str, netem_args) -> CommandResult:
        return self._run_in(ep, [self.tools["tc"], "qdisc", "replace", "dev", iface, "root",
                                 "netem"] + list(netem_args))

    def _tc_clear(self, ep: Endpoint, iface: str) -> CommandResult:
        return self._run_in(ep, [self.tools["tc"], "qdisc", "del", "dev", iface, "root"])

    def _verify_qdisc(self, ep: Endpoint, iface: str, expect_netem: bool):
        res = self._run_in(ep, [self.tools["tc"], "-j", "qdisc", "show", "dev", iface])
        if _truncated(res):
            return "the tc qdisc show output on %s was truncated" % iface
        if not res.ok:
            return "tc qdisc show failed on %s: %s" % (iface, res.stderr.strip())
        try:
            data = json.loads(res.stdout or "[]")
        except json.JSONDecodeError:
            return "tc qdisc show returned invalid JSON on %s" % iface
        has_netem = any(entry.get("kind") == "netem" for entry in data or [])
        if expect_netem and not has_netem:
            return "no netem qdisc is installed on %s" % iface
        if not expect_netem and has_netem:
            return "a netem qdisc is still installed on %s" % iface
        return None

    def _verify_partition(self, ep: Endpoint, table: str):
        exists, error = self._table_exists(ep, table)
        if error:
            return error
        if not exists:
            return "the partition table %s is missing" % table
        res = self._run_in(ep, [self.tools["nft"], "-j", "list", "table", "inet", table])
        if _truncated(res):
            return "the nft list table %s output was truncated" % table
        try:
            data = json.loads(res.stdout or "{}")
        except json.JSONDecodeError:
            return "the partition table %s cannot be listed" % table
        rules = [item for item in data.get("nftables", []) if "rule" in item]
        if len(rules) < 2:
            return "the partition table %s has %d rules, expected 2" % (table, len(rules))
        return None

    def apply_policy(self, link_id: str, policy: dict) -> dict:
        link = self.links.get(link_id)
        if link is None:
            raise BackendError("unknown_link", "no link %r in this topology" % link_id)
        ptype = policy.get("type")
        if ptype not in ("normal", "partition", "latency", "loss"):
            raise BackendError("invalid_policy", "unknown policy type %r" % ptype)
        with self._link_lock(link_id):
            return self._apply_policy_locked(link, policy)

    def _apply_policy_locked(self, link: Link, policy: dict) -> dict:
        try:
            return self._apply_policy_body(link, policy)
        except BackendError as exc:
            # N5: a half applied fault must be recorded, not silently left at
            # the previous value.  The state becomes "partial" with the side
            # that failed, so a caller can see what is installed.
            if link.policy.get("type") != "partial":
                link.policy = {"type": "partial", "requested": dict(policy),
                               "failed_side": None, "error": exc.detail}
            self.write_manifest()
            raise

    def _apply_policy_body(self, link: Link, policy: dict) -> dict:
        ptype = policy["type"]
        sides = ((link.a, link.a_iface, link.a_table),
                 (link.b, link.b_iface, link.b_table))
        evidence = {"nft_tables": [], "qdisc": None, "steps": [], "atomic_transaction": True}

        if ptype == "partition":
            for ep_name, iface, table in sides:
                ep = self.endpoints[ep_name]
                exists, error = self._table_exists(ep, table)
                if error:
                    raise BackendError("policy_apply_failed", error)
                res = self._nft_transaction(ep, table, iface, replace=exists)
                evidence["steps"].append({"endpoint": ep_name, "action": "nft_partition",
                                           "exit_code": res.rc, "stderr": res.stderr.strip()})
                if not res.ok:
                    link.policy = {"type": "partial", "requested": dict(policy),
                                   "failed_side": ep_name,
                                   "error": "nft partition in %s: %s" % (ep_name, res.stderr.strip())}
                    raise BackendError("policy_apply_failed",
                                       "nft partition in %s: %s" % (ep_name, res.stderr.strip()))
                problem = self._verify_partition(ep, table)
                if problem:
                    raise BackendError("policy_apply_failed", problem)
                evidence["nft_tables"].append(table)
        elif ptype in ("latency", "loss"):
            if ptype == "latency":
                ms = int(policy.get("delay_ms", 0))
                if ms <= 0 or ms > 60000:
                    raise BackendError("invalid_policy", "delay_ms out of range: %r" % policy.get("delay_ms"))
                netem = ["delay", "%dms" % ms]
                evidence["qdisc"] = "netem delay %dms" % ms
            else:
                pct = int(policy.get("loss_percent", 0))
                if pct < 0 or pct > 100:
                    raise BackendError("invalid_policy", "loss_percent out of range: %r" % policy.get("loss_percent"))
                netem = ["loss", "%d%%" % pct]
                evidence["qdisc"] = "netem loss %d%%" % pct
            for ep_name, iface, table in sides:
                ep = self.endpoints[ep_name]
                res = self._tc_apply(ep, iface, netem)
                evidence["steps"].append({"endpoint": ep_name, "action": "tc_netem",
                                           "exit_code": res.rc, "stderr": res.stderr.strip()})
                if not res.ok:
                    link.policy = {"type": "partial", "requested": dict(policy),
                                   "failed_side": ep_name,
                                   "error": "tc netem in %s: %s" % (ep_name, res.stderr.strip())}
                    raise BackendError("policy_apply_failed",
                                       "tc netem in %s: %s" % (ep_name, res.stderr.strip()))
                problem = self._verify_qdisc(ep, iface, expect_netem=True)
                if problem:
                    raise BackendError("policy_apply_failed", problem)
        else:  # normal / heal
            for ep_name, iface, table in sides:
                ep = self.endpoints[ep_name]
                exists, error = self._table_exists(ep, table)
                if error:
                    raise BackendError("policy_apply_failed", error)
                if exists:
                    res = self._run_in(ep, [self.tools["nft"], "delete", "table", "inet", table])
                    evidence["steps"].append({"endpoint": ep_name, "action": "nft_delete",
                                               "exit_code": res.rc, "stderr": res.stderr.strip()})
                    if not res.ok:
                        raise BackendError("policy_apply_failed",
                                           "nft delete table %s in %s: %s"
                                           % (table, ep_name, res.stderr.strip()))
                else:
                    evidence["steps"].append({"endpoint": ep_name, "action": "nft_delete",
                                               "exit_code": 0, "stderr": "table already absent"})
                still, error = self._table_exists(ep, table)
                if error:
                    raise BackendError("policy_apply_failed", error)
                if still:
                    raise BackendError("policy_apply_failed",
                                       "the partition table %s still exists after the heal" % table)
                res = self._tc_clear(ep, iface)
                if res.rc != 0 and "Cannot delete qdisc" not in (res.stderr or ""):
                    evidence["steps"].append({"endpoint": ep_name, "action": "tc_qdisc_del",
                                               "exit_code": res.rc, "stderr": res.stderr.strip()})
                    raise BackendError("policy_apply_failed",
                                       "tc qdisc del in %s: %s" % (ep_name, res.stderr.strip()))
                problem = self._verify_qdisc(ep, iface, expect_netem=False)
                if problem:
                    raise BackendError("policy_apply_failed", problem)
        link.policy = dict(policy)
        link.applied_at_ms = now_ms()
        self.write_manifest()
        return {
            "link": link.id,
            "policy": link.policy,
            "applied_at_unix_ms": link.applied_at_ms,
            "applied_at_iso": now_iso(link.applied_at_ms),
            "evidence": evidence,
        }

    # -- counters ----------------------------------------------------------
    def counters(self, link_id: str) -> dict:
        link = self.links.get(link_id)
        if link is None:
            raise BackendError("unknown_link", "no link %r in this topology" % link_id)
        out = {"link": link.id, "policy": link.policy, "sides": {}, "errors": []}
        for ep_name, iface, table, peer in ((link.a, link.a_iface, link.a_table, link.b_addr),
                                            (link.b, link.b_iface, link.b_table, link.a_addr)):
            ep = self.endpoints[ep_name]
            nft = self._run_in(ep, [self.tools["nft"], "-j", "list", "table", "inet", table])
            tc = self._run_in(ep, [self.tools["tc"], "-j", "-s", "qdisc", "show", "dev", iface])
            ipl = self._run_in(ep, [self.tools["ip"], "-j", "-s", "link", "show", "dev", iface])
            # A missing table is the normal state when no fault is applied.
            # Only an unexpected tool failure is an evidence error.
            nft_absent = (not nft.ok) and "no such file or directory" in (nft.stderr or "").lower()
            out["sides"][ep_name] = {
                "interface": iface,
                "nft": _parse_json(nft.stdout),
                "nft_present": nft.ok,
                "nft_absent": nft_absent,
                "nft_error": None if (nft.ok or nft_absent) else nft.stderr.strip(),
                "qdisc": _parse_json(tc.stdout),
                "qdisc_error": None if tc.ok else tc.stderr.strip(),
                "link": _parse_json(ipl.stdout),
                "link_error": None if ipl.ok else ipl.stderr.strip(),
            }
            if not nft.ok and not nft_absent:
                out["errors"].append({"endpoint": ep_name, "source": "nft",
                                       "exit_code": nft.rc, "error": nft.stderr.strip()})
            elif _truncated(nft):
                out["errors"].append({"endpoint": ep_name, "source": "nft",
                                       "exit_code": nft.rc,
                                       "error": "the nft list table output was truncated"})
            for key, res in (("tc", tc), ("ip_link", ipl)):
                if _truncated(res):
                    out["errors"].append({"endpoint": ep_name, "source": key,
                                           "exit_code": res.rc,
                                           "error": "the %s output was truncated" % key})
                elif not res.ok:
                    out["errors"].append({"endpoint": ep_name, "source": key,
                                           "exit_code": res.rc, "error": res.stderr.strip()})
        out["summary"] = self._counter_summary(out)
        out["summary"]["complete"] = not out["errors"]
        out["summary"]["errors"] = out["errors"]
        return out

    def _counter_summary(self, counters: dict) -> dict:
        summary = {"nft_dropped_packets": 0, "nft_dropped_bytes": 0, "tx_packets": 0,
                   "tx_dropped": 0, "rx_packets": 0, "rx_dropped": 0, "qdisc_drops": 0,
                   "qdisc_packets": 0}
        for side in counters.get("sides", {}).values():
            nft = side.get("nft") or {}
            for item in nft.get("nftables", []) or []:
                rule = item.get("rule")
                rules = rule if isinstance(rule, list) else ([rule] if rule else [])
                for one in rules:
                    for expr in one.get("expr", []) or []:
                        counter = expr.get("counter")
                        if counter:
                            summary["nft_dropped_packets"] += int(counter.get("packets", 0))
                            summary["nft_dropped_bytes"] += int(counter.get("bytes", 0))
            for q in (side.get("qdisc") or []):
                stats = q.get("stats") or q
                summary["qdisc_drops"] += int(stats.get("drops", 0) or 0)
                summary["qdisc_packets"] += int(stats.get("packets", 0) or 0)
                xstats = q.get("xstats") or {}
                for key in ("dropped", "overlimits"):
                    if key in xstats:
                        try:
                            summary["tx_dropped"] += int(xstats[key])
                        except (TypeError, ValueError):
                            pass
            for l in (side.get("link") or []):
                stats = l.get("stats64") or l.get("stats") or {}
                tx = stats.get("tx", {}) or {}
                rx = stats.get("rx", {}) or {}
                summary["tx_packets"] += int(tx.get("packets", 0) or 0)
                summary["rx_packets"] += int(rx.get("packets", 0) or 0)
                summary["tx_dropped"] += int(tx.get("dropped", 0) or 0)
                summary["rx_dropped"] += int(rx.get("dropped", 0) or 0)
        return summary

    # -- no bypass ---------------------------------------------------------
    def _routes(self, ep: Endpoint, family: str):
        res = self._run_in(ep, [self.tools["ip"], "-j", "-%s" % family, "route", "show"])
        if _truncated(res):
            return None, "the ip -%s route show output was truncated" % family
        if not res.ok:
            return None, "ip -%s route show failed: %s" % (family, res.stderr.strip())
        try:
            return json.loads(res.stdout or "[]"), None
        except json.JSONDecodeError:
            return None, "ip -%s route show returned invalid JSON" % family

    def _addresses(self, ep: Endpoint, family: str):
        res = self._run_in(ep, [self.tools["ip"], "-j", "-%s" % family, "addr", "show"])
        if _truncated(res):
            return None, "the ip -%s addr show output was truncated" % family
        if not res.ok:
            return None, "ip -%s addr show failed: %s" % (family, res.stderr.strip())
        try:
            return json.loads(res.stdout or "[]"), None
        except json.JSONDecodeError:
            return None, "ip -%s addr show returned invalid JSON" % family

    def _probe(self, ep: Endpoint, host: str, family: str, iface: str = "",
               timeout: float = 1.0) -> dict:
        if not ep.ns.alive():
            return {"outcome": "probe_failed", "errno": None,
                    "error": "the namespace of endpoint %s is gone" % ep.name}
        cmd = [self.nsenter, "-t", str(ep.ns.pid), "-n", "--", sys.executable,
               self.probe_script, "--host", host, "--port", str(PROBE_PORT),
               "--family", family, "--timeout", str(timeout)]
        if iface:
            cmd += ["--iface", iface]
        res = run_capped(cmd, timeout=timeout + 5.0)
        if not res.ok:
            return {"outcome": "probe_failed", "errno": None,
                    "error": res.stderr.strip() or "probe command failed"}
        try:
            return json.loads(res.stdout.strip() or "{}")
        except json.JSONDecodeError:
            return {"outcome": "probe_failed", "errno": None, "error": "invalid probe output"}

    @staticmethod
    def _is_unreachable(probe: dict, family: str) -> bool:
        if probe.get("outcome") != "connect_failed":
            return False
        code = probe.get("errno")
        if code is None:
            return False
        if code == errno.ECONNREFUSED:
            # The target answered: the path is real.
            return False
        if family == "inet":
            return code in IPV4_STRICT_UNREACHABLE
        # For IPv6 link-local targets the neighbour discovery request is
        # itself dropped, so a timeout is the expected "not reachable".
        return code in UNREACHABLE_ERRNOS or code == errno.ETIMEDOUT

    @staticmethod
    def _is_reachable(probe: dict) -> bool:
        if probe.get("outcome") == "connected":
            return True
        return probe.get("errno") == errno.ECONNREFUSED

    def no_bypass_check(self) -> dict:
        checks = []
        violations = []
        faulted = {link.id for link in self.links.values() if link.policy.get("type") != "normal"}

        def violation(kind, **fields):
            violations.append({"kind": kind, **fields})

        # 1. routing tables, IPv4 and IPv6.
        for ep in self.endpoints.values():
            attached = set()
            for addr in ep.addresses:
                attached.add(_subnet_of(addr["addr"], addr["prefix"]))
            for family in ("4", "6"):
                routes, error = self._routes(ep, family)
                if error:
                    violation("evidence_unavailable", endpoint=ep.name, family=family, detail=error)
                    checks.append({"kind": "routing_table", "endpoint": ep.name, "family": family,
                                   "error": error})
                    continue
                default_routes = [r for r in routes if r.get("dst") in (None, "default")]
                foreign = []
                for route in routes:
                    dst = route.get("dst")
                    if dst in (None, "default"):
                        continue
                    if family == "4":
                        prefix = 32
                        if "/" in dst:
                            prefix = int(dst.split("/")[1])
                        if _subnet_of(dst.split("/")[0], prefix) not in attached:
                            foreign.append(route)
                    else:
                        # The only IPv6 routes a lab namespace may carry are the
                        # kernel generated link-local routes of the interfaces it
                        # owns.  A global, ULA, multicast or documentation
                        # prefix route is a bypass path exactly like an
                        # unattached IPv4 route, so it is flagged as well.
                        owned = [a["iface"] for a in ep.addresses]
                        if not dst.startswith("fe80:"):
                            foreign.append(route)
                        else:
                            dev = route.get("dev")
                            if dev and dev not in owned:
                                foreign.append(route)
                checks.append({"kind": "routing_table", "endpoint": ep.name, "family": family,
                               "routes": routes, "attached_subnets": sorted(attached),
                               "default_route": bool(default_routes),
                               "foreign_routes": foreign})
                if default_routes:
                    violation("default_route", endpoint=ep.name, family=family, routes=default_routes)
                if foreign:
                    violation("unattached_route", endpoint=ep.name, family=family, routes=foreign)

        # 2. forwarding must be disabled inside every namespace.
        for ep in self.endpoints.values():
            forwarding = self._read_forwarding(ep)
            ep.forwarding = forwarding
            checks.append({"kind": "forwarding", "endpoint": ep.name, "forwarding": forwarding})
            if forwarding is None:
                violation("evidence_unavailable", endpoint=ep.name,
                          detail="the forwarding sysctls could not be read")
            elif forwarding.get("ipv4") != 0 or forwarding.get("ipv6") != 0:
                violation("forwarding_enabled", endpoint=ep.name, forwarding=forwarding)

        # 3. cross endpoint reachability, IPv4 and IPv6.
        for a in self.endpoints.values():
            a_ifaces = [addr["iface"] for addr in a.addresses] or None
            for b in self.endpoints.values():
                if a.name == b.name:
                    continue
                for addr in b.addresses:
                    link = self.links[addr["link_id"]]
                    adjacent = a.name in (link.a, link.b)
                    probe = self._probe(a, addr["addr"], "inet")
                    reachable = self._is_reachable(probe)
                    unreachable = self._is_unreachable(probe, "inet")
                    checks.append({"kind": "non_link_address" if not adjacent else "link_address",
                                   "from": a.name, "to": b.name, "target": addr["addr"],
                                   "family": "inet", "link_id": addr["link_id"],
                                   "adjacent": adjacent, "probe": probe,
                                   "unreachable": unreachable, "reachable": reachable,
                                   "link_faulted": addr["link_id"] in faulted})
                    if adjacent and not reachable:
                        if addr["link_id"] in faulted:
                            # The link is intentionally faulted: an unreachable
                            # neighbour is the expected state, not a bypass.
                            checks[-1]["note"] = "the link is intentionally faulted"
                        else:
                            violation("link_address_unreachable", **{"from": a.name, "to": b.name,
                                                                    "target": addr["addr"], "probe": probe})
                    if not adjacent and not unreachable:
                        violation("bypass_reachable", **{"from": a.name, "to": b.name,
                                                         "target": addr["addr"], "probe": probe})
                # IPv6 link-local addresses of the same endpoint.
                for entry in b.ipv6_link_local:
                    ll = entry.get("addr")
                    link = self.links.get(entry.get("link_id"))
                    if link is None:
                        # Find the link that owns the interface of this address.
                        for candidate in self.links.values():
                            if entry.get("iface") in (candidate.a_iface, candidate.b_iface):
                                link = candidate
                                break
                    adjacent = link is not None and a.name in (link.a, link.b)
                    scope_iface = None
                    if adjacent and link is not None:
                        scope_iface = self._iface_of(link, a.name)
                    elif a_ifaces:
                        scope_iface = a_ifaces[0]
                    probe = self._probe(a, ll, "inet6", iface=scope_iface or "")
                    reachable = self._is_reachable(probe)
                    unreachable = self._is_unreachable(probe, "inet6")
                    checks.append({"kind": "non_link_address" if not adjacent else "link_address",
                                   "from": a.name, "to": b.name, "target": ll,
                                   "family": "inet6", "adjacent": adjacent, "probe": probe,
                                   "unreachable": unreachable, "reachable": reachable,
                                   "link_faulted": bool(link and link.id in faulted)})
                    if adjacent and not reachable:
                        if link and link.id in faulted:
                            checks[-1]["note"] = "the link is intentionally faulted"
                        else:
                            violation("link_address_unreachable", **{"from": a.name, "to": b.name,
                                                                    "target": ll, "probe": probe})
                    if not adjacent and not unreachable:
                        violation("bypass_reachable", **{"from": a.name, "to": b.name,
                                                         "target": ll, "probe": probe})

        # 4. the host must be unreachable from every namespace.
        for a in self.endpoints.values():
            for family, host_addr in self.host_addresses:
                probe = self._probe(a, host_addr, family)
                unreachable = self._is_unreachable(probe, family)
                checks.append({"kind": "host_address", "from": a.name, "target": host_addr,
                               "family": family, "probe": probe, "unreachable": unreachable})
                if not unreachable:
                    violation("host_reachable", **{"from": a.name, "target": host_addr,
                                                   "probe": probe})

        # 5. the nft ruleset of every namespace must only contain lab tables.
        for a in self.endpoints.values():
            res = self._run_in(a, [self.tools["nft"], "-j", "list", "ruleset"])
            if _truncated(res):
                violation("evidence_unavailable", endpoint=a.name,
                          detail="the nft list ruleset output was truncated")
                checks.append({"kind": "nft_ruleset", "endpoint": a.name,
                               "error": "the nft list ruleset output was truncated"})
                continue
            if not res.ok:
                violation("evidence_unavailable", endpoint=a.name,
                          detail="nft list ruleset failed: %s" % res.stderr.strip())
                checks.append({"kind": "nft_ruleset", "endpoint": a.name, "error": res.stderr.strip()})
                continue
            tables = []
            try:
                for item in json.loads(res.stdout or "[]").get("nftables", []):
                    if "table" in item:
                        tables.append(item["table"].get("family", "") + " " + item["table"].get("name", ""))
            except json.JSONDecodeError:
                violation("evidence_unavailable", endpoint=a.name,
                          detail="nft list ruleset returned invalid JSON")
                continue
            foreign = [t for t in tables if not t.split(" ")[-1].startswith("lab4a_%s" % self.token)]
            checks.append({"kind": "nft_ruleset", "endpoint": a.name, "tables": tables,
                           "foreign_tables": foreign})
            if foreign:
                violation("foreign_nft_table", endpoint=a.name, tables=foreign)

        return {
            "backend": "netns",
            "isolation": "netns",
            "checked_at_unix_ms": now_ms(),
            "checks": checks,
            "violations": violations,
            "ok": not violations,
            "faulted_links": sorted(faulted),
            "criterion": "a non-link address must be refused by the kernel: ENETUNREACH, "
                         "EHOSTUNREACH or EADDRNOTAVAIL for IPv4, the same plus ETIMEDOUT for "
                         "IPv6 link-local (the neighbour discovery request is dropped too). "
                         "ECONNREFUSED or a successful connect means the path is real and is "
                         "reported as a bypass. The adjacency cross-check only applies to "
                         "links whose policy is normal; an intentionally faulted link is "
                         "reported as link_faulted instead of a violation, so the check is "
                         "valid in any state.",
        }

    def _iface_of(self, link: Link, endpoint_name: str) -> str | None:
        if endpoint_name == link.a:
            return link.a_iface
        if endpoint_name == link.b:
            return link.b_iface
        return None

    # -- process management ------------------------------------------------
    def spawn(self, ep_name: str, argv, log_path: str, label: str) -> dict:
        """Spawn a background process inside an endpoint namespace.

        The process is created by the dedicated spawner thread so that its
        lifetime is tied to the supervisor, not to the request worker.
        """
        return self._spawner.submit(self._spawn_in_namespace, ep_name, argv, log_path, label)

    def _spawn_in_namespace(self, ep_name: str, argv, log_path: str, label: str) -> dict:
        ep = self.endpoints.get(ep_name)
        if ep is None:
            raise BackendError("unknown_endpoint", "no endpoint %r" % ep_name)
        if not ep.ns.alive():
            raise BackendError("namespace_gone",
                               "the namespace of endpoint %s is gone" % ep.name)
        log_full = log_path if os.path.isabs(log_path) else os.path.join(self.run_dir, log_path)
        os.makedirs(os.path.dirname(log_full), exist_ok=True)
        cmd = [self.nsenter, "-t", str(ep.ns.pid), "-n", "--", sys.executable,
               self.spawn_helper, "--log", log_full, "--status-fd", "1", "--"] + [str(a) for a in argv]
        try:
            proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                    env={**os.environ, "LAB_RUN_TOKEN": self.token,
                                         "LAB_RUN_ID": self.run_id},
                                    start_new_session=True)
        except OSError as exc:
            raise BackendError("spawn_failed", "cannot start the spawn helper: %s" % exc)
        line = b""
        try:
            # The helper reports the exec status synchronously.
            deadline = time.monotonic() + 10.0
            while b"\n" not in line and time.monotonic() < deadline:
                chunk = proc.stdout.readline()
                if not chunk:
                    break
                line += chunk
        except OSError:
            pass
        try:
            status = json.loads(line.decode("utf-8", "replace").strip() or "{}")
        except json.JSONDecodeError:
            status = {}
        if not status.get("exec_ok"):
            try:
                proc.kill()
            except OSError:
                pass
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                pass
            detail = status.get("error") or "the command did not start"
            raise BackendError("spawn_failed",
                               "%s: %s (errno %s)" % (argv[0], detail, status.get("errno")))
        # The helper's own pipes are not needed any more: the status line is
        # the only thing the supervisor reads from them.
        for stream in (proc.stdout, proc.stderr):
            try:
                stream.close()
            except OSError:
                pass
        child_pid = int(status.get("pid") or 0)
        token = start_time_token(child_pid)
        record = {
            "pid": child_pid,
            "start_time_token": token,
            "wrapper_pid": proc.pid,
            "wrapper_start_time_token": start_time_token(proc.pid),
            "endpoint": ep_name,
            "label": label,
            "argv": [str(a) for a in argv],
            "log_path": log_full,
            "spawned_at_unix_ms": now_ms(),
            "alive": True,
        }
        self.children.append(record)
        self.write_manifest()
        return record

    def _child_state(self, child: dict) -> dict:
        pid = child.get("pid")
        alive = pid_matches(pid, child.get("start_time_token"))
        state = process_state(pid) if pid else None
        return {"alive": alive, "state": state}

    def terminate_children(self):
        killed = []
        for child in reversed(self.children):
            wrapper = child.get("wrapper_pid")
            if wrapper and pid_matches(wrapper, child.get("wrapper_start_time_token")):
                try:
                    os.kill(wrapper, signal.SIGTERM)
                    killed.append({"pid": wrapper, "label": child.get("label"), "signal": "SIGTERM"})
                except OSError:
                    pass
            if pid_matches(child.get("pid"), child.get("start_time_token")):
                try:
                    os.kill(child["pid"], signal.SIGTERM)
                    killed.append({"pid": child["pid"], "label": child.get("label"), "signal": "SIGTERM"})
                except OSError:
                    pass
        deadline = time.monotonic() + 3.0
        while time.monotonic() < deadline:
            alive = [c for c in self.children if pid_matches(c.get("pid"), c.get("start_time_token"))]
            if not alive:
                break
            time.sleep(0.05)
        for child in reversed(self.children):
            if pid_matches(child.get("pid"), child.get("start_time_token")):
                try:
                    os.kill(child["pid"], signal.SIGKILL)
                    killed.append({"pid": child["pid"], "label": child.get("label"), "signal": "SIGKILL"})
                except OSError:
                    pass
        return killed

    def cleanup(self):
        report = {"terminated_children": self.terminate_children(), "terminated_namespaces": []}
        try:
            self._spawner.stop()
        except Exception:
            pass
        for ep in self.endpoints.values():
            if pid_matches(ep.ns.pid, ep.ns.start_token):
                try:
                    os.kill(ep.ns.pid, signal.SIGTERM)
                    report["terminated_namespaces"].append({"endpoint": ep.name, "pid": ep.ns.pid})
                except OSError:
                    pass
        deadline = time.monotonic() + 3.0
        while time.monotonic() < deadline:
            pending = False
            for ep in self.endpoints.values():
                if not ep.ns.pid:
                    continue
                try:
                    done, _ = os.waitpid(ep.ns.pid, os.WNOHANG)
                except OSError:
                    done = ep.ns.pid
                if done == 0:
                    pending = True
            if not pending:
                break
            time.sleep(0.05)
        for ep in self.endpoints.values():
            if pid_matches(ep.ns.pid, ep.ns.start_token):
                try:
                    os.kill(ep.ns.pid, signal.SIGKILL)
                except OSError:
                    pass
        # Only remove the socket this process created.
        if self._socket_created:
            try:
                os.unlink(self.sock_path)
            except OSError:
                pass
        if self._lock_fd is not None:
            try:
                fcntl.flock(self._lock_fd, fcntl.LOCK_UN)
                os.close(self._lock_fd)
            except OSError:
                pass
            self._lock_fd = None
        return report

    # -- request dispatch --------------------------------------------------
    def handle(self, request: dict, queue_wait_ms: int = 0) -> dict:
        op = request.get("op")
        handler = {
            "ping": self._op_ping,
            "status": self._op_status,
            "apply": self._op_apply,
            "heal": self._op_heal,
            "counters": self._op_counters,
            "no-bypass-check": self._op_no_bypass,
            "exec": self._op_exec,
            "spawn": self._op_spawn,
            "manifest": self._op_manifest,
            "down": self._op_down,
        }.get(op)
        if handler is None:
            raise BackendError("unknown_op", "unsupported operation %r" % op)
        result = handler(request)
        if isinstance(result, dict):
            result.setdefault("queue_wait_ms", queue_wait_ms)
        return result

    def _op_ping(self, request):
        return {"ok": True, "op": "ping", "pid": os.getpid(), "run_id": self.run_id,
                "token": self.token, "at_unix_ms": now_ms()}

    def _op_status(self, request):
        wanted = request.get("link")
        if wanted and wanted not in self.links:
            raise BackendError("unknown_link", "no link %r in this topology" % wanted)
        links = [link.as_dict() for link in self.links.values() if not wanted or link.id == wanted]
        endpoints = [ep.as_dict() for ep in self.endpoints.values()]
        children = []
        for child in self.children:
            state = self._child_state(child)
            children.append({**child, **state})
        if wanted:
            link = self.links[wanted]
            names = {link.a, link.b}
            endpoints = [ep for ep in endpoints if ep["name"] in names]
            children = [c for c in children if c.get("endpoint") in names]
        return {
            "ok": True,
            "op": "status",
            "run_id": self.run_id,
            "backend": "netns",
            "at_unix_ms": now_ms(),
            "endpoints": endpoints,
            "links": links,
            "children": children,
            "limits": self.manifest()["limits"],
        }

    def _op_apply(self, request):
        link_id = request.get("link")
        policy = request.get("policy") or {}
        result = self.apply_policy(link_id, policy)
        return {"ok": True, "op": "apply", "run_id": self.run_id, **result}

    def _op_heal(self, request):
        link_id = request.get("link")
        result = self.apply_policy(link_id, {"type": "normal"})
        return {"ok": True, "op": "heal", "run_id": self.run_id, **result}

    def _op_counters(self, request):
        result = self.counters(request.get("link"))
        return {"ok": True, "op": "counters", "run_id": self.run_id, **result}

    def _op_no_bypass(self, request):
        result = self.no_bypass_check()
        return {"ok": True, "op": "no-bypass-check", "run_id": self.run_id, **result}

    def _op_exec(self, request):
        ep_name = request.get("endpoint")
        ep = self.endpoints.get(ep_name)
        if ep is None:
            raise BackendError("unknown_endpoint", "no endpoint %r" % ep_name)
        argv = request.get("argv") or []
        if not argv:
            raise BackendError("invalid_request", "exec needs an argv")
        timeout = _bounded_timeout(request.get("timeout_s"))
        res = self._run_in(ep, argv, timeout=timeout,
                           env={"LAB_RUN_TOKEN": self.token, "LAB_RUN_ID": self.run_id})
        # For `exec`, "ok" reports whether the executed command succeeded; the
        # command's own exit status is also carried explicitly.
        return {"ok": res.ok, "op": "exec", "endpoint": ep_name,
                "command_exit_code": res.rc, **res.as_dict()}

    def _op_spawn(self, request):
        ep_name = request.get("endpoint")
        argv = request.get("argv") or []
        log_path = request.get("log_path") or "child.log"
        label = request.get("label") or "child"
        if not argv:
            raise BackendError("invalid_request", "spawn needs an argv")
        record = self.spawn(ep_name, argv, log_path, label)
        return {"ok": True, "op": "spawn", **record}

    def _op_manifest(self, request):
        return {"ok": True, "op": "manifest", "manifest": self.manifest()}

    def _op_down(self, request):
        report = self.cleanup()
        self._stop = True
        return {"ok": True, "op": "down", "run_id": self.run_id, "cleanup": report}

    # -- server ------------------------------------------------------------
    def _out_of_budget(self) -> str | None:
        now = time.monotonic()
        if self.max_lifetime_s and (now - self._started_at) > self.max_lifetime_s:
            return "max_lifetime"
        # Idle means "no requests and no live children".
        live_children = [c for c in self.children
                         if pid_matches(c.get("pid"), c.get("start_time_token"))]
        if self.idle_timeout_s and not live_children and (now - self._last_activity) > self.idle_timeout_s:
            return "idle_timeout"
        if not os.path.isdir(self.run_dir):
            return "run_directory_removed"
        return None

    def _serve_connection(self, conn, srv):
        accepted = time.monotonic()
        try:
            conn.settimeout(120)
            data = b""
            while b"\n" not in data:
                if len(data) > MAX_REQUEST_BYTES:
                    raise BackendError("request_too_large",
                                       "the request exceeds %d bytes" % MAX_REQUEST_BYTES)
                chunk = conn.recv(65536)
                if not chunk:
                    break
                data += chunk
            line = data.split(b"\n", 1)[0]
            request = json.loads(line.decode("utf-8"))
            queue_wait_ms = int((time.monotonic() - accepted) * 1000)
            op = request.get("op")
            if op == "down":
                response = self.handle(request, queue_wait_ms)
            else:
                semaphore = (self._exec_semaphore
                             if op in ("exec", "spawn")
                             else self._control_semaphore)
                if not semaphore.acquire(timeout=5):
                    raise BackendError("busy", "too many concurrent %s requests" % op)
                try:
                    response = self.handle(request, queue_wait_ms)
                finally:
                    semaphore.release()
        except BackendError as exc:
            response = {"ok": False, "error": {"code": exc.code, "detail": exc.detail}}
        except json.JSONDecodeError as exc:
            response = {"ok": False, "error": {"code": "bad_request", "detail": str(exc)}}
        except Exception as exc:  # pragma: no cover - defensive
            response = {"ok": False, "error": {"code": "internal", "detail": str(exc)}}
        try:
            conn.sendall(json.dumps(response, separators=(",", ":")).encode() + b"\n")
        except OSError:
            pass

    def serve(self, srv):
        srv.settimeout(0.25)
        while not self._stop:
            reason = self._out_of_budget()
            if reason:
                sys.stderr.write("backend stopping: %s\n" % reason)
                break
            try:
                conn, _ = srv.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            self._last_activity = time.monotonic()
            worker = threading.Thread(target=self._serve_connection, args=(conn, srv), daemon=True)
            with self._state_lock:
                self._workers = [w for w in self._workers if w.is_alive()]
                self._workers.append(worker)
            worker.start()
        try:
            srv.close()
        except OSError:
            pass
        deadline = time.monotonic() + 2.0
        for worker in self._workers:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                break
            worker.join(timeout=remaining)


def _bounded_timeout(value):
    if value is None:
        return DEFAULT_TIMEOUT_S
    try:
        seconds = float(value)
    except (TypeError, ValueError):
        raise BackendError("invalid_request", "timeout_s must be a number")
    if seconds != seconds or seconds in (float("inf"), float("-inf")):
        raise BackendError("invalid_request", "timeout_s must be finite")
    if seconds <= 0 or seconds > MAX_TIMEOUT_S:
        raise BackendError("invalid_request",
                           "timeout_s must be in (0, %d]" % MAX_TIMEOUT_S)
    return seconds


def _parse_json(text: str):
    if not text:
        return None
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        return None


def _truncated(res: CommandResult) -> bool:
    """A truncated tool output is not usable evidence."""
    return bool(res.truncated_out or res.truncated_err)


def _subnet_of(addr: str, prefix: int) -> str:
    try:
        parts = [int(p) for p in addr.split(".")]
    except ValueError:
        return addr
    while len(parts) < 4:
        parts.append(0)
    value = (parts[0] << 24) | (parts[1] << 16) | (parts[2] << 8) | parts[3]
    mask = (0xFFFFFFFF << (32 - prefix)) & 0xFFFFFFFF if prefix else 0
    value &= mask
    return "%d.%d.%d.%d/%d" % (value >> 24 & 0xFF, value >> 16 & 0xFF, value >> 8 & 0xFF,
                               value & 0xFF, prefix)


def main() -> int:
    parser = argparse.ArgumentParser(description="LP-4a netns link-fault backend supervisor")
    parser.add_argument("--run-dir", required=True)
    parser.add_argument("--spec", required=True)
    parser.add_argument("--run-token", required=True)
    parser.add_argument("--run-id", default="")
    parser.add_argument("--host-address", action="append", default=[])
    parser.add_argument("--host-address-v6", action="append", default=[])
    parser.add_argument("--fixture-script", default="")
    parser.add_argument("--idle-timeout-s", type=float, default=3600.0)
    parser.add_argument("--max-lifetime-s", type=float, default=7200.0)
    args = parser.parse_args()

    os.umask(0o077)
    with open(args.spec) as fh:
        spec = json.load(fh)

    host_addresses = [("inet", addr) for addr in args.host_address]
    host_addresses += [("inet6", addr) for addr in args.host_address_v6]

    backend = Backend(args.run_dir, spec, args.run_token, host_addresses,
                      run_id=args.run_id, idle_timeout_s=args.idle_timeout_s,
                      max_lifetime_s=args.max_lifetime_s)
    backend.spec["fixture_script"] = args.fixture_script

    def _on_signal(signum, frame):
        backend._stop = True

    signal.signal(signal.SIGTERM, _on_signal)
    signal.signal(signal.SIGINT, _on_signal)

    srv = None
    try:
        backend._load_spec()
        # Ownership first: the lock and the socket are claimed before any
        # namespace or manifest exists, so a second `up` cannot disturb a
        # live run.
        backend._take_run_lock()
        srv = backend._bind_socket()
        backend._create_namespaces()
        backend._create_links()
        backend.write_manifest()
        backend.serve(srv)
    except BackendError as exc:
        payload = {"ok": False, "error": {"code": exc.code, "detail": exc.detail}}
        try:
            with open(backend.error_path, "w") as fh:
                json.dump(payload, fh)
                fh.write("\n")
        except OSError:
            pass
        sys.stderr.write("backend error: %s: %s\n" % (exc.code, exc.detail))
        backend.cleanup()
        return 2
    except Exception as exc:
        sys.stderr.write("backend failure: %s\n" % exc)
        backend.cleanup()
        return 3
    finally:
        backend.cleanup()
    return 0


if __name__ == "__main__":
    sys.exit(main())
