"""Private AP5 evidence and source-bound, local-only I/O. No signer or miner."""
import base64
from contextlib import closing
import hashlib
import json
import os
import re
import shlex
from pathlib import Path
import sqlite3
import stat
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request
from decimal import Decimal

NODE_SHA = "bba4752298aff258adb6cce99c6db849b02f50736a8ec32766ed1710045051aa"
SERVER_SHA = "4cac675bccc017608378382129613ed5202dd90e7300a92c4ed799869abaef9d"
# Exact source-layer admission, applicable only when that code is present.
# R4 approval is not a substitute for actual joined Nix rows/feature checks.
APPROVED_POOL_CONTROL_SNAPSHOTS = frozenset({"d00299c259aaa5e8098542f48f5c3c90c66737be3f965f5d65e5fd2a763914b2"})
POOL_CONTROL_R4_REVIEW = {"path": "/tmp/opencode/pool-wallet-control-source-review-r4.md",
                          "sha256": "956ba9ab524b8946b5d367c7e814abefafd75cfd523fa5bcae129cd9155e5d0b"}
POOL_CONTROL_R4_FILES = {
    # Re-pinned 2026-11-12 by the same-auditor stall-coverage recheck
    # (docs/reviews/2026-11-12-pool-wallet-stall-coverage.md, bound below):
    # the R4 review approved 53f5e789...; the bounded correction
    # (process_key_with_capacity helper + deadline-true stall coverage)
    # is approved by that recheck, not by the R4 review itself.
    "bins/spc-pool/src/ark/wallet_control.rs": "7a3b3ff91883899038c13d745d918aa168b34825ccc03922480710b2d0fb9423",
    "bins/spc-pool/src/ark/wallet_control_io.rs": "c0db03a8925f2a0f8849e00fd2e6271815a8b24e496db13bf6be861b0f13b3e6",
    "bins/spc-pool/src/ark/wallet_control_proof.rs": "1e5a273e0aa279bf87370e47613d642f56ee5c480e411d2d901f4e605043bd94",
    "bins/spc-pool/src/bin/pool-wallet-executor.rs": "81d712ceab685c8097ef2e980fe5216f07c8101c5e0cde5267f72c75cee117d9",
}
# Same-auditor recheck approving the re-pinned wallet_control.rs row above.
# The tracked in-source copy is docs/reviews/2026-11-12-pool-wallet-stall-
# coverage.md; this path points at the preserved reviewed packet bytes.
POOL_CONTROL_STALL_COVERAGE_REVIEW = {"path": "/tmp/opencode/pool-wallet-stall-coverage-20261112/source/docs/reviews/2026-11-12-pool-wallet-stall-coverage.md",
                                      "sha256": "bdeaf18fef0176a20f52a161a7f29a3a6bb0178ce5dae67dd4f6bb02073160af"}
DENIED_POOL_CONTROL_SNAPSHOTS = frozenset({
    "55a488197a0e116ab4d229ba426a12acb551bf16f8313698590de8cf0a685678",
    "dd9f0c37d7e79474a56132170272fcd6509eb3ebdc501aaa93372e8dd8a4102b",
})
PROTECTED = [Path("/tmp/opencode/ark-inspection-20261109T025651Z/run"),
             Path("/tmp/opencode/live-lab-20261108T231027Z/run")]


class Blocked(RuntimeError):
    pass


class RpcUnavailable(Blocked):
    def __init__(self, method, error):
        self.code = error.get("code") if isinstance(error, dict) else None
        super().__init__(f"RPC unavailable: {method}: {error}")


def require(ok, message):
    if not ok:
        raise Blocked(message)


def sha(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def load(path):
    return json.loads(Path(path).read_text())


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), default=str).encode()


def same_json_value(left, right):
    """Compare JSON facts across QA's Decimal and the stdlib float decoder.

    Compare the decimal value represented by each decoder, not float's binary
    approximation. Do not coerce strings/bools or hide precision lost by a
    decoder; a changed represented value remains a mismatch.
    """
    numbers = (int, float, Decimal)
    if type(left) in numbers and type(right) in numbers:
        a, b = Decimal(str(left)), Decimal(str(right))
        return a.is_finite() and b.is_finite() and a == b
    if type(left) is not type(right):
        return False
    if type(left) is dict:
        return left.keys() == right.keys() and all(same_json_value(left[k], right[k]) for k in left)
    if type(left) is list:
        return len(left) == len(right) and all(same_json_value(a, b) for a, b in zip(left, right))
    return type(left) in (str, bool, type(None)) and left == right


def fresh_root(path):
    path = Path(path).absolute()
    # Refuse ancestors too: no broad output root may envelop an inspection run.
    resolved = path.resolve()
    require(all(not (resolved == p or resolved.is_relative_to(p) or p.is_relative_to(resolved))
                for p in PROTECTED), "protected run overlap")
    require(not path.exists() and not path.is_symlink(), "output must be a fresh directory")
    path.mkdir(mode=0o700)
    return path


class Evidence:
    def __init__(self, root):
        self.root = root
        self.sequence = 0
        self.previous = "0" * 64
        self.path = root / "events.jsonl"
        self.file = self.path.open("xb")
        os.chmod(self.path, 0o600)

    def save(self, label, value):
        self.sequence += 1
        name = f"{self.sequence:06d}-{label}.json"
        path = self.root / name
        with path.open("xb") as f:
            os.chmod(path, 0o600)
            f.write(encoded(value) + b"\n")
            f.flush()
            os.fsync(f.fileno())
        row = {"seq": self.sequence, "file": name, "sha256": sha(path),
               "previous": self.previous, "unix_ms": time.time_ns() // 1_000_000}
        self.previous = hashlib.sha256(encoded(row)).hexdigest()
        self.file.write(encoded({**row, "record_sha256": self.previous}) + b"\n")
        self.file.flush()
        os.fsync(self.file.fileno())
        return path

    def close(self):
        self.file.close()


def artifact(record):
    require(set(record) >= {"path", "sha256"}, "artifact needs path and sha256")
    path = Path(record["path"]).resolve(strict=True)
    require(path.is_file() and sha(path) == record["sha256"], f"artifact hash mismatch: {path}")
    return path


def validate_pool_control_admission(policy):
    require(policy.get("source_snapshot_sha256") not in DENIED_POOL_CONTROL_SNAPSHOTS
            and policy.get("source_snapshot_sha256") in APPROVED_POOL_CONTROL_SNAPSHOTS,
            "pool-control admission held: corrected tested restart-compatible source and same-auditor recheck required; dd9r3 is historical only")
    review = policy.get("same_auditor_review")
    require(policy.get("tested_code_snapshot_sha256") == "f764900419db45901f558c367b5485938ed1b20b5ee5abdc039e3b66c230ae0c"
            and type(review) is dict and set(review) == {"path", "sha256"}
            and review["sha256"] == POOL_CONTROL_R4_REVIEW["sha256"], "R4 source/review binding mismatch")
    artifact(review)  # Same immutable reviewed bytes, at this machine's repository path.
    stall = policy.get("stall_coverage_review")
    require(type(stall) is dict and set(stall) == {"path", "sha256"}
            and stall["sha256"] == POOL_CONTROL_STALL_COVERAGE_REVIEW["sha256"], "stall-coverage review binding mismatch")
    artifact(stall)  # Same-auditor recheck approving the re-pinned wallet_control.rs row.
    require(all(policy.get("files", {}).get(name) == value for name, value in POOL_CONTROL_R4_FILES.items()), "R4 approved control rows missing or changed")


def probe_invocation(wrapper):
    """Use the CLI's interpreter/prefix-argument interface, not a shell exec."""
    raw = wrapper.read_bytes()
    if raw.startswith(b"\x7fELF"):
        return wrapper, None
    require(wrapper.is_relative_to("/nix/store"), "probe wrapper must be immutable Nix input")
    lines = raw.decode().splitlines()
    require(len(lines) == 2 and lines[0].startswith("#!/nix/store/") and lines[1].endswith(' "$@"'), "unsupported probe wrapper shape")
    words = shlex.split(lines[1])
    require(len(words) == 4 and words[0] == "exec" and words[-1] == "$@", "probe wrapper contains unverified shell behavior")
    program, script = Path(words[1]).resolve(strict=True), Path(words[2]).resolve(strict=True)
    require(program.is_relative_to("/nix/store") and program.name.startswith("python3")
            and script == wrapper.parent.parent / "share/ark-rpc/probe.py", "probe interpreter/script binding mismatch")
    with program.open("rb") as executable:
        require(executable.read(4) == b"\x7fELF", "probe interpreter is not a native executable")
    require(script.is_file(), "probe script missing")
    return program, script


def bind_probe_invocation(bundle, wrapper):
    """Require the future bundle to list the exact executed target and source."""
    require("probe_invocation" in bundle, "bundle must declare the executed probe interpreter/script/argv binding")
    binding = bundle["probe_invocation"]
    fields = {"schema", "wrapper", "program", "script", "prefix_args", "runtime_subcommand", "source_manifest"}
    require(type(binding) is dict and set(binding) in (fields, fields | {"program_binding"}), "probe invocation closed shape")
    require(binding["schema"] == "spc.lab.ap5.probe-invocation/v1" and binding["runtime_subcommand"] == "node-probe", "probe invocation schema/subcommand")
    require(binding["wrapper"] == bundle["binaries"]["probe"] and artifact(binding["wrapper"]) == wrapper,
            "probe wrapper provenance differs from bundle")
    program, script = probe_invocation(wrapper)
    require(script is not None, "this probe invocation contract requires the canonical profile script")
    for name, expected in (("program", program), ("script", script)):
        require(type(binding[name]) is dict and set(binding[name]) == {"path", "sha256"}
                and binding[name]["path"] == str(expected) and artifact(binding[name]) == expected,
                "executed probe " + name + " is not explicitly bound")
    require(binding["prefix_args"] == [str(script)], "unsupported probe prefix arguments; runtime appends node-probe exactly once")
    if "program_binding" in binding:
        from probe_package import validate_program_binding
        validate_program_binding(binding["program_binding"], binding["source_manifest"], wrapper, program, script)
        return program, script
    source = load(artifact(binding["source_manifest"]))
    require(set(source) == {"schema", "root", "files"} and source["schema"] == "spc.lab.ap5.probe-source/v1"
            and source["root"] == str(script.parent), "probe source manifest root/schema")
    files = {}
    for path in script.parent.rglob("*"):
        require(not path.is_symlink(), "indirect probe source")
        if path.is_file() and "__pycache__" not in path.parts:
            files[str(path.relative_to(script.parent))] = sha(path)
    require(source["files"] == files and files, "probe source file census/hash mismatch")
    return program, script


def check_bundle(bundle, full=False):
    require(bundle.get("schema") == "spc.lab.ap5.bundle/v1", "unsupported AP5 bundle")
    require(type(full) is bool, "bundle scope must be explicit")
    if full:
        require(bundle.get("scope", "full") == "full", "full AP5 cannot promote a diagnostic bundle")
    # These must be receipts for the JOINED executables, not merely the separate
    # source packet receipts. The serial owner produces them after integration.
    source = artifact(bundle["joined_source_manifest"])
    receipt = artifact(bundle["joined_validation_receipt"])
    paths = {name: artifact(bundle["binaries"][name]) for name in
             ("lab", "txd", "pool", "agent", "miner", "scanner", "node", "captaind", "grpcurl", "probe", "redis-server", "redis-cli", "psql")}
    validation = load(receipt)
    require(validation.get("schema") == "spc.lab.ap5.join-validation/v1", "joined validation receipt schema missing")
    require(validation.get("source_manifest_sha256") == bundle["joined_source_manifest"]["sha256"], "validation covers a different source manifest")
    for name in ("lab", "txd", "pool", "agent", "miner", "scanner"):
        require(validation.get("binaries", {}).get(name) == sha(paths[name]), "validation does not bind executable: " + name)
    require(validation.get("checks") and all(type(c.get("exit")) is int and c["exit"] == 0 for c in validation["checks"]), "joined checks did not pass")
    for check in validation["checks"]:
        artifact(check["log"])
    manifest = load(source)
    expected = load(Path(__file__).with_name("critical-sources.json"))["files"]
    roots = set()
    for name in ("lab", "txd", "pool", "agent", "miner", "scanner"):
        executable = paths[name]
        require(executable.is_relative_to("/nix/store"), f"joined {name} must have a Nix derivation")
        package = Path("/nix/store") / executable.relative_to("/nix/store").parts[0]
        drv = subprocess.check_output(["nix-store", "-q", "--deriver", str(package)], text=True).strip()
        require(drv.endswith(".drv"), f"no source derivation for {name}")
        derivation = json.loads(subprocess.check_output(["nix", "derivation", "show", drv], text=True))
        env = next(iter(derivation.values()))["env"]
        env = json.loads(env["__json"]) if "__json" in env else env
        require("src" in env, f"joined {name} needs an explicit build-source binding")
        roots.add(env["src"])
    require(len(roots) == 1, "separate old CLI/agent plus a new txd is not a joined build")
    source_identity = roots.pop()
    require(source_identity.startswith("/nix/store/"), "joined source must be an actual Nix store input")
    build_root = Path(source_identity)
    verify_source(build_root, manifest, expected)
    require(sha(paths["node"]) == NODE_SHA, "AP5 must retain the verified release node")
    require(sha(paths["captaind"]) == SERVER_SHA, "AP5 must retain stock pinned captaind")
    require(bundle.get("bootstrap_and_workers_single_budget") is True, "joined runtime must own the single 40 H/s budget")
    require(bundle.get("host_peering_before_profile_binding") is True, "two-node peer wiring must precede profile binding")
    require(bundle.get("fresh_registration_before_workers") is True, "joined admission must require four fresh wallet leases")
    require(bundle.get("history_version") == 3, "traffic history 3 required")
    pg = Path(bundle["postgres_bin_dir"]).resolve(strict=True)
    require(all((pg / n).is_file() for n in ("initdb", "postgres", "createdb", "psql")), "Postgres tools missing")
    require(paths["psql"] == (pg / "psql").resolve(), "Postgres source binding mismatch")
    from source_scope import check_source_scope
    admitted_class = check_source_scope(bundle, validation, manifest, build_root, source_identity, paths, full)
    if "pool_wallet_executor" in bundle:
        executor = artifact(bundle["pool_wallet_executor"])
        require(executor.is_relative_to("/nix/store"), "pool executor must be source-bound in Nix")
        package = Path("/nix/store") / executor.relative_to("/nix/store").parts[0]
        drv = subprocess.check_output(["nix-store", "-q", "--deriver", str(package)], text=True).strip()
        require(drv.endswith(".drv"), "pool executor source derivation missing")
        env = next(iter(json.loads(subprocess.check_output(["nix", "derivation", "show", drv], text=True)).values()))["env"]
        env = json.loads(env["__json"]) if "__json" in env else env
        require(env.get("src") == source_identity, "pool executor is not built from the admitted joined source")
    # Both executable profiles claim Ark-enabled agents/pools and a compiled
    # UI; the diagnostic cannot reinterpret default-feature tests as coverage.
    from joined_checks import validate_joined_features
    if admitted_class != "received-legacy-no-wallet-control":
        validate_joined_features(validation, bundle, build_root, manifest)
    paths["probe_program"], paths["probe_script"] = bind_probe_invocation(bundle, paths["probe"])
    if full:
        for field, denied in (("pool_control_source_policy", "55a488197a0e116ab4d229ba426a12acb551bf16f8313698590de8cf0a685678"),
                              ("aggregate_source_policy", "415d21fa88ad2124712cc1fe9a8400bc7fbdd951bd005f223f92f879068f2602")):
            require(field in bundle, "tested corrected producer source binding required: " + field)
            policy = load(artifact(bundle[field]))
            require(policy.get("compiled") is True and policy.get("tests_passed") is True
                    and re.fullmatch(r"[0-9a-f]{64}", policy.get("source_snapshot_sha256", ""))
                    and policy["source_snapshot_sha256"] != denied and policy.get("files"), "source-only/known defective producer cannot activate funded acceptance")
            require(policy.get("checks") and all(type(c.get("exit")) is int and c["exit"] == 0 for c in policy["checks"]), "corrected producer needs actual passing check receipts")
            for check in policy["checks"]:
                artifact(check["log"])
            verify_source(build_root, manifest, policy["files"])
        require("vtxo_verifier" in bundle, "full lifecycle needs the independently built public-proof verifier")
        from verifier_admission import admit_compiled_verifier
        admit_compiled_verifier(bundle["vtxo_verifier"])
        require("bark" in bundle["binaries"], "full lifecycle needs pinned stock Bark CLI for public VTXO decoding")
        paths["bark"] = artifact(bundle["binaries"]["bark"])
        require(paths["bark"].is_relative_to("/nix/store"), "Bark decoder must be a Nix artifact")
        revision = paths["bark"].parent.parent / "share/ark/source-revision"
        require(revision.read_text().strip() == "c419159fa54085572f8ca72d2ba1716556a9d3eb", "wrong Bark decoder pin")
        require("pool_wallet_executor" in bundle, "missing same-wallet pool boarding executor; no supported daemon command exists yet")
    return paths, source, receipt


def verify_source(root, manifest, critical):
    indexed = {r["path"]: r["sha256"] for r in manifest["files"]}
    require(len(indexed) == len(manifest["files"]), "duplicate source manifest paths")
    for name, expected in critical.items():
        require(not Path(name).is_absolute() and ".." not in Path(name).parts, "source manifest path escape")
        require(indexed.get(name) == expected, f"joined manifest lacks approved critical source: {name}")
        require((root / name).is_file() and sha(root / name) == expected,
                f"actual Nix build source is stale: {name}")


def request(url, value=None, token=None, cookie=None):
    parsed = urllib.parse.urlsplit(url)
    require(parsed.scheme == "http" and parsed.hostname == "127.0.0.1" and parsed.username is None,
            "only owned loopback HTTP is supported")
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = "Bearer " + token
    if cookie:
        headers["Authorization"] = "Basic " + base64.b64encode(Path(cookie).read_bytes().strip()).decode()
    data = None if value is None else encoded(value)
    req = urllib.request.Request(url, data=data, headers=headers)
    # No environment proxy or redirect can turn a private call into remote I/O.
    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, *args, **kwargs):
            raise Blocked("redirect refused")
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    with opener.open(req, timeout=10) as response:
        raw = response.read(8 * 1024 * 1024 + 1)
        require(len(raw) <= 8 * 1024 * 1024, "response exceeded bound")
        return json.loads(raw, parse_float=Decimal)


class Node:
    READS = {"getblockchaininfo", "getblockcount", "getblockhash", "getblock", "getblockheader",
             "getrawtransaction", "getrawmempool", "getnetworkinfo", "getpeerinfo", "validateaddress"}

    def __init__(self, endpoint, cookie):
        require(endpoint["host"] == "127.0.0.1", "foreign node endpoint")
        self.url = f"http://127.0.0.1:{endpoint['port']}"
        self.cookie = cookie
        self.seq = 0

    def read(self, method, params=()):
        require(method in self.READS, "harness may not mine, change time or broadcast through RPC")
        self.seq += 1
        result = request(self.url, {"jsonrpc": "2.0", "id": self.seq, "method": method, "params": list(params)}, cookie=self.cookie)
        require(result.get("id") == self.seq, "RPC response id mismatch")
        if result.get("error") is not None:
            raise RpcUnavailable(method, result["error"])
        return result["result"]


def sats(value):
    require(value is not None and not isinstance(value, bool), "unknown/boolean Bitcoin amount")
    try:
        n = Decimal(str(value)) * 100_000_000
    except ArithmeticError as exc:
        raise Blocked("invalid Bitcoin amount") from exc
    require(n.is_finite(), "non-finite Bitcoin amount")
    require(n == n.to_integral_value() and n >= 0, "non-integral/negative Bitcoin amount")
    return int(n)


def confirmed_transaction(nodes, txid, depth=1):
    views = []
    for node in nodes:
        try:
            tx = node.read("getrawtransaction", [txid, 1])
        except RpcUnavailable as exc:
            if exc.code == -5:
                return None
            raise
        if "blockhash" not in tx or tx.get("confirmations", 0) < depth:
            return None
        header = node.read("getblockheader", [tx["blockhash"], True])
        if node.read("getblockhash", [header["height"]]) != tx["blockhash"]:
            return None
        block = node.read("getblock", [tx["blockhash"], 1])
        require(txid in block["tx"], "transaction not in asserted canonical block")
        previous = []
        total_in = 0
        for vin in tx["vin"]:
            require("coinbase" not in vin, "funding must use a real transfer")
            parent = node.read("getrawtransaction", [vin["txid"], 1])
            if not parent.get("blockhash"):
                return None
            parent_header = node.read("getblockheader", [parent["blockhash"], True])
            if node.read("getblockhash", [parent_header["height"]]) != parent["blockhash"]:
                return None
            value = sats(parent["vout"][vin["vout"]]["value"])
            total_in += value
            previous.append({"txid": vin["txid"], "vout": vin["vout"], "amount_sats": value,
                             "raw": node.read("getrawtransaction", [vin["txid"], 0]),
                             "height": parent_header["height"], "block_hash": parent["blockhash"],
                             "raw_block": node.read("getblock", [parent["blockhash"], 0])})
        fee = total_in - sum(sats(o["value"]) for o in tx["vout"])
        require(fee >= 0, "negative transaction fee")
        views.append({"txid": txid, "raw": node.read("getrawtransaction", [txid, 0]),
                      "decoded": tx, "height": header["height"], "block_hash": tx["blockhash"],
                      "raw_block": node.read("getblock", [tx["blockhash"], 0]),
                      "inputs": previous, "fee_sats": fee})
    require(views[0]["raw"] == views[1]["raw"] and views[0]["block_hash"] == views[1]["block_hash"],
            "nodes disagree on transaction/chain")
    return views


def readonly_rows(path, sql):
    path = Path(path)
    st = path.lstat()
    require(stat.S_ISREG(st.st_mode) and not path.is_symlink(), "wallet database must be an existing regular file")
    require(sql.lstrip().upper().startswith("SELECT "), "only read-only evidence queries are permitted")
    with closing(sqlite3.connect(path.resolve().as_uri() + "?mode=ro", uri=True, timeout=2)) as db:
        db.execute("PRAGMA query_only=ON")
        db.row_factory = sqlite3.Row
        return [dict(r) for r in db.execute(sql)]


def wallet_snapshot(run, user):
    state = run / f"private/wallet-{user}/state"
    return {"sp": readonly_rows(state / "state.sqlite", "SELECT lower(hex(v.vtxo_id)) AS id,lower(hex(v.u_pubkey)) AS user_pubkey,v.amount_sats,v.state,v.expiry_height,v.expiry_source,lower(hex(r.raw)) AS raw_proof, r.epoch_height,lower(hex(r.epoch_hash)) AS epoch_hash FROM vtxo_records v LEFT JOIN live_receipts r ON v.vtxo_id=r.vtxo_id"),
            "operations": readonly_rows(state / "state.sqlite", "SELECT id,CAST(payload AS TEXT) AS payload,stage,CAST(outputs AS TEXT) AS checkpoint,result FROM live_operations"),
            "reservations": readonly_rows(state / "state.sqlite", "SELECT lower(hex(vtxo_id)) AS id,operation_id FROM live_operation_inputs"),
            "stock": readonly_rows(state / "bark/db.sqlite", "SELECT v.id,v.amount_sat AS amount_sats,v.expiry_height,s.state_kind AS state,lower(hex(v.raw_bare)) AS raw_bare,lower(hex(v.raw_genesis)) AS raw_genesis FROM bark_vtxo v JOIN vtxo_view s ON s.id=v.id")}


def command_id(run_id, name):
    return hashlib.sha256(encoded(["spc.lab.ap5/v1", run_id, name])).hexdigest()


def stock_id_bytes(value):
    txid, index = value.split(":")
    require(len(txid) == 64 and index.isdecimal(), "invalid stock VTXO id")
    return (bytes.fromhex(txid)[::-1] + int(index).to_bytes(4, "little")).hex()


def status_ready(status, now_ms):
    lease = status.get("registration_live_until_unix_ms")
    return (status.get("state") == "connected" and status.get("registered") is True
            and type(lease) is int and now_ms < lease <= now_ms + 30000)
