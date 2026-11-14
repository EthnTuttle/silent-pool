#!/usr/bin/env python3
"""T3/T4b bounded production-controller acceptance driver; Python stdlib only.

Exit 0 = all required acceptance cases proved, 1 = observed contradiction,
2 = missing/unavailable evidence or prerequisites. Never builds or mines.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import base64
import ctypes
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import signal
import socket
import stat
import subprocess
import sys
import time
import threading
import urllib.error
import urllib.parse
import urllib.request
import uuid

from oracle import (Chain, Inconclusive, Invalid, Policy, decode_transaction,
                    integer, require, strict_json, unhex, witness_commitment)
from journal import Journal, hash_budget, native_observations, read_records
from rpc_capture import NodeBinding, Rpc, RpcRecorder
from canonical_capture import CanonicalCapture
from case_contract import COMMON, FUNDED, required, semantic_scenario, verdict, canonical_json


def digest(path):
    h = hashlib.sha256()
    with open(path, "rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def write_private(path, data):
    data = json.dumps(data, sort_keys=True, indent=2).encode() if not isinstance(data, bytes) else data
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def load(path):
    return strict_json(Path(path).read_bytes())


def private(path):
    path = Path(path)
    info = path.lstat()
    require(stat.S_ISREG(info.st_mode) and info.st_uid == os.getuid()
            and stat.S_IMODE(info.st_mode) == 0o600, "private file ownership/mode")
    return path.read_bytes()


def operator_arguments(enabled,binding,scenario):
    require(type(enabled) is bool,"operator UI selection must be boolean")
    if not enabled:return []
    require(not binding.get("topology"),"operator UI requires host runtime")
    require(all("api" in front["roles"] for front in scenario["fronts"]),"operator UI requires every front API role")
    return ["--operator-ui"]


def proc_ticks(pid):
    try:
        raw = Path(f"/proc/{pid}/stat").read_text()
        fields = raw[raw.rfind(")") + 2:].split()
        return None if fields[0] == "Z" else int(fields[19])
    except FileNotFoundError:
        return None


def parent_death(parent):
    def install():
        if ctypes.CDLL(None).prctl(1, signal.SIGKILL, 0, 0, 0) != 0 or os.getppid() != parent:
            os._exit(127)
    return install


class Capture:
    def __init__(self, root):
        self.path = root / "observations.jsonl"
        self.stream = self.path.open("xb")
        self.path.chmod(0o600)
        self.sequence = 0
        self.lock = threading.Lock()

    def add(self, kind, payload):
        with self.lock:
            self.sequence += 1
            row = {"seq": self.sequence, "monotonic_ns": time.monotonic_ns(),
                   "wall_ns": time.time_ns(), "kind": kind, "payload": payload}
            self.stream.write(json.dumps(row, separators=(",", ":"), default=str).encode() + b"\n")
            self.stream.flush()
            os.fsync(self.stream.fileno())
            return self.sequence


class Controller:
    REQUIRED = COMMON | FUNDED

    def __init__(self, args, *, runtime_bundle=None):
        self.args = args
        self.runtime_bundle = runtime_bundle
        self.root = Path(args.run_root).absolute()
        # No adoption of a previous run, even one with the same scenario id.
        self.root.mkdir(mode=0o700, parents=False, exist_ok=False)
        self.priv = self.root / "private"
        self.priv.mkdir(mode=0o700)
        self.run = self.root / "runtime"
        self.capture = Capture(self.priv)
        self.rpc_recorder = RpcRecorder(self.priv, self.capture)
        self.canonical = None
        self.source_capture = None
        self.source_feed_runtime = None
        self.native_capture = None
        self.phase_bootstrap = None
        self.observation_lock = threading.RLock()
        self.live_journal_readers = {}
        self.results = {}
        self.process = None
        self.url = None
        self.nodes, self.views, self.raws = {}, {}, {}
        self.raw_seen = set()
        self.commands = {}
        self.binding = {}
        self.scenario = {}
        self.deadline = time.monotonic() + args.timeout
        self.started_wall_seconds = time.time_ns() // 1_000_000_000
        self.binaries = {}
        self.operator = None
        self.http_workers = ThreadPoolExecutor(max_workers=2, thread_name_prefix="acceptance-http")
        self.hook_commands = {}
        self.fixture_effects = []

    def result(self, name, state, detail=None):
        previous = self.results.get(name)
        if previous and previous["status"] in {"FAILED", "INCONCLUSIVE"}:
            state = "FAILED" if "FAILED" in (state, previous["status"]) else "INCONCLUSIVE"
            detail = {"previous": previous, "later": detail}
        self.results[name] = {"status": state, "detail": detail}

    def check(self, name, callback):
        try:
            detail = callback()
            self.result(name, "PASSED", detail)
        except Invalid as error:
            self.result(name, "FAILED", str(error))
        except (Inconclusive, OSError, KeyError, ValueError, TypeError, IndexError, AttributeError, StopIteration) as error:
            self.capture.add("case_incomplete", {"case": name, "class": type(error).__name__, "detail": str(error)})
            self.result(name, "INCONCLUSIVE", {"class": type(error).__name__, "missing_component": error.component}
                        if hasattr(error, "component") else type(error).__name__)

    def bounded(self):
        if time.monotonic() >= self.deadline:
            raise Inconclusive("acceptance wall-clock deadline")
        if self.process and self.process.poll() is not None:
            raise Inconclusive("production controller exited before completion")

    def until(self, callback, timeout=60, allow_controller_exit=False):
        deadline = min(self.deadline, time.monotonic() + timeout)
        while time.monotonic() < deadline:
            if not allow_controller_exit:
                self.bounded()
            value = callback()
            if value:
                return value
            time.sleep(0.25)
        raise Inconclusive("bounded observation deadline")

    def prerequisites(self):
        self.binding = load(self.args.inputs)
        self.scenario = load(self.args.scenario)
        self.policy = Policy(**self.binding["policy"])
        self.run_id = self.scenario["run"]["id"]
        require(self.binding["schema"] == "spc-lab-funded-acceptance-inputs/1", "input schema")
        require(len(self.scenario["nodes"]) == 2 and len(self.scenario["pools"]) == 2
                and len(self.scenario["identities"]) == len(self.scenario["miners"]) == 4,
                "explicit two-node/two-pool/four-user census")
        workers = [w for m in self.scenario["miners"] for w in m["workers"]]
        require(len(workers) == 4 and sum(int(w["budget"]["hashes_per_second"]) for w in workers) == 40,
                "40 H/s aggregate worker budget")
        traffic = self.scenario["traffic"]
        require(str(traffic["rate"]["ordinary_tx_per_sec"]) == "0.2", "ordinary rate 0.2 tx/s")
        require(sum(a["network_wallets"] for a in traffic["driver"]["actors"]) == 8
                and sum(a["user_payment_wallets"] for a in traffic["driver"]["actors"]) == 4
                and sum(a["hosts_bootstrap_funding"] for a in traffic["driver"]["actors"]) == 1,
                "8 network/4 payment/exactly one funding host allocation")
        require(traffic["driver"]["bootstrap"]["hashes_per_second"] == 40
                and traffic["bootstrap"]["mining_mode"] == "native", "native funding budget")
        from runtime_bootstrap import RuntimeBootstrap, source_admission
        self.phase_bootstrap = RuntimeBootstrap(self.scenario)
        require(self.binding["payout_policy"] == "design-fixed-ladder", "full acceptance requires production fixed ladder")
        for name, item in self.binding["binaries"].items():
            path = Path(item["path"])
            if not (path.is_absolute() and path.is_file() and os.access(path, os.X_OK)):
                raise Inconclusive("explicit executable unavailable: " + name)
            require(digest(path) == item["sha256"], "binary seal mismatch")
            manifest = Path(item["source_manifest"])
            require(digest(manifest) == item["source_manifest_sha256"], "source manifest seal mismatch")
            self.binaries[name] = str(path)
        required = {"lab", "pool", "miner", "txd", "node", "scanner", "traffic-probe",
                    "redis-server", "redis-cli", "initdb", "postgres", "pg_ctl", "psql", "createdb"}
        if self.scenario.get("ark_backends"):
            required |= {"captaind", "grpcurl"}
            require(self.binaries.get("captaind", "").startswith("/nix/store/"), "stock captaind must be Nix-built")
        if self.binding.get("source_feed"):
            required.add("agent")
        if self.binding.get("topology"):
            required |= {"unshare", "nsenter"}
            broker = self.binding["broker_script"]
            require(digest(broker["path"]) == broker["sha256"], "broker script seal")
        if not required <= set(self.binaries):
            raise Inconclusive("missing sealed binary inputs: " + ",".join(sorted(required - set(self.binaries))))
        selected_bundle=getattr(self,'runtime_bundle',None)
        if selected_bundle is None:selected_bundle=self.binding.get('runtime_bundle')
        self.result("runtime_entrypoint_binding", "PASSED", source_admission(self.binding["binaries"]["lab"],bundle=selected_bundle))
        from base_network import reject_known_host_network_failure
        reject_known_host_network_failure(self.binding["binaries"]["lab"])
        if self.args.case == "funded":
            from native_capture import require_audited_sources
            self.result("native_source_applicability", "PASSED", require_audited_sources(self.binding))
        unhex(self.binding["source_snapshot_sha256"], 32)
        postgres_dir = Path(self.binaries["initdb"]).parent
        require(all(Path(self.binaries[name]).parent == postgres_dir for name in ("postgres", "pg_ctl", "psql", "createdb")),
                "PostgreSQL utilities must share the declared actual bin directory")
        probe = self.binding["probe_script"]
        require(digest(probe["path"]) == probe["sha256"], "canonical probe script seal")
        self.capture.add("input_binding", {"inputs_sha256": digest(self.args.inputs), "scenario_sha256": digest(self.args.scenario),
                                            "binary_hashes": {k: v["sha256"] for k, v in self.binding["binaries"].items()}})
        write_private(self.priv / "inputs.json", Path(self.args.inputs).read_bytes())
        write_private(self.priv / "scenario.json", Path(self.args.scenario).read_bytes())

    def manifest(self):
        path = self.run / "run-manifest.json"
        if not path.exists():
            return None
        return load(path)

    def start(self):
        operator_flags=operator_arguments(getattr(self.args,"operator_ui",False),self.binding,self.scenario)
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        self.url = f"http://127.0.0.1:{port}"
        argv = [self.binaries["lab"], self.phase_bootstrap.plan["entrypoint"], "--scenario", str(Path(self.args.scenario).absolute()),
                "--run-dir", str(self.run), "--listen", f"127.0.0.1:{port}",
                "--duration-secs", str(self.args.timeout), "--pow-bits", f"{self.policy.initial_target_bits:08x}",
                 "--base-revision", self.binding["base_revision"], "--dirty-tree-hash", self.binding["source_snapshot_sha256"]]
        argv += operator_flags
        for flag in ("node", "pool", "miner", "scanner", "txd", "traffic-probe", "redis-server", "redis-cli"):
            argv += ["--" + flag, self.binaries[flag]]
        argv += ["--postgres-bin-dir", str(Path(self.binaries["initdb"]).parent),
                 "--traffic-probe-arg", self.binding["probe_script"]["path"]]
        for flag in ("captaind", "grpcurl"):
            if flag in self.binaries:
                argv += ["--" + flag, self.binaries[flag]]
        if "agent" in self.binaries:
            argv += ["--wallet-agent-bin", self.binaries["agent"]]
        if self.binding.get("topology"):
            argv += ["--topology", "--broker-script", self.binding["broker_script"]["path"],
                     "--unshare", self.binaries["unshare"], "--nsenter", self.binaries["nsenter"]]
        from source_capture import SourceCapture
        self.source_capture = SourceCapture(self.run, self.priv, self.capture, self.scenario,
                                            self.binding["binaries"]["pool"], self.policy, self.binding.get("source_feed"))
        if self.binding.get("source_feed"):
            from native_capture import NativeCapture
            self.native_capture = NativeCapture(self)
        self.capture.add("command_spawn_intent", {"argv": argv})
        self.capture.add("runtime_bootstrap_owner", self.phase_bootstrap.plan)
        output = (self.priv / "controller.log").open("xb")
        self.process = subprocess.Popen(argv, stdout=output, stderr=subprocess.STDOUT,
                                        env={"PATH": self.binding["path_env"], "HOME": str(self.priv)},
                                        start_new_session=True, preexec_fn=parent_death(os.getpid()))
        output.close()
        self.capture.add("command_spawned", {"pid": self.process.pid, "start_ticks": proc_ticks(self.process.pid)})
        def ready():
            m = self.manifest()
            return m if m and m.get("traffic_stage") in {"provisioned", "bootstrapping", "active"} else None
        manifest = self.until(ready, self.args.start_timeout)
        for node in self.scenario["nodes"]:
            node_id = node["id"]
            endpoint = manifest["resources"]["listeners"][f"node.{node_id}.rpc"]
            url = f"http://{endpoint['host']}:{endpoint['port']}"
            broker = None
            if self.binding.get("topology"):
                process = next(p for p in manifest["processes"] if p["entity"] == node_id and p["role"] == "node")
                require(proc_ticks(process["pid"]) == process["start_time_ticks"], "owned namespace node identity")
                argv = process["args"]
                require(argv.count("--rpc-listen") == 1, "actual node RPC bind argument")
                url = "http://" + argv[argv.index("--rpc-listen") + 1]
                broker = self.run / "broker" / "control.sock"
            binding = NodeBinding(self.run, manifest, node_id, self.binding["binaries"]["node"],
                                  self.binding["binaries"]["lab"], broker)
            require(binding.url == url, "node resource endpoint differs from bound actual process")
            self.nodes[node_id] = Rpc(binding, self.rpc_recorder, self.args.allow_reorg or self.args.witness)
        self.bind_policy()
        self.canonical = CanonicalCapture(self.policy, self.rpc_recorder, self.capture,
                                          max_blocks=self.args.max_blocks, deadline=self.deadline)
        self.raws, self.views = self.canonical.raws, self.canonical.views
        fresh = self.until(self.common_chain, 10)
        if fresh.tip["height"] == 0:
            self.result("fresh_chain", "PASSED", {"height": 0, "hash": fresh.tip["hash"], "run_id": self.run_id})
        else:
            self.result("fresh_chain", "INCONCLUSIVE", {"reason": "Runtime auto-bootstrap preceded the initial observer sample; earlier genesis evidence required",
                "first_observed_height": fresh.tip["height"], "run_id": self.run_id})
        self.check("config_binding", self.audit_config_binding)
        if self.binding.get("source_feed"):
            from source_feed_runtime import CollectorRuntime
            self.source_feed_runtime=CollectorRuntime(self)
            self.source_feed_runtime.start()

    def audit_config_binding(self):
        actual = load(self.run / "private" / "scenario.json")
        require(actual["run"]["id"] == self.run_id and actual["traffic"] == self.scenario["traffic"], "rendered traffic/run config changed")
        require([(m["id"], [(w["id"], w["budget"]) for w in m["workers"]]) for m in actual["miners"]]
                == [(m["id"], [(w["id"], w["budget"]) for w in m["workers"]]) for m in self.scenario["miners"]],
                "rendered native worker budgets changed")
        require([(p["id"], p["work_profile"]) for p in actual["pools"]]
                == [(p["id"], p["work_profile"]) for p in self.scenario["pools"]], "rendered pool policy changed")
        configs = set((self.run / "private").glob("*.toml"))
        launch_bindings = []
        for process in self.manifest()["processes"]:
            args = process["args"]
            require(args.count("--config") <= 1, "ambiguous process config argument")
            if "--config" in args:
                index = args.index("--config")
                require(index + 1 < len(args), "process config argument missing")
                path = Path(args[index + 1])
                require(path.is_relative_to(self.run / "private") and ".." not in path.parts,
                        "process config outside owned case")
                configs.add(path)
            launch_bindings.append({"role": process["role"], "entity": process["entity"],
                "executable_sha256": process["executable_sha256"], "args_sha256": hashlib.sha256(canonical_json(args)).hexdigest()})
        if not configs:
            raise Inconclusive("rendered runtime config census missing")
        require(len(configs) <= 256 and all(path.stat().st_size <= 4 * 1024 * 1024 for path in configs), "runtime config census/byte bound")
        return {"scenario_semantic_sha256": semantic_scenario(self.scenario),
                "rendered_scenario_sha256": digest(self.run / "private" / "scenario.json"),
                "launch_bindings": launch_bindings,
                "configs": [{"path": str(p), "sha256": hashlib.sha256(private(p)).hexdigest()} for p in sorted(configs)]}

    def http(self, suffix, payload=None, timeout=10):
        headers = {"Content-Type": "application/json", "Origin": self.url}
        actor = "acceptance"
        if self.scenario.get("expanded_access", False):
            claim_path = self.run / "private" / "access" / "operator.json"
            claim = strict_json(private(claim_path))
            require(claim["run_id"] == self.run_id and claim["may"] == ["operator"], "explicit operator claim")
            headers["Authorization"] = "Bearer " + claim["token"]
            actor = claim["actor"]
        if payload is not None:
            payload["actor"] = actor
        body = json.dumps(payload).encode() if payload is not None else None
        request = urllib.request.Request(self.url + f"/api/lab/v1/runs/{self.run_id}/" + suffix, body, headers)
        try:
            with urllib.request.urlopen(request, timeout=timeout) as response:
                result = strict_json(response.read(16 * 1024 * 1024))
        except (OSError, ValueError) as error:
            raise Inconclusive("controller HTTP " + type(error).__name__) from error
        # Authorization headers and claim tokens are never recorded.
        self.capture.add("controller_http", {"route": suffix, "request": payload, "response": result})
        return result

    def prepare_command(self, label, kind):
        require(kind.get("type") != "bootstrap_fund", "Runtime owns the declared bootstrap; no HTTP resubmission or new bootstrap ID")
        if label not in self.commands:
            actor = "acceptance"
            if self.scenario.get("expanded_access", False):
                claim = strict_json(private(self.run / "private" / "access" / "operator.json"))
                require(claim["run_id"] == self.run_id and claim["may"] == ["operator"], "command operator claim")
                actor = claim["actor"]
            self.commands[label] = {"command_id": str(uuid.uuid4()), "run_id": self.run_id,
                                    "issued_at_wall": time.time_ns() // 1_000_000, "actor": actor, "kind": kind}
            write_private(self.priv / ("command-" + label + ".json"), self.commands[label])
        require(self.commands[label]["kind"] == kind, "durable command replay payload changed")
        return self.commands[label]

    def command(self, label, kind, wait=True, timeout=None):
        self.prepare_command(label, kind)
        def send():
            response = self.http("commands", dict(self.commands[label]))
            state = response.get("state")
            if state == "succeeded":
                return response
            if state not in {"accepted", "running"}:
                raise Inconclusive("durable command " + label + " ended " + str(state))
            return None
        return self.until(send, min(self.args.timeout,timeout) if timeout is not None else self.args.timeout) if wait else self.http("commands", dict(self.commands[label]))

    def command_async(self, label, kind):
        envelope = dict(self.prepare_command(label, kind))
        return self.http_workers.submit(self.http, "commands", envelope, 155)

    def audit_rows(self):
        from traffic_audit import read_commands
        return read_commands(self.journal_prefix("commands.jsonl"))

    def journal_prefix(self, name):
        from live_journal import LiveJournalPrefix
        require(name in {"commands.jsonl", "evidence.jsonl"}, "live observer journal selection")
        with self.observation_lock:
            if name not in self.live_journal_readers:
                self.live_journal_readers[name]=LiveJournalPrefix(self.run/"private"/name,self.run,
                    maximum_record=16*1024*1024 if name=="commands.jsonl" else 128*1024*1024)
            reader=self.live_journal_readers[name];previous=reader.observation
            raw=reader.read()
            if reader.observation!=previous:self.capture.add("live_journal_prefix",reader.observation)
            return raw

    def current_incarnation(self, driver):
        sessions = [r["event"]["binding"] for r in self.audit_rows()
                    if r["record"] == "traffic_audit" and r["event"]["event"] == "session"
                    and r["event"]["binding"]["driver_id"] == driver]
        if not sessions:
            raise Inconclusive("actual traffic_audit session not yet available")
        binding = sessions[-1]
        require(binding["acceptance_enabled"] is True and binding["run_id"] == self.run_id
                and binding["controller_sha256"] == self.binding["binaries"]["lab"]["sha256"]
                and binding["actor_sha256"] == self.binding["binaries"]["txd"]["sha256"], "current acceptance session binding")
        return binding["incarnation"]

    def arm_hook(self, label, driver, target, mode):
        require(self.scenario.get("expanded_access") is True and self.scenario["traffic"].get("lab_acceptance") is True
                and self.manifest().get("traffic_lab_acceptance") is True, "acceptance adapter must be explicitly enabled")
        config = self.manifest()["traffic_driver_paths"][driver]["config_path"]
        require(load(config).get("lab_acceptance") is True, "actual actor acceptance opt-in missing")
        key = {"driver_id": driver, "incarnation": self.current_incarnation(driver), "command_id": target}
        kind = {"type": "traffic_acceptance", "key": key,
                "control": {"action": "arm", "mode": mode, "timeout_ms": 120000}}
        self.command(label + "-arm", kind)
        self.hook_commands[label] = key
        return key

    def hook_signed(self, key):
        for row in self.audit_rows():
            if row["record"] != "traffic_audit":
                continue
            event = row["event"]
            if event["event"] == "hook" and event["key"] == key and event["detail"]["transition"] == "signed":
                return row
        return None

    def release_hook(self, label, key, checkpoint):
        self.command(label + "-release", {"type": "traffic_acceptance", "key": key,
                     "control": {"action": "release", "raw_sha256": checkpoint["event"]["detail"]["raw_sha256"]}})

    def acceptance_hooks(self):
        if not self.args.destination:
            raise Inconclusive("hold/loss tests require the explicit payment destination")
        user = sorted(i["id"] for i in self.scenario["identities"])[0]
        payment = {"type": "send_user_payment", "user": user, "destination_address": self.args.destination,
                   "amount_sats": self.args.amount_sats, "fee_rate_sats_vb": 2}
        for miner in self.scenario["miners"]:
            self.command("hooks-pause-" + miner["id"], {"type": "pause_workers", "miner_id": miner["id"],
                         "workers": [w["id"] for w in miner["workers"]]})
        for label, mode in (("hold-payment", "hold_before_intent"), ("loss-payment", "lose_one_response")):
            target = self.prepare_command(label, payment)["command_id"]
            key = self.arm_hook(label, self.driver, target, mode)
            future = self.command_async(label, payment)
            checkpoint = self.until(lambda: self.hook_signed(key), 30)
            if mode == "hold_before_intent":
                start = time.monotonic_ns()
                while time.monotonic_ns() - start < 1_000_000_000:
                    self.bounded()
                    self.common_chain()
                self.release_hook(label, key, checkpoint)
            response = future.result(timeout=155)
            if response.get("state") != "succeeded":
                raise Inconclusive("acceptance payment did not reach its actual durable outcome")
            records = next(rows for _, rows, _ in self.journals() if rows[0]["driver_id"] == self.driver)
            signed = next(r for r in records if r["record"] == "signed_tx" and r["command_id"] == target)
            txid = signed["txid"]
            def in_both_mempools():
                return all(txid in n.call("getrawmempool", []) for n in self.nodes.values())
            self.until(in_both_mempools, 15)
            for node in self.nodes.values():
                require(node.call("getrawtransaction", [txid, 0]) == signed["raw_hex"], "actual hook transaction raw RPC differs")
            self.capture.add("acceptance_hook_command_completed", {"label": label, "key": key})
        for miner in self.scenario["miners"]:
            self.command("hooks-resume-" + miner["id"], {"type": "resume_workers", "miner_id": miner["id"],
                         "workers": [w["id"] for w in miner["workers"]]})

    def traffic_reader(self):
        from traffic_audit import TrafficAudit
        rows = self.audit_rows()  # snapshot command stream before its histories
        snapshots = self.journals()
        histories = {records[0]["driver_id"]: records for _, records, _ in snapshots}
        require(len(histories) == len(snapshots), "multiple journals claim one driver identity")
        manifest = self.manifest()
        rejection_bindings = {driver: load(mapping["config_path"]).get("node_profile_binding")
                              for driver, mapping in manifest["traffic_driver_paths"].items()}
        expected_rejection = self.binding.get("definitive_rejection_binding")
        if expected_rejection is not None:
            require(expected_rejection["binary_sha256"] == self.binding["binaries"]["node"]["sha256"]
                    and all(p["profile_fingerprint"] == expected_rejection["consensus_fingerprint"] for p in manifest["effective_profiles"].values()),
                    "reviewed rejection binding differs from actual nodes")
        audit = TrafficAudit(rows, histories, run_id=self.run_id,
                            controller_sha256=self.binding["binaries"]["lab"]["sha256"],
                            txd_sha256=self.binding["binaries"]["txd"]["sha256"],
                            miner_sha256=self.binding["binaries"]["miner"]["sha256"],
                            node_ids=self.nodes, acceptance_enabled=self.scenario["traffic"].get("lab_acceptance", False),
                            rejection_bindings=rejection_bindings, expected_rejection_binding=expected_rejection)
        audit.history_artifacts = snapshots
        return audit

    def bind_policy(self):
        # Explicit source-version field map: no inferred defaults for omitted
        # allow-min, BIP94, overlay, subsidy or retarget facts.
        mapping = self.binding["consensus_field_map"]
        for node in self.nodes.values():
            require(node.call("getblockhash", [0]) == self.policy.genesis, "actual genesis binding")
            info = node.call("getblockchaininfo")
            require(info["chain"] == "regtest", "actual network binding")
            observed = info["consensus"]
            for field, key in mapping.items():
                if key not in observed:
                    raise Inconclusive("bound consensus field missing: " + key)
                expected = getattr(self.policy, field)
                actual = observed[key]
                if field.endswith("bits") or field == "regtest_pow_bits_overlay":
                    actual = int(actual.removeprefix("0x"), 16) if isinstance(actual, str) else actual
                require(actual == expected, "live consensus policy mismatch: " + field)
            source_facts = self.binding.get("consensus_source_facts", {})
            for field, fact in source_facts.items():
                require(field not in mapping and fact["value"] == getattr(self.policy, field), "source policy binding")
                require(digest(fact["source_path"]) == fact["source_sha256"], "source policy seal")
            mandatory = set(self.policy.__dataclass_fields__) - {"genesis"}
            if set(mapping) | set(source_facts) != mandatory:
                raise Inconclusive("consensus field binding incomplete")

    def common_chain(self):
        with self.observation_lock:
            view = self.canonical.sample(self.nodes.values())
            if self.source_capture is not None:
                self.source_capture.poll(self.manifest(), self.views)
            if self.native_capture is not None:
                self.native_capture.poll(self.manifest())
            return view

    def journals(self):
        paths = sorted((self.run / "private" / "traffic").glob("d-*/private/traffic/history.jsonl"))
        if not paths:
            raise Inconclusive("no production history-3 journals")
        return [(path, *read_records(path)) for path in paths]

    def bootstrap_status(self, require_phase=False):
        with self.observation_lock:
            raw = self.journal_prefix("commands.jsonl")
            evidence = self.journal_prefix("evidence.jsonl") if require_phase else None
            result = self.phase_bootstrap.observe(raw, evidence, require_phase)
            self.capture.add("original_runtime_bootstrap_observation", result)
            return result

    def require_active_finalization(self):
        """Read-only precondition for AP5's one ordinary-traffic quiescence."""
        from source_jobs import MissingComponent
        manifest=self.manifest()
        stage=manifest.get("traffic_stage") if manifest is not None else None
        if manifest is not None:require(manifest["run_id"]==self.run_id,"finalization actual run binding")
        if stage!="active":
            self.capture.add("finalization_stage_unavailable",{"run_id":self.run_id,"traffic_stage":stage,
                "component":"traffic_not_active_no_final_quiescence","completed_funding_proved":False})
            raise MissingComponent("traffic_not_active_no_final_quiescence")
        result=self.bootstrap_status(require_phase=True)
        if result["state"]!="succeeded":raise MissingComponent("original_bootstrap_incomplete_no_final_quiescence")
        return {"run_id":self.run_id,"traffic_stage":stage,"operation_id":result["operation_id"],
            "scope":"original_bootstrap_and_stage_precondition_only"}

    def bootstrap_operation(self):
        from source_jobs import MissingComponent
        self.driver = self.phase_bootstrap.plan["kind"]["driver_id"]
        def begun():
            try:
                return self.bootstrap_status()
            except FileNotFoundError:
                return None
            except MissingComponent as error:
                if error.component in {"original_Runtime_bootstrap_begin_not_observed", "original_Runtime_bootstrap_hash_start"}:
                    return None
                raise
        self.until(begun, self.args.start_timeout, allow_controller_exit=True)
        return self.phase_bootstrap.plan["command_id"]

    def bootstrap(self):
        from source_jobs import MissingComponent
        self.bootstrap_operation()
        def complete():
            self.common_chain()
            status = self.bootstrap_status()
            if status["state"] == "observed_in_flight":
                return None
            if status["state"] != "succeeded":
                raise MissingComponent("original_Runtime_bootstrap_terminal_" + status["state"])
            try:
                self.bootstrap_status(require_phase=True)
            except MissingComponent as error:
                if error.component == "original_Runtime_phase_terminal_observation":
                    return None
                raise
            manifest = self.manifest()
            return manifest if manifest.get("traffic_stage") == "active" else None
        manifest = self.until(complete, self.args.timeout, allow_controller_exit=True)
        boundary = manifest.get("traffic_measured_from")
        if not boundary:
            raise Inconclusive("measured boundary missing")
        self.boundary = boundary
        self.capture.add("measured_boundary", boundary)
        self.measured_height = integer(boundary["tip_height"])
        common = self.until(self.common_chain)
        require(common.blocks[self.measured_height]["hash"] == boundary["tip_hash"], "measured canonical boundary")
        # Actual AddConnection ordering must additionally be checked against
        # production control evidence, not inferred from this manifest state.

    def measured(self):
        time_end = min(self.deadline, time.monotonic() + self.args.measure_seconds)
        while time.monotonic() < time_end:
            self.bounded()
            self.common_chain()
            time.sleep(0.5)
        self.command("pause", {"type": "pause_tx_driver", "driver_id": self.driver})
        before = self.journals()
        start = time.monotonic_ns()
        while time.monotonic_ns() - start < self.args.pause_seconds * 1_000_000_000:
            self.bounded()
            self.common_chain()
            time.sleep(0.25)
        after = self.journals()
        for (p1, r1, _), (p2, r2, _) in zip(before, after):
            require(p1 == p2 and r2[:len(r1)] == r1, "journal history rewritten while paused")
            require(not any(r["record"] in {"broadcast_sent", "reorg_resubmit"} for r in r2[len(r1):]),
                    "send occurred during actual paused interval")
        self.capture.add("pause_read_interval", {"start_monotonic_ns": start, "end_monotonic_ns": time.monotonic_ns(),
                                                  "seconds_requested": self.args.pause_seconds})
        # Full pass also needs zero controller forwarded sends, verified later.
        self.command("resume", {"type": "resume_tx_driver", "driver_id": self.driver})
        if self.args.destination:
            payload = {"type": "send_user_payment", "user": sorted(i["id"] for i in self.scenario["identities"])[0],
                       "destination_address": self.args.destination, "amount_sats": self.args.amount_sats,
                       "fee_rate_sats_vb": 2}
            self.command("manual", payload)
            self.command("manual", payload)  # exact original durable command ID
        if self.args.witness:
            self.witness()
        self.check("acceptance_hook_execution", self.acceptance_hooks)
        self.check("version_rejection_execution", self.version_rejection_case)
        self.check("v3_prior_unknown_retention", self.version_rejection_loss_case)

    def version_rejection_case(self):
        """The frozen closed constructor chooses v3 before production signing."""
        from signature_oracle import verify_legacy_sighash_all
        if not self.args.destination:
            raise Inconclusive("before-sign case requires an explicit actual destination")
        expected_binding = self.binding.get("definitive_rejection_binding")
        frozen = {"source_sha256": "f1592c935e29b0b1fdf6429ad1b84e9ae925e9a9fcccd66b04f3d090680ff20f",
                  "binary_sha256": "bba4752298aff258adb6cce99c6db849b02f50736a8ec32766ed1710045051aa",
                  "consensus_fingerprint": "4634ab3e2ca66aa9d319b35d89131f7ad1fc48c3b315435652282fe9e20b79d3"}
        require(expected_binding == frozen and self.binding["binaries"]["node"]["sha256"] == frozen["binary_sha256"],
                "before-sign rejection case requires the exact reviewed release tuple")
        require(self.scenario.get("expanded_access") is True and self.scenario["traffic"].get("lab_acceptance") is True,
                "before-sign constructor requires explicit operator acceptance mode")
        user = sorted(i["id"] for i in self.scenario["identities"])[0]
        kind = {"type": "acceptance_version_rejection_v1", "user": user, "destination_address": self.args.destination,
                "amount_sats": 100000, "fee_rate_sats_vb": 3}
        envelope = dict(self.prepare_command("version-rejection", kind))
        response = self.http("commands", dict(envelope), timeout=155)
        require(response.get("state") == "failed", "version constructor did not return its expected failed outer state")
        detail = strict_json(response["detail"])
        require(detail.get("status") == "rejected" and detail.get("code") == "rpc_tx_version_nonstandard",
                "version constructor did not return the exact actor rejection")
        replay = self.http("commands", dict(envelope), timeout=155)
        require(replay["state"] == response["state"] and replay["detail"] == response["detail"], "selected operation replay changed outcome")
        changed = {**envelope, "kind": {**kind, "type": "send_user_payment"}}
        conflict = self.http("commands", changed)
        require(conflict.get("state") == "rejected_conflict", "same id with changed construction semantics did not conflict")
        self.until(self.common_chain)
        audit = self.traffic_reader()
        cid = envelope["command_id"]
        located = [(driver, row) for driver, rows in audit.histories.items() for row in rows
                   if row["record"] == "signed_tx" and row["command_id"] == cid]
        require(len(located) == 1, "constructor replay created duplicate/missing signed transaction")
        driver, signed = located[0]
        begins = [r for r in audit.histories[driver] if r["record"] == "command_begin" and r["command_id"] == cid]
        require(len(begins) == 1 and begins[0]["request"].get("construction_effect") == {"schema": "spc-lab-version-rejection/v1"},
                "production before-sign effect is not immutable in CommandBegin")
        proofs = [p for p in audit.outcome_proofs.rejections.values() if p["command_id"] == cid]
        require(len(proofs) == 1 and not proofs[0]["retention_required"], "fresh rejection has prior uncertainty/publication")
        forwards = [r for r in audit.attempts.values() if r["event"]["attempt"]["method"] == "sendrawtransaction"
                    and r["event"]["attempt"]["context"]["command_id"] == cid]
        require(len(forwards) == 1, "constructor replay resent the transaction")
        context = forwards[0]["event"]["attempt"]["context"]
        anchor_record = audit.histories[driver][context["chain_revision"]]
        anchor = anchor_record["tip_block_hash"] if anchor_record["record"] == "chain_observed" else anchor_record["evidence"]["new_anchor"]["block_hash"]
        views, _ = audit.journal_anchor_views(self.views, {d: m["authority_node"] for d, m in self.manifest()["traffic_driver_paths"].items()})
        view = views[anchor]
        signature = verify_legacy_sighash_all(unhex(signed["raw_hex"]), view, expected_version=3)
        require(signature["txid"] == detail["txid"] and signature["fee_sats"] == signed["fee_sats"]
                and signature["vbytes"] == signed["vbytes"], "final production signed bytes/fee/size mismatch")
        from oracle import address_script
        decoded = decode_transaction(unhex(signed["raw_hex"]))
        require(any(o["script"] == address_script(self.args.destination) and o["value"] == 100000 for o in decoded["outputs"]),
                "version constructor changed the requested standard destination")
        journal = Journal(audit.histories[driver], {"run_id": self.run_id, "driver_id": driver}, views, audit.outcome_proofs)
        require(all(i["outpoint"] not in journal.reserved for i in decoded["inputs"]), "fresh definite NoEffect did not release inputs")
        current = self.until(self.common_chain)
        require(signature["txid"] not in current.transactions, "version-rejected candidate was published canonically")
        for node in self.nodes.values():
            mempool = node.call("getrawmempool", [])
            require(isinstance(mempool, list), "candidate absence requires actual mempool array")
            for txid in mempool:
                unhex(txid, 32)
            require(signature["txid"] not in mempool, "version-rejected candidate entered node mempool")
            try:
                node.call("getrawtransaction", [signature["txid"], 0])
            except Inconclusive:
                records = [strict_json(line) for line in private(self.capture.path).splitlines()]
                observed = records[-1]
                require(observed["kind"] == "node_rpc" and observed["payload"]["node"] == node.node_id
                        and observed["payload"]["request"]["method"] == "getrawtransaction"
                        and observed["payload"]["request"]["params"] == [signature["txid"], 0]
                        and observed["payload"]["response"].get("error", {}).get("code") == -5,
                        "candidate absence read was not an actual node unknown-transaction response")
            else:
                raise Invalid("version-rejected candidate found in node raw transaction store")
        self.result("funded_v3_sighash_all", "PASSED", {**signature, "command_id": cid,
                    "actual_forward_count": 1, "same_id_replay_no_resend": True, "changed_semantics_conflicted": True,
                    "raw_candidate_unpublished_at_observation": True, "fresh_no_prior_uncertainty_release": True})

    def version_rejection_loss_case(self):
        from signature_oracle import verify_legacy_sighash_all
        if self.results.get("funded_v3_sighash_all", {}).get("status") != "PASSED":
            raise Inconclusive("prior-unknown constructor case requires the verified fresh funded v3 case")
        user = sorted(i["id"] for i in self.scenario["identities"])[0]
        kind = {"type": "acceptance_version_rejection_v1", "user": user, "destination_address": self.args.destination,
                "amount_sats": 100000, "fee_rate_sats_vb": 3}
        cid = self.prepare_command("version-rejection-loss", kind)["command_id"]
        self.arm_hook("version-rejection-loss", self.driver, cid, "lose_one_response")
        response = self.http("commands", dict(self.commands["version-rejection-loss"]), timeout=155)
        require(response.get("state") == "failed", "suppressed rejection must not become successful payment")
        detail = strict_json(response["detail"])
        require(detail.get("status") == "unknown", "suppressed actual rejection was not delivered as Unknown")
        def retried_rejection():
            self.common_chain()
            rows = next(rows for _, rows, _ in self.journals() if rows[0]["driver_id"] == self.driver)
            unknown = [r for r in rows if r["record"] == "unknown_outcome" and r["command_id"] == cid]
            rejected = [r for r in rows if r["record"] == "rejected" and r["command_id"] == cid]
            return bool(unknown and rejected and unknown[0]["seq"] < rejected[0]["seq"])
        self.until(retried_rejection, 60)
        audit = self.traffic_reader()
        rows = audit.histories[self.driver]
        signed = [r for r in rows if r["record"] == "signed_tx" and r["command_id"] == cid]
        require(len(signed) == 1, "prior-unknown retry regenerated signed bytes")
        attempts = [(seq, r["event"]["attempt"]) for seq, r in audit.attempts.items()
                    if r["event"]["attempt"]["method"] == "sendrawtransaction"
                    and r["event"]["attempt"]["context"]["command_id"] == cid]
        require(len(attempts) >= 2 and len({a["context"]["intent_seq"] for _, a in attempts}) == len(attempts), "retry reused a durable intent")
        first_seq, first = attempts[0]
        first_result = audit.results[first_seq]["event"]
        from traffic_audit import compact, digest as audit_digest
        require(first_result["outcome"] == "node_rejected" and first_result["response_suppressed"]
                and first_result["response_sha256"] == audit_digest(compact({"outcome": "node_rejected", "code": -26, "message": "version"})),
                "prior-unknown test did not suppress the actual exact node rejection")
        proofs = [p for p in audit.outcome_proofs.rejections.values() if p["command_id"] == cid]
        require(proofs and all(p["retention_required"] for p in proofs), "later rejection forgot prior uncertainty")
        views, _ = audit.journal_anchor_views(self.views, {d: m["authority_node"] for d, m in self.manifest()["traffic_driver_paths"].items()})
        anchor_record = rows[first["context"]["chain_revision"]]
        anchor = anchor_record["tip_block_hash"] if anchor_record["record"] == "chain_observed" else anchor_record["evidence"]["new_anchor"]["block_hash"]
        signature = verify_legacy_sighash_all(unhex(signed[0]["raw_hex"]), views[anchor], expected_version=3)
        journal = Journal(rows, {"run_id": self.run_id, "driver_id": self.driver}, views, audit.outcome_proofs)
        require(all(i["outpoint"] in journal.reserved for i in decode_transaction(unhex(signed[0]["raw_hex"]))["inputs"]),
                "prior-unknown rejection released possibly-spent inputs")
        return {"command_id": cid, "txid": signature["txid"], "actual_attempts": len(attempts),
                "fresh_intent_per_retry": True, "all_final_signatures_verified": True, "possibly_spent_inputs_retained": True}

    def witness(self):
        path = Path(__file__).resolve().parent.parent / "lab-witness" / "fixture.py"
        spec = importlib.util.spec_from_file_location("test_only_witness_fixture", path)
        fixture = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(fixture)
        self.capture.add("test_only_fixture_census", {"extra_actors": 1, "known_fixture_keys": 2,
                                                     "funders": 0, "fixture_source_sha256": digest(path)})
        for index, kind in enumerate(("p2wpkh", "p2tr")):
            user = sorted(i["id"] for i in self.scenario["identities"])[index]
            self.command("witness-fund-" + kind, {"type": "send_user_payment", "user": user,
                         "destination_address": fixture.address(kind), "amount_sats": 100000, "fee_rate_sats_vb": 2})
            def funded():
                chain = self.common_chain()
                if chain:
                    return next(((point, output) for point, output in chain.utxos.items()
                                 if output["script"] == fixture.script(kind) and output["value"] == 100000), None)
            point, output = self.until(funded, 120)
            for miner in self.scenario["miners"]:
                self.command("witness-pause-" + kind + "-" + miner["id"],
                             {"type": "pause_workers", "miner_id": miner["id"],
                              "workers": [w["id"] for w in miner["workers"]]})
            # Hold only the real miners, leaving ordinary production traffic
            # running. An explicit additional legacy-input payment guarantees
            # a real mixed template opportunity without synthetic transactions.
            self.command("witness-legacy-" + kind, {"type": "send_user_payment", "user": user,
                         "destination_address": fixture.address(kind), "amount_sats": 20000, "fee_rate_sats_vb": 2})
            txid, vout = point.split(":")
            raw = fixture.spend(kind, txid, int(vout), output["value"], fixture.script(kind), 1000)
            decoded = decode_transaction(raw)
            require(decoded["witness"] and decoded["txid"] != decoded["wtxid"], "test fixture witness shape")
            # Explicit owned test channel, not production T1 witness signing.
            node = next(iter(self.nodes.values()))
            fixture_id = "witness-spend-" + kind
            intent_seq = self.capture.add("test_only_witness_send", {"actor": "witness-fixture", "command_id": fixture_id,
                "node_id": node.node_id, "kind": kind, "funding_outpoint": point, "raw_hex": raw.hex(),
                "txid": decoded["txid"], "fee_sats": 1000})
            require(node.call("sendrawtransaction", [raw.hex()]) == decoded["txid"], "fixture send txid")
            rpc_seq = node.last_observation_seq
            completed = self.capture.add("test_only_witness_result", {"actor": "witness-fixture", "command_id": fixture_id,
                "node_id": node.node_id, "txid": decoded["txid"], "intent_seq": intent_seq, "rpc_seq": rpc_seq})
            self.fixture_effects.append({"intent_seq": intent_seq, "rpc_seq": rpc_seq, "completion_seq": completed})
            mempool = node.call("getrawmempool", [])
            require(decoded["txid"] in mempool, "real witness mempool observation")
            template = node.call("getblocktemplate", [{"mode": "template", "rules": ["segwit"]}])
            decoded_template = [decode_transaction(unhex(item["data"])) for item in template["transactions"]]
            require(any(t["txid"] == decoded["txid"] for t in decoded_template), "witness omitted from actual GBT")
            require(any(not t["witness"] for t in decoded_template), "actual GBT lacks mixed legacy transaction")
            self.capture.add("mixed_witness_gbt", {"kind": kind, "template": template})
            # Permit pool refresh/publication while no native worker hashes.
            refresh_end = min(self.deadline, time.monotonic() + 2)
            while time.monotonic() < refresh_end:
                self.bounded()
                time.sleep(0.1)
            for miner in self.scenario["miners"]:
                self.command("witness-resume-" + kind + "-" + miner["id"],
                             {"type": "resume_workers", "miner_id": miner["id"],
                              "workers": [w["id"] for w in miner["workers"]]})
            def confirmed():
                chain = self.common_chain()
                return chain.transactions.get(decoded["txid"]) if chain else None
            block, actual = self.until(confirmed, 120)
            require(actual["raw"] == raw and actual["fee"] == 1000, "canonical fixture spend bytes/fee")
            proof = witness_commitment(block["transactions"], production=True)
            require(proof is not None, "pool witness block lacks commitment")
            if proof["mixed"]:
                self.result("witness_mixed_block", "PASSED", {"block_hash": block["hash"], "weight": block["weight"]})
            self.capture.add("witness_block_proof", {"height": block["height"], "block_hash": block["hash"],
                                                      "weight": block["weight"], "fees": block["fees"], **proof})
        self.result("witness_spends", "PASSED", "both fixture types canonically spent; job republication gate remains separate")

    def reorg(self):
        if not self.args.allow_reorg:
            return
        # Find an actual confirmed child spending the journaled parent's change.
        chain = self.until(self.common_chain)
        signed = [r for _, rows, _ in self.journals() for r in rows if r["record"] == "signed_tx"]
        pair = None
        for parent in signed:
            if not parent["change"] or parent["txid"] not in chain.transactions:
                continue
            tx = decode_transaction(unhex(parent["raw_hex"]))
            points = {f"{tx['txid']}:{i}" for i, output in enumerate(tx["outputs"])
                      if output["script"].hex() == parent["change"]["script_hex"]}
            for child in signed:
                if child["txid"] in chain.transactions and points & set(child["spent_outpoints"]):
                    pair = parent, child
                    break
        if pair is None and self.args.destination:
            # Bounded production payments consume the selected user's initial
            # distribution outputs; a subsequent confirmed payment must use
            # real change. This creates the desired dependency without signing
            # a second wallet transaction in the harness.
            limit = self.scenario["traffic"]["bootstrap"]["distribution_outputs_per_wallet"] + 1
            user = sorted(i["id"] for i in self.scenario["identities"])[0]
            for index in range(limit):
                label = "reorg-seed-" + str(index)
                self.command(label, {"type": "send_user_payment", "user": user, "destination_address": self.args.destination,
                                    "amount_sats": self.args.amount_sats, "fee_rate_sats_vb": 2})
                target = self.commands[label]["command_id"]
                created = next(r for _, rows, _ in self.journals() for r in rows
                               if r["record"] == "signed_tx" and r["command_id"] == target)
                def seed_confirmed():
                    view = self.common_chain()
                    return view if view and created["txid"] in view.transactions else None
                chain = self.until(seed_confirmed, 45)
                signed = [r for _, rows, _ in self.journals() for r in rows if r["record"] == "signed_tx"]
                for parent in signed:
                    if not parent["change"] or parent["txid"] not in chain.transactions:
                        continue
                    tx = decode_transaction(unhex(parent["raw_hex"]))
                    points = {f"{tx['txid']}:{i}" for i, output in enumerate(tx["outputs"])
                              if output["script"].hex() == parent["change"]["script_hex"]}
                    child = next((c for c in signed if c["txid"] in chain.transactions and points & set(c["spent_outpoints"])), None)
                    if child:
                        pair = parent, child
                        break
                if pair:
                    break
        if pair is None:
            raise Inconclusive("bounded production payments did not produce a confirmed parent/change-child pair")
        parent, child = pair
        driver = next(rows[0]["driver_id"] for _, rows, _ in self.journals()
                      if any(r["record"] == "signed_tx" and r["txid"] == parent["txid"] for r in rows))
        def history_rows():
            return next(rows for _, rows, _ in self.journals() if rows[0]["driver_id"] == driver)
        original_blocks = {txid: chain.transactions[txid][0]["hash"] for txid in (parent["txid"], child["txid"])}
        initial_seq = history_rows()[-1]["seq"]
        displaced = chain.transactions[parent["txid"]][0]
        for miner in self.scenario["miners"]:
            self.command("reorg-pause-" + miner["id"], {"type": "pause_workers", "miner_id": miner["id"],
                                                       "workers": [w["id"] for w in miner["workers"]]})
        self.command("reorg-txd-pause", {"type": "pause_tx_driver", "driver_id": self.driver})
        for node in self.nodes.values():
            node.call("invalidateblock", [displaced["hash"]])
        low = self.until(self.common_chain)
        require(low.tip["height"] < displaced["height"] and parent["txid"] not in low.transactions
                and child["txid"] not in low.transactions, "funded cascade displacement")
        self.capture.add("funded_reorg_displaced", {"parent": parent["txid"], "child": child["txid"],
                                                   "original_block": displaced["hash"], "new_tip": low.tip["hash"]})
        # Wait for the real observer to commit lower-tip proof before restoration.
        first_rollback = self.until(lambda: next((r for r in history_rows() if r["record"] == "chain_rollback"
                                   and r["seq"] > initial_seq and r["evidence"]["new_anchor"]["block_hash"] == low.tip["hash"]), None), 30)
        for node in self.nodes.values():
            node.call("reconsiderblock", [displaced["hash"]])
        def restored():
            view = self.common_chain()
            return view if view and displaced["hash"] in view.by_hash and child["txid"] in view.transactions else None
        self.until(restored, 30)
        def confirmed_originals():
            restored_rows = [r for r in history_rows() if r["record"] == "confirmed" and r["seq"] > first_rollback["seq"]
                             and r["txid"] in original_blocks and r["block_hash"] == original_blocks[r["txid"]]]
            return restored_rows if {r["txid"] for r in restored_rows} == set(original_blocks) else None
        original_confirmations = self.until(confirmed_originals, 30)
        require(not any(r["record"] == "reorg_resubmit" and r["seq"] > first_rollback["seq"] for r in history_rows()),
                "original-block restoration used resubmission")
        self.capture.add("funded_reorg_original_restored", {"block_hash": displaced["hash"]})
        self.result("reorg_original_restore", "PASSED", {"block_hash": displaced["hash"],
                    "confirmations": [r["seq"] for r in original_confirmations]})
        # Separate still-displaced leg: invalidate original again, resume only
        # native miners and production driver. No extra block-search facility.
        for node in self.nodes.values():
            node.call("invalidateblock", [displaced["hash"]])
        second_low = self.until(self.common_chain)
        restored_seq = max(r["seq"] for r in original_confirmations)
        second_rollback = self.until(lambda: next((r for r in history_rows() if r["record"] == "chain_rollback"
                                    and r["seq"] > restored_seq and r["evidence"]["new_anchor"]["block_hash"] == second_low.tip["hash"]), None), 30)
        self.command("reorg-txd-resume", {"type": "resume_tx_driver", "driver_id": self.driver})
        def resubmitted():
            self.common_chain()
            return next((r for r in history_rows() if r["record"] == "reorg_resubmit" and r["seq"] > second_rollback["seq"]
                         and r["txid"] in original_blocks), None)
        resubmit = self.until(resubmitted, 60)
        for miner in self.scenario["miners"]:
            self.command("reorg-resume-" + miner["id"], {"type": "resume_workers", "miner_id": miner["id"],
                                                        "workers": [w["id"] for w in miner["workers"]]})
        def new_branch_confirmed():
            view = self.common_chain()
            return view if view and displaced["hash"] not in view.by_hash and all(t in view.transactions for t in original_blocks) else None
        final_chain = self.until(new_branch_confirmed, 120)
        reader = self.traffic_reader()
        verified_views, _ = reader.journal_anchor_views(self.views, {
            d: mapping["authority_node"] for d, mapping in self.manifest()["traffic_driver_paths"].items()})
        Journal(reader.histories[driver], {"run_id": self.run_id, "driver_id": driver}, verified_views, reader.outcome_proofs)
        self.result("funded_reorg", "PASSED", {"parent": parent["txid"], "child": child["txid"],
                    "original_confirmations_after_rollback": [r["seq"] for r in original_confirmations],
                    "separate_still_displaced_resubmit_seq": resubmit["seq"], "final_tip": final_chain.tip["hash"]})
        self.capture.add("funded_reorg_resubmit_observed", {"original_block": displaced["hash"]})
        self.result("reorg_displaced_retry", "PASSED", {"original_block": displaced["hash"],
                    "intent_seq": resubmit["seq"], "parent": parent["txid"], "child": child["txid"],
                    "raw_parent_sha256": hashlib.sha256(unhex(parent["raw_hex"])).hexdigest(),
                    "raw_child_sha256": hashlib.sha256(unhex(child["raw_hex"])).hexdigest()})
        # Journal validation below rechecks the actual unspent proof and caps.

    def audit_live(self):
        """Shared-run preparation/live facts; final prefix verdicts are deferred."""
        return self.audit(finalize=False)

    def audit(self,*,finalize=True):
        for miner in self.scenario["miners"]:
            self.command("final-pause-" + miner["id"], {"type": "pause_workers", "miner_id": miner["id"],
                                                       "workers": [w["id"] for w in miner["workers"]]})
        self.command("final-txd-pause", {"type": "pause_tx_driver", "driver_id": self.driver})
        chain = self.until(self.common_chain)
        self.result("raw_chain", "PASSED", {"height": chain.tip["height"], "tip": chain.tip["hash"],
                                             "independent_fee_blocks": sum(b["fees"] > 0 for b in chain.blocks[self.measured_height + 1:])})
        manifest = self.manifest()
        def history():
            summaries = []
            audit = self.traffic_reader()
            authority_nodes = {driver: mapping["authority_node"] for driver, mapping in manifest["traffic_driver_paths"].items()}
            verified_views, anchor_proofs = audit.journal_anchor_views(self.views, authority_nodes)
            self.capture.add("actual_journal_anchor_proofs", anchor_proofs)
            for path, rows, hash_ in audit.history_artifacts:
                # Raw archival retrieval is not a canonical-at-time proof:
                # missing independently observed anchors stay inconclusive.
                driver = rows[0]["driver_id"]
                mapping = manifest["traffic_driver_paths"][driver]
                require(Path(mapping["journal_path"]) == path, "manifest journal path binding")
                require(digest(mapping["config_path"]) == mapping["config_sha256"], "actor config binding")
                binding = {"run_id": self.run_id, "driver_id": driver, "allocation_digest": mapping["allocation_digest"]}
                verifier = Journal(rows, binding, verified_views, audit.outcome_proofs)
                summary = {"path": str(path), "sha256": hash_, "balances": verifier.balances()}
                self.capture.add("journal_audit", summary)
                summaries.append(summary)
            return summaries
        self.check("history3", history)
        self.check("hash_budget", self.audit_budget)
        self.check("native_header_observations", self.audit_native)
        self.check("fee_blocks_by_pool", lambda: self.audit_pool_blocks(chain))
        self.check("manual_payment", lambda: self.audit_manual(chain))
        self.check("rpc_positive_funded", lambda: self.audit_rpc(chain))
        self.check("all_sent_raw_observations", self.audit_send_observations)
        self.check("database_capture", self.capture_databases)
        from source_jobs import MissingComponent
        self.check("forward_completeness", lambda: {"actual_forward_attempts": len(self.traffic_reader().attempts),
                                                     "source_schema": "traffic_audit/1"})
        self.check("pause_interval", lambda: self.traffic_reader().pause_intervals())
        self.check("bootstrap_boundary", self.audit_activation)
        self.check("rpc_funded_outcomes", self.audit_funded_outcomes)
        if "hold-payment" in self.hook_commands:
            key = self.hook_commands["hold-payment"]
            self.check("hold_after_sign", lambda: self.traffic_reader().hold_proof(key["driver_id"], key["command_id"]))
        if "loss-payment" in self.hook_commands:
            key = self.hook_commands["loss-payment"]
            self.check("response_loss", lambda: self.traffic_reader().response_loss(key["driver_id"], key["command_id"]))
        def final_drain():
            if self.source_feed_runtime is None:
                raise MissingComponent("actual_checkpoint_retention_and_final_drain_runtime")
            return self.source_feed_runtime.final_drain()
        if finalize:
            self.check("economic_final_drain", final_drain)
            if self.results.get("economic_final_drain",{}).get("status")=="PASSED":self.final_audit_after_drain()
        else:self.capture.add("final_prefix_verdicts_deferred",{"run_id":self.run_id})

    def final_audit_after_drain(self):
        """Consume the exact retained boundary; never stop actors or recapture SQL."""
        from final_audit import FINAL_GATES
        from source_jobs import MissingComponent
        try:
            if self.source_feed_runtime is None:raise MissingComponent("final_audit_collector_unavailable")
            result=self.source_feed_runtime.final_audit_after_drain()
            expected=FINAL_GATES|({"witness_job_republication"} if self.args.witness else set())
            require(set(result["checks"])==expected,"final sealed prefix check census")
            for name,value in result["checks"].items():self.result(name,value["status"],value["detail"])
            states={self.results[name]["status"] for name in result["checks"]}
            state="FAILED" if "FAILED" in states else "INCONCLUSIVE" if "INCONCLUSIVE" in states else "PASSED"
            self.result("final_sealed_prefix",state,result)
            self.capture.add("final_sealed_prefix_audit",result)
            return {**result,"status":self.results["final_sealed_prefix"]["status"]}
        except Exception as error:
            state="FAILED" if isinstance(error,Invalid) else "INCONCLUSIVE"
            for name in FINAL_GATES|{"final_sealed_prefix"}:self.result(name,state,getattr(error,"component",type(error).__name__))
            raise

    def audit_proof_feed(self):
        from source_jobs import MissingComponent
        if self.source_feed_runtime is not None:
            return self.source_feed_runtime.audit_activation()
        if self.source_capture is None or not self.source_capture.components:
            raise MissingComponent("actual_premining_source_components")
        for front, key in self.source_capture.components:
            self.source_capture.for_job(front, key)
        raise MissingComponent("runtime_SourceProofFeed_transport_and_before_publish_ack")

    def audit_funded_outcomes(self):
        if self.results.get("funded_v3_sighash_all", {}).get("status") != "PASSED":
            raise Inconclusive("funded version-3 final signatures have not been verified")
        expected = self.binding.get("definitive_rejection_binding")
        if expected is None:
            raise Inconclusive("run inputs lack the explicit reviewed definitive-rejection binding")
        require(expected["binary_sha256"] == self.binding["binaries"]["node"]["sha256"], "rejection catalog node binary mismatch")
        manifest = self.manifest()
        require(all(profile["profile_fingerprint"] == expected["consensus_fingerprint"]
                    for profile in manifest["effective_profiles"].values()), "rejection catalog live consensus profile mismatch")
        bindings = {driver: load(mapping["config_path"])["node_profile_binding"]
                    for driver, mapping in manifest["traffic_driver_paths"].items()}
        reader = self.traffic_reader()
        rejected = reader.definitive_rejections(bindings, expected)
        if "loss-payment" not in self.hook_commands or self.results.get("acceptance_hook_execution", {}).get("status") != "PASSED":
            raise Inconclusive("actual funded response-loss/mempool observations incomplete")
        key = self.hook_commands["loss-payment"]
        loss = reader.response_loss(key["driver_id"], key["command_id"])
        return {"definitive_rejections": rejected, "actual_success_then_transport_loss": loss}

    def native_payloads(self):
        from storage import private_bytes
        def scalar(path, section, name):
            values, current = [], None
            for line in private_bytes(path, self.run).decode().splitlines():
                line = line.strip()
                if line.startswith("["):
                    current = line
                elif current == "[" + section + "]" and re.match(re.escape(name) + r"\s*=", line):
                    values.append(strict_json(line.split("=", 1)[1].strip()))
            require(len(values) == 1 and isinstance(values[0], str), "actual public pool config scalar missing/ambiguous")
            return values[0]
        actual = load(self.run / "private" / "scenario.json")
        identities = {i["id"]: i["address_source"] for i in actual["identities"]}
        payloads = {}
        for miner in actual["miners"]:
            items = []
            for connection in miner["connections"]:
                front = connection["front_id"]
                config = self.run / "private" / ("pool-" + front + ".toml")
                source = identities[connection["identity_id"]]
                from source_feed_runtime import rendered_address
                address=rendered_address(source)
                workers = []
                for worker in miner["workers"]:
                    if worker["connection_id"] != connection["id"]:
                        continue
                    budget = worker["budget"]
                    workers.append({"id": worker["id"], "budget": {
                        "hashes_per_second": int(budget["hashes_per_second"]) if budget.get("hashes_per_second") is not None else None,
                        "total_attempts": int(budget["total_hashes"]) if budget.get("total_hashes") is not None else None},
                        "ntime_policy": actual["run"]["time_policy"]["miner_ntime"], "max_ntime_roll_seconds": 600, "batch_size": 256})
                endpoint = self.manifest()["resources"]["listeners"]["front." + front + ".sv2"]
                host = endpoint["host"]
                if ":" in host: host = "[" + host + "]"
                items.append({"connection": {"id": connection["id"], "endpoint": host + ":" + str(endpoint["port"]),
                              "identity": address, "authority_public_key": scalar(config, "pool", "authority_public_key"),
                              "min_extranonce_size": 4, "reconnect": {"enabled": True, "initial_backoff_ms": 100,
                               "max_backoff_ms": 5000, "handshake_timeout_ms": 10000}, "workers": workers}})
                if self.native_capture is not None:
                    items[-1]["connection"] = self.native_capture.connection(miner["id"], connection, items[-1]["connection"])
            payloads[miner["id"]] = items
        return payloads

    def audit_activation(self):
        payloads = self.native_payloads()
        manifest_boundary = self.manifest()["traffic_measured_from"]
        view = next((v for v in self.views.values() if manifest_boundary["tip_hash"] in v.by_hash), None)
        if view is None or manifest_boundary["tip_hash"] not in self.raws:
            raise Inconclusive("independent raw measured boundary missing")
        native = []
        for path in (self.run / "private" / "traffic").glob("d-*/native-bootstrap.jsonl"):
            native += [strict_json(line) for line in private(path).splitlines()]
        confirmations = [r for r in native if r["event"] == "distribution_canonical"]
        require(len(confirmations) == 1, "actual canonical distribution completion missing")
        return self.traffic_reader().activation_proof(manifest_boundary, view, self.raws[manifest_boundary["tip_hash"]],
                                                     confirmations[0]["txids"], payloads)

    def node_api_records(self):
        records = {}
        for node_id in self.nodes:
            path = self.run / "private" / ("node-api-" + node_id + ".jsonl")
            data = private(path)
            if not data.endswith(b"\n"):
                raise Inconclusive("node API log absent/truncated")
            records[node_id] = [strict_json(line) for line in data.splitlines()]
        return records

    def audit_send_observations(self):
        from effect_counts import reconcile_node_sends
        data = private(self.capture.path)
        if not data.endswith(b"\n"):
            raise Inconclusive("observer effect trace truncated")
        observations = [strict_json(line) for line in data.splitlines()]
        return reconcile_node_sends(self.traffic_reader(), self.node_api_records(), observations, self.fixture_effects)

    def audit_budget(self):
        path = self.run / "private" / "commands.jsonl"
        data = private(path)
        if not data.endswith(b"\n"):
            raise Inconclusive("truncated command-log budget history")
        records = [strict_json(line) for line in data.splitlines()]
        result = hash_budget(records, self.scenario["traffic"]["driver"]["bootstrap"]["total_attempts"])
        self.capture.add("hash_budget_audit", {"command_log_sha256": digest(path), **result})
        return result

    def audit_native(self):
        command_records = [strict_json(line) for line in private(self.run / "private" / "commands.jsonl").splitlines()]
        records = []
        for path in (self.run / "private" / "traffic").glob("d-*/native-bootstrap.jsonl"):
            data = private(path)
            if not data.endswith(b"\n"):
                raise Inconclusive("truncated native diagnostics")
            records += [strict_json(line) for line in data.splitlines()]
        return native_observations(records, command_records, self.views, self.policy, self.started_wall_seconds - 5)

    def audit_pool_blocks(self, chain):
        front_pools = {f["id"]: f["pool_id"] for f in self.scenario["fronts"]}
        attributed = {p["id"]: set() for p in self.scenario["pools"]}
        ansi = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
        for process in self.manifest()["processes"]:
            if process["entity"] not in front_pools:
                continue
            path = Path(process["log_path"])
            pool = front_pools[process["entity"]]
            for line in path.read_text().splitlines():
                clean = ansi.sub("", line)
                if "spc_evidence" not in clean or 'event="accounted"' not in clean:
                    continue
                if "evidence=" not in clean:
                    raise Inconclusive("accounted evidence shape")
                evidence = strict_json(clean.split("evidence=", 1)[1])
                block_hash = unhex(evidence["share_hash"], 32)[::-1].hex()
                block = chain.by_hash.get(block_hash)
                if block and block["height"] > self.measured_height and block["fees"] > 0:
                    attributed[pool].add(block_hash)
        count = sum(len(v) for v in attributed.values())
        if count < 3 or any(not v for v in attributed.values()):
            raise Inconclusive("need three measured canonical fee blocks, at least one per pool")
        require(len(set().union(*attributed.values())) == count, "block attributed to two pools")
        return {pool: sorted(hashes) for pool, hashes in attributed.items()}

    def audit_manual(self, chain):
        if "manual" not in self.commands:
            raise Inconclusive("explicit manual destination was not supplied")
        cid = self.commands["manual"]["command_id"]
        rows = [r for _, records, _ in self.journals() for r in records
                if r["record"] == "signed_tx" and r["command_id"] == cid]
        require(len(rows) == 1, "same durable manual ID must create exactly one transaction")
        row = rows[0]
        if row["txid"] not in chain.transactions:
            raise Inconclusive("manual payment not canonically confirmed")
        block, tx = chain.transactions[row["txid"]]
        require(tx["raw"] == unhex(row["raw_hex"]) and tx["fee"] > 0, "manual canonical fee-bearing bytes")
        require(len(row["destinations"]) == 1 and row["destinations"][0]["amount_sats"] == self.args.amount_sats,
                "manual requested amount")
        # Exact requested address decoding is required, independently of the
        # journal's claimed destination script.
        from oracle import address_script
        requested = address_script(self.args.destination)
        require(row["destinations"][0]["script_hex"] == requested.hex(), "manual requested destination")
        require(any(o["script"] == requested and o["value"] == self.args.amount_sats for o in tx["outputs"]),
                "manual output not in raw transaction")
        return {"txid": tx["txid"], "height": block["height"], "fee_sats": tx["fee"]}

    def audit_rpc(self, chain):
        signed = [r for _, rows, _ in self.journals() for r in rows if r["record"] == "signed_tx"]
        confirmed = next((r for r in signed if r["txid"] in chain.transactions), None)
        if confirmed is None:
            raise Inconclusive("funded transaction RPC proof unavailable")
        block, tx = chain.transactions[confirmed["txid"]]
        for node in self.nodes.values():
            mempool = node.call("getrawmempool", [])
            require(isinstance(mempool, list), "mempool RPC array")
            for txid in mempool:
                unhex(txid, 32)
            info = node.call("getmempoolinfo", [])
            integer(info["size"])
            for params in ([tx["txid"], 0], [tx["txid"], 0, block["hash"]]):
                require(unhex(node.call("getrawtransaction", params)) == tx["raw"], "raw transaction RPC byte evidence")
            require(unhex(node.call("getblock", [block["hash"], 0]))[:80] == block["raw"], "raw block RPC header evidence")
        return {"canonical_transaction": tx["txid"], "negative_transport_and_rejection_cases": "separate required gate"}

    def capture_databases(self):
        from storage import collect_database
        captures = []
        for db in self.manifest()["resources"]["postgres_databases"]:
            name = "database-" + hashlib.sha256(db["id"].encode()).hexdigest()[:16] + ".json"
            captures.append(collect_database(self.run, db["id"], self.binding["binaries"]["psql"],
                                            self.binding["binaries"]["traffic-probe"],
                                            self.binding["binaries"]["postgres"], self.priv / name,
                                            self.capture.add))
        return captures

    def stop_bootstrap_case(self):
        operation_id = self.bootstrap_operation()
        self.until(lambda: self.manifest().get("traffic_stage") == "bootstrapping", 10)
        start = time.monotonic()
        self.command("stop-during-bootstrap", {"type": "stop_run", "reason": "bounded acceptance cancellation"})
        elapsed = time.monotonic() - start
        require(elapsed <= self.scenario["traffic"]["driver"]["bootstrap"]["cancel_grace_secs"], "bootstrap cancellation deadline")
        def cancelled():
            status = self.bootstrap_status()
            if status["state"] == "observed_in_flight":return None
            require(status["state"] == "cancelled", "original Runtime bootstrap was not cancelled")
            return status
        status = self.until(cancelled, self.scenario["traffic"]["driver"]["bootstrap"]["cancel_grace_secs"], allow_controller_exit=True)
        self.result("bootstrap_stop", "PASSED", {"elapsed_seconds": elapsed, "operation_id": operation_id,
            "original_principal": status["principal"], "original_budget": status["budget"], "debits": status["debits"]})

    def bootstrap_maturity_case(self):
        require(self.args.allow_reorg, "bootstrap maturity case requires explicit owned-node reorg permission")
        operation_id = self.bootstrap_operation()
        # Actual actor/bootstrap_control.rs command-id contract for part zero.
        target = self.phase_bootstrap.plan["actor_command_id"]
        funding_height = self.scenario["traffic"]["bootstrap"]["funding_blocks"]
        def approaching_distribution():
            view = self.common_chain()
            return view if view and view.tip["height"] >= funding_height - 20 else None
        self.until(approaching_distribution, self.args.timeout)
        # Hook deadlines are at most 120 s. Arm late, with the genuine native
        # operation explicitly paused, rather than spend that deadline mining
        # the initial hundred blocks. The bootstrap's own deadline/debit stays.
        self.command("maturity-native-pause", {"type": "bootstrap_control", "operation_id": operation_id, "action": "pause"})
        paused = self.until(self.common_chain)
        require(paused.tip["height"] < funding_height, "native operation reached signing before acceptance hold could be armed")
        key = self.arm_hook("maturity-bootstrap", self.driver, target, "hold_before_intent")
        self.command("maturity-native-resume", {"type": "bootstrap_control", "operation_id": operation_id, "action": "resume"})
        def signed_ready():
            self.common_chain()
            return self.hook_signed(key)
        checkpoint = self.until(signed_ready, self.args.timeout)
        high = self.until(self.common_chain)
        history = next(rows for _, rows, _ in self.journals() if rows[0]["driver_id"] == self.driver)
        signed = [r for r in history if r["record"] == "signed_tx" and r["command_id"] == target]
        require(len(signed) == 1, "maturity hold has no unique actual SignedTx")
        tx = decode_transaction(unhex(signed[0]["raw_hex"]))
        inputs = [i["outpoint"] for i in tx["inputs"]]
        require(inputs and all(p in high.utxos and high.utxos[p]["coinbase"] for p in inputs), "maturity case must retain actual coinbase inputs")
        low_height = max(high.utxos[p]["height"] + self.policy.coinbase_maturity - 1 for p in inputs)
        require(low_height < high.tip["height"] and all(high.utxos[p]["height"] <= low_height for p in inputs),
                "cannot produce retained-coinbase lower-tip case")
        removed = high.blocks[low_height + 1]["hash"]
        for node in self.nodes.values():
            node.call("invalidateblock", [removed])
        low = self.until(self.common_chain)
        require(low.tip["height"] == low_height and all(p in low.utxos for p in inputs), "coinbase funding displaced instead of retained")
        immature = [p for p in inputs if low.tip["height"] - low.utxos[p]["height"] < self.policy.coinbase_maturity]
        require(immature, "retained coinbase is still mature")
        def rollback_seen():
            return any(r["record"] == "chain_rollback" and r["evidence"]["new_anchor"]["block_hash"] == low.tip["hash"]
                       for _, rows, _ in self.journals() for r in rows)
        self.until(rollback_seen, 30)
        self.release_hook("maturity-bootstrap", key, checkpoint)
        def failed_operation():
            status = self.bootstrap_status()
            return status["terminal"] if status["state"] in {"failed", "cancelled"} else None
        terminal = self.until(failed_operation, 45, allow_controller_exit=True)
        reader = self.traffic_reader()
        history = reader.histories[self.driver]
        require(not any(r["record"] in {"broadcast_sent", "reorg_resubmit"} and r["command_id"] == target for r in history),
                "immature retained coinbase acquired a send intent")
        require(not any(r["event"]["attempt"]["method"] == "sendrawtransaction"
                        and r["event"]["attempt"]["context"]["command_id"] == target for r in reader.attempts.values()),
                "immature retained coinbase was forwarded")
        native = []
        for path in (self.run / "private" / "traffic").glob("d-*/native-bootstrap.jsonl"):
            native += [strict_json(line) for line in private(path).splitlines()]
        ends = [r for r in native if r["event"] == "terminal"]
        expected_errors = {"bootstrap_actor_failed: " + json.dumps("journal error: journal integrity: send ancestry " + p +
                           " is invalidated or immature at committed tip") for p in immature}
        if len(ends) != 1 or ends[0].get("success") is not False or ends[0].get("error") not in expected_errors:
            raise Inconclusive("actual core maturity refusal needs its exact source-bound terminal error capture")
        self.result("retained_maturity_rollback", "PASSED", {"signed_command_id": target, "retained_coinbase_inputs": inputs,
                    "signed_raw_sha256": hashlib.sha256(tx["raw"]).hexdigest(), "high_tip_height": high.tip["height"], "low_tip_height": low_height,
                    "new_intents": 0, "actual_forward_attempts": 0, "bootstrap_terminal_state": terminal["outcome"]["state"]})

    def cleanup(self):
        runtime=self.source_feed_runtime
        if runtime is None:return self._cleanup_owned()
        if not runtime.finalize_lock.acquire(blocking=False):
            from source_jobs import MissingComponent
            raise MissingComponent("collector_finalization_already_active_no_controller_teardown")
        try:return self._cleanup_owned(runtime._close)
        finally:runtime.finalize_lock.release()

    def _cleanup_owned(self,collector_close=None):
        from cleanup_proof import assess_cleanup, collect_resource_receipt, identity_observation
        if self.source_feed_runtime is not None:
            self.check("source_feed_cleanup", collector_close)
            if self.results["source_feed_cleanup"]["status"]!="PASSED":
                self.result("cleanup","INCONCLUSIVE","collector/source/checkpoint workers did not close completely")
        if self.process:
            if self.process.poll() is None:
                for label, key in self.hook_commands.items():
                    try:
                        self.command(label + "-cleanup-cancel", {"type": "traffic_acceptance", "key": key,
                                                                "control": {"action": "cancel"}})
                    except Exception:
                        pass
                try:
                    self.http("commands", {"command_id": str(uuid.uuid4()), "run_id": self.run_id,
                                           "issued_at_wall": time.time_ns() // 1_000_000, "actor": "acceptance",
                                           "kind": {"type": "stop_run", "reason": "acceptance cleanup"}})
                except Exception:
                    pass
                try:
                    self.process.wait(timeout=35)
                except subprocess.TimeoutExpired:
                    os.killpg(self.process.pid, signal.SIGTERM)
                    try:
                        self.process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(self.process.pid, signal.SIGKILL)
                        self.process.wait(timeout=5)
            self.capture.add("command_exit", {"pid": self.process.pid, "exit_code": self.process.returncode})
            # Only the production reaper interprets its own full ownership
            # records. Never kill by bare PID or by executable name.
            before = self.manifest()
            if before is None:
                self.result("cleanup", "INCONCLUSIVE", {"code": "cleanup_manifest_missing"})
                return
            try:
                reaped = subprocess.run([self.binaries["lab"], "reap", "--run-dir", str(self.run)],
                                        capture_output=True, timeout=35)
            except subprocess.TimeoutExpired:
                self.result("cleanup", "INCONCLUSIVE", {"code": "reaper_timeout"})
                return
            self.capture.add("reap_exit", {"exit_code": reaped.returncode,
                "stdout_sha256": hashlib.sha256(reaped.stdout).hexdigest(), "stdout_bytes": len(reaped.stdout),
                "stderr_sha256": hashlib.sha256(reaped.stderr).hexdigest(), "stderr_bytes": len(reaped.stderr)})
            if reaped.returncode != 0:
                self.result("cleanup", "FAILED", {"code": "reaper_nonzero", "exit_code": reaped.returncode})
                return
            after = self.manifest()
            if after is None:
                self.result("cleanup", "INCONCLUSIVE", {"code": "post_reap_manifest_missing"})
                return
            identities = {(p["role"], p["entity"]): identity_observation(p) for p in after["processes"]}
            survivors = [list(key) for key, value in identities.items() if value["status"].startswith("surviving")]
            if survivors:
                self.result("cleanup", "FAILED", {"code": "owned_process_survived", "survivors": survivors})
                return
            resource_receipt = collect_resource_receipt(self.run, after)
            write_private(self.priv / "cleanup-resources.json", resource_receipt)
            verdict = assess_cleanup(reaped.returncode, reaped.stdout, before, after, identities, resource_receipt)
            self.result("cleanup", verdict["status"], verdict)

    def receipt(self):
        chain_export = None
        if self.phase_bootstrap is not None and (self.run / "private/commands.jsonl").exists():
            self.check("runtime_bootstrap_ownership", lambda: self.bootstrap_status(require_phase=True))
        if self.canonical is not None:
            try:
                chain_export = self.canonical.export(self.priv / "chain-evidence.json", run_id=self.run_id,
                    binary_hashes={k: v["sha256"] for k, v in self.binding["binaries"].items()},
                    scenario_sha256=digest(self.priv / "scenario.json"),
                    source_snapshot_sha256=self.binding["source_snapshot_sha256"])
                self.capture.add("chain_evidence_export", chain_export)
                if not any(c["status"] == "consistent_common_fence" for c in self.canonical.cohorts):
                    raise Inconclusive("no complete canonical observation to export")
                self.result("chain_evidence_export", "PASSED", chain_export)
            except (OSError, ValueError, Invalid, Inconclusive) as error:
                self.result("chain_evidence_export", "INCONCLUSIVE", type(error).__name__)
        obligations = required(self.args.case, self.args.witness)
        for case in obligations:
            if case not in self.results:
                self.result(case, "INCONCLUSIVE", "required evidence not yet verified")
        status = verdict(self.results, obligations)
        self.capture.stream.close()
        receipt = {"schema": "spc-lab-funded-acceptance-receipt/2", "status": status,
                   "scope": "campaign-case-only", "case": self.args.case, "required_cases": sorted(obligations),
                   "run_id": self.scenario.get("run", {}).get("id"),
                   "bindings": {"inputs_sha256": digest(self.args.inputs), "scenario_sha256": digest(self.args.scenario),
                                "scenario_semantic_sha256": semantic_scenario(self.scenario) if self.scenario else None,
                                "source_snapshot_sha256": self.binding.get("source_snapshot_sha256")},
                   "run_root": str(self.root), "cases": self.results,
                   "observations_sha256": digest(self.capture.path),
                   "chain_evidence": chain_export,
                   "binary_hashes": {k: v["sha256"] for k, v in self.binding.get("binaries", {}).items()},
                   "full_ark_acceptance": False,
                   "reproduce": [sys.executable, "-B", str(Path(__file__).absolute()),
                                 "--inputs", str(Path(self.args.inputs).absolute()),
                                 "--scenario", str(Path(self.args.scenario).absolute()),
                                 "--run-root", "<NEW-PRIVATE-RUN-ROOT>", "--case", self.args.case]}
        write_private(self.root / "acceptance.json", receipt)
        lines = ["# Funded lab acceptance", "", "Status: **" + status + "**", "",
                 "| Case | Result |", "|---|---|"]
        lines += [f"| {case} | {self.results[case]['status']} |" for case in sorted(self.results)]
        lines += ["", "Private raw observations: `private/observations.jsonl`.",
                  "No offline fixture contributes to live acceptance. Ark service lifecycle remains a separate gate."]
        write_private(self.root / "acceptance.md", "\n".join(lines).encode())
        print(json.dumps({"status": status, "receipt": str(self.root / "acceptance.json")}))
        return {"PASSED": 0, "FAILED": 1, "INCONCLUSIVE": 2}[status]


def finalize_controller(controller):
    """Always publish/close the receipt after cleanup failures as well."""
    try:
        controller.cleanup()
    except Exception as error:
        controller.capture.add("cleanup_incomplete", {"class": type(error).__name__})
        if controller.results.get("cleanup", {}).get("status") != "FAILED":
            controller.result("cleanup", "INCONCLUSIVE", "cleanup verification incomplete")
    finally:
        try:
            controller.http_workers.shutdown(wait=True, cancel_futures=True)
        except Exception as error:
            controller.capture.add("worker_cleanup_incomplete", {"class": type(error).__name__})
            if controller.results.get("cleanup", {}).get("status") != "FAILED":
                controller.result("cleanup", "INCONCLUSIVE", "HTTP worker cleanup incomplete")
    if isinstance(controller, Controller) and controller.process is not None:
        controller.check("hash_budget", controller.audit_budget)
    try:
        return controller.receipt()
    finally:
        controller.capture.stream.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inputs", required=True, help="explicit sealed actual binaries/source/profile JSON")
    parser.add_argument("--scenario", required=True)
    parser.add_argument("--run-root", required=True, help="new private owned directory; existing paths refused")
    parser.add_argument("--case", choices=("funded", "bootstrap-stop", "bootstrap-maturity"), default="funded")
    parser.add_argument("--timeout", type=int, default=900)
    parser.add_argument("--start-timeout", type=int, default=180)
    parser.add_argument("--measure-seconds", type=int, default=90)
    parser.add_argument("--pause-seconds", type=int, default=5)
    parser.add_argument("--max-blocks", type=int, default=4096)
    parser.add_argument("--destination", help="explicit manual payment address")
    parser.add_argument("--amount-sats", type=int, default=10000)
    parser.add_argument("--allow-reorg", action="store_true", help="explicit permission for owned-node invalidate/reconsider")
    parser.add_argument("--witness", action="store_true", help="two public fixture keys, production funding, owned test send channel")
    parser.add_argument("--operator-ui", action="store_true", help="explicit host-runtime pool API operator listeners for shared AP5 work")
    args = parser.parse_args()
    for value in (args.timeout, args.start_timeout, args.measure_seconds, args.pause_seconds, args.max_blocks):
        integer(value, 1, 86400)
    os.umask(0o077)
    controller = Controller(args)
    try:
        controller.prerequisites()
        controller.start()
        if args.case == "bootstrap-stop":
            controller.stop_bootstrap_case()
        elif args.case == "bootstrap-maturity":
            controller.bootstrap_maturity_case()
        else:
            controller.bootstrap()
            controller.measured()
            controller.reorg()
            controller.audit()
    except Invalid as error:
        controller.result("execution", "FAILED", str(error))
    except (Inconclusive, OSError, KeyError, ValueError, TypeError, IndexError, AttributeError, StopIteration) as error:
        # Unknown schema/preconditions are not a successful test, and public
        # receipts never include raw exception text that might carry secrets.
        controller.capture.add("execution_incomplete", {"class": type(error).__name__, "detail": str(error)})
        controller.result("execution", "INCONCLUSIVE", type(error).__name__)
    except Exception as error:
        controller.capture.add("execution_incomplete", {"class": type(error).__name__})
        controller.result("execution", "INCONCLUSIVE", "unexpected checker failure")
    finally:
        exit_code = finalize_controller(controller)
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
