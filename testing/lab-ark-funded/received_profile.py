"""Reader for the serial owner's built no-control diagnostic receipt format.

All normal check_bundle deriver/root/critical/pin checks run first. This is a
format adapter with source and log validation, never a hash-based pass shortcut.
"""
import json
from pathlib import Path
import re
import shlex
import subprocess

from common import artifact, encoded, load, require, sha, verify_source

BUNDLE_SCOPE = "confirmed-funding diagnostic only; no pool_wallet_executor, board_proof, aggregate_source_policy, vtxo_verifier, bark or full pool-control claim"
IDENTITY_SCOPE = "confirmed-funding diagnostic; no pool_wallet_executor, board_proof, or full pool-control claim"
CHECKS = {"fmt", "clippy-agent-ark", "clippy-lab", "agent-ark", "http-sse", "wallet-ui", "access", "lib", "replay", "node", "build", "browser-build", "compiled-browser", "nix-closure-build"}
LEGACY_FILES = {
    "bins/spc-pool/src/main.rs": "4e0fdf73f9d672dbb89b41955702c2beadd6b8208ef959ab755614fd0430818c",
    "bins/spc-pool/src/ark/board.rs": "4687c163ac49019b565f9f11745dfc5910c6ee97ea89fbd8b3932c1c42d768f7",
    "bins/spc-pool/src/ark/bark_stock.rs": "c0bc12dbdb2242697b56d37132cf7e843e682da288a43db0c166ff90dddc4690",
}


def supports(bundle):
    return "pool_source_identity" in bundle and "diagnostic_source_policy" not in bundle


def runtime_policy(bundle):
    # Source identity is checked by canonical admission before construction.
    from source_scope import scenario_template_sha256
    require(bundle.get("scope") == BUNDLE_SCOPE, "unsupported received diagnostic scope")
    return {"legacy_maintenance": {"max_runtime_secs": 7200},
            "runtime_bindings": {"scenario_template_sha256": scenario_template_sha256()},
            "scope": "confirmed-funding", "legacy_maintenance_exposure": "enabled, bounded observation; not audited funded Ark acceptance"}


def check(bundle, validation, manifest, root, source_identity, files, paths):
    require(source_identity == '/nix/store/wr1ix2s3hnhjhhmi2dv7v5cfd0mb28gc-source'
            and bundle['joined_source_manifest']['sha256'] == '29b029d748880fd0eff5d6d29806f1729be7858a32f2c1f64ef40e36903753a7',
            'received 332 census is qualified only for the historical g105 source')
    require(bundle.get("scope") == BUNDLE_SCOPE, "received bundle scope is not the closed native-only profile")
    require(not any(k in bundle for k in ("pool_control_source_policy", "pool_wallet_executor", "aggregate_source_policy", "vtxo_verifier"))
            and "bark" not in bundle["binaries"], "received diagnostic contains unrelated/full-lifecycle artifacts")
    identity = load(artifact(bundle["pool_source_identity"]))
    require(set(identity) == {"schema", "scope", "pool_binary", "pool_deriver", "nix_src_root", "pool_source_layers", "pool_control_packet_dd9r3_binding"}, "pool identity closed schema")
    require(identity["schema"] == "spc.lab.ap5.pool-source-identity/v1" and identity["scope"] == IDENTITY_SCOPE
            and identity["pool_binary"] == bundle["binaries"]["pool"] and identity["nix_src_root"] == source_identity, "pool identity/source/binary mismatch")
    rows = identity["pool_source_layers"]
    require(len(rows) == len(files) and {r["path"]: r["sha256"] for r in rows} == files, "received pool identity omits actual source rows")
    verify_source(root, manifest, LEGACY_FILES)
    indexed = {r["path"]: r["sha256"] for r in manifest["files"]}
    actual = {str(p.relative_to(root)) for p in root.rglob("*") if p.is_file()}
    require(set(indexed) == actual, "received Nix source census differs from manifest")
    verify_source(root, manifest, indexed)
    comparison = identity["pool_control_packet_dd9r3_binding"]
    control_manifest = load(artifact(comparison["packet_manifest"]))
    matching = [r["path"] for r in control_manifest["files"] if (root / r["path"]).is_file() and sha(root / r["path"]) == r["sha256"]]
    require(not matching and matching == comparison["rows_identical_in_nix_src"], "received source unexpectedly contains dd9 control rows")
    package = Path("/nix/store") / paths["pool"].relative_to("/nix/store").parts[0]
    drv = subprocess.check_output(["nix-store", "-q", "--deriver", str(package)], text=True).strip()
    require(drv == identity["pool_deriver"], "pool source identity names different real deriver")
    from nix_derivation import derivation_record
    env = derivation_record(subprocess.check_output(["nix", "derivation", "show", drv], text=True), drv)["env"]
    env = json.loads(env["__json"]) if "__json" in env else env
    require(env["src"] == source_identity and env.get("doCheck") == "1", "received Nix source/check phase binding")
    flags = env["cargoBuildFlags"]
    flags = shlex.split(flags) if isinstance(flags, str) else flags
    packages = {flags[i+1] for i, f in enumerate(flags[:-1]) if f == "-p"}
    features = {x for i, f in enumerate(flags[:-1]) if f == "--features" for x in flags[i+1].split(",")}
    require({"spc-lab-cli", "spc-lab-txd", "spc-pool", "spc-agent", "spc-cpu-miner", "spc-scan"} <= packages
            and {"spc-pool/rpc", "spc-pool/ark", "spc-agent/ark"} <= features, "Nix package omits required apps/Ark features")
    checks = {c["name"]: c for c in validation["checks"]}
    require(set(checks) == CHECKS and len(checks) == len(validation["checks"]), "received joined gate is not the complete 14-check set")
    logs = {name: artifact(c["log"]).read_text() for name, c in checks.items()}
    gate = Path(checks["agent-ark"]["log"]["path"]).parent
    require(all(Path(c["log"]["path"]).parent == gate for name, c in checks.items() if name != "nix-closure-build"), "mixed focused gate directories")
    script = (gate.parent / "focused-gate.sh").read_text()
    require(re.search(r"(?m)^step agent-ark cargo test .* -p spc-agent --features ark$", script), "actual gate lacks explicit Ark-feature invocation")
    allowed_deltas = {"flake.nix", "nix/lab/ark-runtime.nix", "docs/lab-build-report.html"}
    for name in CHECKS - {"nix-closure-build"}:
        require((gate / (name + ".exit")).read_text().strip() == "0", "recorded gate exit mismatch")
        for when in ("before", "after"):
            frozen = load(gate / f"{name}-{when}.json")
            import hashlib
            require(hashlib.sha256(encoded(frozen["files"])).hexdigest() == frozen["sha256"], "gate source snapshot digest mismatch")
            verify_source(root, manifest, {r["path"]: r["sha256"] for r in frozen["files"] if r["path"] not in allowed_deltas})
    rust = []
    for name in ("agent-ark", "http-sse", "wallet-ui", "access", "lib", "replay"):
        matches = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed", logs[name])
        require(matches and all(int(failed) == 0 for _, failed in matches), "joined Rust gate lacks passing test results")
        rust.extend(int(passed) for passed, _ in matches)
    require(sum(rust) == 296 and sum(int(n) for n in re.findall(r"test result: ok\. (\d+) passed;", logs["agent-ark"])) == 162
            and "test wallet::" in logs["agent-ark"], "joined 296 Rust/162 Ark-agent gate mismatch")
    require(re.search(r"(?m)^ℹ pass 36$", logs["node"]) and re.search(r"(?m)^ℹ fail 0$", logs["node"]), "joined 36 Node tests mismatch")
    browser = json.loads(logs["compiled-browser"])
    require(browser["passed"] == 15 and Path(browser["out"]) == gate / "compiled-browser", "compiled browser gate mismatch")
    proof = load(gate / "compiled-browser/proof.json")
    require(len(proof["checks"]) == 15, "compiled browser case census")
    routes = {"/": "index.html", "/app.js": "app.js", "/style.css": "style.css", "/lab": "lab.html", "/lab.js": "lab.js", "/lab.css": "lab.css", "/access.js": "access.js"}
    observed = {r["route"]: r for r in proof["assetRoutes"] if r["server"] == "compiled-cli"}
    require(set(observed) == set(routes), "compiled UI route census missing base assets")
    for route, name in routes.items():
        require(observed[route]["exact_equal"] is True and observed[route]["sha256"] == sha(root / "bins/spc-lab/static" / name), "compiled UI asset differs from actual Nix source")
    for binary in load(gate / "binaries.json"):
        artifact(binary)
    # This profile deliberately claims source-bound native observations only.
    # The legacy loop is enabled; no new control/R1 code is invoked and no
    # automatic Ark effect is promoted to funded/lifecycle/native-audit proof.
    return runtime_policy(bundle)
