"""Timed, complete, observation-only RPC capture for owned lab nodes.

Credentials never enter records. Exact JSON request/response bodies are
content-addressed private blobs, including malformed/error responses. A durable
begin without an end remains an unresolved observation, never a successful RPC.
"""
import base64
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import socket
import stat
import struct
import threading
import time
import urllib.error
import urllib.parse
import urllib.request

from oracle import Inconclusive, Invalid, integer, require, strict_json
from storage import ProcFS, option, private_bytes, sha_file

MAX_RESPONSE = 16 * 1024 * 1024
READ_METHODS = {"getbestblockhash", "getblockhash", "getblockheader", "getblock", "getblockchaininfo",
                "getrawtransaction", "getrawmempool", "getmempoolinfo", "getblocktemplate"}
TEST_METHODS = {"invalidateblock", "reconsiderblock", "sendrawtransaction"}


def stamp():
    return {"monotonic_ns": time.monotonic_ns(), "wall_ns": time.time_ns()}


def sync_dir(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


class RpcRecorder:
    def __init__(self, private_root, capture):
        self.root = Path(private_root)
        self.blobs = self.root / "rpc-bodies"
        self.blobs.mkdir(mode=0o700, exist_ok=False)
        sync_dir(self.root)
        self.capture = capture
        self.boot_id = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
        self.counter, self.lock, self.exchanges = 0, threading.Lock(), {}
        self.capture.add("rpc_capture_header", {"schema": "spc-lab-timed-rpc/1", "boot_id": self.boot_id,
                                               "authority": "observation_only", "credentials_recorded": False})

    def blob(self, data):
        require(isinstance(data, bytes) and len(data) <= MAX_RESPONSE, "RPC body resource/type bound")
        digest = hashlib.sha256(data).hexdigest()
        path = self.blobs / (digest + ".bin")
        try:
            fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        except FileExistsError:
            require(path.is_file() and not path.is_symlink() and sha_file(path) == digest, "RPC blob collision/change")
        else:
            with os.fdopen(fd, "wb") as stream:
                stream.write(data); stream.flush(); os.fsync(stream.fileno())
            sync_dir(self.blobs)
        return {"path": str(path.relative_to(self.root)), "sha256": digest, "bytes": len(data)}

    def begin(self, binding, request):
        broker = binding["route"] == "broker"
        body = json.dumps(request, separators=(",", ":"), ensure_ascii=False, sort_keys=broker).encode()
        with self.lock:
            self.counter += 1
            observation_id = self.counter
            entry = {"observation_id": observation_id, "node": binding["node_id"], "binding": binding,
                     "boot_id": self.boot_id, "request_id": request["id"], "method": request["method"],
                     "request_body_origin": "source_reconstructed_spc_lab_rpc_helper" if broker else "actual_http_request_body",
                     "request_body": self.blob(body), "begin": stamp()}
            self.capture.add("rpc_begin", entry)
            self.exchanges[observation_id] = entry
        return observation_id, body

    def end(self, observation_id, *, sent, raw=None, status=None, failure=None, transport=None):
        with self.lock:
            entry = self.exchanges[observation_id]
            require("end" not in entry, "duplicate RPC observation completion")
            result = {"observation_id": observation_id, "node": entry["node"], "boot_id": self.boot_id,
                      "sent": sent, "received": stamp(), "http_status": status, "failure": failure,
                      "response_body": self.blob(raw) if raw is not None else None,
                      "transport": transport or {}}
            self.capture.add("rpc_end", result)
            entry["end"] = result
            return observation_id

    def decoded(self, observation_id):
        entry = self.exchanges[observation_id]
        if "end" not in entry or entry["end"]["failure"] is not None or entry["end"]["response_body"] is None:
            raise Inconclusive("RPC observation is unresolved/failed")
        ref = entry["end"]["response_body"]
        raw = (self.root / ref["path"]).read_bytes()
        require(hashlib.sha256(raw).hexdigest() == ref["sha256"], "captured RPC response body changed")
        response = strict_json(raw)
        require(isinstance(response, dict) and type(response.get("id")) is int
                and response["id"] == entry["request_id"], "captured RPC response id mismatch")
        if response.get("error") is not None:
            raise Inconclusive("captured node RPC error")
        require("result" in response, "captured RPC result missing")
        return response["result"]

    def reference(self, observation_id):
        entry = self.exchanges[observation_id]
        require("end" in entry, "RPC reference is incomplete")
        return {"observation_id": observation_id, "node": entry["node"], "method": entry["method"],
                "request_body": entry["request_body"], "response_body": entry["end"]["response_body"],
                "sent": entry["end"]["sent"], "received": entry["end"]["received"],
                "boot_id": self.boot_id, "failure": entry["end"]["failure"]}


class NodeBinding:
    """Verify a declared actual node and its endpoint before/after every RPC."""
    def __init__(self, run_dir, manifest, node_id, expected_node, helper, broker=None):
        self.root, self.node_id = Path(run_dir), node_id
        records = [p for p in manifest["processes"] if p["role"] == "node" and p["entity"] == node_id]
        require(len(records) == 1, "owned node process record missing/duplicated")
        self.record = records[0]
        require(self.record["executable_sha256"] == expected_node["sha256"]
                and Path(self.record["executable_path"]).resolve() == Path(expected_node["path"]).resolve(), "node differs from explicit binary seal")
        self.system = ProcFS()
        self.controller = self.system.identity(manifest["locks"]["pid"])
        self.helper = helper
        if helper is not None:
            require(sha_file(helper["path"]) == helper["sha256"], "RPC helper binary seal")
        self.address = option(self.record["args"], "--rpc-listen")
        self.cookie = Path(option(self.record["args"], "--rpc-cookie-file"))
        require(self.cookie == self.root / "private" / ("node-rpc-" + node_id + ".cookie"), "node cookie path is not owned run path")
        parsed = urllib.parse.urlsplit("http://" + self.address)
        require(not parsed.username and not parsed.query and not parsed.path, "owned node RPC address shape")
        self.host, self.port = str(ipaddress.ip_address(parsed.hostname)), integer(parsed.port, 1, 65535)
        self.url = "http://" + self.address
        self.netns = self.system.namespace(self.record["pid"], "net")
        self.userns = self.system.namespace(self.record["pid"], "user")
        host_local = self.netns == self.system.namespace(self.controller["pid"], "net")
        require((broker is None) == host_local, "node RPC routing contradicts actual network namespace")
        if host_local:
            require(ipaddress.ip_address(self.host).is_loopback, "host node endpoint must be loopback")
        self.broker = Path(broker) if broker is not None else None
        self.run_id = manifest["run_id"]
        self.exe_identity = self._exe_identity()
        require(sha_file(f"/proc/{self.record['pid']}/exe") == expected_node["sha256"], "actual running node executable seal")
        self.verify()

    def _exe_identity(self):
        s = os.stat(f"/proc/{self.record['pid']}/exe")
        return s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns

    def verify(self):
        facts = self.system.identity(self.record["pid"])
        require(facts["start_time_ticks"] == self.record["start_time_ticks"] and facts["ppid"] == self.controller["pid"], "node PID/start/controller binding changed")
        require(self.system.identity(self.controller["pid"]) == self.controller, "owning controller changed")
        require(self._exe_identity() == self.exe_identity and self.system.namespace(facts["pid"], "net") == self.netns
                and self.system.namespace(facts["pid"], "user") == self.userns, "node executable/namespace changed")
        args = [p.decode() for p in Path(f"/proc/{facts['pid']}/cmdline").read_bytes().split(b"\0") if p]
        require(option(args, "--rpc-listen") == self.address and option(args, "--rpc-cookie-file") == str(self.cookie), "actual node RPC argv changed")
        tokens = [p.split(b"=", 1)[1] for p in Path(f"/proc/{facts['pid']}/environ").read_bytes().split(b"\0") if p.startswith(b"SPC_LAB_RUN_TOKEN=")]
        require(tokens == [self.record["run_token"].encode()], "node run ownership token mismatch")
        listener = self.system.tcp_listener(facts["pid"], self.host, self.port)
        return {"run_id": self.run_id, "node_id": self.node_id, "pid": facts["pid"], "start_time_ticks": facts["start_time_ticks"],
                "executable_sha256": self.record["executable_sha256"], "netns": self.netns, "userns": self.userns,
                "rpc_address": self.address, "route": "broker" if self.broker else "host", "listener": listener}

    def broker_peer(self, stream):
        data = private_bytes(self.root / "broker" / "manifest.json", self.root)
        publication = strict_json(data)
        endpoints = [e for e in publication["endpoints"] if e["name"] == self.node_id]
        require(len(endpoints) == 1, "node namespace endpoint publication missing")
        holder = endpoints[0]["namespace"]
        owner = self.system.identity(holder["holder_pid"])
        require(str(owner["start_time_ticks"]) == holder["start_time_token"]
                and self.system.namespace(owner["pid"], "net") == self.netns, "node namespace holder changed")
        peer_pid, uid, _ = struct.unpack("3i", stream.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
        require(uid == os.getuid() and self.system.namespace(peer_pid, "user") == self.userns, "RPC broker peer ownership/namespace")
        args = [p.decode() for p in Path(f"/proc/{peer_pid}/cmdline").read_bytes().split(b"\0") if p]
        require(option(args, "--run-dir") == str(self.root / "broker")
                and option(args, "--run-token") == publication["run_token"], "RPC broker peer belongs to another run")
        return self.system.identity(peer_pid)


class Rpc:
    READS, TEST_WRITES = READ_METHODS, TEST_METHODS

    def __init__(self, binding, recorder, allow_writes=False):
        self.binding, self.recorder = binding, recorder
        self.node_id = binding.node_id
        self.allow_writes, self.request_id, self.last_exchange = allow_writes, 0, None
        self.last_observation_seq = None

    def call(self, method, params=None):
        return self.recorded(method, params)[0]

    def recorded(self, method, params=None):
        require(method in self.READS or self.allow_writes and method in self.TEST_WRITES, "oracle RPC method forbidden")
        public_binding = self.binding.verify()
        self.request_id += 1
        request = {"jsonrpc": "1.0", "id": 1 if self.binding.broker else self.request_id,
                   "method": method, "params": [] if params is None else params}
        observation, body = self.recorder.begin(public_binding, request)
        raw, status, metadata, sent = None, None, {}, stamp()
        try:
            if self.binding.broker:
                argv = [self.binding.helper["path"], "rpc", "--url", self.binding.url, "--method", method,
                        "--params", json.dumps(request["params"], separators=(",", ":")), "--auth-file", str(self.binding.cookie)]
                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as stream:
                    stream.settimeout(12)
                    stream.connect(str(self.binding.broker))
                    peer = self.binding.broker_peer(stream)
                    stream.sendall(json.dumps({"op": "exec", "endpoint": self.node_id, "argv": argv, "timeout_s": 8}).encode() + b"\n")
                    with stream.makefile("rb") as reader:
                        wrapper = reader.readline(MAX_RESPONSE * 2 + 1)
                    require(len(wrapper) <= MAX_RESPONSE * 2 and wrapper.endswith(b"\n"), "broker RPC response resource/tail bound")
                    response = strict_json(wrapper)
                    metadata = {"broker_pid": peer["pid"], "broker_start_time_ticks": peer["start_time_ticks"],
                                "helper_exit_code": response.get("command_exit_code"), "wrapper_sha256": hashlib.sha256(wrapper).hexdigest()}
                    if isinstance(response.get("stdout"), str):
                        raw = response["stdout"].encode()
                    require(self.binding.system.identity(peer["pid"]) == peer, "broker changed during RPC")
                    if (response.get("error") is not None or response.get("command_exit_code") != 0
                            or response.get("stdout_truncated") or response.get("stderr_truncated") or response.get("timed_out")):
                        raise Inconclusive("bounded owned broker RPC failed/truncated")
            else:
                cookie = private_bytes(self.binding.cookie, self.binding.root, 4096).strip()
                headers = {"Content-Type": "application/json", "Authorization": "Basic " + base64.b64encode(cookie).decode()}
                outgoing = urllib.request.Request(self.binding.url, body, headers)
                try:
                    with urllib.request.urlopen(outgoing, timeout=5) as response:
                        status = response.status
                        raw = response.read(MAX_RESPONSE + 1)
                except urllib.error.HTTPError as error:
                    status, raw = error.code, error.read(MAX_RESPONSE + 1)
            require(raw is not None and len(raw) <= MAX_RESPONSE, "node RPC response body bound")
            self.binding.verify()
            value = strict_json(raw)
            require(isinstance(value, dict) and type(value.get("id")) is int and value["id"] == request["id"], "RPC response correlation")
            require("result" in value or value.get("error") is not None, "RPC response result missing")
            require(status is None or 200 <= status < 300 or value.get("error") is not None,
                    "HTTP failure cannot establish successful RPC")
        except (OSError, ValueError, KeyError, TypeError, Invalid, Inconclusive) as error:
            # Preserve complete raw response bytes when bounded; oversized data
            # is explicitly incomplete rather than retained as a valid prefix.
            if raw is not None and len(raw) > MAX_RESPONSE:
                metadata["oversized_response_bytes_read"] = len(raw)
                raw = None
            self.recorder.end(observation, sent=sent, raw=raw, status=status, failure=type(error).__name__, transport=metadata)
            self.last_exchange = observation
            raise Inconclusive("owned RPC incomplete: " + method) from error
        self.recorder.end(observation, sent=sent, raw=raw, status=status, transport=metadata)
        self.last_exchange = observation
        # Existing cross-stream effect checks consume this complete parsed copy;
        # exact timed bytes remain independently retained in rpc_begin/rpc_end.
        self.last_observation_seq = self.recorder.capture.add("node_rpc", {
            "node": self.node_id, "request": request, "response": value, "rpc_observation_id": observation})
        if value.get("error") is not None:
            raise Inconclusive("node RPC rejected " + method)
        return value["result"], observation
