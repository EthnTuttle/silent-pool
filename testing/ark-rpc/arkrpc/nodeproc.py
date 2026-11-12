"""Owned fresh regtest node lifecycle for probe runs.

Every node started by this module is:

- fresh: a brand-new datadir under a unique run-token scratch root
  (/tmp/opencode/AP1a-<token> by default); no chain state is reused;
- isolated: loopback RPC only, --no-listen --no-seeds --no-discover,
  so the node never talks to peers;
- secondary bound: --max-run-secs applies after startup, not spawn-to-exit;
- private: the RPC credential lives only in a cookie file (chmod 0600,
  private at rest for the node's whole lifetime) and never appears in
  argv, reports, or logs;
- owned: started and reaped by this process (SIGTERM, then SIGKILL,
  with waitpid verification); nothing is left running behind us;
- cleaned up: remove_scratch() deletes the run's cookie file and
  datadir after the report is written (default), unless the operator
  passes --keep-scratch for inspection. Frozen evidence directories
  under testing/ark-rpc/reports are never touched by cleanup.

The node binary and scratch directories are supplied by the caller;
this module never picks a binary on its own. The caller also supplies
a source_name (F7): the human-readable pin name of the binary's
origin tree (e.g. "rbitcoin-lab-retarget-profile" vs
"rbitcoin-v0.9-lab"), recorded in the manifest so different pin
generations cannot be confused in evidence.
"""

from __future__ import annotations

import os
import secrets
import shutil
import socket
import subprocess
import time
import signal
import json
from pathlib import Path

from .client import JsonRpcClient


class NodeError(RuntimeError):
    def __init__(self, message, log_tail=""):
        super().__init__(message + (("\nnode log tail:\n" + log_tail) if log_tail else ""))
        self.message = message
        self.log_tail = log_tail


def new_run_token() -> str:
    return "AP1a-" + secrets.token_hex(6)


def sha256_file(path: str) -> str:
    import hashlib
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 16), b""):
            h.update(chunk)
    return h.hexdigest()


def _free_port() -> int:
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]
    finally:
        sock.close()


class OwnedNode:
    """Legacy default probe mode; inherited env and no PDEATHSIG guarantee.

    Runtime preflight must use ProfileOwnedNode, never this legacy mode.
    """

    def __init__(self, binary, scratch_root, label, *, max_run_secs=300,
                 dialect=True, ready_timeout_s=90.0, rpc_deadline_s=5.0,
                 source_name=None):
        self.binary = os.path.abspath(binary)
        if not os.path.isfile(self.binary) or not os.access(self.binary, os.X_OK):
            raise NodeError("node binary is not an executable file: %s" % self.binary)
        self.label = label
        # F7: pin-name provenance, distinct from the run label.
        self.source_name = source_name or os.path.basename(
            os.path.dirname(self.binary))
        self.root = os.path.join(scratch_root, label)
        self.datadir = os.path.join(self.root, "datadir")
        self.cookie_path = os.path.join(self.root, "rpc.cookie")
        self.log_path = os.path.join(self.root, "node.log")
        self.max_run_secs = int(max_run_secs)
        self.ready_timeout_s = float(ready_timeout_s)
        self.rpc_deadline_s = float(rpc_deadline_s)
        self.dialect = bool(dialect)
        self.port = _free_port()
        self.rpc_url = "http://127.0.0.1:%d" % self.port
        self.argv = [
            self.binary,
            "--network", "regtest",
            "--datadir", self.datadir,
            "--rpc",
            "--rpc-listen", "127.0.0.1:%d" % self.port,
            "--rpc-cookie-file", self.cookie_path,
            "--no-listen",
            "--no-seeds",
            "--no-discover",
            "--max-run-secs", str(self.max_run_secs),
        ]
        self.env_extra = {"RBITCOIN_RPC_PACKAGE_DIALECT": "1"} if self.dialect else {}
        self.proc = None
        self._log_fh = None
        self.exit_info = None
        self.binary_sha256 = sha256_file(self.binary)

    def start(self):
        os.makedirs(self.datadir, exist_ok=True)
        os.makedirs(self.root, exist_ok=True)
        # rbitcoin READS a pre-existing Core-format cookie (user:pass,
        # no trailing newline) and rejects modes wider than 0600. We
        # generate the credential here; it exists only in this file and
        # never appears in argv, reports, or logs.
        cookie_user = "ap1a"
        cookie_pass = secrets.token_hex(24)
        with open(self.cookie_path, "w") as fh:
            fh.write("%s:%s" % (cookie_user, cookie_pass))
        os.chmod(self.cookie_path, 0o600)
        env = dict(os.environ)
        env.update(self.env_extra)
        self._log_fh = open(self.log_path, "wb")
        started = time.time()
        try:
            self.proc = subprocess.Popen(
                self.argv, env=env,
                stdout=self._log_fh, stderr=subprocess.STDOUT,
                stdin=subprocess.DEVNULL,
            )
        except OSError as exc:
            self._log_fh.close()
            raise NodeError("failed to start node: %s" % exc) from None
        self.started_monotonic = time.monotonic()
        self.started_wall = time.time()
        return self

    def wait_ready(self) -> JsonRpcClient:
        """Wait until the RPC answers getnetworkinfo; return a client."""
        deadline = time.monotonic() + self.ready_timeout_s
        last_err = None
        while time.monotonic() < deadline:
            if self.proc.poll() is not None:
                raise NodeError(
                    "node exited before becoming ready (code %s)"
                    % self.proc.returncode, self.log_tail())
            if os.path.exists(self.cookie_path):
                try:
                    client = JsonRpcClient(self.rpc_url, self.cookie_path,
                                           deadline_s=self.rpc_deadline_s)
                    rec = client.call("getnetworkinfo", [])
                    if rec.error is None:
                        return client
                    last_err = rec.error
                except (ValueError, OSError) as exc:
                    last_err = exc
            time.sleep(0.25)
        raise NodeError("node RPC not ready within %.0fs (last: %s)"
                        % (self.ready_timeout_s, last_err), self.log_tail())

    def stop(self, term_timeout_s=10.0) -> dict:
        """Terminate and reap the node; returns exit info."""
        if self.proc is None:
            return self.exit_info or {"returncode": None, "note": "never started"}
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=term_timeout_s)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()
        # Reap verification: poll() must now return an exit status.
        if self.proc.poll() is None:
            raise NodeError("node process %d could not be reaped" % self.proc.pid)
        self.exit_info = {
            "returncode": self.proc.returncode,
            "signal": -(self.proc.returncode) if self.proc.returncode and self.proc.returncode < 0 else None,
            "uptime_s": round(time.monotonic() - self.started_monotonic, 3),
        }
        if self._log_fh is not None:
            self._log_fh.close()
            self._log_fh = None
        return self.exit_info

    def log_tail(self, max_bytes=8192) -> str:
        try:
            with open(self.log_path, "rb") as fh:
                fh.seek(0, os.SEEK_END)
                size = fh.tell()
                fh.seek(max(0, size - max_bytes))
                return fh.read().decode("utf-8", "replace")
        except OSError:
            return ""

    def describe(self) -> dict:
        """Provenance record for the manifest (no credentials)."""
        return {
            "label": self.label,
            "source_name": self.source_name,
            "binary": self.binary,
            "binary_sha256": self.binary_sha256,
            "argv": list(self.argv),
            "env": dict(self.env_extra),
            "rpc_url": self.rpc_url,
            "cookie_file": os.path.basename(self.cookie_path),
            "cookie_note": ("written by the launcher before start "
                            "(user:pass, mode 0600, no trailing newline); "
                            "private at rest for the node's lifetime; "
                            "contents never recorded; removed with the "
                            "scratch dir unless --keep-scratch"),
            "network": "regtest",
            "isolation": ["--no-listen", "--no-seeds", "--no-discover",
                          "loopback rpc only"],
            "max_run_secs": self.max_run_secs,
            "dialect_env": self.env_extra or None,
        }

    def remove_scratch(self) -> bool:
        """Delete this run's scratch root (cookie + datadir + log).

        Default cleanup path after a report is written (F9): the cookie
        credential must not linger on disk. Never touches anything
        outside self.root; frozen report directories are elsewhere.
        """
        if self.root and os.path.isdir(self.root):
            shutil.rmtree(self.root, ignore_errors=True)
            return not os.path.exists(self.root)
        return True

    def __enter__(self):
        self.start()
        return self

    def __exit__(self, exc_type, exc, tb):
        self.stop()
        return False


def start_ticks(pid):
    """Linux proc stat field 22; comm may contain spaces and parentheses."""
    text = Path("/proc/%d/stat" % pid).read_text()
    return int(text[text.rfind(")") + 2:].split()[19])


def validate_parent_handoff(helper, helper_sha256, parent_pid, parent_ticks):
    """Level 1 must already be installed by the declared spc-lab executable."""
    import ctypes
    from .launchprofile import require, _path, _hash
    _path(helper)
    _hash(helper_sha256)
    require(os.path.isfile(helper) and os.access(helper, os.X_OK) and
            sha256_file(helper) == helper_sha256, "helper binary binding mismatch")
    require(type(parent_pid) is int and parent_pid > 1 and type(parent_ticks) is int and parent_ticks > 0,
            "invalid controller identity")
    require(os.getppid() == parent_pid and start_ticks(parent_pid) == parent_ticks,
            "controller identity mismatch")
    require(sha256_file("/proc/%d/exe" % parent_pid) == helper_sha256,
            "controller executable mismatch")
    sig = ctypes.c_int()
    libc = ctypes.CDLL(None, use_errno=True)
    require(libc.prctl(2, ctypes.byref(sig), 0, 0, 0) == 0 and sig.value == signal.SIGKILL,
            "probe lacks level-1 PDEATHSIG")
    require(os.getppid() == parent_pid and start_ticks(parent_pid) == parent_ticks,
            "controller died during handoff")


def private_json(path, value):
    """Exclusive, durable lifecycle record; never overwrite another launch."""
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "w") as fh:
        json.dump(value, fh, separators=(",", ":"), sort_keys=True)
        fh.flush()
        os.fsync(fh.fileno())
    directory = os.open(os.path.dirname(path), os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


class ProfileOwnedNode(OwnedNode):
    """Fresh owned profile mode. Exact env, two-level actor-exec lifetime."""

    def __init__(self, binary, out, label, raw_profile, *, helper, helper_sha256,
                 controller_pid, controller_ticks, owned_endpoints=(), **kwargs):
        from .launchprofile import normalize, isolated_profile, digest, require
        normalize(raw_profile, owned_endpoints=owned_endpoints)
        validate_parent_handoff(helper, helper_sha256, controller_pid, controller_ticks)
        require(os.path.isabs(binary) and sha256_file(binary) == raw_profile["binary_sha256"],
                "node binary binding mismatch")
        require(bool(__import__("re").fullmatch("[A-Za-z0-9_-]{1,128}", label)), "invalid label")
        super().__init__(binary, out, label, dialect=False, **kwargs)
        self.profile = isolated_profile(raw_profile, datadir=self.datadir,
            rpc_listen="127.0.0.1:%d" % self.port, cookie_file=self.cookie_path,
            api_log=os.path.join(self.root, "api.jsonl"), owned_endpoints=owned_endpoints)
        self.comparison_digest = digest(normalize(self.profile))
        self.raw_profile_digest = digest(raw_profile)
        self.argv = [self.binary] + self.profile["argv"]
        self.helper, self.helper_sha256 = helper, helper_sha256
        self.controller_pid, self.controller_ticks = controller_pid, controller_ticks
        self.identity_ticks = None
        self.intent_path = os.path.join(out, "node-launch-intent.json")
        self.observed_path = os.path.join(out, "node-launch-observed.json")
        self.created_root = False
        from .client import register_secret
        register_secret(self.profile["env"]["SPC_LAB_RUN_TOKEN"])

    def start(self):
        from .launchprofile import digest, require, normalize
        validate_parent_handoff(self.helper, self.helper_sha256, self.controller_pid, self.controller_ticks)
        require(sha256_file(self.binary) == self.binary_sha256 and
                digest(normalize(self.profile)) == self.comparison_digest, "launch inputs changed")
        os.mkdir(self.root, 0o700)  # fresh, exclusive; no exist_ok
        self.created_root = True
        os.mkdir(self.datadir, 0o700)
        fd = os.open(self.cookie_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w") as fh:
            fh.write("ap1a:" + secrets.token_hex(24))
        probe_pid, probe_ticks = os.getpid(), start_ticks(os.getpid())
        launch = [self.helper, "actor-exec", "--parent-pid", str(probe_pid),
                  "--parent-start-ticks", str(probe_ticks), "--"] + self.argv
        private_json(self.intent_path, {
            "version": 1, "program": self.helper, "helper_sha256": self.helper_sha256,
            "argv": launch[1:], "env_sha256": digest(self.profile["env"]),
            "datadir": self.datadir, "log": self.log_path,
            "profile_sha256": self.comparison_digest, "probe_pid": probe_pid,
            "probe_start_ticks": probe_ticks, "controller_pid": self.controller_pid,
            "controller_start_ticks": self.controller_ticks})
        self._log_fh = open(self.log_path, "xb")
        os.chmod(self.log_path, 0o600)
        self.started_monotonic = time.monotonic()
        self.started_wall = time.time()
        try:
            self.proc = subprocess.Popen(launch, env=dict(self.profile["env"]),
                stdin=subprocess.DEVNULL, stdout=self._log_fh, stderr=subprocess.STDOUT,
                start_new_session=True)
            self.identity_ticks = start_ticks(self.proc.pid)
            private_json(self.observed_path, {"version": 1, "pid": self.proc.pid,
                "start_ticks": self.identity_ticks, "pgid": self.proc.pid,
                "profile_sha256": self.comparison_digest})
        except BaseException:
            self.stop()
            raise
        return self

    def stop(self, term_timeout_s=2.0):
        if self.proc is None:
            if self._log_fh is not None:
                self._log_fh.close()
                self._log_fh = None
            return {"returncode": None, "note": "never started"}
        if self.proc.poll() is None:
            # Our unreaped Popen child cannot have its PID reused. Recovery
            # tooling must additionally use persisted start ticks; never names.
            if self.identity_ticks is not None and start_ticks(self.proc.pid) != self.identity_ticks:
                raise NodeError("owned node identity changed; refusing signal")
            os.killpg(self.proc.pid, signal.SIGTERM)
            try:
                self.proc.wait(timeout=term_timeout_s)
            except subprocess.TimeoutExpired:
                os.killpg(self.proc.pid, signal.SIGKILL)
                self.proc.wait(timeout=2.0)
        if self._log_fh is not None:
            self._log_fh.close()
            self._log_fh = None
        self.exit_info = {"returncode": self.proc.returncode, "reaped": True}
        return self.exit_info

    def log_tail(self, max_bytes=8192):
        return "profile-mode private log retained; contents omitted"

    def describe(self):
        return {"label": self.label, "source_name": self.source_name,
                "binary_sha256": self.binary_sha256, "network": "regtest",
                "launch_profile": {"profile_version": 1,
                    "comparison_sha256": self.comparison_digest,
                    "input_sha256": self.raw_profile_digest,
                    "helper_sha256": self.helper_sha256,
                    "parent_death_levels": 2,
                    "environment": "finalized-no-inheritance",
                    "transport": "validated_lab_transport"}}

    def remove_scratch(self):
        return super().remove_scratch() if self.created_root else True
