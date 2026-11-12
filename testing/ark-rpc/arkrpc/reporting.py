"""Report and manifest generation.

report.json: machine-readable, schema-validated (see schemadef.py, v2).
report.md:  human-readable, led by the verdict and the gap matrix.
manifest.json: provenance freeze: base commit, per-file sha256 recipe,
node binary hash, source name, argv, dialect provenance, contract hash.

Privacy: reports never contain credentials. The cookie file CONTENTS are
never recorded (only its basename); all free text passes through
client.redact(), which is credential-exact (it removes registered secret
VALUES, not every line mentioning the word "cookie"), so non-secret
argv such as `--rpc-cookie-file <path>` survives redaction intact.
"""

from __future__ import annotations

import datetime
import decimal
import json
import os

from . import PROBE_ID, __version__
from .client import redact
from .schemadef import SCHEMA_VERSION
from .contracts import TRAFFIC_CONSUMER

CHECK_ORDER_HINT = (
    "getnetworkinfo", "getnetworkinfo.gate", "getblockchaininfo",
    "getindexinfo", "getblockcount", "getbestblockhash", "getblockhash",
    "getblock.", "getblockheader", "getrawtransaction", "gettxout",
    "getrawmempool", "getmempoolinfo", "getmempoolentry", "estimatesmartfee",
    "protocol.",
    "testmempoolaccept", "submitpackage", "sendrawtransaction",
)


def utc_now() -> str:
    return datetime.datetime.now(datetime.timezone.utc).strftime(
        "%Y-%m-%dT%H:%M:%SZ")


def build_report(probe_result, *, target_label, target_kind, run_token,
                 contract_hash_value, node_info=None, profile_name=None):
    profile = dict(probe_result["profile"])
    if profile_name:
        profile.setdefault("name", profile_name)
    report = {
        "probe": PROBE_ID,
        "probe_version": __version__,
        "schema_version": SCHEMA_VERSION,
        "generated_utc": utc_now(),
        "run_token": run_token,
        "target": {"label": target_label, "kind": target_kind},
        "profile": {
            "name": profile.get("name", "bark-c419159"),
            "server_min_rpc_version": profile["server_min_rpc_version"],
            "wallet_min_rpc_version": profile["wallet_min_rpc_version"],
            "expected_network": profile["expected_network"],
            "expected_genesis": profile["expected_genesis"],
            "fee_targets": profile.get("fee_targets"),
            "fee_estimate_mode": profile.get("fee_estimate_mode"),
            "source_citations": profile.get("source_citations"),
        },
        "contract_hash": contract_hash_value,
        "traffic_consumer": TRAFFIC_CONSUMER,
        "consumer_model_failures": [dict(row) for row in probe_result["checks"]
                                    if row["id"] == "getmempoolinfo" and
                                    "failed" in (row["availability"], row["behavior"])],
        "checks": _redact_value(probe_result["checks"]),
        "observations": _redact_value(probe_result["observations"]),
        "verdict": probe_result["verdict"],
    }
    if node_info is not None:
        report["node"] = _redact_value(node_info)
    return report


def _redact_value(value):
    """Redact credentials and make values JSON-serializable.

    Decimal diagnostics become strings; numeric observations must never
    acquire a float round trip. Traffic fee facts retain raw lexemes separately.
    """
    if isinstance(value, str):
        return redact(value)
    if isinstance(value, decimal.Decimal):
        return str(value)
    if isinstance(value, list):
        return [_redact_value(v) for v in value]
    if isinstance(value, dict):
        return {k: _redact_value(v) for k, v in value.items()}
    return value


def write_report(out_dir, report, exchanges=None):
    """Write report.json, report.md, and raw_responses.json; returns paths."""
    os.makedirs(out_dir, exist_ok=True)
    report_path = os.path.join(out_dir, "report.json")
    md_path = os.path.join(out_dir, "report.md")
    with open(report_path, "w") as fh:
        json.dump(report, fh, indent=2, sort_keys=False, default=str)
        fh.write("\n")
    with open(md_path, "w") as fh:
        fh.write(render_markdown(report))
        fh.write("\n")
    written = [report_path, md_path]
    if exchanges is not None:
        raw_path = os.path.join(out_dir, "raw_responses.json")
        with open(raw_path, "w") as fh:
            json.dump({"run_token": report["run_token"],
                       "exchanges": _redact_value(exchanges)},
                      fh, indent=2, default=str)
            fh.write("\n")
        written.append(raw_path)
    return written


def _check_sort_key(check):
    cid = check["id"]
    for i, prefix in enumerate(CHECK_ORDER_HINT):
        if cid.startswith(prefix):
            return (i, cid)
    return (len(CHECK_ORDER_HINT), cid)


def render_markdown(report) -> str:
    verdict = report["verdict"]
    obs = report.get("observations", {})
    lines = []
    lines.append("# AP-1a ark-rpc probe report: %s" % report["target"]["label"])
    lines.append("")
    lines.append("- probe: `%s` v%s (report schema v%s)"
                 % (report["probe"], report["probe_version"],
                    report.get("schema_version", 1)))
    lines.append("- generated (UTC): %s" % report["generated_utc"])
    lines.append("- run token: `%s`" % report["run_token"])
    lines.append("- target kind: %s" % report["target"]["kind"])
    lines.append("- profile: `%s` (server >= %d, wallet >= %d, network `%s`)"
                 % (report["profile"]["name"],
                    report["profile"]["server_min_rpc_version"],
                    report["profile"]["wallet_min_rpc_version"],
                    report["profile"]["expected_network"]))
    lines.append("- contract hash: `%s`" % report["contract_hash"])
    node = report.get("node")
    if node:
        lines.append("- node binary: `%s` (source `%s`, sha256 %s)"
                     % (node.get("binary"), node.get("source_name"),
                        node.get("binary_sha256")))
    lines.append("")
    lines.append("## Verdict")
    lines.append("")
    lines.append("**%s**" % verdict["status"])
    lines.append("")
    lines.append("Reasons: %s" % ", ".join("`%s`" % r for r in verdict["reasons"]))
    lines.append("")
    lines.append("%s" % verdict["summary"])
    lines.append("")
    lines.append("- funded state: **%s** (read-only mode cannot observe "
                 "funded broadcast/mempool behavior)" % verdict["funded_state"])
    lines.append("- full acceptance: **%s** (requires a separate funded "
                 "packet; a version bump plus read-only passes can never "
                 "grant it)" % verdict["full_acceptance"])
    if verdict.get("unproven_checks"):
        lines.append("")
        lines.append("Unproven (unknown/not_run) checks: %s"
                     % ", ".join("`%s`" % c
                                 for c in verdict["unproven_checks"]))
    lines.append("")
    lines.append("## Gap matrix")
    lines.append("")
    lines.append("| Check | Method | Availability | Behavior | Detail |")
    lines.append("|---|---|---|---|---|")
    for check in sorted(report["checks"], key=_check_sort_key):
        lines.append("| `%s` | `%s` | %s | %s | %s |"
                     % (check["id"], check["method"],
                        _badge(check["availability"]),
                        _badge(check["behavior"]),
                        _md_cell(check["detail"])))
    lines.append("")
    lines.append("## Observations")
    lines.append("")
    for key in sorted(obs):
        lines.append("- **%s**: %s" % (key, _md_cell(obs[key])))
    lines.append("")
    lines.append("## Non-claims")
    lines.append("")
    lines.append("- Funded broadcast paths (testmempoolaccept / submitpackage "
                 "/ sendrawtransaction with valid funded transactions) were "
                 "deliberately NOT exercised; their behavior is `not_run`.")
    lines.append("- No transaction funding, mining, or block generation was "
                 "performed; the node stayed read-only.")
    lines.append("- Fee/mempool data on an empty mempool is `unknown`, "
                 "never a pass.")
    lines.append("- Read-only mode offers no full-compat outcome: the best "
                 "reachable result is `surface_compatible` with "
                 "`funded-unproven`; full acceptance requires a separate "
                 "funded packet.")
    lines.append("- No Core version was faked, shimmed, or overridden; the "
                 "version gate fails honestly on nodes reporting 190000.")
    lines.append("- The genesis-coinbase `getrawtransaction` -5 and "
                 "`gettxout` null outcomes are Core-identical behavior, "
                 "not rbitcoin quirks.")
    return "\n".join(lines)


def _badge(status):
    return status


def _md_cell(value):
    text = value if isinstance(value, str) else json.dumps(value, default=str)
    return text.replace("|", "\\|").replace("\n", " ")


# ---------------------------------------------------------------------------
# Manifest
# ---------------------------------------------------------------------------


def _hash_tree(root, exclude_dirs=("reports", "__pycache__", ".pytest_cache",
                                   "devcheck")):
    entries = {}
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames
                       if d not in exclude_dirs and not d.startswith(".")]
        for name in sorted(filenames):
            if name.endswith((".pyc", ".pyo")):
                continue
            path = os.path.join(dirpath, name)
            rel = os.path.relpath(path, root)
            entries[rel] = _sha256_file(path)
    return dict(sorted(entries.items()))


def _sha256_file(path):
    import hashlib
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 16), b""):
            h.update(chunk)
    return h.hexdigest()


def git_base_commit(repo_dir):
    import subprocess
    out = subprocess.run(["git", "rev-parse", "HEAD"], cwd=repo_dir,
                         capture_output=True, text=True)
    if out.returncode != 0:
        return None
    return out.stdout.strip()


def build_manifest(*, run_token, contract_hash_value, node_description,
                   node_exit, probe_root, base_commit, dialect_note=None,
                   extra_sources=None, dev_verification=None):
    files = _hash_tree(probe_root)
    manifest = {
        "manifest_version": 2,
        "generated_utc": utc_now(),
        "run_token": run_token,
        "probe": PROBE_ID,
        "probe_version": __version__,
        "report_schema_version": SCHEMA_VERSION,
        "contract_hash": contract_hash_value,
        "base_commit": base_commit,
        "base_commit_note": (
            "worktree HEAD at probe time; the probe files themselves are "
            "uncommitted changes in this packet, hashed below"),
        "freeze_recipe": {
            "description": (
                "sha256 of every source file under testing/ark-rpc at probe "
                "time (reports/, devcheck/, __pycache__ and caches excluded). "
                "To reproduce: re-run the probe from a tree whose file hashes "
                "match this map and the same pinned node binary."),
            "files": files,
        },
        "node": {
            "description": node_description,
            "exit": node_exit,
        },
        "dialect_provenance": {
            "RBITCOIN_RPC_PACKAGE_DIALECT": "1",
            "note": dialect_note or (
                "set in the node environment by the probe launcher. The "
                "package-topology dialect is an RPC-surface knob in the "
                "rbitcoin source (mempool submitpackage gates on Core "
                "IsChildWithParents topology only when this is set), not a "
                "consensus parameter. Source-inspected at "
                "crates/rbitcoin-rpc/src/methods/mempool.rs "
                "(PACKAGE_TOPOLOGY_DISALLOWED)."),
        },
        "upstream_source_pins": extra_sources or [],
    }
    if dev_verification is not None:
        # D7: devcheck/ is excluded from freeze_recipe BY DESIGN (it is
        # a dev-only crate with its own lockfile); this key binds its
        # exact sources AND the gate result receipt to this run
        # explicitly, so the golden vectors backing the contract
        # mini-language are pinned in the same manifest.
        manifest["dev_verification"] = dev_verification
    if "launch_profile" in node_description:
        manifest["dialect_provenance"] = {
            "mode": "finalized-launch-profile",
            "note": "No injection or inheritance; environment bound by private comparison digest.",
        }
    return manifest


def write_manifest(out_dir, manifest):
    path = os.path.join(out_dir, "manifest.json")
    with open(path, "w") as fh:
        json.dump(manifest, fh, indent=2, default=str)
        fh.write("\n")
    return path


# ---------------------------------------------------------------------------
# Compare / gap matrix across nodes
# ---------------------------------------------------------------------------


def compare_reports(reports):
    """reports: list of (label, report dict). Returns markdown string."""
    all_ids = []
    seen = set()
    for _, report in reports:
        for check in report["checks"]:
            if check["id"] not in seen:
                seen.add(check["id"])
                all_ids.append(check["id"])
    all_ids.sort(key=lambda cid: _check_sort_key({"id": cid}))

    labels = [label for label, _ in reports]
    by_label = {}
    for label, report in reports:
        by_label[label] = {c["id"]: c for c in report["checks"]}

    lines = []
    lines.append("# AP-1a ark-rpc gap matrix")
    lines.append("")
    lines.append("Generated (UTC): %s" % utc_now())
    lines.append("")
    lines.append("| Check | %s |" % " | ".join(labels))
    lines.append("|---|%s|" % "|".join(["---"] * len(labels)))
    for cid in all_ids:
        cells = []
        for label in labels:
            check = by_label[label].get(cid)
            if check is None:
                cells.append("-")
            else:
                cells.append("%s / %s" % (check["availability"], check["behavior"]))
        lines.append("| `%s` | %s |" % (cid, " | ".join(cells)))
    lines.append("")
    lines.append("Cells are `availability / behavior`. Statuses: "
                 "`passed`, `failed`, `unknown` (ran but unproven), "
                 "`not_run` (deliberately not exercised).")
    lines.append("")
    lines.append("## Verdicts")
    lines.append("")
    for label, report in reports:
        v = report["verdict"]
        lines.append("- **%s**: %s (%s; funded %s, full acceptance %s)"
                     % (label, v["status"], ", ".join(v["reasons"]),
                        v.get("funded_state", "n/a"),
                        v.get("full_acceptance", "n/a")))
    return "\n".join(lines) + "\n"
