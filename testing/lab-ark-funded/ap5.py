#!/usr/bin/env python3
"""Headless AP5 controller. All hashing/signing belongs to the joined runtime.

Only a fresh run is accepted. Missing joins or client operations are concrete
dependency failures, never replaced with inert daemons or synthetic receipts.
"""
import argparse
import json
import os
from pathlib import Path
import secrets
import signal
import subprocess
import time
import tomllib

from common import (Blocked, Evidence, Node, artifact, check_bundle, command_id,
                    confirmed_transaction, encoded, fresh_root, load, readonly_rows,
                    request, require, sats, sha, status_ready, stock_id_bytes, wallet_snapshot, same_json_value)
from scenario import USERS, POOLS, FLOAT_TARGET, PAID_WINDOW, scenario
from pool_control import Operation, fresh_snapshot, TERMINAL_STATES, strict_json
from board_proof import bind_retained_proof
from cleanup import Census, coverage_errors, REQUIRED_ROLES
from component_verify import (public_vtxo, raw_transaction, board_policy, round_value,
                              offboard_value, final_verdict, mandatory_components, replay_component, native_funding_verdict)
from source_scope import NATIVE_COMPONENTS, scenario_template_sha256


def process_identity(pid):
    text = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    return int(text[19]), text[0]


def alive(pid, ticks):
    try:
        actual, state = process_identity(pid)
        return actual == ticks and state != "Z"
    except (OSError, ValueError):
        return False


def p2pkh_address(script):
    """Public T1 receive script -> address, no seed or signing capability."""
    import hashlib
    raw = bytes.fromhex(script)
    require(len(raw) == 25 and raw[:3] == b"\x76\xa9\x14" and raw[-2:] == b"\x88\xac",
            "offboard return must be an actually allocated T1 P2PKH script")
    payload = b"\x6f" + raw[3:23]
    payload += hashlib.sha256(hashlib.sha256(payload).digest()).digest()[:4]
    n, result = int.from_bytes(payload, "big"), ""
    alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
    while n:
        n, digit = divmod(n, 58)
        result = alphabet[digit] + result
    return result


class Controller:
    def __init__(self, bundle, paths, root, evidence, spec, timeout, scope="full"):
        require(scope in {"full", "confirmed-funding"}, "unknown controller scope")
        self.scope, self.timeout_secs = scope, timeout
        if scope == "confirmed-funding":
            from received_profile import supports, runtime_policy
            policy = runtime_policy(bundle) if supports(bundle) else load(artifact(bundle["diagnostic_source_policy"]))
            require(0 < timeout <= policy["legacy_maintenance"]["max_runtime_secs"], "diagnostic runtime exceeds tested maintenance bound")
            require(scenario_template_sha256(spec) == policy["runtime_bindings"]["scenario_template_sha256"], "diagnostic scenario differs from source-bound profile")
        self.bundle, self.paths, self.root, self.ev, self.spec = bundle, paths, root, evidence, spec
        tool_dirs = {str(Path(path).parent) for path in paths.values()}
        if bundle.get("postgres_bin_dir"):
            tool_dirs.add(bundle["postgres_bin_dir"])
        self.execution_path = os.pathsep.join(sorted(tool_dirs))
        self.run = root / "run"
        self.id = spec["run"]["id"]
        self.child = None
        self.url = None
        self.token = None
        self.nodes = []
        self.deadline = time.monotonic() + timeout
        self.owned = {}
        self.census = Census(self.run)
        self.tool_token = "ap5:" + self.id
        self.census.tokens.add(self.tool_token)
        self.components = []
        self.proofs = {}
        self.manual_operations = []
        self.pool_requests = {}
        self.log = None
        self.reap_record = None
        self.cleanup = {"schema": "spc.lab.ap5.cleanup/v2", "status": "inconclusive",
                        "exhaustive": False, "all_resources_released": False}

    def manifest(self):
        value = load(self.run / "run-manifest.json")
        for row in value["processes"]:
            self.owned[(row["pid"], row["start_time_ticks"])] = row["role"]
        controller = None if self.child is None else {"pid": self.child.pid, "ticks": self.controller_ticks}
        self.census.capture(value, controller)
        return value

    def component(self, identity, transactions=(), vtxos=(), **extra):
        if self.scope == "confirmed-funding":
            require(identity in NATIVE_COMPONENTS and not vtxos and set(extra) <= {"payments", "ark_info"}, "diagnostic accepts native funding components only")
        facts = self.nodes[0].read("getblockchaininfo")["consensus"]
        maturity = facts["coinbase_maturity"]
        require(type(maturity) is int and maturity > 0, "actual maturity policy missing")
        policy = extra.pop("ark_info", None)
        data = {"schema": "spc.lab.ap5.raw-component/v2", "id": identity, "maturity": maturity,
                "transactions": list(transactions), "vtxos": list(vtxos),
                "ark_info": policy if policy is not None else self.ark_policy(), **extra}
        file = self.ev.save("raw-component", data)
        record = {"id": identity, "status": "inconclusive", "evidence": {"file": file.name, "sha256": sha(file)}}
        self.components.append(record)
        try:
            result = replay_component(self.root, record, self.bundle.get("vtxo_verifier"), self.run_tool)
        except Exception:
            record["status"] = "failed"
            raise
        record["status"] = "passed"
        record["requires"] = result["requires"]
        record["verification"] = result
        self.ev.save("component-verdict", record)
        return result

    def verify_public_proof(self, raw_hex, decoded, owner, policy=None, depth=1):
        self.require_full()
        info = policy if policy is not None else self.ark_policy()
        expected = {"user_pubkey": owner, "server_pubkey": info["bound"]["server_pubkey"],
                    "amount_sats": decoded["amount_sat"], "expiry_height": decoded["expiry_height"],
                    "exit_delta": info["ark_info"]["vtxoExitDelta"],
                    "max_exit_depth": info["ark_info"]["maxVtxoExitDepth"]}
        anchor = decoded["chain_anchor"].split(":")[0]
        views = self.wait("proof-anchor", lambda: confirmed_transaction(self.nodes, anchor, depth))
        proof = {"raw_hex": raw_hex, "anchor_views": views, "expected": expected}
        maturity = self.nodes[0].read("getblockchaininfo")["consensus"]["coinbase_maturity"]
        value = public_vtxo(self.bundle["vtxo_verifier"], raw_hex, views, expected, maturity, self.run_tool)
        self.proofs[decoded["id"]] = proof
        self.ev.save("full-vtxo-proof-verified", {"proof": proof, "derived": value})
        return value

    def user_stock_keys(self, user):
        db = self.run / f"private/wallet-{user}/state/bark/db.sqlite"
        return {r["public_key"] for r in readonly_rows(db, "SELECT public_key FROM bark_vtxo_key")}

    def run_tool(self, argv, input=None, timeout=30, env=None):
        require(env is None or env.get("PATH", self.execution_path) == self.execution_path, "auxiliary PATH differs from admitted tool closure")
        # All auxiliary clients use the real actor-exec PDEATHSIG helper and
        # contribute actual PID/start identities to the cleanup census.
        parent = os.getpid()
        ticks = process_identity(parent)[0]
        command = [str(self.paths["lab"]), "actor-exec", "--parent-pid", str(parent),
                   "--parent-start-ticks", str(ticks), "--", *map(str, argv)]
        self.ev.save("tool-intent", {"executable": str(argv[0]), "executable_sha256": sha(argv[0]),
                                     "argv_sha256": __import__("hashlib").sha256(encoded(list(map(str, argv)))).hexdigest()})
        child = subprocess.Popen(command, stdin=subprocess.PIPE if input is not None else subprocess.DEVNULL,
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True,
                                  env={"PATH": self.execution_path, "HOME": str(self.root), "LC_ALL": "C", **(env or {}),
                                      "SPC_LAB_RUN_TOKEN": self.tool_token})
        who = {"pid": child.pid, "ticks": process_identity(child.pid)[0]}
        self.census.add(who, "tool:" + Path(argv[0]).name)
        try:
            stdout, stderr = child.communicate(input, timeout=timeout)
        except BaseException:
            if alive(who["pid"], who["ticks"]):
                os.killpg(child.pid, signal.SIGKILL)
            stdout, stderr = child.communicate()
            self.ev.save("tool-exit", {**who, "exit": child.returncode, "interrupted": True,
                                      "stdout_hex": stdout.hex(), "stderr_hex": stderr.hex(),
                                      "stdout_sha256": __import__("hashlib").sha256(stdout).hexdigest(),
                                      "stderr_sha256": __import__("hashlib").sha256(stderr).hexdigest()})
            raise
        self.ev.save("tool-exit", {**who, "exit": child.returncode, "timeout": False,
                                  "stdout_hex": stdout.hex(), "stderr_hex": stderr.hex(),
                                  "stdout_sha256": __import__("hashlib").sha256(stdout).hexdigest(),
                                  "stderr_sha256": __import__("hashlib").sha256(stderr).hexdigest()})
        return subprocess.CompletedProcess(argv, child.returncode, stdout, stderr)

    def tool_output(self, argv, **kwargs):
        result = self.run_tool(argv, **kwargs)
        require(result.returncode == 0, "source-bound auxiliary tool failed: " + Path(argv[0]).name)
        return result.stdout

    def ark_policy(self):
        import base64
        original = load(self.run / "private/ark-info.json")
        endpoint = original["endpoint"].removeprefix("http://")
        require(endpoint.startswith("127.0.0.1:"), "foreign Ark policy endpoint")
        proto = self.paths["captaind"].parent.parent / "share/ark/protos"
        raw = self.tool_output([str(self.paths["grpcurl"]), "-plaintext", "-emit-defaults", "-max-time", "10",
            "-import-path", str(proto), "-proto", "bark_server.proto", endpoint, "bark_server.ArkService/GetArkInfo"], timeout=15)
        value = json.loads(raw)
        require(value["network"] == "regtest", "foreign Ark network")
        require(base64.b64decode(value["serverPubkey"], validate=True).hex() == original["bound"]["server_pubkey"]
                and base64.b64decode(value["mailboxPubkey"], validate=True).hex() == original["bound"]["mailbox_pubkey"], "Ark server/mailbox binding changed")
        import hashlib
        bound = dict(original["bound"])
        bound["fees_sha256"] = hashlib.sha256(encoded(value["fees"])).hexdigest()
        observation = {"endpoint": original["endpoint"], "bound": bound, "ark_info": value,
                       "observed_at_unix_ms": time.time_ns() // 1_000_000, "raw_response_hex": raw.hex()}
        self.ev.save("live-ark-policy", observation)
        return observation

    def wait(self, label, observe, timeout=600):
        deadline = min(self.deadline, time.monotonic() + timeout)
        last = None
        while time.monotonic() < deadline:
            require(self.child is None or self.child.poll() is None, "controller exited: " + label)
            try:
                last = observe()
                if last is not None and last is not False:
                    self.ev.save(label, last)
                    return last
            except (FileNotFoundError, json.JSONDecodeError):
                pass  # Atomic readiness artifacts may not exist yet.
            time.sleep(0.5)
        self.ev.save(label + "-timeout", {"last_observation": last, "outcome": "unknown"})
        raise Blocked("deadline without evidence: " + label)

    def start(self):
        scenario_path = self.ev.save("scenario", self.spec)
        argv = [str(self.paths["lab"]), "runtime", "--scenario", str(scenario_path), "--run-dir", str(self.run),
                "--base-revision", self.bundle["base_revision"],
                "--dirty-tree-hash", self.bundle["joined_source_manifest"]["sha256"],
                "--listen", "127.0.0.1:0", "--operator-ui", "--duration-secs", str(self.timeout_secs),
                "--postgres-bin-dir", self.bundle["postgres_bin_dir"]]
        for flag, name in [("node", "node"), ("pool", "pool"), ("miner", "miner"), ("scanner", "scanner"),
                           ("wallet-agent-bin", "agent"), ("txd", "txd"), ("traffic-probe", "probe_program"),
                           ("captaind", "captaind"), ("grpcurl", "grpcurl"),
                           ("redis-server", "redis-server"), ("redis-cli", "redis-cli")]:
            argv.extend(["--" + flag, str(self.paths[name])])
        binding = self.bundle["probe_invocation"]
        require(binding["program"]["path"] == str(self.paths["probe_program"]), "runtime probe differs from admitted program")
        for argument in binding["prefix_args"]:
            argv.extend(["--traffic-probe-arg", argument])
        self.ev.save("probe-invocation", binding)
        self.ev.save("launch", {"argv": argv, "scope": "actual AP5 default; no inert actors"})
        self.log = (self.root / "controller.log").open("xb")
        os.chmod(self.root / "controller.log", 0o600)
        self.child = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=self.log, stderr=subprocess.STDOUT,
                                      start_new_session=True,
                                       env={"PATH": self.execution_path, "HOME": str(self.root), "LC_ALL": "C",
                                           "SPC_LAB_RUN_TOKEN": self.tool_token})
        self.controller_ticks = process_identity(self.child.pid)[0]
        self.census.add({"pid": self.child.pid, "ticks": self.controller_ticks}, "controller")
        def http_ready():
            for line in (self.root / "controller.log").read_text().splitlines():
                if line.startswith("spc-lab-runtime: http="):
                    self.url = line.split("=", 1)[1].strip()
            claim = self.run / "private/access/operator.json"
            if self.url and claim.exists():
                self.token = load(claim)["token"]
                return {"url": self.url}
        self.wait("controller-ready", http_ready, 180)
        self.observe_started_runtime()

    def adopt_shared_runtime(self, owner):
        """Bind this observer to the same in-process owner's newly spawned runtime.

        This is not disk adoption and never starts or bootstraps a runtime.
        It also binds partial startup so the coordinator can clean up on error.
        """
        require(self.child is None or self.child is owner.process, "shared runtime process replaced")
        require(owner.run_id == self.id and same_json_value(owner.scenario, self.spec), "shared runtime scenario/run mismatch")
        require(owner.run.resolve().is_relative_to(owner.root.resolve()), "shared runtime outside owner root")
        if self.run != owner.run:
            require(not self.owned and self.child is None, "cannot replace an observed run")
            self.run = owner.run
            self.census = Census(self.run)
            self.census.tokens.add(self.tool_token)
        self.child = owner.process
        self.url = owner.url
        self.deadline = min(self.deadline, owner.deadline)
        if self.child is not None:
            try:
                self.controller_ticks = process_identity(self.child.pid)[0]
            except FileNotFoundError:
                require(self.child.poll() is not None, "live shared controller identity unavailable")
                self.controller_ticks = 0  # Exited child: never used as signal authority.
            if self.controller_ticks:
                self.census.add({"pid": self.child.pid, "ticks": self.controller_ticks}, "controller")
        claim = self.run / "private/access/operator.json"
        if claim.exists():
            value = strict_json(claim.read_bytes())
            require(value["run_id"] == self.id and value["may"] == ["operator"], "shared operator claim binding")
            self.token = value["token"]
        self.ev.save("shared-runtime-binding", {"run_id": self.id, "run_dir": str(self.run),
                     "pid": None if self.child is None else self.child.pid,
                     "start_ticks": getattr(self, "controller_ticks", None), "runtime_started_by_ap5": False})

    def observe_started_runtime(self):
        self.wait("native-bootstrap-complete", lambda: self.manifest() if self.manifest().get("traffic_stage") == "active" else None, 3700)
        manifest = self.manifest()
        require(not coverage_errors(manifest), "default active resource census incomplete")
        self.ev.save("active-resource-census", self.census.active_manifest)
        self.nodes = [Node(manifest["resources"]["listeners"][f"node.node-{n}.rpc"],
                           self.run / f"private/node-rpc-node-{n}.cookie") for n in ("a", "b")]
        require(len([r for r in manifest["processes"] if r["role"] == "wallet"]) == 4, "four real wallet processes required")
        require(len([r for r in manifest["processes"] if r["role"] == "miner"]) == 4, "four real miner processes required")
        require(len([r for r in manifest["processes"] if r["role"] == "pool-ark"]) == 2, "two separate pool Ark processes required")
        for r in manifest["processes"]:
            require("fixture" not in Path(r["executable_path"]).name, "inert actor cannot enter AP5")
            if r["role"] in REQUIRED_ROLES:
                require(alive(r["pid"], r["start_time_ticks"]) and sha(Path(f"/proc/{r['pid']}/exe")) == r["executable_sha256"], "actor identity mismatch")
            else:
                require(r["role"] == "traffic_probe" and not alive(r["pid"], r["start_time_ticks"])
                        and sha(Path(r["executable_path"])) == r["executable_sha256"], "unknown or inconsistent transient actor")
        self.wait("four-live-registration-leases", self.registrations, 60)
        self.ev.save("initial-node-facts", [n.read("getblockchaininfo") for n in self.nodes])

    def registrations(self):
        m = self.manifest()
        result = {}
        for user in USERS:
            endpoint = m["resources"]["listeners"][f"wallet.{user}.status"]
            status = request(f"http://127.0.0.1:{endpoint['port']}/status")
            if not status_ready(status, time.time_ns() // 1_000_000):
                return None
            result[user] = status
        return result

    def command(self, name, kind):
        if self.scope == "confirmed-funding":
            require(kind.get("type") in {"bootstrap_fund", "send_user_payment", "stop_run"}, "command outside native-funding diagnostic scope")
            if kind["type"] == "bootstrap_fund":
                require(name == "native-bootstrap" and kind == {"type": "bootstrap_fund", "driver_id": "txd-0", "node_id": "node-a"}, "diagnostic bootstrap route mismatch")
        payload = {"command_id": command_id(self.id, name), "run_id": self.id, "actor": "operator",
                   "issued_at_wall": time.time_ns() // 1_000_000, "kind": kind}
        self.ev.save("command-intent", payload)
        # No retry with a new ID. An ambiguous reply remains unknown in evidence.
        try:
            result = request(self.url + f"/api/lab/v1/runs/{self.id}/commands", payload, token=self.token)
        except Exception as exc:
            self.ev.save("command-unknown", {"command_id": payload["command_id"], "error": str(exc)})
            raise Blocked("command response unknown; retain original id: " + payload["command_id"]) from exc
        self.ev.save("command-reply", result)
        require(result.get("command_id") == payload["command_id"], "controller command correlation mismatch")
        if result.get("state") in {"accepted", "running"}:
            result = self.wait("command-terminal-" + name, lambda: self.command_terminal(payload),
                               3700 if kind["type"] == "bootstrap_fund" else 600)
        require(result.get("state") == "succeeded", "command not confirmed successful: " + name)
        return result

    def command_terminal(self, payload):
        manifest = self.manifest()
        path = Path(manifest["artifact_paths"]["private_commands"])
        require(path.resolve().is_relative_to(self.run.resolve()) and not path.is_symlink(), "command journal escapes owned run")
        raw = path.read_bytes()
        require(len(raw) <= 64 * 1024 * 1024, "command journal exceeds bounded observation")
        began, outcome = False, None
        for line in raw.splitlines(keepends=True):
            if not line.endswith(b"\n"):
                break  # The writer may be appending its current record.
            row = json.loads(line)
            if row.get("record") == "begin" and row["command"]["command_id"] == payload["command_id"]:
                require(encoded(row["command"]) == encoded(payload), "durable command payload changed")
                began = True
            if row.get("record") == "finish" and row["command_id"] == payload["command_id"]:
                require(began and row["outcome"]["command_id"] == payload["command_id"], "command terminal binding lacks original begin")
                outcome = row["outcome"]
        return outcome if outcome and outcome["state"] not in {"accepted", "running"} else None

    def fund(self):
        # Get the ACTUAL current stock server wallet address through supported RPC.
        config = self.run / "private/ark-server-local-ark/captaind.toml"
        server = json.loads(self.tool_output([str(self.paths["captaind"]), "--config", str(config), "rpc", "wallet"]))
        self.ev.save("server-wallet-before", server)
        info = load(self.run / "private/ark-info.json")
        self.ev.save("actual-arkinfo", info)
        transfers = [("server-alice", "alice", server["rounds"]["address"], 200_000_000),
                     ("server-bob", "bob", server["rounds"]["address"], 200_000_000)]
        for pool, user in zip(POOLS, ("carol", "dave")):
            provision = load(self.run / f"private/pool-ark-{pool}/wallet/provision.json")
            transfers.append((pool, user, provision["onchain_funding_address"], 120_000_000))
        confirmed = []
        for label, user, address, amount in transfers:
            validation = self.nodes[0].read("validateaddress", [address])
            require(validation.get("isvalid") is True, "published funding address rejected by actual node")
            response = self.command("fund-" + label, {"type": "send_user_payment", "user": user,
                "destination_address": address, "amount_sats": amount, "fee_rate_sats_vb": 3})
            detail = json.loads(response["detail"])
            require(detail.get("status") == "accepted_by_rpc", "funding broadcast is unknown")
            proof = self.wait("confirmed-" + label,
                              lambda: confirmed_transaction(self.nodes, detail["txid"], depth=3), 600)
            self.wait("pool-mined-confirmation-" + label,
                      lambda: self.pooled_block(proof[0]["block_hash"]), 60)
            require(any(o["scriptPubKey"]["hex"] == validation["scriptPubKey"] and sats(o["value"]) == amount
                        for o in proof[0]["decoded"]["vout"]), "funding transfer destination/value mismatch")
            confirmed.append({"purpose": label, "user": user, "txid": detail["txid"], "amount_sats": amount,
                              "fee_sats": proof[0]["fee_sats"], "height": proof[0]["height"]})
            self.component("fund-" + label, transactions=[proof],
                           payments=[{"txid": detail["txid"], "script": validation["scriptPubKey"], "sats": amount}])
        self.ev.save("native-funding-confirmed", confirmed)
        return confirmed

    def pool_wallet(self, pool, action, **fields):
        self.require_full()
        spec = self.bundle.get("pool_wallet_executor")
        require(spec is not None, "tested joined pool-wallet-executor required")
        executable = artifact(spec)
        suffix = f"-{self.ev.sequence}" if action in {"snapshot", "board_proof_v1"} else ""
        req = {"schema": "spc.lab.ap5.pool-wallet-request/v1", "operation_id": command_id(self.id, f"{pool}-{action}{suffix}"),
               "action": action, **fields}
        require(not set(req) & {"sp_id", "B_scan", "B_spend", "P_i", "block_hash"}, "identity crossed Ark boundary")
        path = self.ev.save("pool-wallet-request", req)
        self.pool_requests[req["operation_id"]] = req
        request_hash = sha(path)
        requested_at = time.time_ns() // 1_000_000
        operation = Operation(req)
        deadline = min(self.deadline, time.monotonic() + 600)
        argv = [str(executable), "--config", str(self.run / f"private/pool-ark-{pool}/pool.toml"), "--request", str(path)]
        while time.monotonic() < deadline:
            require(sha(path) == request_hash, "pool request file changed during same-id polling")
            try:
                result = self.run_tool(argv, timeout=60)
            except subprocess.TimeoutExpired:
                self.ev.save("pool-wallet-unknown", {"operation_id": req["operation_id"], "reason": "client_timeout", "same_request_sha256": request_hash})
            else:
                if result.returncode:
                    self.ev.save("pool-wallet-unknown", {"operation_id": req["operation_id"], "exit": result.returncode, "same_request_sha256": request_hash})
                else:
                    value = operation.observe(result.stdout)
                    self.ev.save("pool-wallet-result", value)
                    if value["state"] in TERMINAL_STATES:
                        if value["state"] == "observed":
                            fresh_snapshot(value, requested_at, time.time_ns() // 1_000_000)
                        return value
            # This repeats the SAME durable request, including after Unknown.
            # It never invokes a fresh-id mutation or assumes timeout cancelled it.
            time.sleep(2)
        self.ev.save("pool-wallet-unresolved", {"request": req, "request_file": str(path), "request_sha256": request_hash})
        raise Blocked("pool operation still unresolved; retain the original request and reservation")

    def board(self):
        self.require_full()
        for pool in POOLS:
            policy = self.ark_policy()
            before = self.pool_wallet(pool, "snapshot")
            require(type(before.get("onchain_trusted_sats")) is int, "pool onchain balance unknown")
            result = self.pool_wallet(pool, "board_float", net_target_sats=FLOAT_TARGET, max_fee_sats=5_000_000)
            require(result["state"] in ("already_at_target", "confirmed"), "board operation rejected without effect")
            if result["state"] == "confirmed":
                txid = result["funding_txid"]
                self.wait("board-confirmed-" + pool, lambda: confirmed_transaction(self.nodes, txid, result["board"]["required_confirmations"]))
                self.verify_board(pool, result, policy)
            else:
                self.ev.save("float-already-at-target-" + pool, result)
            def enough_float():
                status = self.pool_wallet(pool, "snapshot")
                return status if status["state"] == "observed" and status["float_spendable_sats"] >= FLOAT_TARGET else None
            current = self.wait("boarded-float-" + pool, enough_float)
            self.verify_float(pool, current, result)

    def pool_keys(self, pool):
        db = self.run / f"private/pool-ark-{pool}/wallet/wallet.sqlite"
        return {r["public_key"] for r in readonly_rows(db, "SELECT public_key FROM bark_vtxo_key")}

    def verify_board(self, pool, result, policy):
        # Confirmed is historical: the original VTXO may already have been
        # refreshed/spent. Verify its ORIGINAL full proof and anchor without
        # requiring it to remain currently spendable. Current float is separate.
        board = result["board"]
        read_result = self.pool_wallet(pool, "board_proof_v1", original_board_operation_id=result["operation_id"])
        read_request = self.pool_requests[read_result["operation_id"]]
        board_request = self.pool_requests[result["operation_id"]]
        original = bind_retained_proof(read_request, read_result, board_request, result)
        # Retained same-wallet bytes survive stock refresh/spend/server pruning.
        # Never reconstruct this original object through a remote GetVtxo call.
        decoded = self.decode_vtxo(original["original_full_vtxo_hex"])
        require(decoded["amount_sat"] == board["net_board_sats"]
                and decoded["expiry_height"] == board["expiry_height"]
                and decoded["id"] == original["original_vtxo_id"]
                and decoded["chain_anchor"] == f"{original['anchor_txid']}:{original['anchor_vout']}", "historical board proof differs from receipt")
        self.verify_public_proof(original["original_full_vtxo_hex"], decoded, original["owner_pubkey"],
                                 policy=policy, depth=board["required_confirmations"])
        proof = self.proofs[decoded["id"]]
        require(all(view["raw"] == original["raw_anchor_tx_hex"] for view in proof["anchor_views"]), "retained anchor differs from actual node bytes")
        self.component("board-" + pool, transactions=[proof["anchor_views"]], vtxos=[proof],
                       board=board, ark_info=policy, board_request=board_request, confirmed_receipt=result,
                       proof_read_request=read_request, proof_read_result=read_result)

    def verify_float(self, pool, current, board_result):
        db = self.run / f"private/pool-ark-{pool}/wallet/wallet.sqlite"
        rows = readonly_rows(db, "SELECT id,amount_sat AS amount_sats,expiry_height,state_kind AS state FROM vtxo_view WHERE state_kind='Spendable'")
        keys = self.pool_keys(pool)
        tip = min(n.read("getblockcount") for n in self.nodes)
        proofs = []
        for row in rows:
            if row["expiry_height"] < tip + 64:
                continue
            decoded = self.fetch_stock_proof(row["id"], owner_keys=keys)
            require(decoded["amount_sat"] == row["amount_sats"] and decoded["expiry_height"] == row["expiry_height"], "current float DB/proof mismatch")
            proofs.append(self.proofs[decoded["id"]])
        require(sum(p["expected"]["amount_sats"] for p in proofs) >= FLOAT_TARGET, "fresh target not supported by current full VTXO proofs")
        self.component("float-" + pool, vtxos=proofs, already_at_target=True,
                       target_sats=FLOAT_TARGET, fresh_snapshot=current,
                       board_result=board_result, board_request=self.pool_requests[board_result["operation_id"]],
                       float_rows=[r for r in rows if r["expiry_height"] >= tip + 64])

    def wallet_command(self, user, ids, action, label):
        self.require_full()
        self.ev.save("wallet-before-" + user, wallet_snapshot(self.run, user))
        operation = {"operation_id": command_id(self.id, user + "-" + label), "vtxo_ids": ids, "action": action}
        path = self.ev.save("wallet-command-" + user, operation)
        result = self.run_tool([str(self.paths["agent"]), "--config", str(self.run / f"private/wallet-{user}/agent.toml"),
                                 "--wallet-command", str(path)])
        require(result.returncode == 0, "wallet queue submission failed")
        self.manual_operations.append(operation["operation_id"])
        self.ev.save("wallet-command-queued", {"user": user, "operation_id": operation["operation_id"], "stdout": result.stdout.decode()})
        return operation["operation_id"]

    def operation(self, user, operation_id, stages):
        snapshot = wallet_snapshot(self.run, user)
        row = next((r for r in snapshot["operations"] if r["id"] == operation_id), None)
        if row and row["stage"] in ("noop", "rejected_no_effect"):
            raise Blocked("wallet operation had no effect: " + row["stage"])
        return {"operation": row, "snapshot": snapshot} if row and row["stage"] in stages else None

    def received(self, user):
        statuses = self.registrations()
        if not statuses or statuses[user].get("spendable_sats") is None:
            return None
        s = wallet_snapshot(self.run, user)
        reserved = {r["id"] for r in s["reservations"]}
        rows = [r for r in s["sp"] if r["state"] == "spendable" and r["raw_proof"] and r["expiry_height"] is not None and r["id"] not in reserved]
        return {"inputs": rows[:64], "snapshot": s} if rows else None

    def payout_attribution(self, user, ids):
        self.require_full()
        # SPc-side read only. These identity-bearing rows never enter pool_wallet().
        pool = "pool-a" if user in USERS[:2] else "pool-b"
        rows = self.pool_rows(pool, "ark_payout")
        selected = [r for r in rows if str(r.get("vtxo_id", "")).removeprefix("\\x").lower() in ids and r.get("status") == "sent"]
        require({str(r["vtxo_id"]).removeprefix("\\x").lower() for r in selected} >= set(ids),
                "missing frozen mining-derived payout attribution; direct delivery is not aggregate proof")
        self.ev.save("mining-payout-attribution-" + user, selected)
        return selected

    def pool_rows(self, pool, table):
        require(pool in POOLS and table in ("ark_payout", "block_found", "distribution_snapshot"), "read-only table allowlist")
        suffix = pool[-1]
        cfg = tomllib.loads((self.run / f"private/pool-front-{suffix}1.toml").read_text())
        from urllib.parse import urlsplit, unquote
        url = urlsplit(cfg["storage"]["postgres"])
        require(url.hostname == "127.0.0.1", "foreign pool database")
        cmd = [str(self.paths["psql"]), "-X", "-h", url.hostname, "-p", str(url.port), "-U", unquote(url.username),
               "-d", url.path.lstrip("/"), "-v", "ON_ERROR_STOP=1", "-tAc",
               "SELECT COALESCE(json_agg(row_to_json(t)),'[]') FROM " + table + " t"]
        return json.loads(self.tool_output(cmd, env={"PGPASSWORD": unquote(url.password), "LC_ALL": "C",
                                                     "PGOPTIONS": "-c default_transaction_read_only=on"}, timeout=10))

    def pooled_block(self, rpc_hash):
        internal = "\\x" + bytes.fromhex(rpc_hash)[::-1].hex()
        found = [{"pool": pool, "block": r} for pool in POOLS for r in self.pool_rows(pool, "block_found")
                 if r.get("block_hash") == internal]
        require(len(found) <= 1, "conflicting pool attribution for funding-confirmation block")
        return found or None

    def return_address(self, user):
        index = USERS.index(user)
        for driver, mapping in self.manifest()["traffic_driver_paths"].items():
            path = Path(mapping["journal_path"])
            require(not path.is_symlink() and path.resolve().is_relative_to(self.run.resolve()), "return-address journal escapes owned run")
            raw = path.read_bytes()
            require(len(raw) <= 64 * 1024 * 1024, "return-address journal exceeds bounded observation")
            for line in raw.splitlines(keepends=True):
                if not line.endswith(b"\n"):
                    break  # A live writer's unfinished tail is not a retained record.
                row = strict_json(line)
                if row.get("record") == "funding_observed" and row.get("owner_role") == {"kind": "user_payment", "index": index}:
                    address = p2pkh_address(row["script_hex"])
                    self.ev.save("allocated-return-address-" + user, {"driver_id": driver, "funding_observed": row,
                                 "address": address, "journal_path": str(path),
                                 "journal_read_sha256": __import__("hashlib").sha256(raw).hexdigest()})
                    return address
        raise Blocked("actual T1 user receive script unavailable")

    def decode_vtxo(self, raw_hex):
        value = json.loads(self.tool_output([str(self.paths["bark"]), "--datadir", str(self.root / "decoder"),
                                            "dev", "vtxo", "decode", raw_hex]))
        info = load(self.run / "private/ark-info.json")
        require(value["server_pubkey"] == info["bound"]["server_pubkey"], "VTXO decoder returned a foreign provider")
        self.ev.save("decoded-public-vtxo", {"raw_hex": raw_hex, "decoded": value})
        return value

    def fetch_stock_proof(self, display_id, owner_keys=None):
        self.require_full()
        import base64
        info = load(self.run / "private/ark-info.json")
        endpoint = info["endpoint"].removeprefix("http://")
        require(endpoint.startswith("127.0.0.1:"), "foreign provider")
        proto = self.paths["captaind"].parent.parent / "share/ark/protos"
        payload = json.dumps({"vtxoId": base64.b64encode(bytes.fromhex(stock_id_bytes(display_id))).decode()})
        output = self.tool_output([str(self.paths["grpcurl"]), "-plaintext", "-max-time", "10",
            "-import-path", str(proto), "-proto", "bark_server.proto", "-d", payload,
            endpoint, "bark_server.ArkService/GetVtxo"], timeout=15)
        raw = base64.b64decode(json.loads(output)["vtxo"], validate=True).hex()
        decoded = self.decode_vtxo(raw)
        require(decoded["id"] == display_id, "GetVtxo returned a different outpoint")
        require(owner_keys is not None and decoded["user_pubkey"] in owner_keys, "VTXO policy key is not in the owning wallet's public key registry")
        self.verify_public_proof(raw, decoded, decoded["user_pubkey"])
        return decoded

    def retained_round_children(self, user, checkpoint, snapshot, funding_txid):
        """The exact first-refresh outputs, even if background renewal already spent them.

        The tested round client retains Full bytes in RoundCheckpoint.released
        before import/round_confirmed. Use those historical bytes, rather than
        treating all newly observed stock rows as outputs of this operation or
        depending on the server to retain a spent VTXO.
        """
        require(type(checkpoint.get("version")) is int and checkpoint["version"] == 2
                and checkpoint.get("locked") and checkpoint.get("proof_binding"), "retained round checkpoint incomplete")
        released = checkpoint.get("released")
        require(type(released) is list and 0 < len(released) <= 64, "retained round output census")
        rows = snapshot["stock"]
        stock = {r["id"]: r for r in rows}
        require(len(stock) == len(rows), "duplicate stock row identity")
        keys = self.user_stock_keys(user)
        children, proofs, seen = [], [], set()
        for value in released:
            require(type(value) is list and value and len(value) <= 4 * 1024 * 1024
                    and all(type(b) is int and 0 <= b <= 255 for b in value), "retained round Full bytes malformed")
            raw = bytes(value).hex()
            decoded = self.decode_vtxo(raw)
            identity = decoded["id"]
            require(identity not in seen and identity in stock, "retained round output missing/duplicate in owning wallet")
            seen.add(identity)
            row = stock[identity]
            require(decoded["chain_anchor"].split(":")[0] == funding_txid
                    and decoded["user_pubkey"] in keys
                    and decoded["amount_sat"] == row["amount_sats"]
                    and decoded["expiry_height"] == row["expiry_height"], "retained round output/wallet/anchor mismatch")
            self.verify_public_proof(raw, decoded, decoded["user_pubkey"])
            children.append(row)
            proofs.append(self.proofs[identity])
        return children, proofs

    def renewal_candidate(self, user, child_ids, child_expiry, first_operation):
        s = wallet_snapshot(self.run, user)
        matches = [r for r in s["operations"] if r["id"] != first_operation and r["stage"] == "stock_submitted"
                   and strict_json(r["payload"])["action"]["kind"] == "refresh"
                   and child_ids.intersection(strict_json(r["payload"])["vtxo_ids"])]
        # Renewal can complete before first-refresh observation. Its operation
        # inputs and verified round anchors establish lineage; exclusion from
        # an earlier snapshot does not establish it.
        fresh = [r for r in s["stock"] if stock_id_bytes(r["id"]) not in child_ids
                 and r["state"] == "Spendable" and r["expiry_height"] > child_expiry]
        old = [r for r in s["stock"] if stock_id_bytes(r["id"]) in child_ids]
        if matches and fresh and {stock_id_bytes(r["id"]) for r in old} == child_ids and all(r["state"] == "Spent" for r in old) and self.registrations():
            # Candidate only: lifecycle verifies the automatic ID recipe, exact
            # consumed set and output full proofs/round anchors before credit.
            return {"operations": matches, "outputs": fresh, "snapshot": s}

    def lifecycle(self):
        self.require_full()
        for user in USERS:
            receive = self.wait("sp-owned-received-" + user, lambda: self.received(user), 900)
            ids = [r["id"] for r in receive["inputs"]]
            received_proofs = []
            for value in receive["inputs"]:
                decoded = self.decode_vtxo(value["raw_proof"])
                require(stock_id_bytes(decoded["id"]) == value["id"]
                        and decoded["amount_sat"] == value["amount_sats"]
                        and decoded["expiry_height"] == value["expiry_height"], "SP ledger/proof mismatch")
                anchor = decoded["chain_anchor"].split(":")[0]
                self.wait("received-anchor-" + user, lambda anchor=anchor: confirmed_transaction(self.nodes, anchor))
                self.verify_public_proof(value["raw_proof"], decoded, value["user_pubkey"])
                received_proofs.append(self.proofs[decoded["id"]])
            attribution = self.payout_attribution(user, ids)
            self.component("receive-" + user, vtxos=received_proofs, payout_attribution=attribution)
            op = self.wallet_command(user, ids, {"kind": "refresh"}, "first-refresh")
            first = self.wait("sp-round-confirmed-" + user, lambda: self.operation(user, op, {"round_confirmed"}))
            checkpoint = strict_json(first["operation"]["checkpoint"])
            require(checkpoint.get("version") == 2 and checkpoint.get("locked") and checkpoint.get("released") and checkpoint.get("proof_binding"),
                    "SP refresh lacks retained pre-forfeit/output proof")
            self.wait("round-chain-proof-" + user, lambda: confirmed_transaction(self.nodes, first["operation"]["result"]))
            first_funding = first["operation"]["result"]
            child, child_proofs = self.retained_round_children(user, checkpoint, first["snapshot"], first_funding)
            fees = round_value(receive["inputs"], [p["expected"] for p in child_proofs])
            self.component("sp-refresh-" + user, vtxos=received_proofs + child_proofs,
                           round={"funding_txid": first_funding, "inputs": ids,
                                  "outputs": [r["id"] for r in child], "value": fees,
                                  "checkpoint": checkpoint, "checkpoint_raw": first["operation"]["checkpoint"]})
            child_ids = {stock_id_bytes(r["id"]) for r in child}
            child_expiry = min(r["expiry_height"] for r in child)
            second = self.wait("automatic-second-renewal-" + user,
                               lambda: self.renewal_candidate(user, child_ids, child_expiry, op), 2400)
            round_ids = {r["result"].split(":", 1)[1] for r in second["operations"]
                         if r["result"] and r["result"].startswith("round_submitted:")}
            renewed = []
            for output in second["outputs"]:
                decoded = self.fetch_stock_proof(output["id"], self.user_stock_keys(user))
                if decoded["chain_anchor"].split(":")[0] in round_ids:
                    require(decoded["expiry_height"] > child_expiry, "renewal did not extend actual expiry")
                    renewed.append(output)
            require(renewed, "no proved outputs of the automatic SECOND round")
            for txid in round_ids:
                self.wait("second-round-chain-" + user, lambda txid=txid: confirmed_transaction(self.nodes, txid))
            import hashlib
            tip = min(n.read("getblockcount") for n in self.nodes)
            require(tip <= 100000, "automatic-operation verification height bound exceeded")
            for operation in second["operations"]:
                payload = json.loads(operation["payload"])
                require(len(payload["vtxo_ids"]) == 1, "automatic stock command must name one input")
                material = b"SPc/StockAutoRefresh/v1" + bytes.fromhex(payload["vtxo_ids"][0])
                matches = [h for h in range(tip + 1) if hashlib.sha256(material + h.to_bytes(4, "big")).hexdigest() == operation["id"]]
                require(len(matches) == 1, "no actual-height automatic-renewal operation binding")
                operation["verified_auto_trigger_height"] = matches[0]
            self.component("second-renewal-" + user, vtxos=child_proofs + [self.proofs[r["id"]] for r in renewed],
                           renewal={"automatic": True, "operations": second["operations"],
                                    "inputs": sorted(child_ids), "outputs": [r["id"] for r in renewed],
                                    "previous_expiry": child_expiry, "round_ids": sorted(round_ids),
                                    "manual_operation_ids": list(self.manual_operations)})
            destination = self.return_address(user)
            op = self.wallet_command(user, [stock_id_bytes(r["id"]) for r in renewed[:64]],
                                     {"kind": "offboard", "address": destination}, "stock-offboard")
            ordinary = self.wait("stock-offboard-submitted-" + user, lambda: self.operation(user, op, {"stock_submitted"}))
            result = ordinary["operation"]["result"]
            require(result.startswith("offboard_broadcast:"), "stock offboard lacks broadcast identity")
            stock_views = self.wait("stock-offboard-confirmed-" + user, lambda: confirmed_transaction(self.nodes, result.split(":", 1)[1], 3))
            script = self.nodes[0].read("validateaddress", [destination])["scriptPubKey"]
            maturity = self.nodes[0].read("getblockchaininfo")["consensus"]["coinbase_maturity"]
            amounts = offboard_value(renewed[:64], stock_views, script, maturity)
            self.component("stock-offboard-" + user, transactions=[stock_views], vtxos=[self.proofs[r["id"]] for r in renewed[:64]],
                           offboard={"destination_script": script, "destination_address": destination, "inputs": [r["id"] for r in renewed[:64]], "value": amounts,
                                     "operation": ordinary["operation"]})
            direct = self.wait("new-sp-owned-inputs-" + user, lambda: self.received(user))
            op = self.wallet_command(user, [r["id"] for r in direct["inputs"]],
                                     {"kind": "offboard", "address": destination}, "sp-offboard")
            derived = self.wait("sp-offboard-confirmed-" + user, lambda: self.operation(user, op, {"offboard_confirmed"}))
            direct_views = self.wait("sp-offboard-chain-proof-" + user, lambda: confirmed_transaction(self.nodes, derived["operation"]["result"], 3))
            direct_proofs = []
            for item in direct["inputs"]:
                decoded = self.decode_vtxo(item["raw_proof"])
                self.verify_public_proof(item["raw_proof"], decoded, item["user_pubkey"])
                direct_proofs.append(self.proofs[decoded["id"]])
            amounts = offboard_value(direct["inputs"], direct_views, script, maturity)
            self.component("sp-offboard-" + user, transactions=[direct_views], vtxos=direct_proofs,
                           offboard={"destination_script": script, "destination_address": destination, "inputs": [r["id"] for r in direct["inputs"]], "value": amounts,
                                     "operation": derived["operation"]})
            self.ev.save("wallet-final-" + user, wallet_snapshot(self.run, user))

    def require_full(self):
        require(self.scope == "full", "Ark lifecycle/client operations are forbidden in confirmed-funding scope")

    def verdict(self, bindings):
        if self.scope == "confirmed-funding":
            return native_funding_verdict(self.components, self.cleanup, bindings)
        return final_verdict(self.components, mandatory_components(USERS, POOLS), self.cleanup, bindings)

    def stop(self):
        if self.child is None:
            return
        try:
            if self.url and self.token and self.child.poll() is None:
                self.command("stop", {"type": "stop_run", "reason": "AP5 bounded execution complete or blocked"})
        finally:
            if self.child.poll() is None and alive(self.child.pid, self.controller_ticks):
                os.killpg(self.child.pid, signal.SIGTERM)
                try:
                    self.child.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    os.killpg(self.child.pid, signal.SIGKILL)
                    self.child.wait(timeout=15)
            try:
                if (self.run / "run-manifest.json").exists():
                    before = self.manifest()
                    reaped = self.run_tool([str(self.paths["lab"]), "reap", "--run-dir", str(self.run)], timeout=30)
                    self.reap_record = {"before": before, "after": self.manifest(), "exit": reaped.returncode,
                                        "stdout": reaped.stdout, "stderr": reaped.stderr}
                    require(reaped.returncode == 0, "source-bound auxiliary tool failed: reap")
            finally:
                try:
                    self.cleanup = self.census.finish()
                    self.ev.save("cleanup-inventory", self.cleanup)
                finally:
                    if self.log is not None:
                        self.log.close()
            survivors = [{"pid": pid, "ticks": ticks, "role": role} for (pid, ticks), role in self.owned.items() if alive(pid, ticks)]
            self.ev.save("cleanup", {"survivors": survivors, "controller_exit": self.child.returncode, "all_owned_actors_stopped": not survivors})
            require(not survivors, "owned actors survived cleanup")
            require(self.cleanup["status"] == "passed", "exhaustive owned-resource cleanup is not proven")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=("prepare", "check-bundle", "run"))
    parser.add_argument("--bundle", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--through", choices=("confirmed-funding", "full"), default="full")
    parser.add_argument("--timeout-secs", type=int, default=7200)
    args = parser.parse_args()
    root = fresh_root(args.out)
    ev = Evidence(root)
    controller = None
    result = {"schema": "spc.lab.ap5.result/v2", "scope": args.through,
              "status": "blocked", "whole_lab_complete": False, "funded_ark_lifecycle": False,
              "native_funding_confirmed": False}
    try:
        spec = scenario("ap5-" + secrets.token_hex(8), secrets.randbelow(2**52))
        ev.save("declared-default", {"scenario": spec, "pool_float_target_sats": FLOAT_TARGET,
                                    "paid_window_heights": PAID_WINDOW, "global_hashes_per_second": 40})
        if args.command == "prepare":
            result["status"] = "prepared-not-executed"
            return
        require(args.bundle is not None, "source-bound joined --bundle required")
        bundle = load(args.bundle)
        paths, source, receipt = check_bundle(bundle, full=args.through == "full")
        ev.save("verified-bundle", {"bundle": bundle, "source_manifest_sha256": sha(source), "validation_receipt_sha256": sha(receipt)})
        if args.command == "check-bundle":
            result["status"] = "bundle-verified-not-executed"
            return
        require(args.timeout_secs > 0, "positive bounded timeout required")
        controller = Controller(bundle, paths, root, ev, spec, args.timeout_secs, scope=args.through)
        controller.start()
        controller.fund()
        result["native_funding_confirmed"] = True
        if args.through == "confirmed-funding":
            result["status"] = "confirmed-native-transfers-only"
            return
        controller.board()
        controller.lifecycle()
        result["status"] = "funded-lifecycle-observed"
        result["funded_ark_lifecycle"] = True
        # T3/T4b/ladder independent oracle results are owned by other packets.
        # This controller never promotes whole-lab completion from its own flow.
    except Exception as exc:
        result["error"] = str(exc)
        raise
    finally:
        try:
            if controller:
                try:
                    controller.stop()
                except Exception as exc:
                    result["status"] = "cleanup-failed"
                    result["cleanup_error"] = str(exc)
                    raise
        finally:
            if controller:
                verdict = controller.verdict(
                    {"bundle_sha256": sha(args.bundle), "bundle_artifact": {"path": str(args.bundle.resolve()), "sha256": sha(args.bundle)}, "joined_source_manifest": bundle["joined_source_manifest"],
                      "joined_validation_receipt": bundle["joined_validation_receipt"], "binaries": bundle["binaries"],
                      "vtxo_verifier": bundle.get("vtxo_verifier"),
                      "pool_wallet_executor": bundle.get("pool_wallet_executor")})
                if args.through == "confirmed-funding":
                    verdict_path = ev.save("native-funding-verdict", verdict)
                    result["native_funding_verdict"] = {"file": verdict_path.name, "sha256": sha(verdict_path)}
                    result["diagnostic_status"] = verdict["diagnostic_status"]
                    result["status"] = "native-funding-diagnostic-" + verdict["diagnostic_status"]
                else:
                    verdict_path = ev.save("ap5-component-verdict", verdict)
                    result["component_verdict"] = {"file": verdict_path.name, "sha256": sha(verdict_path)}
                    result["component_status"] = verdict["component_status"]
                    if verdict["component_status"] == "passed":
                        result["status"] = "ap5-component-passed"
                    elif verdict["component_status"] == "failed":
                        result["status"] = "ap5-component-failed"
                    elif result.get("funded_ark_lifecycle"):
                        result["status"] = "ap5-component-inconclusive"
            ev.save("result", result)
            ev.close()
            # External consumers pin this descriptor by hash before replay.
            seal = {"root": str(root), "events_sha256": sha(ev.path),
                    "last_seq": ev.sequence, "last_digest": ev.previous}
            with (root / "seal.json").open("x") as file:
                os.chmod(root / "seal.json", 0o600)
                json.dump(seal, file, sort_keys=True)
                file.write("\n")


if __name__ == "__main__":
    main()
