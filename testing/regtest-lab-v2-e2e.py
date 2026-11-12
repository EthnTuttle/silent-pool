#!/usr/bin/env python3
"""LP-live: independent real-rbitcoin/native-SV2 v2 end-to-end test.

Run under `nix develop -c`.  Every long-lived process is a direct, uniquely
identified child of this supervisor.  The default is an explicitly PROVISIONAL
run; acceptance additionally requires frozen Fable LP-1/LP-2 source manifests
and approval records.  No templates, credits, blocks, or scanner results are
injected by this harness.
"""

from __future__ import annotations

import argparse
import base64
import dataclasses
import datetime as dt
import hashlib
import http.client
import json
import os
from pathlib import Path
import queue
import re
import secrets as pysecrets
import shutil
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import urllib.parse
import uuid

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR / "lab-live"))
import oracle  # noqa: E402

SUMMARY_VERSION = 1
APPROVED_LP2_SNAPSHOT = "5e27e9180e765e8049cbcccbcadc38b24c85d402505108f8f20c320f73961250"
BASE58_PUBLIC_RE = re.compile(r"(?:[1-9A-HJ-NP-Za-km-z]{30,60})")
SECRET_KEY_RE = re.compile(
    r"(?i)(scan_key|spend_key|authority_secret|rpc_pass|password|cookie|private_key|tweak|seed)"
)


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z")


def sha256_bytes(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def atomic_write(path: Path, content, mode: int = 0o600) -> None:
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    if isinstance(content, (dict, list)):
        content = json.dumps(content, indent=2, sort_keys=True) + "\n"
    if isinstance(content, str):
        content = content.encode("utf-8")
    temporary = path.with_name(f".{path.name}.{os.getpid()}.{pysecrets.token_hex(4)}.tmp")
    try:
        with temporary.open("xb") as output:
            output.write(content)
            output.flush()
            os.fsync(output.fileno())
        os.chmod(temporary, mode)
        os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


def append_json_line(path: Path, value, lock: threading.Lock | None = None) -> None:
    raw = (json.dumps(value, separators=(",", ":"), sort_keys=True) + "\n").encode()
    if lock:
        with lock:
            with path.open("ab") as output:
                output.write(raw)
                output.flush()
    else:
        with path.open("ab") as output:
            output.write(raw)
            output.flush()


class LabError(Exception):
    def __init__(self, code: str, message: str, inconclusive: bool = False):
        super().__init__(message)
        self.code = code
        self.message = message
        self.inconclusive = inconclusive

    def as_dict(self):
        return {
            "code": self.code,
            "message": self.message,
            "outcome": "unknown" if self.inconclusive else "failed",
        }


def require(condition: bool, code: str, message: str, inconclusive: bool = False) -> None:
    if not condition:
        raise LabError(code, message, inconclusive)


def wait_until(description: str, timeout: float, probe, interval: float = 0.1):
    deadline = time.monotonic() + timeout
    last_error = None
    while time.monotonic() < deadline:
        try:
            value = probe()
            if value:
                return value
        except (OSError, ValueError, LabError) as error:
            last_error = error
        time.sleep(interval)
    detail = f"; last error: {last_error}" if last_error else ""
    raise LabError("bounded_wait_timeout", f"timed out waiting for {description}{detail}", True)


def load_json(path: Path):
    with path.open("r", encoding="utf-8") as source:
        return json.load(source)


@dataclasses.dataclass(frozen=True)
class Scenario:
    raw: dict

    @classmethod
    def load(cls, path: Path):
        raw = load_json(path)
        allowed = {
            "scenario_version", "id", "network", "pow_bits", "reference_target",
            "share_work_multiplier", "window_n", "max_slice_shares", "confirmation_depth",
            "fee_bps", "dust_limit_sats", "coinbase_threshold_sats", "max_payout_slots",
            "pool_payout_script", "pool_payout_descriptor", "worker_hashes_per_second",
            "worker_batch_size", "startup_timeout_secs", "scenario_timeout_secs",
            "shutdown_timeout_secs", "max_node_height", "payout_policy",
            "require_full_window", "ladder_claim", "ark_claim", "retarget_claim",
            "live_hashes_per_second", "changed_hashes_per_second", "coverage_height",
            "first_full_period_height", "second_full_period_height",
        }
        require(isinstance(raw, dict), "scenario_shape", "scenario root must be an object")
        unknown = sorted(set(raw) - allowed)
        require(not unknown, "scenario_unknown_fields", f"unknown scenario fields: {unknown}")
        retarget_only = {"live_hashes_per_second", "changed_hashes_per_second", "coverage_height", "first_full_period_height", "second_full_period_height"}
        required = allowed - retarget_only
        missing = sorted(required - set(raw))
        require(not missing, "scenario_missing_fields", f"missing scenario fields: {missing}")
        result = cls(raw)
        result.validate()
        return result

    def validate(self):
        raw = self.raw
        require(raw["scenario_version"] == 1, "scenario_version", "only scenario_version 1 is supported")
        require(raw["network"] == "regtest", "network_not_regtest", "LP-live refuses every non-regtest network")
        require(isinstance(raw["id"], str) and re.fullmatch(r"[a-z0-9-]{3,80}", raw["id"]),
                "scenario_id", "scenario id must be 3-80 lowercase DNS-safe characters")
        integer_fields = [
            "share_work_multiplier", "window_n", "max_slice_shares", "confirmation_depth",
            "fee_bps", "dust_limit_sats", "coinbase_threshold_sats", "max_payout_slots",
            "worker_hashes_per_second", "worker_batch_size", "startup_timeout_secs",
            "scenario_timeout_secs", "shutdown_timeout_secs", "max_node_height",
        ]
        for name in integer_fields:
            require(type(raw[name]) is int and raw[name] > 0, "scenario_integer", f"{name} must be a positive integer")
        for name in ["require_full_window", "ladder_claim", "ark_claim", "retarget_claim"]:
            require(type(raw[name]) is bool, "scenario_boolean", f"{name} must be boolean")
        require(not any(raw[name] for name in ["ladder_claim", "ark_claim"]),
                "unsupported_claim", "this packet cannot claim ladder or Ark capabilities")
        if raw["retarget_claim"]:
            for name in ["live_hashes_per_second", "changed_hashes_per_second", "coverage_height", "first_full_period_height", "second_full_period_height"]:
                require(name in raw, "scenario_missing_fields", f"retarget scenario missing {name}")
                require(type(raw[name]) is int and raw[name] > 0, "scenario_integer", f"{name} must be a positive integer")
            require(raw["pow_bits"] == "0x1e7fffff", "retarget_profile", "retarget mode uses the floor seed and omits --regtest-pow-bits")
            require((raw["coverage_height"], raw["first_full_period_height"], raw["second_full_period_height"]) == (24, 48, 72),
                    "retarget_boundaries", "this journey declares live boundaries 24/48/72")
        require(raw["payout_policy"] == "pool_template_exact_owed_pre_ladder",
                "payout_policy", "declare the exact-owed pre-ladder profile explicitly; it is not ladder conformance")
        require(raw["fee_bps"] <= 10_000, "fee_bps", "pool fee exceeds 10000 bps")
        require(raw["coinbase_threshold_sats"] >= raw["dust_limit_sats"],
                "threshold", "coinbase threshold is below dust")
        require(raw["max_slice_shares"] <= 65_000, "max_slice_shares", "v2 slice cap exceeds 65000")
        require(raw["worker_batch_size"] <= 4096, "worker_batch", "worker batch exceeds miner protocol cap")
        require(raw["share_work_multiplier"] <= 1 << 32, "multiplier_cap", "configured multiplier exceeds hard cap")
        reference = self.reference
        oracle.reference_exponent(reference)
        pow_limit = oracle.decode_compact_target(0x207FFFFF)
        require(reference >= pow_limit, "reference_below_pow_limit", "reference target is below regtest pow limit")
        bits = int(raw["pow_bits"], 0)
        require(0 < bits <= 0xFFFFFFFF, "pow_bits", "pow_bits must be uint32")
        oracle.decode_compact_target(bits)
        require(raw["max_node_height"] > raw["confirmation_depth"] + 2,
                "height_bound", "node height bound cannot reach settlement")
        script = bytes.fromhex(raw["pool_payout_script"])
        require(len(script) == 34 and script[:2] == b"\x51\x20", "pool_script", "pool payout must be P2TR hex")
        require(isinstance(raw["pool_payout_descriptor"], str) and raw["pool_payout_descriptor"].startswith("tr("),
                "pool_descriptor", "pool descriptor must be a watch-only taproot descriptor")

    @property
    def reference(self):
        return int(self.raw["reference_target"], 16)

    @property
    def pow_bits(self):
        return int(self.raw["pow_bits"], 0)

    @property
    def fingerprint(self):
        return oracle.work_unit_fingerprint(self.reference).hex()


class ProcessIdentity:
    @staticmethod
    def start_token(pid: int):
        text = Path(f"/proc/{pid}/stat").read_text()
        closing = text.rfind(") ")
        require(closing > 0, "process_identity", f"cannot parse /proc/{pid}/stat")
        fields = text[closing + 2:].split()
        require(len(fields) > 19, "process_identity", f"short /proc/{pid}/stat")
        return fields[19]

    @staticmethod
    def parent_pid(pid: int):
        text = Path(f"/proc/{pid}/stat").read_text()
        closing = text.rfind(") ")
        require(closing > 0, "process_identity", f"cannot parse /proc/{pid}/stat")
        fields = text[closing + 2:].split()
        require(len(fields) > 1, "process_identity", f"short /proc/{pid}/stat")
        return int(fields[1])

    @staticmethod
    def has_run_token(pid: int, token: str):
        try:
            environment = Path(f"/proc/{pid}/environ").read_bytes().split(b"\x00")
        except FileNotFoundError:
            return False
        return f"SPC_LAB_RUN_TOKEN={token}".encode() in environment


@dataclasses.dataclass
class OwnedProcess:
    name: str
    popen: subprocess.Popen
    token: str
    run_token: str
    argv: list[str]
    log_path: Path


class ProcessManager:
    def __init__(self, run_dir: Path, run_token: str):
        self.run_dir = run_dir
        self.run_token = run_token
        self.processes: list[OwnedProcess] = []
        self.handles = []
        self.cleanup_errors = []

    def start(self, name: str, argv: list[str], log_path: Path, stdin=False, extra_env=None, pipe_stdout=False):
        require(Path(argv[0]).is_absolute(), "process_path", f"{name} executable must be absolute")
        environment = os.environ.copy()
        environment["SPC_LAB_RUN_TOKEN"] = self.run_token
        if extra_env:
            environment.update(extra_env)
        handle = log_path.open("ab")
        self.handles.append(handle)
        try:
            popen = subprocess.Popen(
                argv,
                stdin=subprocess.PIPE if stdin else subprocess.DEVNULL,
                stdout=subprocess.PIPE if pipe_stdout else handle,
                stderr=handle if pipe_stdout else subprocess.STDOUT,
                env=environment,
                start_new_session=True,
                close_fds=True,
            )
        except Exception:
            handle.close()
            self.handles.remove(handle)
            raise
        token = wait_until(f"{name} process identity", 2, lambda: ProcessIdentity.start_token(popen.pid))
        owned = OwnedProcess(name, popen, token, self.run_token, list(argv), log_path)
        self.processes.append(owned)
        return owned

    def check(self, owned: OwnedProcess):
        if owned.popen.poll() is not None:
            return False
        require(ProcessIdentity.parent_pid(owned.popen.pid) == os.getpid(),
                "unsafe_cleanup", f"refusing to signal {owned.name}: not a direct child of this supervisor")
        require(ProcessIdentity.start_token(owned.popen.pid) == owned.token,
                "unsafe_cleanup", f"refusing to signal {owned.name}: PID start token mismatch")
        return True

    def stop(self, owned: OwnedProcess, grace: float, force=True):
        if owned.popen.poll() is None:
            self.check(owned)
            os.killpg(owned.popen.pid, signal.SIGTERM)
            try:
                owned.popen.wait(timeout=grace)
            except subprocess.TimeoutExpired:
                if force:
                    self.check(owned)
                    os.killpg(owned.popen.pid, signal.SIGKILL)
                    owned.popen.wait(timeout=grace)
                else:
                    raise
        else:
            owned.popen.wait(timeout=0)
        return owned.popen.returncode

    def stop_all(self, grace: float):
        for owned in reversed(self.processes):
            try:
                self.stop(owned, grace)
            except Exception as error:  # cleanup continues for all other owned children
                self.cleanup_errors.append(f"{owned.name}: {error}")
        for handle in self.handles:
            try:
                handle.close()
            except OSError:
                pass


class PortAllocator:
    def __init__(self):
        self.assigned = {}

    def reserve(self, name: str, requested: int):
        require(0 <= requested <= 65535, "port_range", f"{name} port is outside uint16")
        if name in self.assigned:
            require(self.assigned[name] == requested, "port_collision", f"duplicate allocation for {name}")
            return self.assigned[name]
        if requested in self.assigned.values() and requested != 0:
            raise LabError("port_collision", f"port {requested} requested twice")
        if requested == 0:
            with socket.socket() as sock:
                sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 0)
                sock.bind(("127.0.0.1", 0))
                requested = sock.getsockname()[1]
        else:
            with socket.socket() as sock:
                sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 0)
                try:
                    sock.bind(("127.0.0.1", requested))
                except OSError as error:
                    raise LabError("port_collision", f"{name} port {requested} is unavailable: {error}")
        require(requested not in self.assigned.values(), "port_collision", f"duplicate actual port {requested}")
        self.assigned[name] = requested
        return requested


class RpcClient:
    def __init__(self, port: int, user: str, password: str):
        self.port = port
        self.authorization = base64.b64encode(f"{user}:{password}".encode()).decode()
        self.counter = 0

    def call(self, method: str, params=None, timeout=30):
        self.counter += 1
        body = json.dumps({"jsonrpc": "1.0", "id": f"lp-live-{self.counter}", "method": method, "params": params or []})
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=timeout)
        try:
            connection.request("POST", "/", body, {
                "Authorization": f"Basic {self.authorization}",
                "Content-Type": "application/json",
            })
            response = connection.getresponse()
            raw = response.read(128 * 1024 * 1024)
        except (OSError, http.client.HTTPException) as error:
            raise LabError("node_rpc_transport", f"{method}: {type(error).__name__}: {error}", True)
        finally:
            connection.close()
        if response.status != 200:
            raise LabError("node_rpc_transport", f"{method} returned HTTP {response.status}", True)
        reply = json.loads(raw)
        if reply.get("error") is not None:
            raise LabError("node_rpc_error", f"{method}: {reply['error']}", True)
        return reply.get("result")


class PostgresClient:
    def __init__(self, psql: Path, port: int, user: str, password: str, database="spc"):
        self.psql = psql
        self.port = port
        self.user = user
        self.password = password
        self.database = database

    def query_lines(self, sql: str):
        environment = os.environ.copy()
        environment["PGPASSWORD"] = self.password
        result = subprocess.run([
            str(self.psql), "--no-psqlrc", "--set", "ON_ERROR_STOP=1", "--quiet",
            "--tuples-only", "--no-align", "--host", "127.0.0.1", "--port", str(self.port),
            "--username", self.user, "--dbname", self.database, "--command", sql,
        ], env=environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=15, check=False)
        if result.returncode:
            raise LabError("postgres_query", f"psql failed with status {result.returncode}: {result.stderr.decode(errors='replace')[:500]}")
        return [json.loads(line) for line in result.stdout.decode().splitlines() if line.strip()]

    def one(self, sql: str):
        rows = self.query_lines(sql)
        require(len(rows) <= 1, "postgres_shape", f"query returned {len(rows)} rows, expected at most one")
        return rows[0] if rows else None

    @staticmethod
    def bytea(hex_value: str):
        require(re.fullmatch(r"[0-9a-f]{64}", hex_value) is not None, "hex32", "invalid internal 32-byte hash")
        return f"'\\x{hex_value}'"

    def block(self, raw_hash_hex: str):
        literal = self.bytea(raw_hash_hex)
        return self.one(f"""
SELECT jsonb_build_object(
  'block_hash', encode(bf.block_hash, 'hex'), 'height', bf.height,
  'fingerprint', encode(bf.fingerprint, 'hex'), 'status', bf.status,
  'settled_at', bf.settled_at,
  'snapshot', (SELECT jsonb_build_object(
      'height', ds.height, 'fingerprint', encode(ds.fingerprint, 'hex'),
      'a_send_x', encode(ds.a_send_x, 'hex'), 'entries', ds.entries,
      'high_water', ds.high_water, 'window_target', ds.window_target::text,
      'work_unit_version', ds.work_unit_version,
      'work_unit_fingerprint', encode(ds.work_unit_fingerprint, 'hex'),
      'window_rule', ds.window_rule)
    FROM distribution_snapshot ds WHERE ds.fingerprint = bf.fingerprint),
  'ledger', COALESCE((SELECT jsonb_agg(jsonb_build_object(
      'sp_id', encode(lh.sp_id, 'hex'), 'rail', lh.rail, 'paid_sats', lh.paid_sats,
      'ark_due_sats', lh.ark_due_sats, 'forfeited_sats', lh.forfeited_sats,
      'pending_delta', lh.pending_delta))
    FROM ledger_history lh WHERE lh.block_hash = bf.block_hash), '[]'::jsonb),
  'ark_payouts', COALESCE((SELECT jsonb_agg(jsonb_build_object(
      'sp_id', encode(ap.sp_id, 'hex'), 'sats', ap.sats, 'due_height', ap.due_height,
      'status', ap.status)) FROM ark_payout ap WHERE ap.block_hash = bf.block_hash), '[]'::jsonb)
) FROM block_found bf WHERE bf.block_hash = {literal}
""")

    def slices(self, high_water: int):
        require(0 <= high_water < (1 << 63), "high_water_range", "high-water mark exceeds signed BIGINT")
        rows = self.query_lines(f"""
WITH selected AS MATERIALIZED (
  SELECT * FROM slice WHERE slice_id <= {high_water}
  ORDER BY height DESC, slice_id DESC LIMIT 4097
)
SELECT COALESCE(jsonb_agg(jsonb_build_object(
  'slice_id', s.slice_id::text, 'height', s.height,
  'difficulty', s.difficulty::text, 'ref_fees', s.ref_fees, 'n_shares', s.n_shares,
  'shares_hex', encode(s.shares, 'hex'), 'work_unit_version', s.work_unit_version,
  'work_unit_fingerprint', encode(s.work_unit_fingerprint, 'hex'),
  'diff_fee_sum', COALESCE((SELECT SUM(sc0.diff_fee_sum)::text FROM slice_credit sc0
                            WHERE sc0.slice_id = s.slice_id), '0'),
  'credits', COALESCE((SELECT jsonb_agg(jsonb_build_object(
      'sp_id', encode(sc.sp_id, 'hex'), 'diff_sum', sc.diff_sum::text,
      'diff_fee_sum', sc.diff_fee_sum::text))
    FROM slice_credit sc WHERE sc.slice_id = s.slice_id), '[]'::jsonb)
) ORDER BY s.height DESC, s.slice_id DESC), '[]'::jsonb) FROM selected s
""")
        require(len(rows) == 1, "postgres_shape", "slice query did not return one array")
        return rows[0]

    def pending_before(self, height: int):
        result = self.one(f"""
SELECT COALESCE(jsonb_object_agg(encode(sp_id, 'hex'), total), '{{}}'::jsonb) FROM (
  SELECT lh.sp_id, SUM(lh.pending_delta)::text AS total
  FROM ledger_history lh JOIN block_found bf ON bf.block_hash = lh.block_hash
  WHERE bf.height < {height} AND bf.status = 'confirmed'
  GROUP BY lh.sp_id
) delta_by_identity
""")
        return result or {}


class MinerCollector:
    def __init__(self, owned: OwnedProcess, run_dir: Path):
        self.owned = owned
        self.path = run_dir / "private" / "miner.jsonl"
        self.path.touch(mode=0o600, exist_ok=True)
        self.queue = queue.Queue(maxsize=65536)
        self.events = []
        self.results = {}
        self.condition = threading.Condition()
        self.write_lock = threading.Lock()
        self.collector_drops = 0
        self.stream_errors = []
        self.previous_id = 0
        self.previous_dropped = 0
        self.closed = False
        self.reader = threading.Thread(target=self._read, name="miner-stdout-drain", daemon=True)
        self.writer = threading.Thread(target=self._write, name="miner-evidence-writer", daemon=True)
        self.writer.start()
        self.reader.start()

    def _read(self):
        try:
            for line in self.owned.popen.stdout:
                try:
                    value = json.loads(line)
                except ValueError:
                    value = {"kind": "invalid_json_line", "raw_hex": line.hex()}
                try:
                    self.queue.put_nowait(value)
                except queue.Full:
                    with self.condition:
                        self.collector_drops += 1
                        self.condition.notify_all()
        finally:
            with self.condition:
                self.closed = True
                self.condition.notify_all()
            self.queue.put(None)

    def _write(self):
        while True:
            value = self.queue.get()
            if value is None:
                return
            append_json_line(self.path, value, self.write_lock)
            with self.condition:
                if value.get("kind") == "event":
                    self.events.append(value)
                    event_id = int(value.get("event_id", 0))
                    dropped = int(value.get("dropped_before", 0))
                    if event_id <= self.previous_id or dropped < self.previous_dropped:
                        self.stream_errors.append("non-monotone event cursor/drop counter")
                    else:
                        gap = event_id - self.previous_id - 1 if self.previous_id else event_id - 1
                        if gap != dropped - self.previous_dropped:
                            self.stream_errors.append("event ID gap differs from cumulative drop counter")
                    self.previous_id = event_id
                    self.previous_dropped = dropped
                elif value.get("kind") == "command_result":
                    self.results[value.get("command_id")] = value
                self.condition.notify_all()

    def command(self, operation: str, payload=None, timeout=10):
        command_id = f"lp-live-{uuid.uuid4()}"
        envelope = {
            "protocol": "spc-cpu-miner-control", "version": 1, "command_id": command_id,
            "operation": operation, "payload": payload or {},
        }
        with self.condition:
            require(self.owned.popen.poll() is None, "miner_exited", "miner exited before control command")
            self.owned.popen.stdin.write((json.dumps(envelope) + "\n").encode())
            self.owned.popen.stdin.flush()
        wait_until(f"miner {operation} result", timeout, lambda: self._result(command_id))
        result = self.results[command_id]
        require(result.get("outcome") in ("succeeded", "running"), "miner_command", f" miner {operation} failed: {result.get('error')}")
        return result

    def _result(self, command_id):
        with self.condition:
            return command_id in self.results

    def snapshot(self):
        result = self.command("snapshot", timeout=10)
        return result.get("result", {})

    def stop(self):
        if self.owned.popen.poll() is None:
            try:
                self.command("stop", timeout=5)
            finally:
                try:
                    self.owned.popen.stdin.close()
                except (OSError, AttributeError):
                    pass

    def event_values(self):
        with self.condition:
            return list(self.events)

    def assert_complete(self):
        with self.condition:
            require(self.collector_drops == 0 and not self.stream_errors,
                    "miner_evidence_unknown",
                    f"miner event loss/cursor error: drops={self.collector_drops}, errors={self.stream_errors}",
                    True)


class PoolEvidenceTail:
    EVIDENCE_NAMES = ("job_issued", "accounted", "multiplier_stepdown", "multiplier_stepup", "share_outcome_unknown")

    def __init__(self, path: Path):
        self.path = path
        self.events = []
        self.errors = []
        self.lock = threading.Lock()
        self.stop_event = threading.Event()
        self.thread = threading.Thread(target=self._tail, name="pool-evidence-tail", daemon=True)
        self.thread.start()

    def _tail(self):
        buffer = b""
        with self.path.open("rb") as source:
            while not self.stop_event.is_set():
                chunk = source.read(1024 * 1024)
                if not chunk:
                    time.sleep(0.05)
                    continue
                buffer += chunk
                lines = buffer.split(b"\n")
                buffer = lines.pop()
                for line in lines:
                    self._parse(line)

    def _parse(self, line):
        if b"spc_evidence" not in line:
            return
        text = re.sub(r"\x1b\[[0-9;]*[A-Za-z]", "", line.decode(errors="replace"))
        for name in self.EVIDENCE_NAMES:
            match = re.search(rf"event=(?:\"{name}\"|{name})(?:\s|\x1b|$)", text)
            if not match:
                continue
            evidence_at = text.find("evidence=")
            if evidence_at < 0:
                self.errors.append(f"missing evidence payload for {name}")
                return
            payload = text[evidence_at + len("evidence="):].lstrip()
            try:
                value, _ = json.JSONDecoder().raw_decode(payload)
            except ValueError as error:
                self.errors.append(f"cannot decode {name}: {error}")
                return
            with self.lock:
                self.events.append({"kind": name, "evidence": value, "raw_log_sha256": sha256_bytes(line)})
            return

    def values(self):
        with self.lock:
            return list(self.events), list(self.errors)


def run_checked(argv, log_path: Path, timeout=30, env=None, input_bytes=None, combined=False):
    environment = os.environ.copy()
    if env:
        environment.update(env)
    result = subprocess.run(argv, input=input_bytes, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            env=environment, timeout=timeout, check=False)
    atomic_write(log_path, b"ARGV_SHA256=" + sha256_bytes(json.dumps(argv).encode()).encode() + b"\nSTDOUT:\n" + result.stdout + b"\nSTDERR:\n" + result.stderr)
    require(result.returncode == 0, "command_failed", f"command exited {result.returncode}; private log {log_path}")
    return result.stdout + result.stderr if combined else result.stdout


def source_git_provenance(source_dir: Path, log_path: Path):
    source_dir = source_dir.resolve()
    require(source_dir.is_dir(), "source_provenance", f"source directory missing: {source_dir}")
    head = run_checked(["git", "-C", str(source_dir), "rev-parse", "HEAD"], log_path).decode().strip()
    diff = subprocess.run(["git", "-C", str(source_dir), "diff", "--no-ext-diff", "--binary", "HEAD", "--"],
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True).stdout
    untracked = subprocess.run(["git", "-C", str(source_dir), "ls-files", "--others", "--exclude-standard"],
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True).stdout.decode().splitlines()
    files = []
    for relative in sorted(untracked):
        path = safe_source_path(source_dir, relative)
        files.append({"path": relative, "sha256": sha256_file(path)})
    aggregate = sha256_bytes(json.dumps({"head": head, "diff_sha256": sha256_bytes(diff), "untracked": files}, sort_keys=True, separators=(",", ":")).encode())
    return {"source_dir": str(source_dir), "head": head, "tracked_diff_sha256": sha256_bytes(diff), "untracked": files, "aggregate_sha256": aggregate}


def safe_source_path(source_dir: Path, relative: str):
    relative_path = Path(relative)
    require(not relative_path.is_absolute() and ".." not in relative_path.parts,
            "source_manifest_path", f"unsafe source manifest path: {relative}")
    path = (source_dir / relative_path).resolve()
    require(path.is_relative_to(source_dir.resolve()) and path.is_file(),
            "source_manifest_path", f"manifest member missing or escapes source: {relative}")
    return path



APPROVED_LP1_SNAPSHOT = "c16032a78902e88b0157efed6780bc8b424e942b1c3bf46893d8fb09ae4708ee"
APPROVED_L2_SNAPSHOT = "d2f37b9113e8855245866289ed393043aed6e6153e8101375072cbb23624d499"


def approval_verdicts(text: str):
    """Return explicit (verdict, full-snapshot, section) associations.

    Associations are confined to one top-level review pass, not the whole
    multi-pass file. A verdict without a parseable full snapshot is ambiguous,
    never an approval. Snapshot citations in other passes are not candidates.
    """
    lines = text.splitlines()
    starts = [0] + [i for i, line in enumerate(lines) if re.match(r"^# [A-Z]", line)]
    if starts[0] != 0:
        starts.insert(0, 0)
    starts.append(len(lines))
    verdicts = []
    for begin, end in zip(starts, starts[1:]):
        section_lines = lines[begin:end]
        body = "\n".join(section_lines)
        headings = [
            line for line in section_lines
            if re.match(r"^## (?:Verdict|Result)(?:\s|\(|$)", line)
        ]
        if not headings:
            continue
        if len(headings) != 1:
            verdicts.append({"kind": "ambiguous", "snapshot": None,
                             "section": section_lines[0]})
            continue
        kind = "ambiguous"
        for candidate in ["APPROVE", "REQUEST_CHANGES"]:
            if re.search(rf"\*\*{candidate}\b", body):
                kind = candidate if kind == "ambiguous" else "ambiguous" if kind != candidate else kind
        snapshots = set()
        if len(re.findall(r"tied to snapshot\b", body)) > 1:
            snapshots.add("<multiple-explicit-snapshot-associations>")
        for tied_line in body.splitlines():
            if "tied to snapshot" in tied_line:
                if len(set(re.findall(r"[0-9a-f]{64}", tied_line))) > 1:
                    snapshots.add("<ambiguous-explicit-snapshot-associations>")
        for match in re.finditer(r"tied to snapshot\s+`?([0-9a-f]{64})", body):
            snapshots.add(match.group(1))
        for match in re.finditer(r"snapshot_sha256[`\s]*([0-9a-f]{64})", body):
            snapshots.add(match.group(1))
        prefixes = re.findall(r"tied to snapshot\s+`?([0-9a-f]{7,63})", body)
        if prefixes and not any(snapshot.startswith(prefix) for snapshot in snapshots for prefix in prefixes):
            snapshots.clear()
        for match in re.finditer(r"(?m)^([0-9a-f]{64})\s*\(matches\)\s*$", body):
            if not prefixes or any(match.group(1).startswith(prefix) for prefix in prefixes):
                snapshots.add(match.group(1))
        snapshot = snapshots.pop() if len(snapshots) == 1 else None
        verdicts.append({"kind": kind, "snapshot": snapshot,
                         "section": section_lines[0],
                         "ambiguous_snapshot": len(snapshots) > 0})
    return verdicts


def approval_allows_snapshot(text: str, snapshot: str):
    """Fail closed on missing, revoked, ambiguous, or partial associations."""
    require(re.fullmatch(r"[0-9a-f]{64}", snapshot) is not None,
            "approval_snapshot_shape", "expected a full 64-hex snapshot, never a prefix")
    matches = [v for v in approval_verdicts(text) if v["snapshot"] == snapshot]
    if len(matches) != 1:
        return False
    # Any explicit rejection of this same hash is a revocation even when an
    # earlier approval exists. Do not rely on ordering or suffix similarity.
    return matches[0]["kind"] == "APPROVE"


def verify_approval_record(path: Path, snapshot: str, component: str):
    text = path.read_text(errors="replace")
    if not approval_allows_snapshot(text, snapshot):
        raise LabError(
            "approval_mismatch",
            f"{component}: no unrevoked, unambiguous full-snapshot APPROVE verdict for {snapshot} in {path}",
        )
    return {"approval_record": str(path), "approval_record_sha256": sha256_file(path)}

def manifest_snapshot(manifest):
    files = [{"path": row["path"], "sha256": row["sha256"]} for row in manifest["files"]]
    encoded = json.dumps({"base": manifest["base"], "files": files}, sort_keys=True, separators=(",", ":")).encode()
    return sha256_bytes(encoded)


def verify_manifest(source_dir: Path, manifest_path: Path, expected_snapshot=None):
    manifest = load_json(manifest_path)
    require(isinstance(manifest.get("files"), list) and isinstance(manifest.get("base"), str),
            "source_manifest_shape", "source manifest needs base and files")
    actual_snapshot = manifest_snapshot(manifest)
    require(manifest.get("snapshot_sha256") == actual_snapshot,
            "source_manifest_snapshot", f"manifest recipe mismatch: computed {actual_snapshot}")
    if expected_snapshot:
        require(actual_snapshot == expected_snapshot,
                "source_snapshot_mismatch", f"expected {expected_snapshot}, got {actual_snapshot}")
    for row in manifest["files"]:
        require(re.fullmatch(r"[0-9a-f]{64}", row.get("sha256", "")) is not None,
                "source_manifest_shape", f"bad SHA256 for {row.get('path')}")
        path = safe_source_path(source_dir, row["path"])
        require(sha256_file(path) == row["sha256"],
                "source_drift", f"source differs from frozen manifest at {row['path']}")
    return {
        "source_dir": str(source_dir.resolve()), "manifest_path": str(manifest_path.resolve()),
        "snapshot_sha256": actual_snapshot, "file_count": len(manifest["files"]), "verified": True,
    }


class LiveRun:
    def __init__(self, args, scenario: Scenario):
        self.args = args
        self.scenario = scenario
        self.run_id = f"lp-live-{dt.datetime.now(dt.timezone.utc).strftime('%Y%m%dT%H%M%S')}-{uuid.uuid4().hex[:12]}"
        self.run_token = uuid.uuid4().hex
        self.run_dir = args.run_dir.resolve()
        self.private = self.run_dir / "private"
        self.public = self.run_dir / "public"
        self.observations = []
        self.observation_lock = threading.RLock()
        self.manager = ProcessManager(self.run_dir, self.run_token)
        self.ports = {}
        self.secrets = []
        self.provenance = {}
        self.binary_records = {}
        self.selected = None
        self.nonblock = None
        self.rejected_candidates = {}
        self.retarget_evidence = None
        self.process_stopped = False
        self.summary_extra = {}

    def observe(self, kind: str, payload):
        observation = {"observed_at": utc_now(), "kind": kind, "payload": payload}
        with self.observation_lock:
            self.observations.append(observation)
            append_json_line(self.private / "observations.jsonl", observation, self.observation_lock)

    def mark_secret(self, value: str):
        if value:
            self.secrets.append(value)

    def redacted_error(self, error):
        if isinstance(error, LabError):
            value = error.as_dict()
        else:
            value = {"code": "internal_error", "message": str(error), "outcome": "failed"}
        text = json.dumps(value)
        for secret in sorted(self.secrets, key=len, reverse=True):
            text = text.replace(secret, "<redacted>")
        text = re.sub(r'("(?:scan_key|spend_key|authority_secret_key|rpc_pass|password)"\s*:\s*")[^"]*',
                      r'\1<redacted>', text, flags=re.IGNORECASE)
        return json.loads(text)

    def prepare_run_directory(self):
        if self.run_dir.exists():
            entries = list(self.run_dir.iterdir())
            require(not entries, "run_dir_not_empty", f"run directory must be new or empty: {self.run_dir}")
        self.private.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.public.mkdir(parents=True, exist_ok=True, mode=0o700)
        os.chmod(self.run_dir, 0o700)
        os.chmod(self.private, 0o700)
        os.chmod(self.public, 0o700)
        for path in [self.private / "observations.jsonl", self.private / "processes.json", self.private / "db-diagnostics.json"]:
            atomic_write(path, [] if path.suffix == ".json" else b"")
        atomic_write(self.private / "scenario.redacted.json", self.args.scenario.read_text())
        self.observe("run_created", {
            "run_id": self.run_id, "base_revision": "b0ab4ca34fda07c61a6335d05f598eaacc69678c",
            "mode": self.args.mode, "retarget_claim": self.scenario.raw["retarget_claim"],
        })

    def check_binary(self, name: str, value: str):
        path = Path(value).resolve()
        require(path.is_file() and os.access(path, os.X_OK), "binary_missing", f"{name} is not executable: {path}")
        return path

    def capture_sources(self):
        bindings = [
            ("pool", self.args.pool_source_dir, self.args.pool_source_manifest, self.args.pool_approval_record, APPROVED_LP1_SNAPSHOT),
            ("miner", self.args.miner_source_dir, self.args.miner_source_manifest, self.args.miner_approval_record, APPROVED_LP2_SNAPSHOT),
            ("scanner", self.args.scanner_source_dir, self.args.scanner_source_manifest, None, None),
        ]
        for name, source, manifest, approval, expected in bindings:
            if source is None:
                if manifest is not None:
                    raise LabError("source_provenance", f"--{name}-source-dir is required with its manifest")
                if self.args.mode == "acceptance" and name in ("pool", "miner"):
                    raise LabError("source_provenance", f"acceptance requires --{name}-source-dir and manifest")
                continue
            source = Path(source).resolve()
            if manifest:
                record = verify_manifest(source, Path(manifest).resolve(), expected)
            else:
                require(self.args.mode != "acceptance", "source_provenance", f"acceptance requires --{name}-source-manifest")
                record = source_git_provenance(source, self.private / f"{name}-source-provenance.log")
            self.provenance[name] = record
            if approval:
                approval_path = Path(approval).resolve()
                snapshot = record.get("snapshot_sha256")
                self.provenance[name].update(verify_approval_record(approval_path, snapshot, name))
            elif self.args.mode == "acceptance" and name in ("pool", "miner"):
                raise LabError("approval_required", f"acceptance requires --{name}-approval-record")
        if self.args.node_source_manifest:
            require(self.args.node_source_dir is not None, "source_provenance", "--node-source-dir is required with its manifest")
            self.provenance["node"] = verify_manifest(
                Path(self.args.node_source_dir).resolve(), Path(self.args.node_source_manifest).resolve(),
                APPROVED_L2_SNAPSHOT if self.scenario.raw["retarget_claim"] else None,
            )
            if self.args.node_approval_record:
                approval_path = Path(self.args.node_approval_record).resolve()
                snapshot = self.provenance["node"]["snapshot_sha256"]
                self.provenance["node"].update(verify_approval_record(approval_path, snapshot, "node"))
            if self.scenario.raw["retarget_claim"]:
                require(self.args.node_approval_record is not None,
                        "approval_required", "retarget acceptance requires --node-approval-record")
        elif self.args.node_source_dir:
            require(not self.scenario.raw["retarget_claim"] or self.args.mode != "acceptance",
                    "approval_required", "retarget acceptance requires the frozen L2 source manifest")
            self.provenance["node"] = source_git_provenance(Path(self.args.node_source_dir).resolve(), self.private / "node-source-provenance.log")
        elif self.args.mode == "acceptance":
            raise LabError("source_provenance", "acceptance requires --node-source-dir for rbitcoin provenance")

    def prepare_binaries(self):
        self.pool_binary = self.check_binary("pool", self.args.pool_binary)
        self.miner_binary = self.check_binary("miner", self.args.miner_binary)
        self.scanner_binary = self.check_binary("scanner", self.args.scanner_binary)
        self.node_binary = self.check_binary("node", self.args.node_binary)
        self.postgres_bin = Path(self.args.postgres_bin_dir).resolve()
        self.initdb = self.check_binary("initdb", str(self.postgres_bin / "initdb"))
        self.postgres = self.check_binary("postgres", str(self.postgres_bin / "postgres"))
        self.createdb = self.check_binary("createdb", str(self.postgres_bin / "createdb"))
        self.psql = self.check_binary("psql", str(self.postgres_bin / "psql"))
        self.pg_isready = self.check_binary("pg_isready", str(self.postgres_bin / "pg_isready"))
        self.redis_server = self.check_binary("redis-server", self.args.redis_server)
        self.redis_cli = self.check_binary("redis-cli", self.args.redis_cli)
        for name, path in [
            ("pool", self.pool_binary), ("miner", self.miner_binary), ("scanner", self.scanner_binary),
            ("node", self.node_binary), ("postgres", self.postgres), ("redis", self.redis_server),
        ]:
            stat_result = path.stat()
            self.binary_records[name] = {
                "path": str(path), "sha256": sha256_file(path), "size": stat_result.st_size,
                "mtime_ns": stat_result.st_mtime_ns,
            }
        help_specs = [
            ("node", [str(self.node_binary), "--help"], ["--regtest-pow-bits", "--rpc-cookie-file", "--no-listen", "--no-seeds", "--no-discover", "--max-outbound", "--api-log"]),
            ("pool", [str(self.pool_binary), "--help"], ["--config", "--roles"]),
            ("miner", [str(self.miner_binary), "--help"], ["--config", "--check-config"]),
            ("scanner-find", [str(self.scanner_binary), "find", "--help"], ["--address", "--scan-key", "--from", "--to", "--rpc"]),
            ("scanner-light", [str(self.scanner_binary), "light", "--help"], ["--address", "--scan-key", "--proofs"]),
            ("scanner-gen", [str(self.scanner_binary), "gen-address", "--help"], ["--network"]),
        ]
        for name, argv, required in help_specs:
            output = run_checked(argv, self.private / f"help-{name}.log", combined=True).decode(errors="replace")
            for flag in required:
                require(flag in output, "unsupported_cli", f"{name} does not advertise required {flag}")
        if self.scenario.raw["retarget_claim"]:
            node_help = (self.private / "help-node.log").read_text(errors="replace")
            require("--retarget-regtest" in node_help, "unsupported_cli", "retarget scenario requires --retarget-regtest support")
        self.capture_sources()

    def allocate_ports(self):
        allocator = PortAllocator()
        for name, requested in [
            ("node_rpc", self.args.rpc_port), ("sv2", self.args.sv2_port),
            ("jd", self.args.jd_port), ("api", self.args.api_port),
            ("redis", self.args.redis_port), ("postgres", self.args.postgres_port),
        ]:
            self.ports[name] = allocator.reserve(name, requested)
        self.ports["p2p"] = None

    def generate_identities(self):
        self.identities = []
        generated = []
        for index in range(3):
            stdout = run_checked([
                str(self.scanner_binary), "gen-address", "--network", "regtest",
            ], self.private / f"generated-identity-{index}.json")
            value = json.loads(stdout)
            for key in ["address", "b_scan", "b_spend", "scan_key", "spend_key"]:
                require(isinstance(value.get(key), str) and value[key], "identity_generation", f"scanner omitted {key}")
                self.mark_secret(value[key]) if key in ("scan_key", "spend_key") else None
            generated.append(value)
        for index, name in enumerate(("alice", "bob")):
            value = generated[index]
            require(value["address"].startswith("tsp1q"), "identity_network", "scanner generated a non-regtest identity")
            identity = dict(value)
            identity["id"] = name
            identity["sp_id"] = oracle.sp_id(bytes.fromhex(value["b_scan"]), bytes.fromhex(value["b_spend"])).hex()
            self.identities.append(identity)
        require(self.identities[0]["address"] != self.identities[1]["address"] and
                self.identities[0]["sp_id"] != self.identities[1]["sp_id"],
                "identity_collision", "fresh identities are not distinct")
        atomic_write(self.private / "identities.json", self.identities)
        authority = generated[2]
        public, private = oracle.noise_authority_from_secp_material(
            bytes.fromhex(authority["b_scan"]), bytes.fromhex(authority["scan_key"]),
        )
        self.authority_public = public
        self.authority_secret = private
        self.mark_secret(private)
        atomic_write(self.private / "noise-authority.json", {"public_key": public, "secret_key": private})

    def storage_credentials(self):
        self.rpc_user = "spc"
        self.rpc_password = pysecrets.token_hex(24)
        self.pg_user = "spc"
        self.pg_password = pysecrets.token_urlsafe(30)
        self.redis_password = pysecrets.token_urlsafe(30)
        for secret in [self.rpc_password, self.pg_password, self.redis_password]:
            self.mark_secret(secret)
        cookie = self.private / "node-rpc.cookie"
        atomic_write(cookie, f"{self.rpc_user}:{self.rpc_password}", 0o600)
        require(not cookie.read_bytes().endswith(b"\n") and stat.S_IMODE(cookie.stat().st_mode) == 0o600,
                "cookie_permissions", "private node cookie format/mode is invalid")

    def start_postgres(self):
        pg_data = self.private / "postgres-data"
        password_file = self.private / "postgres-init.password"
        atomic_write(password_file, self.pg_password + "\n")
        run_checked([
            str(self.initdb), "--pgdata", str(pg_data), "--username", self.pg_user,
            "--pwfile", str(password_file), "--auth", "scram-sha-256",
            "--auth-host", "scram-sha-256", "--auth-local", "trust", "--no-instructions",
        ], self.private / "initdb.log", timeout=60)
        password_file.unlink()
        socket_dir = self.private / "postgres-socket"
        socket_dir.mkdir(mode=0o700)
        self.postgres_owned = self.manager.start("postgres", [
            str(self.postgres), "-D", str(pg_data), "-p", str(self.ports["postgres"]),
            "-h", "127.0.0.1", "-k", str(socket_dir),
        ], self.private / "postgres.log")
        environment = os.environ.copy()
        environment["PGPASSWORD"] = self.pg_password
        wait_until("Postgres readiness", self.scenario.raw["startup_timeout_secs"], lambda: subprocess.run([
            str(self.pg_isready), "-h", "127.0.0.1", "-p", str(self.ports["postgres"]),
            "-U", self.pg_user, "-d", "postgres", "-q",
        ], env=environment, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0)
        run_checked([
            str(self.createdb), "--host", "127.0.0.1", "--port", str(self.ports["postgres"]),
            "--username", self.pg_user, "--no-password", "spc",
        ], self.private / "createdb.log", env={"PGPASSWORD": self.pg_password})
        self.db = PostgresClient(self.psql, self.ports["postgres"], self.pg_user, self.pg_password)
        require(self.db.one("SELECT 1") == 1, "postgres_ready", "Postgres SELECT 1 did not return 1")

    def start_redis(self):
        redis_dir = self.private / "redis-data"
        redis_dir.mkdir(mode=0o700)
        config = self.private / "redis.conf"
        atomic_write(config, f"""bind 127.0.0.1
port {self.ports['redis']}
protected-mode yes
daemonize no
save \"\"
appendonly no
dir {redis_dir}
requirepass {self.redis_password}
loglevel notice
""")
        self.redis_owned = self.manager.start("redis", [str(self.redis_server), str(config)], self.private / "redis.log")
        environment = os.environ.copy()
        environment["REDISCLI_AUTH"] = self.redis_password
        wait_until("Redis readiness", self.scenario.raw["startup_timeout_secs"], lambda: subprocess.run([
            str(self.redis_cli), "-h", "127.0.0.1", "-p", str(self.ports["redis"]),
            "--no-auth-warning", "PING",
        ], env=environment, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL).stdout.strip() == b"PONG")

    def start_node(self):
        data_dir = self.private / "rbitcoin-datadir"
        data_dir.mkdir(mode=0o700)
        require(not any(data_dir.iterdir()), "datadir_not_fresh", "rbitcoin datadir is not fresh")
        self.node_owned = self.manager.start("node", [
            str(self.node_binary), "--network", "regtest", "--datadir", str(data_dir),
            "--rpc", "--rpc-listen", f"127.0.0.1:{self.ports['node_rpc']}",
            "--rpc-cookie-file", str(self.private / "node-rpc.cookie"),
        ] + (["--retarget-regtest"] if self.scenario.raw["retarget_claim"] else ["--regtest-pow-bits", self.scenario.raw["pow_bits"]]) + [
            "--no-listen", "--no-seeds", "--no-discover", "--max-outbound", "1",
            "--api-log", str(self.private / "node-api.jsonl"), "--log-level", "info",
        ], self.private / "node.log")
        self.rpc = RpcClient(self.ports["node_rpc"], self.rpc_user, self.rpc_password)
        info = wait_until("rbitcoin RPC readiness", self.scenario.raw["startup_timeout_secs"],
                          lambda: self.rpc.call("getblockchaininfo"))
        require(info.get("chain") == "regtest" and int(info.get("blocks", -1)) == 0,
                "node_genesis", f"node must expose a fresh regtest at height 0, got {info}")
        genesis_hash = self.rpc.call("getblockhash", [0])
        genesis_header = self.rpc.call("getblockheader", [genesis_hash, True])
        template = self.rpc.call("getblocktemplate", [{"rules": ["segwit"]}])
        self.observe("node_baseline", {
            "chain": info["chain"], "height": info["blocks"], "genesis": genesis_header,
            "initial_template_bits": template.get("bits"), "initial_template_height": template.get("height"),
            "p2p": "listen disabled; seeds/discovery/manual peers disabled; one unused outbound slot (node minimum 1)",
            "profile": "self-adjusting retarget profile: independent boundary arithmetic and applied budget commands only" if self.scenario.raw["retarget_claim"] else "baseline real PoW overlay; no retarget-profile claim",
        })

    def start_pool(self):
        cfg = self.scenario.raw
        pg_url = "postgres://{}:{}@127.0.0.1:{}/spc".format(
            urllib.parse.quote(self.pg_user, safe=""), urllib.parse.quote(self.pg_password, safe=""),
            self.ports["postgres"],
        )
        redis_url = "redis://:{}@127.0.0.1:{}".format(
            urllib.parse.quote(self.redis_password, safe=""), self.ports["redis"],
        )
        pool_config = self.private / "spc-pool.toml"
        atomic_write(pool_config, f"""[pool]
network = \"regtest\"
roles = [\"front\", \"payout\", \"api\"]
listen = \"127.0.0.1:{self.ports['sv2']}\"
authority_public_key = \"{self.authority_public}\"
authority_secret_key = \"{self.authority_secret}\"
pool_payout_descriptor = \"{cfg['pool_payout_descriptor']}\"
pool_payout_script = \"{cfg['pool_payout_script']}\"
fee_bps = {cfg['fee_bps']}
front_id = 7

[template]
source = \"rpc\"
rpc_url = \"http://127.0.0.1:{self.ports['node_rpc']}\"
rpc_user = \"{self.rpc_user}\"
rpc_pass = \"{self.rpc_password}\"
extranonce_bytes = 16
refresh_interval_ms = 250

[sv1]
enabled = false

[accounting]
work_unit_version = 2
work_reference_target = \"{cfg['reference_target']}\"
share_work_multiplier = {cfg['share_work_multiplier']}
max_work_multiplier = 4294967296
max_window_target_units = \"18446744073709551616\"
window_rule = \"network_block_work\"
window_n = {cfg['window_n']}
pow_limit_bits = 0x207fffff
slice_fee_delta_sats = 0
max_slice_shares = {cfg['max_slice_shares']}
dust_limit_sats = {cfg['dust_limit_sats']}
coinbase_threshold_sats = {cfg['coinbase_threshold_sats']}
max_payout_slots = {cfg['max_payout_slots']}
retain_windows = 16
scrub_after_blocks = 100
confirmation_depth = {cfg['confirmation_depth']}
allow_unchecked_chain = false

[extensions]
share_accounting = false
window_hot_blocks = 6

[jd]
listen = \"127.0.0.1:{self.ports['jd']}\"
distribution_interval_secs = 60
distribution_rebalance_bps = 100
grace_same_height_only = true

[storage]
postgres = \"{pg_url}\"
redis = \"{redis_url}\"
front_cursor_heartbeat_secs = 30
front_cursor_ttl_secs = 120

[api]
listen = \"127.0.0.1:{self.ports['api']}\"
""")
        os.chmod(pool_config, 0o600)
        pool_log = self.private / "pool.log"
        pool_log.touch(mode=0o600, exist_ok=True)
        self.pool_owned = self.manager.start("pool", [
            str(self.pool_binary), "--config", str(pool_config), "--roles", "front,payout,api",
        ], pool_log, extra_env={"RUST_LOG": "info,spc_pool=debug,spc_evidence=info"})
        self.pool_tail = PoolEvidenceTail(pool_log)
        pool_info = wait_until("pool API/work profile readiness", self.scenario.raw["startup_timeout_secs"],
                               lambda: http_json(self.ports["api"], "/api/pool"))
        profile = pool_info.get("work_profile", {})
        expected = {
            "work_unit_version": 2, "work_reference_target": cfg["reference_target"],
            "share_work_multiplier": cfg["share_work_multiplier"],
            "work_unit_fingerprint": self.scenario.fingerprint,
            "window_rule": "network_block_work", "window_n": cfg["window_n"],
            "pow_limit_bits": 0x207FFFFF, "share_difficulty": None,
        }
        for key, value in expected.items():
            require(profile.get(key) == value, "pool_profile", f"/api/pool {key}={profile.get(key)!r}, expected {value!r}")
        require("0x0020" not in pool_info.get("extensions", []),
                "pool_type32", "v2 type-32 must not be advertised")
        self.observe("pool_profile", pool_info)
        wait_until("SV2 listener", self.scenario.raw["startup_timeout_secs"], lambda: tcp_probe(self.ports["sv2"]))

    def start_miner(self):
        connections = []
        for identity in self.identities:
            name = identity["id"]
            connections.append(f"""[[connections]]
id = \"{name}-connection\"
endpoint = \"127.0.0.1:{self.ports['sv2']}\"
identity = \"{identity['address']}\"
authority_public_key = \"{self.authority_public}\"
min_extranonce_size = 4
reconnect = {{ enabled = true, initial_backoff_ms = 100, max_backoff_ms = 1000, handshake_timeout_ms = 10000 }}
workers = [
  {{ id = \"{name}-worker\", budget = {{ hashes_per_second = {self.scenario.raw['worker_hashes_per_second']} }}, ntime_policy = \"hold\", max_ntime_roll_seconds = 600, batch_size = {self.scenario.raw['worker_batch_size']} }},
]
""")
        miner_config = self.private / "miner.toml"
        atomic_write(miner_config, "config_version = 1\nminer_id = \"lp-live-miner\"\nevent_queue_capacity = 65536\n\n" + "\n".join(connections))
        run_checked([str(self.miner_binary), "--config", str(miner_config), "--check-config"],
                    self.private / "miner-check-config.log")
        self.miner_owned = self.manager.start("miner", [str(self.miner_binary), "--config", str(miner_config)],
                                              self.private / "miner.stderr", stdin=True, pipe_stdout=True)
        self.miner = MinerCollector(self.miner_owned, self.run_dir)
        names = {identity["id"] + "-connection" for identity in self.identities}
        wait_until("two native SV2 channels", self.scenario.raw["startup_timeout_secs"], lambda: self.channels_ready(names))
        wait_until("job_issued records for both channels", self.scenario.raw["startup_timeout_secs"],
                   lambda: self.jobs_ready(names))
        self.observe("measured_work_started", {
            "identities": len(self.identities), "bootstrap_generation_blocks": 0,
            "hash_budget_hps_per_worker": self.scenario.raw["worker_hashes_per_second"],
            "note": "all chain advancement below is native SV2 real work; no setup generation or datadir restart",
        })

    def channels_ready(self, names):
        active = set()
        for envelope in self.miner.event_values():
            event = envelope.get("event", {})
            if event.get("name") == "connection_state" and event.get("payload", {}).get("state") == "channel_open":
                active.add(event["payload"]["connection_id"])
            if event.get("name") == "job_activated":
                active.add(event["payload"]["connection_id"])
        return names <= active

    def jobs_ready(self, names):
        channels = set()
        for event in self.pool_tail.values()[0]:
            if event["kind"] == "job_issued":
                channels.add(int(event["evidence"]["channel_id"]))
        active_by_name = {}
        for envelope in self.miner.event_values():
            event = envelope.get("event", {})
            if event.get("name") == "job_activated":
                payload = event["payload"]
                active_by_name[payload["connection_id"]] = payload["job"]["job_id"]
        return len(channels) >= 2 and len(active_by_name) == 2 and set(active_by_name) == names

    def preflight(self):
        self.prepare_run_directory()
        self.prepare_binaries()
        self.allocate_ports()
        atomic_write(self.private / "preflight.json", {
            "binaries": self.binary_records, "provenance": self.provenance, "ports": self.ports,
            "node_network": "regtest", "p2p": "disabled", "pow_bits": self.scenario.raw["pow_bits"],
            "profile": "self_adjusting_retarget" if self.scenario.raw["retarget_claim"] else "baseline_real_pow",
            "retarget": self.scenario.raw["retarget_claim"],
        })
        self.observe("preflight", {"binaries": self.binary_records, "provenance": self.provenance, "ports": self.ports})

    def prepare_live(self):
        self.generate_identities()
        self.storage_credentials()
        self.start_postgres()
        self.start_redis()
        self.start_node()
        self.start_pool()
        self.start_miner()

    def check_frozen_inputs(self):
        for name, path in [("pool", self.pool_binary), ("miner", self.miner_binary),
                           ("scanner", self.scanner_binary), ("node", self.node_binary)]:
            require(sha256_file(path) == self.binary_records[name]["sha256"],
                    "binary_drift", f"{name} binary changed during the run")
        for name, source, manifest in [
            ("pool", self.args.pool_source_dir, self.args.pool_source_manifest),
            ("miner", self.args.miner_source_dir, self.args.miner_source_manifest),
            ("scanner", self.args.scanner_source_dir, self.args.scanner_source_manifest),
            ("node", self.args.node_source_dir, getattr(self.args, "node_source_manifest", None)),
        ]:
            if source and manifest:
                verify_manifest(Path(source).resolve(), Path(manifest).resolve(),
                                APPROVED_LP2_SNAPSHOT if name == "miner" else (
                                    APPROVED_L2_SNAPSHOT if name == "node" and self.scenario.raw["retarget_claim"] else None))

    def cleanup(self):
        errors = []
        if getattr(self, "miner", None):
            try:
                self.miner.stop()
            except Exception as error:
                errors.append(f"miner control: {error}")
        if getattr(self, "pool_tail", None):
            self.pool_tail.stop_event.set()
            self.pool_tail.thread.join(timeout=2)
        self.manager.stop_all(self.scenario.raw["shutdown_timeout_secs"])
        errors.extend(self.manager.cleanup_errors)
        if getattr(self, "miner", None):
            self.miner.writer.join(timeout=2)
            self.miner.reader.join(timeout=2)
        atomic_write(self.private / "processes.json", {
            "processes": [{
                "name": owned.name, "pid": owned.popen.pid, "returncode": owned.popen.poll(),
                "start_time_token": owned.token, "argv_sha256": sha256_bytes(json.dumps(owned.argv).encode()),
                "log": str(owned.log_path),
            } for owned in self.manager.processes],
            "cleanup_errors": errors,
        })
        if errors:
            self.observe("cleanup_errors", errors)
        self.process_stopped = True


    def miner_maps(self):
        submissions = []
        jobs = {}
        for envelope in self.miner.event_values():
            event = envelope.get("event", {})
            name = event.get("name")
            payload = event.get("payload", {})
            if name == "submitted":
                submissions.append(payload["share"])
            elif name == "job_activated":
                job = payload["job"]
                key = (payload["connection_id"], miner_job_number(job["job_id"]), str(job["target_generation"]))
                jobs.setdefault(key, [])
                if job not in jobs[key]:
                    jobs[key].append(job)
        return submissions, jobs

    def pool_job_map(self):
        jobs = {}
        events, errors = self.pool_tail.values()
        require(not errors, "pool_evidence_unknown", f"pool log evidence decode errors: {errors}", True)
        for event in events:
            if event["kind"] != "job_issued":
                continue
            record = event["evidence"]
            key = (int(record["channel_id"]), pool_job_number(record["job_id"]), str(record["generation"]))
            required = ["nbits", "share_target", "m_issue", "work_unit_version", "work_unit_fingerprint"]
            fingerprint = {field: record.get(field) for field in required}
            if key in jobs:
                require(jobs[key][0] == fingerprint, "pool_job_conflict", f"conflicting issued job record for {key}")
            else:
                jobs[key] = (fingerprint, [])
            jobs[key][1].append(record)
        return jobs

    @staticmethod
    def identity_for_connection(identities, connection_id):
        for identity in identities:
            if identity["id"] + "-connection" == connection_id:
                return identity
        raise LabError("unknown_connection", f"unexpected miner connection {connection_id}")

    def reconstruct_submission(self, share, jobs):
        key = (share["connection_id"], miner_job_number(share["job_id"]), str(share["target_generation"]))
        require(key in jobs, "miner_job_missing", f"no activated job record for {key}", True)
        matches = {}
        for job in jobs[key]:
            coinbase = b"".join([
                bytes.fromhex(job["coinbase_tx_prefix"]), bytes.fromhex(job["extranonce_prefix"]),
                bytes.fromhex(share["extranonce"]), bytes.fromhex(job["coinbase_tx_suffix"]),
            ])
            root = oracle.sha256d(coinbase)
            for sibling in job["merkle_path_le"]:
                root = oracle.sha256d(root + bytes.fromhex(sibling))
            header = b"".join([
                int(share["version"]).to_bytes(4, "little"), bytes.fromhex(job["prev_hash_le"]), root,
                int(share["ntime"]).to_bytes(4, "little"), int(job["nbits"]).to_bytes(4, "little"),
                int(share["nonce"]).to_bytes(4, "little"),
            ])
            if header.hex() == share["header"]:
                matches[(coinbase, header)] = job
        require(len(matches) == 1, "miner_job_ambiguous",
                f"expected exactly one job/coinbase matching the actual header, found {len(matches)}", True)
        (coinbase, header), job = next(iter(matches.items()))
        root = header[36:68]
        digest = oracle.sha256d(header)
        require(digest.hex() == share["hash_le"], "miner_hash", "recorded hash_le is not SHA256d(header)")
        require(oracle.display_hash(digest) == share["block_hash"], "miner_display_hash", "display hash is not digest reverse order")
        network_target = oracle.decode_compact_target(int(job["nbits"]))
        require(network_target == oracle.target_from_le_hex(share["network_target_le"]),
                "network_target", "miner network target does not match independently decoded job nbits")
        require(oracle.target_from_le_hex(share["share_target_le"]) == oracle.target_from_le_hex(job["share_target_le"]),
                "share_target", "submission target differs from its activated job")
        qualified = int.from_bytes(digest, "little") <= network_target
        require(qualified == share["block_qualified"], "block_classification", "miner block qualification differs from actual hash/network target")
        return {"job": job, "header": header, "digest": digest, "coinbase": coinbase, "network_target": network_target, "block_qualified": qualified}

    def node_raw_block_if_present(self, display: str):
        try:
            raw_hex = self.rpc.call("getblock", [display, 0])
            return bytes.fromhex(raw_hex)
        except (LabError, ValueError):
            return None

    def node_api_submissions(self):
        path = self.private / "node-api.jsonl"
        if not path.exists():
            return []
        records = []
        for line in path.read_text(errors="replace").splitlines():
            try:
                value = json.loads(line)
            except ValueError:
                continue
            if value.get("method") == "submitblock":
                records.append(value)
        return records

    def candidate_pending_or_record(self, share, reconstructed, miner_jobs):
        display = reconstructed["digest"][::-1].hex()
        raw_block_bytes = self.node_raw_block_if_present(display)
        if raw_block_bytes is None:
            self.observe("canonical_at", {"block_hash": display, "height": None, "anchored_tip": None, "result": "unknown", "detail": "node has not returned a block body"})
            raise CandidatePending("canonical block body unavailable")
        raw_block = oracle.parse_block(raw_block_bytes)
        require(raw_block["header"] == reconstructed["header"], "canonical_header", "node canonical header differs from miner submission")
        require(raw_block["hash"] == reconstructed["digest"], "canonical_hash", "node block hash differs from independent SHA256d")
        require(raw_block["transactions"][0]["raw"] == reconstructed["coinbase"],
                "canonical_coinbase", "node coinbase differs from actual job prefix/extranonce/suffix")
        require(oracle.merkle_root([tx["txid"] for tx in raw_block["transactions"]]) == reconstructed["header"][36:68],
                "canonical_merkle", "independently parsed node transaction Merkle root differs from header")
        header_rpc = self.rpc.call("getblockheader", [display, True])
        require(header_rpc.get("hash") == display and int(header_rpc.get("confirmations", -1)) > 0,
                "canonical_membership", "node did not anchor the candidate in its active chain")
        height = int(header_rpc["height"])
        tip_height = int(self.rpc.call("getblockcount"))
        canonical_hash = self.rpc.call("getblockhash", [height])
        require(canonical_hash == display, "canonical_membership", "node active hash at candidate height differs")
        submissions = self.node_api_submissions()
        matching_submissions = [item for item in submissions if reconstructed["header"].hex() in item.get("params", "") and item.get("ok") is True]
        if not matching_submissions:
            self.observe("candidate", {"block_hash": display, "height": height, "result": "unknown", "detail": "no successful node submitblock record containing the exact header"})
            raise CandidatePending("pool-to-node candidate submission evidence unavailable")
        require(reconstructed["block_qualified"], "candidate_classification", "canonical candidate did not independently meet network target")
        block = self.db.block(reconstructed["digest"].hex())
        if block is None or block.get("snapshot") is None:
            raise CandidatePending("frozen block event/distribution row not persisted yet")
        record = self.audit_frozen_distribution(share, reconstructed, miner_jobs, raw_block, block, height)
        record["candidate"] = {
            "node_submit_ok": True, "api_log_count": len(matching_submissions),
            "canonical_at": {"result": "yes", "anchored_tip": tip_height, "observed_at": utc_now()},
        }
        return record

    def audit_frozen_distribution(self, share, reconstructed, miner_jobs, raw_block, block, height):
        cfg = self.scenario.raw
        snapshot = block["snapshot"]
        require(int(block["height"]) == height == int(snapshot["height"]), "frozen_height", "distribution and canonical heights differ")
        require(int(snapshot["work_unit_version"]) == 2 and int(snapshot["window_rule"]) == 1,
                "frozen_v2_rule1", "frozen distribution is not work v2 / window rule 1")
        require(snapshot["work_unit_fingerprint"] == self.scenario.fingerprint,
                "frozen_fingerprint", "frozen work-unit fingerprint differs from independent reference hash")
        network_target = reconstructed["network_target"]
        expected_q = oracle.window_target(self.scenario.reference, cfg["window_n"], network_target)
        require(int(snapshot["window_target"]) == expected_q,
                "frozen_q", f"frozen Q={snapshot['window_target']}, independently expected {expected_q}")
        high_water = snapshot["high_water"]
        require(isinstance(high_water, dict) and len(high_water) == 1, "high_water_shape", "single-front high-water vector required")
        high_value = next(iter(high_water.values()))
        mark = int(high_value)
        rows = self.db.slices(mark)
        for row in rows:
            require(int(row["work_unit_version"]) == 2 and row["work_unit_fingerprint"] == self.scenario.fingerprint,
                    "slice_v2_fingerprint", "sealed slice has wrong work version/fingerprint")
        selected = oracle.select_window(rows, expected_q)
        require(not selected["ambiguous"], "window_unknown", "slice retention cap makes frozen window ambiguous", True)
        if selected["rows"]:
            require(int(selected["rows"][0]["slice_id"]) == mark, "high_water", "high-water mark is not the newest consumed slice")
        if cfg["require_full_window"] and selected["underfill"]:
            raise CandidatePending(f"window warmup: {selected['actual_sum']} < Q {expected_q}")
        stats = self.rpc.call("getblockstats", [reconstructed["digest"][::-1].hex(), ["subsidy", "totalfee", "height"]])
        subsidy, fees = int(stats["subsidy"]), int(stats["totalfee"])
        require(subsidy >= 0 and fees >= 0, "block_economics", "negative node economics")
        coinbase = raw_block["transactions"][0]
        actual_total = sum(output["value"] for output in coinbase["outputs"])
        require(actual_total == subsidy + fees, "actual_revenue", f"actual coinbase {actual_total} != node subsidy+fees {subsidy + fees}")
        entries = oracle.parse_entries(snapshot["entries"])
        require(len({entry["sp_id"] for entry in entries}) == len(entries), "distribution_entries", "duplicate identity in frozen entries")
        expected = oracle.expected_raw_payouts(selected["rows"], subsidy, fees, cfg["fee_bps"])
        identity_ids = {identity["sp_id"] for identity in self.identities}
        owed = {identity: expected["raw"].get(identity, 0) for identity in identity_ids}
        if any(value <= 0 for value in owed.values()):
            raise CandidatePending("one or both identities have no frozen pre-finding credit yet")
        paid = self.actual_paid_outputs(coinbase["outputs"], entries)
        for identity in identity_ids:
            if paid.get(identity, 0) <= 0:
                raise CandidatePending("one or both identities are not represented in this candidate coinbase")
        return {
            "share": share, "reconstructed": reconstructed, "raw_block": raw_block,
            "block": block, "entries": entries, "selected_window": selected,
            "expected": expected, "subsidy": subsidy, "fees": fees, "paid": paid,
            "height": height, "network_target": network_target, "expected_q": expected_q,
        }

    def actual_paid_outputs(self, outputs, entries):
        expected_scripts = {}
        for entry in entries:
            if entry["p_x"]:
                script = b"\x51\x20" + bytes.fromhex(entry["p_x"])
                if script in expected_scripts:
                    raise LabError("distribution_scripts", "duplicate frozen payout script")
                expected_scripts[script] = entry["sp_id"]
        pool_script = bytes.fromhex(self.scenario.raw["pool_payout_script"])
        require(outputs and outputs[0]["script"] == pool_script, "pool_output", "first coinbase output is not the declared pool output")
        paid = {}
        for index, output in enumerate(outputs[1:], start=1):
            identity = expected_scripts.get(output["script"])
            require(identity is not None, "unknown_coinbase_output", f" unrecognized coinbase output at vout {index}")
            paid[identity] = paid.get(identity, 0) + output["value"]
        return paid

    def seek_candidate(self):
        submission_values, miner_jobs = self.miner_maps()
        self.miner.assert_complete()
        for share in submission_values:
            if not share.get("block_qualified"):
                continue
            digest_hex = share["hash_le"]
            if self.selected and self.selected["share"]["hash_le"] == digest_hex:
                continue
            reconstructed = self.reconstruct_submission(share, miner_jobs)
            require(reconstructed["block_qualified"], "candidate_target", "block-qualified submission is above network target")
            try:
                record = self.candidate_pending_or_record(share, reconstructed, miner_jobs)
            except CandidatePending as pending:
                self.rejected_candidates[digest_hex] = str(pending)
                continue
            self.selected = record
            atomic_write(self.private / "frozen-candidate.json", self.public_candidate_record(private=True))
            self.observe("candidate_selected", self.public_candidate_record(private=True))
            return True
        return False

    def public_candidate_record(self, private=False):
        selected = self.selected
        return {
            "block_hash": selected["reconstructed"]["digest"][::-1].hex(),
            "hash_le": selected["reconstructed"]["digest"].hex(), "height": selected["height"],
            "nbits": int.from_bytes(selected["reconstructed"]["header"][72:76], "little"),
            "network_target": str(selected["network_target"]), "expected_q": selected["expected_q"],
            "actual_window_work": selected["selected_window"]["actual_sum"],
            "window_underfill": selected["selected_window"]["underfill"],
            "window_overshoot": selected["selected_window"]["overshoot"],
            "slice_ids": [row["slice_id"] for row in selected["selected_window"]["rows"]],
            "raw_by_identity": selected["expected"]["raw"] if private else {"count": len(selected["expected"]["raw"])},
            "subsidy": selected["subsidy"], "fees": selected["fees"],
            "candidate": selected["candidate"],
        }

    def wait_settlement(self):
        target = self.selected
        height, depth = target["height"], self.scenario.raw["confirmation_depth"]
        digest = target["reconstructed"]["digest"].hex()
        def settled():
            block = self.db.block(digest)
            if not block:
                return False
            tip = int(self.rpc.call("getblockcount"))
            confirmations = tip - height + 1
            if block["status"] == "confirmed" and block["settled_at"] is not None:
                require(confirmations >= depth, "early_settlement",
                        f"block settled at {confirmations} confirmations, configured depth {depth}")
                require(self.rpc.call("getblockhash", [height]) == target["reconstructed"]["digest"][::-1].hex(),
                        "settled_noncanonical", "settled block is not canonical at anchored tip")
                return block
            require(block["status"] != "orphaned", "candidate_orphaned", "selected candidate was orphaned before settlement")
            return False
        block = wait_until(f"frozen block settlement at depth {depth}",
                           max(1, self.scenario.raw["scenario_timeout_secs"] // 3), settled, 0.2)
        self.observe("settled", {
            "block_hash": target["reconstructed"]["digest"][::-1].hex(), "height": height,
            "configured_depth": depth, "status": block["status"], "settled_at": block["settled_at"],
        })
        self.selected["settlement"] = block
        return block

    def pending_before_selected(self):
        values = self.db.pending_before(self.selected["height"])
        pending = {}
        for identity in self.identities:
            pending[identity["sp_id"]] = int(values.get(identity["sp_id"], 0))
        return pending

    def verify_conservation(self):
        selected = self.selected
        block = selected["settlement"]
        require(block["status"] == "confirmed" and block["fingerprint"] == selected["block"]["fingerprint"],
                "settled_fingerprint", "settlement did not use the frozen distribution fingerprint")
        ledger = block["ledger"]
        by_identity = {}
        for row in ledger:
            identity = row["sp_id"]
            require(identity not in by_identity, "ledger_duplicate", "one identity has multiple ledger rails/rows")
            by_identity[identity] = row
        pending_before = self.pending_before_selected()
        allocation_after = {}
        for identity in self.identities:
            sp_id = identity["sp_id"]
            require(sp_id in by_identity, "missing_ledger_identity", f"{identity['id']} has no settled ledger row")
            row = by_identity[sp_id]
            owed = selected["expected"]["raw"].get(sp_id, 0) + pending_before[sp_id]
            pending_after = pending_before[sp_id] + int(row["pending_delta"])
            require(pending_after >= 0, "pending_negative", "pending ledger becomes negative")
            allocation = int(row["paid_sats"]) + int(row["ark_due_sats"]) + int(row["forfeited_sats"]) + pending_after
            require(allocation == owed, "identity_conservation",
                    f"{identity['id']}: paid+Ark+forfeit+pending_after={allocation}, independently owed={owed}")
            require(int(row["paid_sats"]) == selected["paid"].get(sp_id, 0),
                    "paid_mismatch", "ledger paid_sats differs from independently parsed actual coinbase")
            if self.scenario.raw["payout_policy"] == "pool_template_exact_owed_pre_ladder":
                require(row["rail"] == "coinbase" and int(row["paid_sats"]) == owed,
                        "declared_payout_shape", f"{identity['id']} does not receive exact owed in the declared pre-ladder profile")
            allocation_after[sp_id] = {"ark": int(row["ark_due_sats"]), "forfeit": int(row["forfeited_sats"]), "pending": pending_after}
        total_paid = sum(int(row["paid_sats"]) for row in ledger)
        pay_p = selected["raw_block"]["transactions"][0]["outputs"][0]["value"]
        require(total_paid + pay_p == selected["subsidy"] + selected["fees"],
                "coinbase_conservation", "sum of independent paid outputs plus pay_P differs from actual revenue")
        backing = selected["expected"]["pool_sats"] + sum(
            value["ark"] + value["forfeit"] + value["pending"] - pending_before[identity]
            for identity, value in allocation_after.items()
        )
        require(pay_p == backing, "pool_output_conservation", f"pay_P={pay_p}, independently expected {backing}")
        for payout in block["ark_payouts"]:
            require(int(payout["due_height"]) == selected["height"] + self.scenario.raw["confirmation_depth"],
                    "ark_due_height", "Ark scheduler does not use configured found-height plus confirmation depth")
        tip = int(self.rpc.call("getblockcount"))
        require(tip - selected["height"] + 1 >= self.scenario.raw["confirmation_depth"],
                "settlement_depth", "anchored tip lacks required confirmations")
        self.selected["conservation"] = {
            "pay_p": pay_p, "total_paid": total_paid, "independent_pool_sats": selected["expected"]["pool_sats"],
            "identities": len(by_identity), "confirmations_at_check": tip - selected["height"] + 1,
        }

    def verify_nonblock_accounted(self):
        submissions, miner_jobs = self.miner_maps()
        pool_jobs = self.pool_job_map()
        accounted = [event["evidence"] for event in self.pool_tail.values()[0] if event["kind"] == "accounted"]
        by_hash = {}
        for record in accounted:
            require(record["share_hash"] not in by_hash, "accounted_collision", "duplicate accounted share hash evidence")
            by_hash[record["share_hash"]] = record
        rows = self.db.slices((1 << 62))
        require(len(rows) < 4097, "share_evidence_retention", "retained sealed-slice history is capped; nonblock proof unknown", True)
        leaves = {}
        for row in rows:
            decoded = json.loads(bytes.fromhex(row["shares_hex"]))
            for leaf in decoded:
                normalized = dict(leaf)
                normalized["extranonce"] = serialized_hex(normalized["extranonce"])
                key = share_leaf_key(normalized)
                require(key not in leaves, "leaf_collision", "duplicate sealed share leaf key")
                leaves[key] = (row, normalized)
        checked = []
        for share in submissions:
            if share["block_qualified"] or share["hash_le"] not in by_hash:
                continue
            try:
                checked.append(self.check_nonblock(share, miner_jobs, pool_jobs, by_hash[share["hash_le"]], leaves, by_hash))
            except LabError as error:
                if error.code != "nonblock_not_sealed":
                    raise
        require(checked, "nonblock_accounted_missing", "no sealed non-block share has independent accounted evidence", True)
        required_identities = {identity["id"] for identity in self.identities}
        for identity in required_identities:
            require(any(item["identity"] == identity for item in checked),
                    "identity_not_accounted", f"identity {identity} has no independently verified accounted non-block share")
        self.nonblock = checked[0]
        atomic_write(self.private / "nonblock-proofs.json", checked)
        self.observe("nonblock_accounted", checked[0])
        return checked

    def check_nonblock(self, share, miner_jobs, pool_jobs, account, leaves, accounts_by_hash):
        reconstructed = self.reconstruct_submission(share, miner_jobs)
        require(not reconstructed["block_qualified"] and int.from_bytes(reconstructed["digest"], "little") > reconstructed["network_target"],
                "nonblock_target", "selected non-block actually meets network target")
        identity = self.identity_for_connection(self.identities, share["connection_id"])
        generation = str(account["generation"])
        key = (int(account["channel_id"]), pool_job_number(account["job_id"]), generation)
        require(key in pool_jobs, "job_issued_missing", f"missing job_issued {key}", True)
        issued, _ = pool_jobs[key]
        expected_m = oracle.issuance_multiplier(self.scenario.reference, self.scenario.raw["share_work_multiplier"], reconstructed["network_target"])
        require(int(issued["nbits"]) == int(reconstructed["job"]["nbits"]), "issued_nbits", "issued job nbits differs from hashed header")
        require(int(issued["m_issue"]) == expected_m == int(account["m_issue"]) == int(account["d"]),
                "issued_multiplier", f"accounted m != independently issued m {expected_m}")
        require(int(issued["work_unit_version"]) == 2 and issued["work_unit_fingerprint"] == self.scenario.fingerprint,
                "issued_v2", "issued/accounted profile is not v2 with expected fingerprint")
        expected_share_target = oracle.share_target(self.scenario.reference, expected_m)
        require(int(issued["share_target"], 16) == expected_share_target == oracle.target_from_le_hex(share["share_target_le"]),
                "issued_target", "share target differs from the exact v2 family member")
        require(int.from_bytes(reconstructed["digest"], "little") <= expected_share_target,
                "hash_above_share_target", "accounted non-block hash does not meet job share target")
        require(account["sp_id"] == identity["sp_id"], "accounted_identity", "accounted share belongs to a different identity")
        for field in ["nonce", "ntime", "version"]:
            require(int(account[field]) == int(share[field]), "accounted_header_field", f"accounted {field} differs from miner header")
        full_extranonce = bytes.fromhex(reconstructed["job"]["extranonce_prefix"]) + bytes.fromhex(share["extranonce"])
        require(account["extranonce"] == full_extranonce.hex(), "accounted_extranonce", "accounted extranonce is not pool-prefix || miner local")
        require(miner_job_number(share["job_id"]) == pool_job_number(account["job_id"]),
                "accounted_generation", "accounted job/generation differs from miner job")
        normalized = dict(share)
        normalized["extranonce"] = full_extranonce.hex()
        normalized["share_index"] = share["sequence_number"]
        previous_display = reconstructed["header"][4:36][::-1].hex()
        previous_header = self.rpc.call("getblockheader", [previous_display, True])
        reconstructed["height"] = int(previous_header["height"]) + 1
        normalized["job_id"] = parse_pool_job_id(account["job_id"], reconstructed)
        key_leaf = share_leaf_key(normalized)
        require(key_leaf in leaves, "nonblock_not_sealed", "accounted non-block leaf is absent from sealed DB shares", True)
        row, leaf = leaves[key_leaf]
        require(leaf == normalized or all(str(leaf.get(field)) == str(normalized.get(field)) for field in ["nonce", "ntime", "version", "job_id", "share_index"]),
                "sealed_leaf", "sealed DB leaf differs from accounted submission")
        matched = match_slice_leaves(row, leaves, accounts_by_hash)
        sums = {}
        for matched_account in matched:
            require(matched_account["sp_id"] in [item["sp_id"] for item in self.identities], "slice_identity", "slice contains unknown fixture identity")
            sums[matched_account["sp_id"]] = sums.get(matched_account["sp_id"], 0) + int(matched_account["d"])
        credit = {item["sp_id"]: int(item["diff_sum"]) for item in row["credits"]}
        require(sums == credit, "slice_credit_exact", f"independent per-job m sums {sums} != slice_credit {credit}")
        return {
            "identity": identity["id"], "share_hash": share["hash_le"], "header": share["header"],
            "job_id": str(share["job_id"]), "pool_generation": generation, "miner_target_generation": str(share["target_generation"]), "m_issue": expected_m,
            "share_target": f"{expected_share_target:064x}", "network_target": f"{reconstructed['network_target']:064x}",
            "slice_id": row["slice_id"], "slice_work_recomputed": sum(sums.values()),
        }

    def pause_and_scan(self):
        workers = [identity["id"] + "-worker" for identity in self.identities]
        result = self.miner.command("pause_workers", {"worker_ids": workers}, timeout=10)
        snapshot = self.miner.snapshot()
        atomic_write(self.private / "miner-final-snapshot.json", snapshot)
        atomic_write(self.private / "miner-pause-result.json", result)
        selected = self.selected
        raw_block = selected["raw_block"]
        branch = oracle.merkle_branch_zero([tx["txid"] for tx in raw_block["transactions"]])
        proof = oracle.light_proof(raw_block["header"], branch, raw_block["transactions"][0]["raw"])
        proof_path = self.private / "canonical-light-proof.bin"
        atomic_write(proof_path, proof)
        recoveries = []
        for identity in self.identities:
            output = run_checked([
                str(self.scanner_binary), "light", "--address", identity["address"],
                "--scan-key", identity["scan_key"], "--proofs", str(proof_path),
            ], self.private / f"scan-{identity['id']}.log")
            lines = [json.loads(line) for line in output.decode().splitlines() if line.strip()]
            found = [item for line in lines for item in line.get("found", [])]
            entry = next(item for item in selected["entries"] if item["sp_id"] == identity["sp_id"])
            expected_script = b"\x51\x20" + bytes.fromhex(entry["p_x"])
            expected = [(index, output) for index, output in enumerate(raw_block["transactions"][0]["outputs"])
                        if output["script"] == expected_script]
            require(found, "scanner_no_output", f"scanner did not recover {identity['id']} expected output")
            for item in found:
                require(oracle.parse_serialized_bytes(item["output_key"]).hex() == entry["p_x"], "scanner_output_key", "scanner output key differs from frozen entry")
                require(any(int(item["vout"]) == index and int(item["amount"]) == output["value"] and int(item["height"]) == selected["height"]
                            for index, output in expected),
                        "scanner_amount", "scanner vout/amount/height differs from canonical coinbase")
            require(len(found) == len(expected), "scanner_output_count", "scanner recovered wrong output count for exact-owed profile")
            recoveries.append({"identity": identity["id"], "found": found})
        atomic_write(self.private / "scanner-recovery.json", recoveries)
        self.selected["scanner"] = recoveries

    def capture_api(self):
        pool = http_json(self.ports["api"], "/api/pool")
        blocks = http_json(self.ports["api"], f"/api/pool/blocks?limit={self.scenario.raw['max_node_height'] + 1}")
        liability = http_json(self.ports["api"], "/api/pool/liability")
        window = http_json(self.ports["api"], "/api/pool/window")
        atomic_write(self.private / "api-final.json", {"pool": pool, "blocks": blocks, "liability": liability, "window": window})
        target = self.selected["reconstructed"]["digest"].hex()
        matches = [item for item in blocks.get("blocks", []) if item.get("block_hash") == target]
        require(matches and matches[0].get("status") == "confirmed", "api_block", "API does not expose the confirmed internal block hash")
        self.observe("api_evidence", {"confirmed_block_rows": len(matches), "work_profile": pool.get("work_profile")})

    def validate_retarget_profile(self):
        consensus = self.rpc.call("getblockchaininfo")["consensus"]
        expected = {
            "network": "regtest", "pow_limit_bits": "207fffff", "retarget_floor_bits": "1e7fffff",
            "pow_target_spacing": 10, "pow_target_timespan": 240,
            "difficulty_adjustment_interval": 24, "no_pow_retargeting": False,
            "allow_min_difficulty_blocks": False, "enforce_bip94": False,
            "regtest_pow_bits_overlay": "1e7fffff",
        }
        for key, value in expected.items():
            require(consensus.get(key) == value, "retarget_profile_mismatch",
                    f"consensus.{key}={consensus.get(key)!r}, expected {value!r}")
        # Do not require optional subsidy fields not present on older builds.
        for forbidden in ["ladder", "ark_backend", "witness_commitment_required"]:
            require(forbidden not in consensus or not consensus[forbidden],
                    "unsupported_capability", f"scenario does not assert {forbidden}")
        atomic_write(self.private / "retarget-profile.json", consensus)
        return consensus

    def wait_chain_height(self, height: int):
        phase_deadline = time.monotonic() + self.scenario.raw["scenario_timeout_secs"]
        while True:
            current = int(self.rpc.call("getblockcount"))
            if current >= height:
                require(current == height, "phase_boundary",
                        f"chain moved past declared height {height} to {current}; phase cannot be replayed/relabelled")
                snapshot = self.miner.snapshot()
                atomic_write(self.private / f"height-{height}-miner-snapshot.json", snapshot)
                self.observe("height_reached", {"height": height, "snapshot": snapshot})
                return snapshot
            require(time.monotonic() < phase_deadline, "height_timeout",
                    f"bounded wait for chain height {height} timed out at {current}", True)
            for owned in [self.node_owned, self.miner_owned, self.pool_owned, self.redis_owned, self.postgres_owned]:
                require(owned.popen.poll() is None, "blocked_on_height",
                        f"{owned.name} exited while a height producer was required", True)
            require(current < self.scenario.raw["max_node_height"], "height_bound", "node height bound reached")
            time.sleep(0.1)

    def pause_all_workers(self):
        workers = [identity["id"] + "-worker" for identity in self.identities]
        result = self.miner.command("pause_workers", {"worker_ids": workers}, timeout=15)
        self.observe("workers_paused", result)
        return result

    def resume_all_workers(self):
        workers = [identity["id"] + "-worker" for identity in self.identities]
        result = self.miner.command("resume_workers", {"worker_ids": workers}, timeout=15)
        self.observe("workers_resumed", result)
        return result

    def change_all_hash_budgets(self, hashes_per_second: int):
        results = []
        for identity in self.identities:
            worker = identity["id"] + "-worker"
            payload = {"worker_id": worker, "budget": {"hashes_per_second": hashes_per_second, "total_attempts": None}}
            result = self.miner.command("set_worker_budget", payload, timeout=15)
            results.append({"worker": worker, "payload": payload, "result": result})
        snapshot = self.miner.snapshot()
        self.observe("hash_budget_changed", {
            "requested_hps_per_worker": hashes_per_second,
            "requested_hps_combined": hashes_per_second * len(self.identities),
            "commands": results, "snapshot": snapshot,
        })
        atomic_write(self.private / f"budget-{hashes_per_second}-snapshot.json", snapshot)
        return snapshot

    def run_retarget_phases(self):
        consensus = self.validate_retarget_profile()
        cfg = self.scenario.raw
        phases = [
            (cfg["coverage_height"], cfg["live_hashes_per_second"], "coverage_only_first_genesis_clamped_period"),
            (cfg["first_full_period_height"], cfg["changed_hashes_per_second"], "live_full_period_at_declared_live_hps_per_worker"),
            (cfg["second_full_period_height"], None, "live_full_period_at_changed_hps_per_worker"),
        ]
        self.observe("retarget_phase_plan", {
            "phases": [{"height": height, "next_hps_per_worker": hps, "label": label} for height, hps, label in phases],
            "no_setup_generation": True, "initial_hps_per_worker": cfg["worker_hashes_per_second"],
            "initial_combined_hps": cfg["worker_hashes_per_second"] * 2,
            "note": "every block comes from native SV2 SHA256d; no post-solution holding, no RPC generation, no mock time",
        })
        for height, next_hps, label in phases:
            self.wait_chain_height(height)
            self.pause_all_workers()
            if next_hps is not None:
                self.change_all_hash_budgets(next_hps)
            self.observe("retarget_phase_boundary", {"height": height, "label": label})
            self.resume_all_workers()
        evidence = self.audit_retarget_headers(consensus)
        self.retarget_evidence = evidence
        atomic_write(self.private / "retarget-evidence.json", evidence)
        self.observe("retarget_oracle_passed", evidence)

    def audit_retarget_headers(self, consensus):
        cfg = self.scenario.raw
        headers = []
        for height in range(cfg["second_full_period_height"] + 1):
            block_hash = self.rpc.call("getblockhash", [height])
            header = self.rpc.call("getblockheader", [block_hash, True])
            header_hex = self.rpc.call("getblockheader", [block_hash, False])
            header_bytes = bytes.fromhex(header_hex)
            require(len(header_bytes) == 80 and oracle.display_hash(oracle.sha256d(header_bytes)) == block_hash,
                    "header_hash", f"independent SHA256d failed at height {height}")
            require(int(header["bits"], 16) == int.from_bytes(header_bytes[72:76], "little"),
                    "header_bits", f"RPC bits and raw header differ at height {height}")
            headers.append({
                "height": height, "hash": block_hash, "bits": header["bits"],
                "time": int(header["time"]), "header_hex": header_hex,
            })
        require(not oracle.validate_retarget_vector(headers, consensus),
                "retarget_oracle_mismatch", "one or more actual header nBits values violate the independent retarget vector")
        required_profile = {
            "pow_limit_bits": "207fffff", "retarget_floor_bits": "1e7fffff",
            "pow_target_spacing": 10, "pow_target_timespan": 240,
            "difficulty_adjustment_interval": 24, "no_pow_retargeting": False,
            "allow_min_difficulty_blocks": False, "enforce_bip94": False,
            "regtest_pow_bits_overlay": "1e7fffff",
        }
        for key, expected in required_profile.items():
            require(consensus[key] == expected, "profile_fingerprint", f"unexpected effective profile field {key}")
        boundaries = []
        for height in [cfg["coverage_height"], cfg["first_full_period_height"], cfg["second_full_period_height"]]:
            expected = oracle.expected_retarget_bits(headers[: height + 1], height, consensus)
            actual = int(headers[height]["bits"], 16)
            require(actual == expected, "boundary_bits", f"height {height}: actual {actual:08x}, expected {expected:08x}")
            first = headers[height - int(consensus["difficulty_adjustment_interval"])]
            elapsed = int(headers[height - 1]["time"]) - int(first["time"])
            boundaries.append({
                "height": height, "period_first_height": first["height"], "period_first_time": first["time"],
                "parent_time": headers[height - 1]["time"], "actual_elapsed_seconds": elapsed,
                "clamped_elapsed_seconds": min(int(consensus["pow_target_timespan"]) * 4, max(int(consensus["pow_target_timespan"]) // 4, elapsed)),
                "parent_bits": headers[height - 1]["bits"], "actual_bits": headers[height]["bits"],
                "independently_expected_bits": f"{expected:08x}", "block_hash": headers[height]["hash"],
            })
        atomic_write(self.private / "retarget-evidence-partial.json", {
            "profile": consensus, "boundaries": boundaries, "headers": headers,
            "note": "retained before boundary-identity predicates; a lack of movement is not an arithmetic mismatch",
        })
        first_boundary = boundaries[0]
        require(first_boundary["actual_bits"] == cfg["pow_bits"][2:], "first_boundary_floor",
                "genesis-anchored first boundary is not the documented floor clamp")
        first_full_period = boundaries[1]
        require(first_full_period["actual_bits"] != first_boundary["actual_bits"],
                "first_full_period_unchanged", "boundary 48 bits equal the floor; no retarget movement observed in the first full period")
        second_full_period = boundaries[2]
        require(second_full_period["actual_bits"] != first_full_period["actual_bits"],
                "second_full_period_unchanged", "boundary 72 bits equal boundary 48")
        final_snapshot = self.miner.snapshot()
        workers = final_snapshot.get("workers", [])
        require(len(workers) == 2 and all(int(worker["attempted"]) > 0 for worker in workers),
                "real_hash_attempts", "both LP2 workers must have positive real attempts")
        retarget_duty = retarget_duty_cycle(self.private, {"headers": headers, "profile": consensus, "boundaries": boundaries})
        return {
            "profile": consensus, "boundaries": boundaries, "headers": headers,
            "duty_cycle": retarget_duty,
            "coverage_declaration": {
                "coverage_heights": "1..24 excluded from budget-causation claims; first boundary is genesis-clamped",
                "live_periods": [f"25..47 at {cfg['live_hashes_per_second']} requested hps/worker", f"49..71 at {cfg['changed_hashes_per_second']} requested hps/worker"],
                "budget_control": "pause_workers -> set_worker_budget -> snapshot -> resume_workers; pausing never holds a found solution",
                "full_period_boundaries": [48, 72],
                "final_worker_measurements": workers,
                "assumption": "requested rates are not measured rates; achieved_hps and attempted counters are recorded instead of promised exact timing",
            },
        }



    def execute(self):
        self.preflight()
        if self.args.validate_only:
            self.observe("validation_only", "binary/CLI/source/ports/scenario checks passed; no children started")
            return
        self.prepare_live()
        if self.scenario.raw["retarget_claim"]:
            self.run_retarget_phases()
        deadline = time.monotonic() + self.scenario.raw["scenario_timeout_secs"]
        while self.selected is None and time.monotonic() < deadline:
            for owned in [self.node_owned, self.redis_owned, self.postgres_owned, self.pool_owned, self.miner_owned]:
                require(owned.popen.poll() is None, "child_exited", f"required child {owned.name} exited with {owned.popen.poll()}; private logs retained")
            self.miner.assert_complete()
            if int(self.rpc.call("getblockcount")) > self.scenario.raw["max_node_height"]:
                raise LabError("height_bound", "scenario height bound reached without an eligible frozen block")
            self.seek_candidate()
            if self.selected is None:
                time.sleep(0.1)
        require(self.selected is not None, "eligible_candidate_timeout",
                f"no eligible candidate before bounded deadline; candidates: {list(self.rejected_candidates.values())[-10:]}", True)
        while time.monotonic() < deadline:
            block = self.selected.get("settlement")
            if block is not None:
                break
            remaining = deadline - time.monotonic()
            require(remaining > 0, "settlement_timeout", "bounded settlement wait expired", True)
            self.wait_settlement()
            self.verify_conservation()
            self.verify_nonblock_accounted()
            self.pause_and_scan()
            self.capture_api()
        require(self.selected.get("settlement") is not None, "settlement_timeout", "settlement did not complete", True)
        require(self.nonblock is not None, "nonblock_missing", "nonblock proof did not complete", True)
        require("scanner" in self.selected, "scanner_missing", "recipient-side proof did not complete", True)
        self.check_frozen_inputs()


class CandidatePending(Exception):
    pass




def retarget_phase(wall_ms, boundaries):
    if wall_ms < boundaries[24]:
        return "coverage_1_to_24"
    if wall_ms < boundaries[48]:
        return "live_period_25_to_47"
    if wall_ms < boundaries[72]:
        return "changed_period_49_to_71"
    return "other_outside_declared_periods"


def retarget_boundaries_from_observations(path):
    boundaries = {}
    for line in path.read_text().splitlines():
        value = json.loads(line)
        if value.get("kind") == "retarget_phase_boundary":
            height = int(value["payload"]["height"])
            instant = dt.datetime.fromisoformat(value["observed_at"].replace("Z", "+00:00"))
            boundaries[height] = instant.timestamp() * 1000
    for height in [24, 48, 72]:
        require(height in boundaries, "retarget_replay_evidence", f"missing phase boundary {height} in {path}")
    return boundaries


def empty_duty_counts():
    return {
        "submitted": 0,
        "ingress_ack": 0,
        "ingress_rejected_stale_share": 0,
        "block_qualified": 0,
    }


def mine_phase_duty_counts(miner_path, observations_path):
    """Classify actual wire/control records by their event-time phase markers."""
    boundaries = retarget_boundaries_from_observations(observations_path)
    phases = {name: empty_duty_counts() for name in [
        "coverage_1_to_24", "live_period_25_to_47", "changed_period_49_to_71",
        "other_outside_declared_periods",
    ]}
    counts = {"submitted": 0, "ingress_ack": 0,
              "ingress_rejected_stale_share": 0, "block_qualified": 0}
    for envelope in map(json.loads, miner_path.open(encoding="utf-8")):
        if envelope.get("kind") != "event":
            continue
        event = envelope["event"]
        payload = event.get("payload", {})
        name = event.get("name")
        phase = retarget_phase(envelope["occurred_at_wall_ms"], boundaries)
        if name == "submitted":
            phases[phase]["submitted"] += 1
            counts["submitted"] += 1
        elif name == "ingress_ack":
            phases[phase]["ingress_ack"] += 1
            counts["ingress_ack"] += 1
        elif name == "ingress_rejected" and payload.get("error_code") == "stale-share":
            phases[phase]["ingress_rejected_stale_share"] += 1
            counts["ingress_rejected_stale_share"] += 1
        elif name == "block_qualified":
            phases[phase]["block_qualified"] += 1
            counts["block_qualified"] += 1
    return phases, counts


def worker_phase_rates(snapshot_dir):
    snapshots = {
        height: load_json(snapshot_dir / f"height-{height}-miner-snapshot.json")
        for height in [24, 48, 72]
    }
    rates = []
    for first, second, label in [(24, 48, "live_period_25_to_47"),
                                 (48, 72, "changed_period_49_to_71")]:
        workers = []
        for before, after in zip(snapshots[first]["workers"], snapshots[second]["workers"]):
            require(before["id"] == after["id"], "worker_identity", "worker identities changed between phase snapshots")
            attempts = int(after["attempted"]) - int(before["attempted"])
            elapsed_ms = int(after["elapsed_ms"]) - int(before["elapsed_ms"])
            achieved = attempts * 1000 // elapsed_ms if elapsed_ms else None
            workers.append({
                "worker_id": after["id"], "attempt_delta": attempts, "elapsed_ms_delta": elapsed_ms,
                "achieved_hps_delta": achieved, "attempts_total": int(after["attempted"]),
                "achieved_hps_reported": after["achieved_hps"],
                "requested_hps": after["requested"]["hashes_per_second"],
            })
        rates.append({"phase": label, "workers": workers})
    return rates


def read_pool_share_outcome_unknown(path):
    records = []
    for line in path.read_bytes().splitlines():
        if b"share_outcome_unknown" not in line:
            continue
        text = re.sub(r"\x1b\[[0-9;]*[A-Za-z]", "", line.decode(errors="replace"))
        marker = text.find("evidence=")
        if marker >= 0:
            evidence, _ = json.JSONDecoder().raw_decode(text[marker + len("evidence="):].lstrip())
            records.append(evidence)
    return records


def timestamp_millis(value):
    if isinstance(value, (int, float)):
        return value
    return dt.datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp() * 1000


CANONICAL_PHASES = (
    "coverage_1_to_24", "live_period_25_to_47", "changed_period_49_to_71",
    "other_outside_declared_periods",
)


def observed_tip_records(node_log_path):
    """Observed best-tip anchors, retaining alternatives for reorg analysis."""
    records = {}
    for line in node_log_path.read_text(errors="replace").splitlines():
        match = re.search(r"tip: best=([0-9a-f]{64}) height=(\d+)", line)
        if not match:
            continue
        block_hash, height_text = match.groups()
        height = int(height_text)
        row = {"height": height, "block_hash": block_hash,
               "source": "node.log tip: best"}
        records.setdefault(height, {})
        records[height][block_hash] = row
    return records


def canonical_header_records(retarget_evidence, node_log_path):
    records = {}
    for header in retarget_evidence["headers"]:
        raw = bytes.fromhex(header["header_hex"])
        require(len(raw) == 80 and oracle.display_hash(oracle.sha256d(raw)) == header["hash"],
                "canonical_header_vector", f"invalid retarget header at height {header['height']}")
        records.setdefault(header["height"], {})[header["hash"]] = {
            "height": header["height"], "block_hash": header["hash"],
            "header": raw, "source": "retarget-evidence.headers",
        }
    observed = observed_tip_records(node_log_path)
    for height, alternatives in observed.items():
        records.setdefault(height, {}).update(alternatives)
    return records


def canonical_join_problems(canonical_records):
    conflicts = []
    for height, alternatives in sorted(canonical_records.items()):
        if len(alternatives) > 1:
            conflicts.append({
                "height": height,
                "block_hashes": sorted(alternatives),
                "evidence": "different hashes observed as best tip at one height; possible reorg or conflicting observation, not assumed permanent",
            })
    return conflicts


def retarget_period_intervals(retarget_evidence):
    consensus = retarget_evidence["profile"]
    interval = int(consensus["difficulty_adjustment_interval"])
    require(interval > 0, "retarget_profile_interval", "profile adjustment interval must be positive")
    rows = []
    boundaries = sorted(retarget_evidence["boundaries"], key=lambda row: row["height"])
    for boundary in boundaries:
        boundary_height = int(boundary["height"])
        require(boundary_height % interval == 0, "retarget_boundary_profile", "recorded boundary is not on effective interval")
        rows.append({
            "next_boundary_height": boundary_height,
            "period_start_height": boundary_height - interval,
            "period_end_height_inclusive": boundary_height - 1,
            "retarget_interval_count": interval - 1,
            "count_kind": "height differences inside the named full period, not wall-clock phase block counts",
        })
    return rows


def submit_header_candidate(params):
    # rbitcoin's optional api log truncates long params with an ellipsis, so a
    # valid 160-hex header prefix must be accepted without full-block JSON.
    match = re.match(r"\[\s*\"([0-9a-fA-F]+)", params)
    if not match:
        return None
    raw_hex = match.group(1)
    if len(raw_hex) < 160:
        return None
    try:
        raw = bytes.fromhex(raw_hex[:160])
    except ValueError:
        return None
    return raw if len(raw) == 80 else None



def retarget_canonical_evidence(private, retarget_evidence):
    canonical_records = canonical_header_records(retarget_evidence, private / "node.log")
    by_hash = {}
    for height, alternatives in canonical_records.items():
        for block_hash, record in alternatives.items():
            require(block_hash not in by_hash or by_hash[block_hash]["height"] == height,
                    "canonical_hash_height", "one best-tip hash is reported at different heights")
            by_hash[block_hash] = record
    conflicts = canonical_join_problems(canonical_records)
    conflict_hashes = {block_hash for row in conflicts for block_hash in row["block_hashes"]}
    counts = {name: 0 for name in CANONICAL_PHASES}
    records = []
    seen_hashes = set()
    previous_height = None
    for row in retarget_evidence["headers"]:
        height = int(row["height"])
        if previous_height is not None:
            require(height == previous_height + 1, "header_chain_height", "recorded header vector has a height gap")
        previous_height = height
    total_api_records = 0
    phase_boundaries = retarget_boundaries_from_observations(private / "observations.jsonl")
    for line in (private / "node-api.jsonl").open(encoding="utf-8"):
        api = json.loads(line)
        if api.get("method") != "submitblock":
            continue
        total_api_records += 1
        phase = retarget_phase(timestamp_millis(api["ts"]), phase_boundaries)
        raw = submit_header_candidate(api.get("params", ""))
        base = {"ts": api["ts"], "phase_by_wall_time": phase}
        if raw is None:
            records.append({**base, "classification": "unknown_unparsed_header"})
            continue
        display = oracle.display_hash(oracle.sha256d(raw))
        base["block_hash"] = display
        base["raw_header_hex"] = raw.hex()
        if api.get("ok") is not True:
            records.append({**base, "classification": "rejected_submission_not_counted"})
            continue
        if display in seen_hashes:
            records.append({**base, "classification": "duplicate_submission_not_recounted"})
            continue
        seen_hashes.add(display)
        if display not in by_hash:
            records.append({**base, "classification": "unknown_missing_canonical_join"})
            continue
        if display in conflict_hashes:
            records.append({**base, "classification": "unknown_reorg_or_conflicting_tip"})
            continue
        tip = by_hash[display]
        if tip.get("header") is not None:
            require(tip["header"] == raw, "canonical_header_join", "canonical tip/header bytes differ")
        parent_display = raw[4:36][::-1].hex()
        parent = by_hash.get(parent_display)
        if int(tip["height"]) > 0 and parent is None:
            records.append({**base, "classification": "unknown_missing_parent_join"})
            continue
        if parent is not None:
            require(int(parent["height"]) + 1 == int(tip["height"]), "canonical_parent_join", "parent height does not precede tip")
        counts[phase] += 1
        records.append({**base, "classification": "observed_canonical_best_tip", "height": tip["height"]})
    unique_non_genesis = {height for height in canonical_records if height > 0}
    total_accepted = sum(counts.values())
    return {
        "count_semantics": "unique successfully submitted headers joined to observed node best-tips; rejected, duplicate, unknown, missing-parent, and reorg-conflicting records are not promoted",
        "phase_membership": "wall-clock timestamps against retarget_phase_boundary observations; distinct from height-based retarget periods",
        "canonical_blocks_by_phase": counts,
        "observed_non_genesis_tip_heights": len(unique_non_genesis),
        "total_accepted_canonical": total_accepted,
        "total_submitblock_records": total_api_records,
        "unique_non_genesis_total_matches": total_accepted == len(unique_non_genesis),
        "reorg_or_conflicting_tip_records": conflicts,
        "submission_records": records,
    }


def duty_cycle_current_canonical(private):
    evidence = load_json(private / "retarget-evidence.json")
    return retarget_canonical_evidence(private, evidence)



def verify_light_proof_artifact(proof_path, summary):
    raw = proof_path.read_bytes()
    require(len(raw) >= 93, "light_proof_shape", "truncated canonical light proof")
    expected_len = int.from_bytes(raw[:4], "little")
    require(expected_len == len(raw) - 4, "light_proof_shape", "outer light proof length mismatch")
    header = raw[4:84]
    require(oracle.display_hash(oracle.sha256d(header)) == summary["candidate"]["block_hash"],
            "light_header", "light proof hash differs from original public candidate")
    sibling_count = raw[84]
    offset = 85
    siblings = []
    for _ in range(sibling_count):
        siblings.append(raw[offset:offset + 32])
        offset += 32
    require(all(len(sibling) == 32 for sibling in siblings), "light_branch", "truncated light branch")
    coinbase_len = int.from_bytes(raw[offset:offset + 4], "little")
    offset += 4
    coinbase = raw[offset:]
    require(len(coinbase) == coinbase_len and offset + coinbase_len == len(raw),
            "light_coinbase", "coinbase length does not finish the light proof")
    root = oracle.sha256d(coinbase)
    for sibling in siblings:
        root = oracle.sha256d(root + sibling)
    require(root == header[36:68], "light_merkle", "coinbase/branch does not match header Merkle root")
    transaction, end = oracle.parse_transaction(coinbase)
    require(end == len(coinbase) and not transaction["has_witness"], "light_transaction", "coinbase parser mismatch")
    total = sum(output["value"] for output in transaction["outputs"])
    require(total == summary["candidate"]["subsidy"] + summary["candidate"]["fees"], "light_revenue", "coinbase total differs from original economics")
    conservation = summary["candidate"]["conservation"]
    require(transaction["outputs"][0]["value"] == conservation["pay_p"], "light_pay_p", "actual pool output differs from conservation")
    return {
        "header_sha256_verified": True,
        "coinbase_merkle_verified": True,
        "coinbase_total_sats": total,
        "pool_output_sats": transaction["outputs"][0]["value"],
        "subsidy_verified": True,
    }


def verify_payout_recovery_artifacts(private, summary):
    identities = load_json(private / "identities.json")
    frozen = load_json(private / "frozen-candidate.json")
    recovery = load_json(private / "scanner-recovery.json")
    expected = frozen["raw_by_identity"]
    actual = {}
    for identity in identities:
        record = next((item for item in recovery if item["identity"] == identity["id"]), None)
        require(record is not None, "payout_replay_identity", f"missing scanner recovery for {identity['id']}")
        amount = sum(int(found["amount"]) for found in record["found"])
        require(amount == expected[identity["sp_id"]], "payout_replay_amount", f"raw payout mismatch for {identity['id']}")
        actual[identity["id"]] = amount
    require(len(recovery) == len(expected), "payout_replay_shape", "recovered identity count differs from raw frozen map")
    return {
        "frozen_raw_map_sha256": sha256_bytes(json.dumps(expected, sort_keys=True, separators=(",", ":")).encode()),
        "recovered_amounts": actual,
        "policy": summary["tested_payout_policy"],
        "verified_against_historical_scanner_results": True,
    }



def retarget_duty_cycle(private, retarget_evidence):
    phases, totals = mine_phase_duty_counts(
        private / "miner.jsonl", private / "observations.jsonl",
    )
    canonical = retarget_canonical_evidence(private, retarget_evidence)
    periods = retarget_period_intervals(retarget_evidence)
    unknown = read_pool_share_outcome_unknown(private / "pool.log")
    return {
        "phases": phases,
        "independent_run_totals_from_raw_miner_jsonl": totals,
        "canonical_blocks_by_phase": canonical["canonical_blocks_by_phase"],
        "retarget_period_intervals": periods,
        "canonical_evidence": canonical,
        "achieved_work_rates_from_consecutive_snapshots": worker_phase_rates(private),
        "pool_share_outcome_unknown": {
            "count": len(unknown),
            "events": unknown,
            "classification": "pool evidence state is unknown; neither accepted nor definitively invalid",
        },
        "causal_attribution": "not established by this evidence block",
    }


def derive_historical_retarget(input_run, output_dir):
    input_run = input_run.resolve()
    private = input_run / "private"
    evidence_path = private / "retarget-evidence.json"
    summary_path = input_run / "public" / "summary.json"
    require(evidence_path.is_file() and summary_path.is_file(),
            "retarget_replay_input", f"{input_run} lacks frozen retarget evidence/summary")
    evidence = load_json(evidence_path)
    summary = load_json(summary_path)
    input_paths = [
        evidence_path, summary_path, private / "miner.jsonl", private / "pool.log",
        private / "observations.jsonl", private / "scenario.redacted.json",
        private / "canonical-light-proof.bin", private / "identities.json",
        private / "scanner-recovery.json", private / "frozen-candidate.json",
    ] + [private / f"height-{height}-miner-snapshot.json" for height in [24, 48, 72]]
    for path in input_paths:
        require(path.is_file(), "retarget_replay_input", f"missing input artifact {path}")
    input_manifest = [
        {"path": str(path), "sha256": sha256_file(path)} for path in input_paths
    ]
    require(not output_dir.exists() or not any(output_dir.iterdir()),
            "derived_output_collision", f"derived output must be new or empty: {output_dir}")
    output_dir.mkdir(parents=True, mode=0o700)
    duty_cycle = retarget_duty_cycle(private, evidence)
    phases = duty_cycle["phases"]
    totals = duty_cycle["independent_run_totals_from_raw_miner_jsonl"]
    unknown = duty_cycle["pool_share_outcome_unknown"]["events"]
    headers = evidence["headers"]
    profile = evidence["profile"]
    expected_boundary = {}
    for boundary in evidence["boundaries"]:
        height = boundary["height"]
        expected = oracle.expected_retarget_bits(headers[: height + 1], height, profile)
        require(expected == int(boundary["actual_bits"], 16),
                "retarget_replay_mismatch", f"revised oracle differs at height {height}")
        expected_boundary[str(height)] = f"{expected:08x}"
    require(oracle.validate_retarget_vector(headers, profile) == [],
            "retarget_replay_mismatch", "revised full header vector no longer matches")
    light = verify_light_proof_artifact(private / "canonical-light-proof.bin", summary)
    payout_replay = verify_payout_recovery_artifacts(private, summary)
    provenance = {
        "derived_from_historical_artifacts": True,
        "generated_by_script_sha256": sha256_file(Path(__file__).resolve()),
        "input_run": str(input_run),
        "input_manifest": input_manifest,
        "original_public_summary_sha256": {item["path"]: item["sha256"] for item in input_manifest}[str(summary_path)],
        "no_original_artifact_modified": True,
    }
    duties = {
        "artifact_type": "retarget_duty_cycle_derived_v1", **provenance, **duty_cycle,
        "phase_counting_rule": "actual miner event occurred_at_wall_ms against explicit retarget_phase_boundary timestamps; other post-boundary work is retained as other_outside_declared_periods",
        "phases": phases,
        "independent_run_totals_from_raw_miner_jsonl": totals,

        "achieved_work_rates_from_consecutive_snapshots": duty_cycle["achieved_work_rates_from_consecutive_snapshots"],
        "pool_share_outcome_unknown": duty_cycle["pool_share_outcome_unknown"],
        "causal_attribution": "not established by this evidence block",
    }
    replay = {
        "artifact_type": "retarget_oracle_replay_v1", **provenance,
        "full_header_mismatches": [],
        "boundary_oracle": expected_boundary,
        "light_proof": light,
        "payout_replay": payout_replay,
        "fees_in_selected_blocks": 0,
        "all_settled_blocks_fee_note": "all frozen chain revenue is subsidy; optional fee-term coverage is synthetic-unit-only",
        "retarget_profile_claim": "effective-profile boundary arithmetic only; no causal rate/budget attribution",
    }
    atomic_write(output_dir / "retarget-duty-cycle-derived.json", duties)
    atomic_write(output_dir / "retarget-oracle-replay-derived.json", replay)
    for row in input_manifest:
        require(sha256_file(Path(row["path"])) == row["sha256"], "historical_artifact_modified", f"artifact changed during replay: {row['path']}")
    return {
        "status": "derived_ok",
        "output_dir": str(output_dir),
        "artifacts": [str(output_dir / "retarget-duty-cycle-derived.json"), str(output_dir / "retarget-oracle-replay-derived.json")],
        "independent_run_totals_from_raw_miner_jsonl": totals,
        "pool_share_outcome_unknown": len(unknown),

        "boundary_oracle": expected_boundary,
        "proof_source": "historical frozen artifacts; not a new live run or an overwrite of historical summaries",
        "payout_replay": payout_replay,
    }


def miner_job_number(value):
    """The approved miner's integer JSON job id, never an overloaded guess."""
    try:
        number = int(value)
    except (TypeError, ValueError):
        raise ValueError("miner job id is not an integer") from None
    if not 0 <= number <= 0xFFFFFFFF:
        raise ValueError("miner job id is outside uint32")
    return number


def pool_job_number(value):
    """LP-1's eight-lowercase-hex job id, including all-digit identifiers."""
    if isinstance(value, int):
        number = value
    else:
        text = str(value)
        if not re.fullmatch(r"[0-9a-f]{8}", text):
            raise ValueError("pool job id is not eight lowercase hex characters")
        number = int(text, 16)
    if not 0 <= number <= 0xFFFFFFFF:
        raise ValueError("pool job id is outside uint32")
    return number


def serialized_hex(value):
    if isinstance(value, list):
        return bytes(value).hex()
    return value.removeprefix("\\x").lower()


def parse_pool_job_id(job_id, reconstructed):
    # Pool-template SV2 jobs are numeric; the DB leaf adds the actual template height.
    numeric = pool_job_number(job_id)
    height = int(reconstructed.get("height", 0))
    if height == 0:
        # Prefer the canonical height from the job's previous-hash context only
        # when the caller supplies it; the wire job id itself is never guessed.
        job = reconstructed["job"]
        height = int(job.get("height", 0))
    return (height << 32) | numeric


def share_leaf_key(normalized):
    return (
        int(normalized["nonce"]), int(normalized["ntime"]), int(normalized["version"]),
        normalized["extranonce"], int(normalized["job_id"]), int(normalized["share_index"]),
    )


def match_slice_leaves(row, leaves, accounts_by_hash):
    # Match every leaf without using pool accounting as an oracle: the log only
    # supplies the hash, while m is independently recomputed in check_nonblock.
    decoded = json.loads(bytes.fromhex(row["shares_hex"]))
    matched = []
    for leaf in decoded:
        normalized = dict(leaf)
        normalized["extranonce"] = serialized_hex(normalized["extranonce"])
        key = share_leaf_key(normalized)
        found = None
        for account in accounts_by_hash.values():
            candidate_key = (
                int(account["nonce"]), int(account["ntime"]), int(account["version"]),
                account["extranonce"], int(normalized["job_id"]), int(account["share_index"]),
            )
            if candidate_key == key:
                found = account
                break
        require(found is not None, "slice_evidence_unknown", "sealed leaf has no accounted/job evidence", True)
        matched.append(found)
    return matched



def tcp_probe(port: int):
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=0.5):
            return True
    except OSError:
        return False


def http_json(port: int, path: str):
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=3)
    try:
        connection.request("GET", path)
        response = connection.getresponse()
        raw = response.read(64 * 1024 * 1024)
    finally:
        connection.close()
    if response.status != 200:
        raise LabError("api_readiness", f"GET {path} returned HTTP {response.status}", True)
    return json.loads(raw)


GENESIS_HEADER = (
    "01000000" + "00" * 32
    + "3ba3edfd7a7b12b27ac72c3e67768f617fc81bc3888a51323a9fb8aa4b1e5e4a"
    + "29ab5f49" + "ffff001d" + "1dac2b7c"
)


class SelfTests(unittest.TestCase):
    def test_genesis_sha256d_and_target(self):
        raw = bytes.fromhex(GENESIS_HEADER)
        digest = oracle.sha256d(raw)
        self.assertEqual(oracle.display_hash(digest), "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f")
        target = oracle.decode_compact_target(0x1D00FFFF)
        self.assertEqual(int.from_bytes(digest, "little") <= target, True)
        self.assertEqual(oracle.target_to_le_hex(target), oracle.target_to_le_hex(oracle.target_from_le_hex(oracle.target_to_le_hex(target))))

    def test_v2_vectors_and_stepdown(self):
        reference = (1 << 255) - 1
        self.assertEqual(self.fingerprint(reference), "7fe9ed1a4e2d4279b302cbb0bcf709758f03dc44db6dab82ac40c04eb0d77c6b")
        initial = oracle.decode_compact_target(0x200FFFFF)
        pow_limit = oracle.decode_compact_target(0x207FFFFF)
        self.assertEqual(oracle.window_target(reference, 8, initial), 65)
        self.assertEqual(oracle.window_target(reference, 8, pow_limit), 9)
        self.assertEqual(oracle.issuance_multiplier(reference, 2, initial), 2)
        self.assertEqual(oracle.issuance_multiplier(reference, 2, pow_limit), 1)
        self.assertEqual(oracle.issuance_multiplier(reference, 8, (1 << 252) - 1), 8)
        self.assertEqual(oracle.issuance_multiplier(reference, 8, 1 << 252), 4)
        self.assertEqual((8 * (reference + 1) - 1) // (initial + 1) + 1, 65)
        with self.assertRaises(ValueError):
            oracle.reference_exponent(1 << 254)
        with self.assertRaises(ValueError):
            oracle.issuance_multiplier(reference, 3, initial)

    @staticmethod
    def fingerprint(reference):
        return oracle.work_unit_fingerprint(reference).hex()

    def test_compact_consensus_boundaries(self):
        self.assertGreater(oracle.decode_compact_target(0x2100FFFF), 0)
        self.assertGreater(oracle.decode_compact_target(0x220000FF), 0)
        for bits in [0x21010000, 0x22000100, 0x00123456, 0x20800000]:
            with self.assertRaises(ValueError):
                oracle.decode_compact_target(bits)

    def test_base58_key_utils_vector(self):
        secret = bytes.fromhex("65995eb19631f478a46ffa5cf1e545091efe950eaeac7482ffdc06eb6a89f697")
        public = bytes.fromhex("24ee3c3804a1aaa4c03b80ea19f7a5863c916e8994b7db94a3bad7ee092b6ce7")
        # The generator above receives a valid compressed point from spc-scan;
        # the expected x-only payload here is checked against the pinned pair.
        compressed = bytes.fromhex("02" + public.hex())
        pub, sec = oracle.noise_authority_from_secp_material(compressed, secret)
        self.assertEqual(pub, "9auqWEzQDVyd2oe1JVGFLMLHZtCo2FFqZwtKA5gd9xbuEu7PH72")
        self.assertEqual(sec, "mkDLTBBRxdBv998612qipDYoTK3YUrqLe8uWw7gu3iXbSrn2n")


    def test_payout_arithmetic_vector(self):
        a = "11" * 32
        b = "22" * 32
        rows = [
            {"slice_id": "2", "difficulty": "2", "diff_fee_sum": "20",
             "credits": [{"sp_id": b, "diff_sum": "2", "diff_fee_sum": "20"}]},
            {"slice_id": "1", "difficulty": "1", "diff_fee_sum": "10",
             "credits": [{"sp_id": a, "diff_sum": "1", "diff_fee_sum": "10"}]},
        ]
        selected = oracle.select_window(rows, 3)
        self.assertEqual(selected["actual_sum"], 3)
        result = oracle.expected_raw_payouts(selected["rows"], 1000, 100, 100)
        self.assertEqual(result["raw"], {a: 379, b: 709})
        self.assertEqual(result["pool_sats"], 12)
        changed = json.loads(json.dumps(rows))
        changed[0]["difficulty"] = "3"
        with self.assertRaises(ValueError):
            oracle.select_window(changed, 3)

    def test_wire_parser_and_light_proof(self):
        def tx(script, value):
            return (
                (2).to_bytes(4, "little") + b"\x01" + b"\x00" * 32 + b"\xff" * 4
                + bytes([len(script)]) + script + b"\x00" * 4 + b"\x01"
                + value.to_bytes(8, "little") + b"\x02\x51\x20" + b"\x00" * 4
            )
        # Replace the intentionally short output with two properly serialized
        # transactions. This keeps the vector readable while checking bounds.
        def proper(extra):
            return (
                (2).to_bytes(4, "little") + b"\x01" + b"\x00" * 32 + b"\xff" * 4
                + bytes([len(extra)]) + extra + b"\x01\x02\x03\x04" + b"\x01"
                + (500).to_bytes(8, "little") + b"\x02\x51\x20" + b"\xff" * 4
            )
        cb = proper(b"\x03\x01\x02\x03")
        other = proper(b"\x03\x04\x05\x06")
        raw = b"\x11" * 80 + b"\x02" + cb + other
        parsed = oracle.parse_block(raw)
        self.assertEqual(len(parsed["transactions"]), 2)
        self.assertEqual(oracle.merkle_root([item["txid"] for item in parsed["transactions"]]),
                         oracle.sha256d(parsed["transactions"][0]["txid"] + parsed["transactions"][1]["txid"]))
        proof = oracle.light_proof(parsed["header"], oracle.merkle_branch_zero([x["txid"] for x in parsed["transactions"]]), cb)
        self.assertEqual(int.from_bytes(proof[:4], "little"), len(proof) - 4)
        self.assertEqual(proof[-len(cb):], cb)

    def test_duplicate_active_jobs_are_disambiguated_only_by_header(self):
        def job(suffix):
            target = oracle.decode_compact_target(0x200FFFFF)
            return {
                "job_id": "7", "target_generation": 0, "version": 2,
                "prev_hash_le": "11" * 32, "min_ntime": 100, "nbits": 0x200FFFFF,
                "share_target_le": oracle.target_to_le_hex(target >> 2),
                "network_target_le": oracle.target_to_le_hex(target),
                "merkle_path_le": [], "coinbase_tx_prefix": "aa", "coinbase_tx_suffix": suffix,
                "extranonce_prefix": "dd",
            }
        right = job("bb")
        wrong = job("cc")
        coinbase = b"".join([bytes.fromhex("aa"), bytes.fromhex("dd"), bytes.fromhex("ee"), bytes.fromhex("bb")])
        header = b"".join([
            (2).to_bytes(4, "little"), bytes.fromhex("11") * 32, oracle.sha256d(coinbase),
            (100).to_bytes(4, "little"), (0x200FFFFF).to_bytes(4, "little"),
            (0).to_bytes(4, "little"),
        ])
        digest = oracle.sha256d(header)
        share = {
            "connection_id": "alice-connection", "worker_id": "alice-worker", "session_slot": 0,
            "sequence_number": 0, "job_id": "7", "target_generation": 0,
            "nonce": 0, "ntime": 100, "version": 2, "extranonce": "ee",
            "header": header.hex(), "block_hash": oracle.display_hash(digest), "hash_le": digest.hex(),
            "share_target_le": right["share_target_le"], "network_target_le": right["network_target_le"],
            "block_qualified": False, "status": "submitted", "accounted": "unknown",
            "candidate_submission": "not_applicable",
        }
        jobs = {("alice-connection", 7, "0"): [wrong, right, right]}
        reconstructed = LiveRun.reconstruct_submission(self, share, jobs)
        self.assertEqual(reconstructed["coinbase"], coinbase)
        self.assertEqual(reconstructed["job"], right)
        share["header"] = "00" * 80
        with self.assertRaises(LabError):
            LiveRun.reconstruct_submission(self, share, jobs)

    def test_retarget_oracle_detects_boundary_and_nonboundary_drift(self):
        profile = {
            "pow_limit_bits": "207fffff", "retarget_floor_bits": "1e7fffff",
            "pow_target_timespan": 240, "difficulty_adjustment_interval": 24,
            "regtest_pow_bits_overlay": "1e7fffff", "no_pow_retargeting": False,
            "allow_min_difficulty_blocks": False,
        }
        headers = [{"bits": "207fffff", "time": 1296688602}]
        for height in range(1, 25):
            headers.append({"bits": "1e7fffff", "time": 1793940000 + height * 10})
        self.assertEqual(oracle.expected_retarget_bits(headers, 24, profile), 0x1E7FFFFF)
        for height in range(25, 49):
            headers.append({"bits": "1e7fffff", "time": 1793940000 + 240 + (height - 24) * 1000})
        self.assertEqual(oracle.expected_retarget_bits(headers, 48, profile), 0x1E7FFFFF)
        self.assertEqual(oracle.validate_retarget_vector(headers, profile), [])
        headers[48]["bits"] = "1e1fffff"
        self.assertEqual(oracle.validate_retarget_vector(headers, profile), [48])

    def test_approval_record_binding_and_revocation(self):
        a = "aa" * 32
        b = "bb" * 32
        two_pass = f"""# Initial review
## Result
**REQUEST_CHANGES**, tied to snapshot `{a}`
# Next pass
## Result
**APPROVE**, tied to snapshot `{b}`
"""
        self.assertFalse(approval_allows_snapshot(two_pass, a))
        self.assertTrue(approval_allows_snapshot(two_pass, b))
        revoked = two_pass + f"\n# Subsequent revocation\n## Result\n**REQUEST_CHANGES**, tied to snapshot `{b}`\n"
        self.assertFalse(approval_allows_snapshot(revoked, b))
        unrelated = two_pass + f"\n# Commentary, not a verdict\nThe unrelated citation is `{a}`\n"
        self.assertTrue(approval_allows_snapshot(unrelated, b))
        collision = b[:8] + "cc" * 28
        self.assertFalse(approval_allows_snapshot(two_pass, collision))
        self.assertFalse(approval_allows_snapshot("# Review\n## Result\n**APPROVE**, no full snapshot\n", b))
        self.assertFalse(approval_allows_snapshot("# Review\n## Result\n**APPROVE**, tied to snapshot `" + b + "` and `" + a + "`\n", b))

    def test_actual_approval_and_all_previous_rejections(self):
        rows = [
            ("lab-lp1-fable-review.md", APPROVED_LP1_SNAPSHOT,
             ["81fa17d27ee80641167aaeb53e97f627ba8daa4688e56a227a1ef44c9776b329",
              "69ac72b6ed7ab514f1870cfc55832d8c8d202576a9c01bc27aaeb72d2a20da6d"]),
            ("lab-lp2-fable-review.md", APPROVED_LP2_SNAPSHOT,
             ["af5313704b81ee20868d634dc19127c97687a8d89b541e00c9d8d105a120e635",
              "bcb9b34280fb8d21b0f5d10d07af1a18a4e4de8cc8e148bc6c519d21d24849bb",
              "6e2eb2aa6537187bc78fb9ba822f0a564f895c4a3e8be26d85fc12ed3b146676"]),
            ("lab-l2-fable-review.md", APPROVED_L2_SNAPSHOT,
             ["646656a84cbb5001f96559d98511499eaba35501d93038dc9560cd787bc764ff"]),
        ]
        for filename, approved, rejected in rows:
            path = SCRIPT_DIR.parent / "docs" / "reviews" / filename
            if not path.exists():
                continue
            text = path.read_text()
            self.assertTrue(approval_allows_snapshot(text, approved), filename)
            for snapshot in rejected:
                self.assertFalse(approval_allows_snapshot(text, snapshot), f"{filename}: {snapshot}")

    def test_compact_left_shift_and_effective_clamps(self):
        self.assertEqual(oracle.encode_compact_target(0x1234), 0x02123400)
        self.assertEqual(oracle.encode_compact_target(1), 0x01010000)
        profile = {
            "pow_limit_bits": "207fffff", "retarget_floor_bits": "1e7fffff",
            "pow_target_timespan": 400, "difficulty_adjustment_interval": 8,
            "regtest_pow_bits_overlay": "1e7fffff", "no_pow_retargeting": False,
            "allow_min_difficulty_blocks": False,
        }
        headers = [{"bits": "207fffff", "time": 1}, {"bits": "1e7fffff", "time": 2}]
        for height in range(2, 9):
            headers.append({"bits": "1e7fffff", "time": 2 + (height - 1) * 1000})
        expected = oracle.expected_retarget_bits(headers, 8, profile)
        self.assertEqual(expected, 0x1E7FFFFF)
        with self.assertRaises(ValueError):
            oracle.require_timespan(0)
        oracle.require_timespan(400)
        self.assertTrue(oracle.validate_retarget_vector(headers[:8], profile) == [])
        self.assertEqual(miner_job_number("12345678"), 12345678)
        self.assertEqual(pool_job_number("12345678"), 0x12345678)
        self.assertNotEqual(miner_job_number("12345678"), pool_job_number("12345678"))

    def test_canonical_evidence_join_and_nonuniform_intervals(self):
        # Historical local archive (the 2026-11-08 retarget campaign run);
        # not a fresh-setup precondition - the test skips when absent.
        source = Path(os.environ.get(
            "SPC_LAB_RETARGET_EVIDENCE_ROOT",
            "/tmp/opencode/LP-live-retarget-run6/private"))
        if not (source / "retarget-evidence.json").exists():
            self.skipTest("frozen retarget artifact not present")
        evidence = load_json(source / "retarget-evidence.json")
        original_api = [json.loads(line) for line in (source / "node-api.jsonl").read_text().splitlines()
                        if json.loads(line).get("method") == "submitblock"]
        duplicate = dict(original_api[0])
        rejected = dict(original_api[0])
        rejected["ok"] = False
        unknown = dict(original_api[0])
        unknown["params"] = '["' + "00" * 80 + '"]'
        fixture_rows = original_api + [duplicate, rejected, unknown]
        with tempfile.TemporaryDirectory() as temporary:
            private = Path(temporary) / "private"
            private.mkdir()
            for name in ["observations.jsonl", "node.log"]:
                shutil.copyfile(source / name, private / name)
            with (private / "node-api.jsonl").open("w") as output:
                for row in fixture_rows:
                    output.write(json.dumps(row) + "\n")
            result = retarget_canonical_evidence(private, evidence)
            self.assertEqual(result["total_accepted_canonical"], 73)
            self.assertEqual(result["canonical_blocks_by_phase"], {
                "coverage_1_to_24": 24, "live_period_25_to_47": 24,
                "changed_period_49_to_71": 24, "other_outside_declared_periods": 1,
            })
            classifications = [row["classification"] for row in result["submission_records"]]
            self.assertIn("duplicate_submission_not_recounted", classifications)
            self.assertIn("rejected_submission_not_counted", classifications)
            self.assertIn("unknown_missing_canonical_join", classifications)
            self.assertEqual(classifications.count("observed_canonical_best_tip"), 73)
            self.assertEqual(result["total_submitblock_records"], 76)
            self.assertTrue(result["unique_non_genesis_total_matches"])
            tail = next(row for row in result["submission_records"]
                        if row.get("phase_by_wall_time") == "other_outside_declared_periods"
                        and row["classification"] == "observed_canonical_best_tip")
            self.assertEqual(tail["height"], 73)
            modified = dict(evidence)
            modified["profile"] = dict(evidence["profile"], difficulty_adjustment_interval=8)
            modified["boundaries"] = [{"height": 8}]
            periods = retarget_period_intervals(modified)
            self.assertEqual(periods, [{
                "next_boundary_height": 8, "period_start_height": 0,
                "period_end_height_inclusive": 7, "retarget_interval_count": 7,
                "count_kind": "height differences inside the named full period, not wall-clock phase block counts",
            }])

    def test_approval_prefix_does_not_match_unrelated_full_hash(self):
        prefix = "1234abcd"
        unrelated = "f" * 64
        text = f"# Review\n## Result\n**APPROVE**, tied to snapshot `{prefix}`\n{unrelated}   (matches)\n"
        self.assertFalse(approval_allows_snapshot(text, unrelated))
        matching = "1234abcd" + "0" * 56
        text = f"# Review\n## Result\n**APPROVE**, tied to snapshot `{prefix}`\n{matching}   (matches)\n"
        self.assertTrue(approval_allows_snapshot(text, matching))

    def test_scenario_strictness_and_permissions(self):
        baseline = Scenario.load(SCRIPT_DIR / "lab-live" / "baseline.json")
        self.assertEqual(baseline.raw["confirmation_depth"], 6)
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "scenario.json"
            atomic_write(path, baseline.raw)
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
            bad = dict(baseline.raw)
            bad["unexpected"] = True
            atomic_write(path, bad)
            with self.assertRaises(LabError):
                Scenario.load(path)
            bad = dict(baseline.raw)
            bad["network"] = "mainnet"
            atomic_write(path, bad)
            with self.assertRaises(LabError):
                Scenario.load(path)
            bad = dict(baseline.raw)
            bad["retarget_claim"] = True
            atomic_write(path, bad)
            with self.assertRaises(LabError):
                Scenario.load(path)

    def test_ports_and_manifest(self):
        allocator = PortAllocator()
        port = allocator.reserve("first", 0)
        with self.assertRaises(LabError):
            allocator.reserve("second", port)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            atomic_write(root / "member", b"abc")
            manifest = {"base": "00" * 20, "files": [{"path": "member", "sha256": sha256_file(root / "member")}]}
            manifest["snapshot_sha256"] = manifest_snapshot(manifest)
            atomic_write(root / "manifest.json", manifest)
            record = verify_manifest(root, root / "manifest.json")
            self.assertTrue(record["verified"])
            atomic_write(root / "member", b"changed")
            with self.assertRaises(LabError):
                verify_manifest(root, root / "manifest.json")

    def test_owned_graceful_forced_and_foreign_cleanup(self):
        code = "import signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); print('ready',flush=True); time.sleep(60)"
        with tempfile.TemporaryDirectory() as temporary:
            manager = ProcessManager(Path(temporary), uuid.uuid4().hex)
            try:
                log = Path(temporary) / "child.log"
                owned = manager.start("child", [sys.executable, "-c", code], log)
                wait_until("negative-cleanup child ready", 3, lambda: "ready" in log.read_text())
                with self.assertRaises(LabError):
                    original = owned.token
                    owned.token = "not-the-start-token"
                    manager.stop(owned, 0.1)
                owned.token = original
                self.assertIsNone(owned.popen.poll())
                manager.stop(owned, 0.1)
                self.assertIsNotNone(owned.popen.poll())
                self.assertEqual(owned.popen.returncode, -signal.SIGKILL)
            finally:
                manager.stop_all(0.2)
            self.assertEqual(len(manager.processes), 1)
            self.assertIsNotNone(manager.processes[0].popen.poll())

    def test_redaction_and_pool_log_parse(self):
        run = object.__new__(LiveRun)
        run.secrets = ["very-secret"]
        redacted = run.redacted_error(LabError("x", "do not print very-secret"))
        self.assertNotIn("very-secret", json.dumps(redacted))
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "pool.log"
            atomic_write(path, '2026 INFO spc_evidence: event=accounted evidence={"share_hash":"aa","generation":"2"}\n')
            tail = PoolEvidenceTail(path)
            wait_until("pool evidence parser", 2, lambda: tail.values()[0])
            tail.stop_event.set()
            tail.thread.join(timeout=2)
            events, errors = tail.values()
            self.assertEqual(events[0]["kind"], "accounted")
            self.assertFalse(errors)


def make_parser():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--self-test", action="store_true", help="run dependency-free oracle and graceful/forced/foreign cleanup tests")
    parser.add_argument("--check-scenario", type=Path, help="strictly validate a scenario and exit")
    parser.add_argument("--derive-retarget-from", type=Path, help="historical retarget run directory; derive new, read-only analysis artifacts")
    parser.add_argument("--derived-output-dir", type=Path, help="new/empty directory for derived historical-artifact output")
    parser.add_argument("--scenario", type=Path, default=SCRIPT_DIR / "lab-live" / "baseline.json")
    parser.add_argument("--validate-only", action="store_true", help="run binary/source/CLI/port/scenario preflight without starting children")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--acceptance", action="store_true", help="require verified Fable-approved frozen LP-1 and LP-2 inputs")
    mode.add_argument("--provisional", action="store_true", help="label the run provisional even if all live assertions pass (default)")
    for name in ["pool", "miner", "scanner", "node"]:
        parser.add_argument(f"--{name}-binary", help=f"absolute path to the exact {name} executable")
        parser.add_argument(f"--{name}-source-dir", help=f"source root used for {name} provenance")
        if name != "node":
            parser.add_argument(f"--{name}-source-manifest", help=f"canonical base+files manifest for {name}")
    parser.add_argument("--node-source-manifest", help="canonical base+files manifest for the rbitcoin node")
    parser.add_argument("--node-approval-record", help="Fable review record naming the L2 manifest snapshot")
    parser.add_argument("--pool-approval-record", help="Fable review record naming the LP-1 manifest snapshot")
    parser.add_argument("--miner-approval-record", help="Fable review record naming the approved LP-2 snapshot")
    parser.add_argument("--run-dir", type=Path, help="new/empty artifact directory; private files are 0600 and nothing is auto-deleted")
    parser.add_argument("--postgres-bin-dir", default=str(Path(shutil.which("initdb") or "/usr/bin/initdb").parent))
    parser.add_argument("--redis-server", default=shutil.which("redis-server") or "/usr/bin/redis-server")
    parser.add_argument("--redis-cli", default=shutil.which("redis-cli") or "/usr/bin/redis-cli")
    for name, default in [("rpc", 0), ("sv2", 0), ("jd", 0), ("api", 0), ("redis", 0), ("postgres", 0)]:
        parser.add_argument(f"--{name}-port", type=int, default=default, help=f"explicit {name} TCP port, or 0 to reserve and record an ephemeral port")
    return parser


def summary_for(run: LiveRun, status: str, error=None):
    complete = status == "passed"
    source_status = "acceptance" if complete and run.args.mode == "acceptance" else "provisional"
    summary = {
        "summary_version": SUMMARY_VERSION, "run_id": run.run_id, "status": status,
        "source_gate": source_status, "scenario": run.scenario.raw["id"],
        "observed_at": utc_now(), "base_revision": "b0ab4ca34fda07c61a6335d05f598eaacc69678c",
        "binaries": run.binary_records, "source_provenance": run.provenance, "ports": run.ports,
        "network": "regtest", "profile": "self_adjusting_10s_24block_real_pow_v2" if run.scenario.raw["retarget_claim"] else "baseline_real_rbitcoin_pow_v2",
        "pow_bits": run.scenario.raw["pow_bits"], "retarget_claim": run.scenario.raw["retarget_claim"],
        "tested_payout_policy": run.scenario.raw["payout_policy"],
        "ladder_claim": False, "witness_claim": False, "ark_claim": False,
        "type32_v2_claim": False, "multi_pool_claim": False,
        "assertions": {
            "real_native_sv2_work": complete,
            "nonblock_share_accounted_at_job_m": complete,
            "two_identities_contributed": complete,
            "candidate_matches_node_hash_and_actual_coinbase": complete,
            "settled_through_configured_confirmations": complete,
            "db_v2_rule1_fingerprint_frozen_q": complete,
            "exact_actual_block_conservation": complete,
            "scanner_recovered_expected_output": complete,
            "source_inputs_unchanged": complete,
        },
        "artifacts": {
            "private_dir": str(run.private), "observations": str(run.private / "observations.jsonl"),
            "processes": str(run.private / "processes.json"),
            "note": "identity keys, credentials, headers, DB rows, scanner tweaks and full logs are private fixture material; no private values are copied here",
        },
    }
    if run.args.validate_only and status == "validated":
        summary["assertions"] = {"preflight_only": True}
    if getattr(run, "retarget_evidence", None):
        summary["retarget_evidence"] = run.retarget_evidence
        if "duty_cycle" in run.retarget_evidence:
            summary["retarget_duty_cycle"] = run.retarget_evidence["duty_cycle"]
        summary["assertions"]["retarget_headers_match_independent_oracle"] = complete
        summary["assertions"]["budget_commands_applied"] = complete
    if run.selected:
        public = run.public_candidate_record(private=False)
        public.pop("raw_by_identity", None)
        summary["candidate"] = public
        summary["candidate"]["scanner_output_count"] = sum(len(item["found"]) for item in run.selected.get("scanner", []))
        summary["candidate"]["conservation"] = run.selected.get("conservation")
    if run.nonblock:
        summary["nonblock"] = run.nonblock
    if error is not None:
        summary["error"] = run.redacted_error(error)
        summary["candidate_wait_reasons"] = list(run.rejected_candidates.values())[-10:]
    if run.manager.cleanup_errors:
        summary["cleanup_errors"] = run.manager.cleanup_errors
    return summary


def main(argv=None):
    parser = make_parser()
    args = parser.parse_args(argv)
    if args.derive_retarget_from:
        require(args.derived_output_dir is not None, "derived_output_required", "--derived-output-dir is required with --derive-retarget-from")
        print(json.dumps(derive_historical_retarget(args.derive_retarget_from, args.derived_output_dir.resolve()), separators=(",", ":"), sort_keys=True))
        return 0
    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(SelfTests)
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        return 0 if result.wasSuccessful() else 1
    if args.check_scenario:
        scenario = Scenario.load(args.check_scenario)
        print(json.dumps({"status": "validated", "scenario": scenario.raw["id"]}, sort_keys=True))
        return 0
    args.mode = "acceptance" if args.acceptance else "provisional"
    for name in ["pool_binary", "miner_binary", "scanner_binary", "node_binary", "run_dir"]:
        if getattr(args, name) is None:
            parser.error(f"--{name.replace('_', '-')} is required for a live run")
    scenario = Scenario.load(args.scenario)
    run = LiveRun(args, scenario)
    status, error = "passed", None
    try:
        run.execute()
        if args.validate_only:
            status = "validated"
    except Exception as caught:
        error = caught
        status = "unknown" if isinstance(caught, LabError) and caught.inconclusive else "failed"
    finally:
        try:
            run.cleanup()
        except Exception as cleanup_error:
            run.manager.cleanup_errors.append(str(cleanup_error))
            if error is None:
                error = cleanup_error
                status = "failed"
    summary = summary_for(run, status, error)
    atomic_write(run.public / "summary.json", summary)
    print(json.dumps(summary, separators=(",", ":"), sort_keys=True))
    return 0 if status in ("passed", "validated") else 1


if __name__ == "__main__":
    sys.exit(main())
