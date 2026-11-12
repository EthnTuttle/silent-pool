"""Command-line interface for the AP-1a ark-rpc probe.

Subcommands:
  probe          probe an already-running node via --rpc-url (read-only)
  node-probe     start an owned fresh regtest node, probe it, reap it
  self-test      run all fixture scenarios against the in-process fake
  contract-show  print the canonical contract JSON and its sha256
  schema-dump    write report.schema.json
  make-fixtures  normalize a real-run capture into fixtures/base_responses.json
  compare        render a gap matrix across two or more reports

Exit codes: 0 when the probe itself ran correctly (regardless of the
node's verdict -- an honestly-failing version gate is a correct
outcome), 1 on usage/operational errors, 2 when our own report fails
schema validation (a probe bug).

Scratch handling (F9): by default the per-run scratch directory (node
datadir + rpc cookie) is REMOVED after the report is written, so the
cookie credential does not linger on disk. Pass --keep-scratch to
retain it for inspection. Frozen evidence under testing/ark-rpc/reports
is never touched by cleanup.
"""

from __future__ import annotations

import argparse
import decimal
import json
import os
import sys

from . import PROBE_ID, __version__
from .client import JsonRpcClient
from .contracts import (ProfileError, canonical_contract_json, contract_hash,
                        get_profile)
from .nodeproc import NodeError, OwnedNode, new_run_token
from .probe import run_probe
from .reporting import (build_manifest, build_report, compare_reports,
                        git_base_commit, write_manifest, write_report)
from .schemadef import SCHEMA, dump_schema, validate_report
from . import selftest as selftest_mod

EXIT_OK = 0
EXIT_ERROR = 1
EXIT_SCHEMA = 2


def _die(message, code=EXIT_ERROR):
    print("error: %s" % message, file=sys.stderr)
    raise SystemExit(code)


def _resolve_profile(args):
    try:
        profile = get_profile(args.ark_profile)
    except ProfileError as exc:
        _die(str(exc))
    # The profile is the contract; --expected-network / --expected-genesis
    # are sanity assertions, not free-form configuration. Fail closed on
    # any disagreement instead of silently redefining the contract.
    if getattr(args, "expected_network", None) not in (None, profile["expected_network"]):
        _die("--expected-network %r disagrees with profile %s (%r); the "
             "profile is the contract" % (args.expected_network,
                                          args.ark_profile,
                                          profile["expected_network"]))
    if getattr(args, "expected_genesis", None) not in (None, profile["expected_genesis"]):
        _die("--expected-genesis %r disagrees with profile %s (%r); the "
             "profile is the contract" % (args.expected_genesis,
                                          args.ark_profile,
                                          profile["expected_genesis"]))
    return profile


def _run_and_report(client, profile, *, target_label, target_kind, run_token,
                    out_dir, node_info=None, exchanges=None):
    result = run_probe(client, profile)
    report = build_report(result, target_label=target_label,
                          target_kind=target_kind, run_token=run_token,
                          contract_hash_value=contract_hash(),
                          node_info=node_info)
    try:
        validate_report(report)
    except Exception as exc:  # our bug, not the node's
        print("internal error: generated report fails its own schema: %s" % exc,
              file=sys.stderr)
        raise SystemExit(EXIT_SCHEMA)
    written = write_report(out_dir, report,
                           exchanges=result.get("exchanges") if exchanges is None else exchanges)
    return report, written


def _print_verdict(report):
    verdict = report["verdict"]
    print("verdict: %s" % verdict["status"])
    print("reasons: %s" % ", ".join(verdict["reasons"]))
    print("%s" % verdict["summary"])
    print("(an honestly-failing version gate is a correct probe outcome; "
          "exit code reflects probe health, not node compatibility)")


# ---------------------------------------------------------------------------


def cmd_probe(args):
    profile = _resolve_profile(args)
    run_token = args.run_token or new_run_token()
    try:
        client = JsonRpcClient(args.rpc_url, args.cookie_file,
                               deadline_s=args.deadline)
    except (ValueError, OSError) as exc:
        _die("cannot build RPC client: %s" % exc)
    report, written = _run_and_report(
        client, profile, target_label=args.label, target_kind="remote-url",
        run_token=run_token, out_dir=args.out)
    for path in written:
        print("wrote %s" % path)
    _print_verdict(report)
    return EXIT_OK


def _load_devcheck_receipt(path, probe_root):
    """D7: load the devcheck gate receipt and bind it to the exact
    devcheck sources, so the manifest pins both the vectors' code and
    their result for this run."""
    try:
        with open(path) as fh:
            receipt = json.load(fh)
    except (OSError, ValueError) as exc:
        _die("cannot read devcheck receipt %s: %s" % (path, exc))
    for key in ("exit_code", "output"):
        if key not in receipt:
            _die("devcheck receipt %s is missing %r (expected the JSON "
                 "written by run_two_nodes.sh)" % (path, key))
    if receipt["exit_code"] != 0:
        _die("devcheck gate FAILED (exit %d) for this run; refusing to "
             "write evidence the golden vectors do not back"
             % receipt["exit_code"])
    import hashlib
    sources = {}
    dev_dir = os.path.join(probe_root, "devcheck")
    for rel in ("Cargo.toml", "Cargo.lock", "src/main.rs"):
        full = os.path.join(dev_dir, rel)
        try:
            with open(full, "rb") as fh:
                sources["devcheck/%s" % rel] = hashlib.sha256(
                    fh.read()).hexdigest()
        except OSError as exc:
            _die("cannot hash devcheck source %s: %s" % (full, exc))
    return {
        "description": (
            "devcheck/ golden-vector gate run immediately before this "
            "probe (run_two_nodes.sh). devcheck/ is excluded from "
            "freeze_recipe BY DESIGN (dev-only crate, own lockfile); "
            "this key binds its exact sources and the gate's exit "
            "receipt to this run."),
        "sources": sources,
        "exit_code": receipt["exit_code"],
        "vectors": receipt.get("vectors"),
        "receipt": receipt["output"],
    }


def cmd_node_probe(args):
    if args.launch_profile:
        return cmd_profile_node_probe(args)
    if any((args.actor_exec, args.actor_exec_sha256, args.controller_pid, args.controller_start_ticks, args.owned_endpoints)):
        _die("parent handoff arguments require --launch-profile")
    profile = _resolve_profile(args)
    run_token = args.run_token or new_run_token()
    scratch = os.path.join(args.scratch_root, run_token)
    try:
        os.makedirs(scratch, exist_ok=True)
    except OSError as exc:
        _die("cannot create scratch dir %s: %s" % (scratch, exc))
    node = None
    try:
        try:
            node = OwnedNode(args.binary, scratch, args.label,
                             max_run_secs=args.max_run_secs, dialect=True,
                             ready_timeout_s=args.ready_timeout,
                             source_name=args.source_name)
        except NodeError as exc:
            _die(str(exc))
        print("run token:   %s" % run_token)
        print("node binary: %s (source %s, sha256 %s)"
             % (node.binary, node.source_name, node.binary_sha256))
        print("rpc url:     %s" % node.rpc_url)
        print("scratch:     %s" % scratch)
        node.start()
        exit_info = None
        try:
            client = node.wait_ready()
            print("node ready; probing (read-only)...")
            node_info = node.describe()
            result = run_probe(client, profile)
            report = build_report(result, target_label=args.label,
                                  target_kind="owned-fresh-regtest-node",
                                  run_token=run_token,
                                  contract_hash_value=contract_hash(),
                                  node_info=node_info)
            try:
                validate_report(report)
            except Exception as exc:
                print("internal error: report fails schema: %s" % exc,
                      file=sys.stderr)
                raise SystemExit(EXIT_SCHEMA)
        finally:
            exit_info = node.stop()
            print("node stopped: %s" % json.dumps(exit_info))
        report["node"]["exit"] = exit_info
        written = write_report(args.out, report,
                               exchanges=result["exchanges"])

        probe_root = os.path.dirname(os.path.dirname(
            os.path.abspath(__file__)))
        dev_verification = None
        if getattr(args, "devcheck_receipt", None):
            dev_verification = _load_devcheck_receipt(
                args.devcheck_receipt, probe_root)
        manifest = build_manifest(
            run_token=run_token,
            contract_hash_value=contract_hash(),
            node_description=node.describe(),
            node_exit=exit_info,
            probe_root=probe_root,
            base_commit=git_base_commit(probe_root),
            extra_sources=[
                {"name": "bark (pinned)", "commit": "c419159",
                 "role": "RPC contract source of truth (see UPSTREAM_DEPS.md)"},
                {"name": node.source_name,
                 "binary_sha256": node.binary_sha256,
                 "role": "probed node binary (pin name given via "
                         "--source-name; distinct from the run label)"},
            ],
            dev_verification=dev_verification,
        )
        write_manifest(args.out, manifest)
        for path in written + [os.path.join(args.out, "manifest.json")]:
            print("wrote %s" % path)
        _print_verdict(report)
    finally:
        # D6: scratch cleanup runs on success AND on any failure
        # (startup, probe, report, manifest), so the rpc cookie never
        # lingers on disk unless --keep-scratch was given.
        if args.keep_scratch:
            print("node scratch retained for inspection under %s "
                  "(contains the rpc cookie; remove it when done)" % scratch)
        else:
            if node is not None:
                removed = node.remove_scratch()
            else:
                import shutil
                shutil.rmtree(scratch, ignore_errors=True)
                removed = not os.path.exists(scratch)
            if removed:
                print("node scratch removed (cookie credential cleaned up)")
            else:
                print("warning: could not fully remove scratch %s; remove it "
                      "manually (it contains the rpc cookie)" % scratch)
    return EXIT_OK


def cmd_profile_node_probe(args):
    from pathlib import Path
    import stat
    from .launchprofile import loads, normalize, require, _path, LaunchProfileError
    from .nodeproc import ProfileOwnedNode, private_json
    from .trafficfacts import effective_consensus
    from .contracts import ContractError
    profile = _resolve_profile(args)
    node = None
    try:
        require(all((args.actor_exec, args.actor_exec_sha256, args.controller_pid,
                     args.controller_start_ticks)), "profile requires complete controller handoff")
        require(args.run_token is None, "profile report token is generated separately from ownership token")
        profile_stat = Path(args.launch_profile).lstat()
        require(stat.S_ISREG(profile_stat.st_mode) and stat.S_IMODE(profile_stat.st_mode) == 0o600
                and profile_stat.st_uid == os.getuid(), "profile input must be an owned regular 0600 file")
        raw = loads(Path(args.launch_profile).read_bytes())
        endpoints = []
        if args.owned_endpoints:
            data = loads(Path(args.owned_endpoints).read_bytes())
            require(type(data) is dict and set(data) == {"endpoints"} and
                    type(data["endpoints"]) is list and all(type(x) is str for x in data["endpoints"]),
                    "invalid owned endpoint map")
            endpoints = data["endpoints"]
        normalize(raw, owned_endpoints=endpoints)
        _path(args.out)
        require(not os.path.lexists(args.out), "output must be fresh")
        # Every existing ancestor is a real directory; output is exclusively
        # created only after all profile/binary/handoff validation succeeds.
        for parent in Path(args.out).parents:
            mode = parent.lstat().st_mode
            require(stat.S_ISDIR(mode) and not stat.S_ISLNK(mode), "output ancestor is not a real directory")
        node = ProfileOwnedNode(args.binary, args.out, args.label, raw,
            helper=args.actor_exec, helper_sha256=args.actor_exec_sha256,
            controller_pid=args.controller_pid, controller_ticks=args.controller_start_ticks,
            owned_endpoints=endpoints, ready_timeout_s=args.ready_timeout,
            source_name=args.source_name)
        os.mkdir(args.out, 0o700)
        run_token = new_run_token()  # public report id; never SPC_LAB_RUN_TOKEN
        try:
            node.start()
            client = node.wait_ready()
            rec = client.call("getblockchaininfo", [])
            require(rec.error is None and type(rec.result) is dict and rec.result.get("chain") == "regtest",
                    "effective chain unavailable")
            consensus = effective_consensus(rec.result)
            result = run_probe(client, profile)
            info = node.describe()
            info["effective_consensus"] = consensus
            report = build_report(result, target_label=args.label,
                target_kind="owned-fresh-regtest-node", run_token=run_token,
                contract_hash_value=contract_hash(), node_info=info)
            validate_report(report)
        finally:
            exit_info = node.stop()
            private_json(os.path.join(args.out, "node-launch-exit.json"), exit_info)
        report["node"]["exit"] = exit_info
        write_report(args.out, report, exchanges=result["exchanges"])
        probe_root = str(Path(__file__).resolve().parents[1])
        manifest = build_manifest(run_token=run_token, contract_hash_value=contract_hash(),
            node_description=node.describe(), node_exit=exit_info, probe_root=probe_root,
            base_commit=git_base_commit(probe_root),
            dialect_note="profile mode: exact finalized environment; no injected dialect or inheritance")
        write_manifest(args.out, manifest)
        _print_verdict(report)
        return EXIT_OK
    except (OSError, ValueError, NodeError, ContractError) as exc:
        # Profile values / log tails / ownership token never enter diagnostics.
        _die(str(exc) if isinstance(exc, LaunchProfileError) else "profile probe failed; private lifecycle receipts retained")
    finally:
        if node is not None:
            node.stop()
            if not args.keep_scratch:
                node.remove_scratch()


def cmd_self_test(args):
    fixtures = args.fixtures or selftest_mod.FIXTURES_DIR
    if not os.path.isfile(os.path.join(fixtures, "base_responses.json")):
        _die("fixtures not found under %s (run a real probe and "
             "`make-fixtures` first)" % fixtures)
    results = selftest_mod.run_all_scenarios(fixtures, verbose=args.verbose)
    failed = [name for name, failures in results if failures]
    print("\n%d scenarios, %d failed" % (len(results), len(failed)))
    return EXIT_OK if not failed else EXIT_ERROR


def cmd_contract_show(_args):
    print(canonical_contract_json())
    print("# contract sha256: %s" % contract_hash())
    return EXIT_OK


def cmd_schema_dump(args):
    with open(args.out, "w") as fh:
        fh.write(dump_schema())
    print("wrote %s" % args.out)
    return EXIT_OK


def cmd_make_fixtures(args):
    with open(args.capture) as fh:
        capture = json.load(fh, parse_float=decimal.Decimal)
    exchanges = capture["exchanges"]
    entries = []
    seen = set()
    for key in sorted(exchanges):
        e = exchanges[key]
        # The same (method, params) may be exercised under both
        # envelopes (e.g. getrawtransaction [txid, true] as 1.0 and
        # 2.0); the fake server matches on (method, params) only, so
        # keep the first replay entry and skip duplicates.
        dedupe_key = (e["method"], json.dumps(e.get("params", []),
                                              sort_keys=True,
                                              default=str))
        if dedupe_key in seen:
            continue
        seen.add(dedupe_key)
        entry = {"method": e["method"], "params": e["params"]}
        if e.get("error") is not None:
            err = e["error"]
            entry["error"] = {"code": err.get("code"),
                              "message": err.get("detail", "")}
        else:
            entry["result"] = _jsonable(e["result"])
        entries.append(entry)
    normalizations = []
    for entry in entries:
        if entry.get("method") == "getblockchaininfo" and "result" in entry:
            if "size_on_disk" in entry["result"]:
                entry["result"]["size_on_disk"] = 296
                normalizations.append(
                    "getblockchaininfo.size_on_disk pinned to 296 "
                    "(datadir-dependent on a real node)")
    out = {
        "description": ("normalized RPC exchanges captured from a real "
                        "owned fresh regtest node; the self-test fake "
                        "server replays them"),
        "captured_run_token": capture.get("run_token"),
        "normalizations": normalizations,
        "entries": entries,
    }
    with open(args.out, "w") as fh:
        json.dump(_jsonable(out), fh, indent=2)
        fh.write("\n")
    print("wrote %s (%d entries)" % (args.out, len(entries)))
    return EXIT_OK


def _jsonable(value):
    if isinstance(value, decimal.Decimal):
        return float(value)
    if isinstance(value, list):
        return [_jsonable(v) for v in value]
    if isinstance(value, dict):
        return {k: _jsonable(v) for k, v in value.items()}
    return value


def cmd_compare(args):
    reports = []
    for path in args.reports:
        with open(path) as fh:
            report = json.load(fh)
        validate_report(report)
        reports.append((report["target"]["label"], report))
    markdown = compare_reports(reports)
    if args.out:
        with open(args.out, "w") as fh:
            fh.write(markdown)
        print("wrote %s" % args.out)
    else:
        sys.stdout.write(markdown)
    return EXIT_OK


# ---------------------------------------------------------------------------


def build_parser():
    parser = argparse.ArgumentParser(
        prog="probe.py", description="AP-1a read-only ark-rpc node probe")
    parser.add_argument("--version", action="version",
                        version="%s %s" % (PROBE_ID, __version__))
    sub = parser.add_subparsers(dest="command", required=True)

    p = sub.add_parser("probe", help="probe an already-running node (read-only)")
    p.add_argument("--rpc-url", required=True,
                   help="http://host:port of the node RPC (loopback lab nodes)")
    p.add_argument("--cookie-file", required=True,
                   help="Core-format user:pass cookie file (never argv)")
    p.add_argument("--ark-profile", default="bark-c419159")
    p.add_argument("--expected-network", default=None)
    p.add_argument("--expected-genesis", default=None)
    p.add_argument("--label", default="remote-node")
    p.add_argument("--out", required=True, help="artifact directory")
    p.add_argument("--deadline", type=float, default=10.0)
    p.add_argument("--run-token", default=None)
    p.set_defaults(func=cmd_probe)

    p = sub.add_parser("node-probe",
                       help="start an owned fresh regtest node and probe it")
    p.add_argument("--binary", required=True, help="rbitcoin-node binary path")
    p.add_argument("--launch-profile", help="closed spc-lab-launch-profile/1 JSON (owned fresh mode only)")
    p.add_argument("--actor-exec", help="absolute already-built spc-lab executable (actor-exec subcommand)")
    p.add_argument("--actor-exec-sha256")
    p.add_argument("--controller-pid", type=int)
    p.add_argument("--controller-start-ticks", type=int)
    p.add_argument("--owned-endpoints", help="private controller-owned endpoint map JSON")
    p.add_argument("--label", required=True)
    p.add_argument("--source-name", required=True,
                   help="pin name of the binary's origin tree (e.g. "
                        "rbitcoin-lab-retarget-profile or rbitcoin-v0.9-lab); "
                        "recorded in the manifest, distinct from the label")
    p.add_argument("--out", required=True, help="artifact directory")
    p.add_argument("--ark-profile", default="bark-c419159")
    p.add_argument("--expected-network", default=None)
    p.add_argument("--expected-genesis", default=None)
    p.add_argument("--scratch-root", default="/tmp/opencode")
    p.add_argument("--max-run-secs", type=int, default=300)
    p.add_argument("--ready-timeout", type=float, default=90.0)
    p.add_argument("--run-token", default=None)
    p.add_argument("--keep-scratch", action="store_true",
                   help="retain the scratch dir (contains the rpc cookie) "
                        "for inspection; default is to remove it")
    p.add_argument("--devcheck-receipt", default=None,
                   help="JSON receipt of the devcheck golden-vector gate "
                        "(written by run_two_nodes.sh); binds devcheck "
                        "sources + exit code into the run manifest. "
                        "Fails closed if the gate did not pass")
    p.set_defaults(func=cmd_node_probe)

    p = sub.add_parser("self-test", help="run fixture scenarios against a fake node")
    p.add_argument("--fixtures", default=None)
    p.add_argument("--verbose", action="store_true")
    p.set_defaults(func=cmd_self_test)

    p = sub.add_parser("contract-show",
                       help="print the canonical contract JSON + sha256")
    p.set_defaults(func=cmd_contract_show)

    p = sub.add_parser("schema-dump", help="write report.schema.json")
    p.add_argument("--out", default="report.schema.json")
    p.set_defaults(func=cmd_schema_dump)

    p = sub.add_parser("make-fixtures",
                       help="normalize a real-run capture into base fixtures")
    p.add_argument("--capture", required=True,
                   help="raw_responses.json from a real probe run")
    p.add_argument("--out", required=True)
    p.set_defaults(func=cmd_make_fixtures)

    p = sub.add_parser("compare", help="gap matrix across two or more reports")
    p.add_argument("--reports", nargs="+", required=True)
    p.add_argument("--out", default=None)
    p.set_defaults(func=cmd_compare)

    return parser


def main(argv=None):
    parser = build_parser()
    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
