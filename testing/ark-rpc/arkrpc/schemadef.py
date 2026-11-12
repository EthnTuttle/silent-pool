"""Report JSON schemas (stdlib mini-validator, JSON-Schema subset).

Two schema versions coexist (F5):

- SCHEMA_V1: the original report shape. The frozen evidence in
  reports/l2-eb257af7 and reports/v09lab-bcb6d5ee was generated under
  it and stays immutable; it is kept ONLY to keep validating those old
  records.
- SCHEMA (v2): the current shape. The verdict no longer offers a
  full_compat outcome (unreachable on ANY node in read-only mode):
  status is incompatible | surface_compatible, funded_state is
  permanently "unproven", and full_acceptance is permanently false.

validate_report_consistency() cross-validates a v2 report against the
check manifest in arkrpc.contracts: no verdict/check disagreement and
no missing check silently passed.

The subset supports: type, enum, required, properties, items. This is
intentionally small; the schema exists so that every generated report
and every piece of committed evidence can be mechanically validated
(and so the future Rust `node_probe` absorption knows the exact shape).
"""

from __future__ import annotations

import json

from .contracts import CHECKS, MANIFEST_VERSION, VERDICT_RULES, TRAFFIC_CONSUMER, contract_hash

SCHEMA_VERSION = 2

SCHEMA = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "$id": "ap1a-ark-rpc-probe/report.schema.json",
    "title": "AP-1a ark-rpc probe report (schema v2)",
    "type": "object",
    "required": [
        "probe", "probe_version", "schema_version", "generated_utc",
        "run_token", "target", "profile", "contract_hash", "checks",
        "observations", "verdict",
    ],
    "properties": {
        "probe": {"type": "string"},
        "probe_version": {"type": "string"},
        "schema_version": {"type": "integer"},
        "generated_utc": {"type": "string"},
        "run_token": {"type": "string"},
        "target": {
            "type": "object",
            "required": ["label", "kind"],
            "properties": {
                "label": {"type": "string"},
                "kind": {"enum": ["owned-fresh-regtest-node", "remote-url"]},
            },
        },
        "profile": {
            "type": "object",
            "required": ["name", "server_min_rpc_version",
                         "wallet_min_rpc_version", "expected_network",
                         "expected_genesis"],
            "properties": {
                "name": {"type": "string"},
                "server_min_rpc_version": {"type": "integer"},
                "wallet_min_rpc_version": {"type": "integer"},
                "expected_network": {"type": "string"},
                "expected_genesis": {"type": "string"},
                "fee_targets": {"type": "array", "items": {"type": "integer"}},
                "fee_estimate_mode": {"type": "string"},
                "source_citations": {"type": "array",
                                     "items": {"type": "string"}},
            },
        },
        "contract_hash": {"type": "string"},
        "traffic_consumer": {"enum": [TRAFFIC_CONSUMER]},
        "consumer_model_failures": {"type": "array", "items": {"type": "object"}},
        "checks": {
            "type": "array",
            "items": {
                "type": "object",
                "required": ["id", "method", "availability", "behavior",
                             "detail"],
                "properties": {
                    "id": {"type": "string"},
                    "method": {"type": ["string", "null"]},
                    "availability": {"enum": ["passed", "failed", "unknown"]},
                    "behavior": {"enum": ["passed", "failed", "unknown",
                                          "not_run"]},
                    "detail": {"type": "string"},
                    "fail_kind": {"type": "string"},
                    "evidence": {"type": "object"},
                },
            },
        },
        "observations": {"type": "object"},
        "verdict": {
            "type": "object",
            "required": ["status", "reasons", "summary", "funded_state",
                         "full_acceptance"],
            "properties": {
                "status": {"enum": ["incompatible", "surface_compatible",
                                    "unproven"]},
                "reasons": {"type": "array", "items": {"type": "string"}},
                "summary": {"type": "string"},
                "funded_state": {"enum": ["unproven"]},
                "full_acceptance": {"enum": [False]},
                "unproven_checks": {"type": "array",
                                    "items": {"type": "string"}},
            },
        },
        "node": {
            "type": "object",
            "properties": {
                "label": {"type": "string"},
                "binary": {"type": "string"},
                "binary_sha256": {"type": "string"},
                "source_name": {"type": "string"},
                "argv": {"type": "array", "items": {"type": "string"}},
                "env": {"type": "object"},
                "exit": {"type": "object"},
                "rpc_url": {"type": "string"},
            },
        },
    },
}

# v1 schema: kept solely to validate the frozen 2026-11-07 evidence
# directories, which must not be regenerated.
SCHEMA_V1 = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "$id": "ap1a-ark-rpc-probe/report.schema.json (v1, frozen)",
    "title": "AP-1a ark-rpc probe report (schema v1, frozen)",
    "type": "object",
    "required": [
        "probe", "probe_version", "generated_utc", "run_token", "target",
        "profile", "contract_hash", "checks", "observations", "verdict",
    ],
    "properties": {
        "probe": {"type": "string"},
        "probe_version": {"type": "string"},
        "generated_utc": {"type": "string"},
        "run_token": {"type": "string"},
        "target": {
            "type": "object",
            "required": ["label", "kind"],
            "properties": {
                "label": {"type": "string"},
                "kind": {"enum": ["owned-fresh-regtest-node", "remote-url"]},
            },
        },
        "profile": {
            "type": "object",
            "required": ["name", "server_min_rpc_version",
                         "wallet_min_rpc_version", "expected_network",
                         "expected_genesis"],
            "properties": {
                "name": {"type": "string"},
                "server_min_rpc_version": {"type": "integer"},
                "wallet_min_rpc_version": {"type": "integer"},
                "expected_network": {"type": "string"},
                "expected_genesis": {"type": "string"},
                "fee_targets": {"type": "array", "items": {"type": "integer"}},
                "fee_estimate_mode": {"type": "string"},
                "source_citations": {"type": "array",
                                     "items": {"type": "string"}},
            },
        },
        "contract_hash": {"type": "string"},
        "checks": {
            "type": "array",
            "items": {
                "type": "object",
                "required": ["id", "method", "availability", "behavior",
                             "detail"],
                "properties": {
                    "id": {"type": "string"},
                    "method": {"type": "string"},
                    "availability": {"enum": ["passed", "failed", "unknown"]},
                    "behavior": {"enum": ["passed", "failed", "unknown",
                                          "not_run"]},
                    "detail": {"type": "string"},
                    "fail_kind": {"type": "string"},
                    "evidence": {"type": "object"},
                },
            },
        },
        "observations": {"type": "object"},
        "verdict": {
            "type": "object",
            "required": ["status", "reasons", "summary"],
            "properties": {
                # v1 offered "full_compat"; v2 removed it as unreachable
                # in read-only mode (F5).
                "status": {"enum": ["incompatible", "incomplete",
                                    "full_compat"]},
                "reasons": {"type": "array", "items": {"type": "string"}},
                "summary": {"type": "string"},
                "incomplete_checks": {"type": "array",
                                      "items": {"type": "string"}},
            },
        },
        "node": {
            "type": "object",
            "properties": {
                "label": {"type": "string"},
                "binary": {"type": "string"},
                "binary_sha256": {"type": "string"},
                "argv": {"type": "array", "items": {"type": "string"}},
                "env": {"type": "object"},
                "exit": {"type": "object"},
                "rpc_url": {"type": "string"},
            },
        },
    },
}


class SchemaError(ValueError):
    pass


def _type_ok(value, expected):
    if expected == "object":
        return isinstance(value, dict)
    if expected == "array":
        return isinstance(value, list)
    if expected == "string":
        return isinstance(value, str)
    if expected == "boolean":
        return isinstance(value, bool)
    if expected == "integer":
        return isinstance(value, int) and not isinstance(value, bool)
    if expected == "number":
        return isinstance(value, (int, float)) and not isinstance(value, bool)
    if expected == "null":
        return value is None
    raise SchemaError("unknown type %r in schema" % expected)


def validate(instance, schema, path="$"):
    if "enum" in schema:
        if instance not in schema["enum"]:
            raise SchemaError("%s: %r not in enum %s"
                              % (path, instance, schema["enum"]))
        return
    expected = schema.get("type")
    if expected is not None:
        if isinstance(expected, list):
            if not any(_type_ok(instance, t) for t in expected):
                raise SchemaError("%s: expected type %s" % (path, expected))
        elif not _type_ok(instance, expected):
            raise SchemaError("%s: expected type %s, got %r"
                              % (path, expected, instance))
    if isinstance(instance, dict):
        for req in schema.get("required", []):
            if req not in instance:
                raise SchemaError("%s: missing required property %r" % (path, req))
        for key, subschema in schema.get("properties", {}).items():
            if key in instance:
                validate(instance[key], subschema, "%s.%s" % (path, key))
    if isinstance(instance, list):
        item_schema = schema.get("items")
        if item_schema:
            for i, item in enumerate(instance):
                validate(item, item_schema, "%s[%d]" % (path, i))


def validate_report(report):
    """Validate against the schema matching the report's own version."""
    version = report.get("schema_version", 1)
    if version == 1:
        validate(report, SCHEMA_V1)
    elif version == SCHEMA_VERSION:
        validate(report, SCHEMA)
        validate_report_consistency(report)
    else:
        raise SchemaError("unknown schema_version %r" % version)


def validate_report_schema_only(report):
    """Structural validation WITHOUT current-manifest consistency.

    For historical evidence frozen under an earlier contract hash
    (e.g. the r1 reports, pinned to their historical hash): the report
    must still parse against the versioned schema, but its check
    manifest and verdict reasons belong to the contract revision it
    was generated under, so re-deriving them against the CURRENT
    contracts would be anachronistic. Consistency was proven at
    freeze time by that revision's own tests.
    """
    version = report.get("schema_version", 1)
    if version == 1:
        validate(report, SCHEMA_V1)
    elif version == SCHEMA_VERSION:
        validate(report, SCHEMA)
    else:
        raise SchemaError("unknown schema_version %r" % version)


def validate_report_consistency(report):
    """Cross-validate a v2 report against the contracts check manifest.

    F5 rules, enforced mechanically:
    - every check id in the report must be declared in the manifest;
    - every manifest check must be PRESENT in the report (a missing
      check may not be silently skipped or silently passed);
    - the verdict status must agree with the check statuses, using the
      same three-status rule the probe applies: any failed
      availability/behavior => incompatible; else any gating check
      (coverage required/encoding/protocol) with availability != passed
      or behavior not passed/legitimately-unknown => unproven; else
      surface_compatible;
    - funded_state is always "unproven" and full_acceptance always false;
    - "funded-unproven" is always among the reasons, and
      "surface-unproven" appears exactly when the status is "unproven";
    - the reasons are RE-DERIVED from the actual failed kinds and the
      verdict status (D7): every "fail-<kind>" reason must correspond
      to a check that actually failed with that kind (after the
      traffic/observational skips), "surface-unproven" exactly when
      unproven, "funded-unproven" always -- so a fabricated or missing
      reason is rejected, not just a mis-ordered one;
    - reason order matches the frozen verdict rules;
    - unproven_checks matches the checks whose behavior is
      unknown/not_run;
    - checks with coverage class "traffic" (getmempoolinfo) or
      "observational" (getbestblockhash, getnetworkinfo.observe) never
      gate the verdict in either direction: bark c419159 does not
      consume these methods, so their rows are honest traffic-slice /
      API-surface information only (mirroring probe.compute_verdict).
    """
    if report.get("schema_version") != SCHEMA_VERSION:
        raise SchemaError("consistency validation requires schema_version 2")
    if report.get("contract_hash") != contract_hash():
        raise SchemaError("contract lineage mismatch; historical evidence requires schema-only validation")
    if report.get("traffic_consumer") != TRAFFIC_CONSUMER:
        raise SchemaError("missing or mismatched traffic consumer")
    failures = [row for row in report["checks"] if row["id"] == "getmempoolinfo" and
                "failed" in (row["availability"], row["behavior"])]
    if report.get("consumer_model_failures") != failures:
        raise SchemaError("consumer_model_failures must preserve the full model failure verbatim")
    node = report.get("node", {})
    if "launch_profile" in node:
        from .launchprofile import require, _hash, LaunchProfileError
        from .trafficfacts import effective_consensus
        try:
            require(set(node) <= {"label", "source_name", "binary_sha256", "network", "launch_profile",
                                  "effective_consensus", "exit"}, "unknown public node metadata")
            meta = node["launch_profile"]
            require(type(meta) is dict and set(meta) == {"profile_version", "comparison_sha256", "input_sha256",
                "helper_sha256", "parent_death_levels", "environment", "transport"}, "invalid profile metadata")
            require(type(meta["profile_version"]) is int and meta["profile_version"] == 1 and
                    type(meta["parent_death_levels"]) is int and meta["parent_death_levels"] == 2,
                    "invalid lifetime metadata")
            for key in ("comparison_sha256", "input_sha256", "helper_sha256"):
                _hash(meta[key])
            require(meta["environment"] == "finalized-no-inheritance" and
                    meta["transport"] == "validated_lab_transport", "invalid profile metadata policy")
            require(node.get("network") == "regtest", "invalid node network")
            _hash(node.get("binary_sha256"))
            facts = node.get("effective_consensus")
            require(effective_consensus({"consensus": facts}) == facts, "invalid consensus metadata")
        except (ValueError, TypeError, KeyError) as exc:
            raise SchemaError("invalid closed launch-profile metadata") from exc
    by_id = {}
    for check in report["checks"]:
        cid = check["id"]
        if cid not in CHECKS:
            raise SchemaError("check %r is not declared in the contract "
                              "manifest (contracts.CHECKS)" % cid)
        if cid in by_id:
            raise SchemaError("check %r appears more than once" % cid)
        by_id[cid] = check

    missing = [cid for cid in CHECKS if cid not in by_id]
    if missing:
        raise SchemaError(
            "manifest checks missing from the report (a check may not be "
            "silently skipped or silently passed): %s" % sorted(missing))

    verdict = report["verdict"]
    failed_kinds = set()
    surface_unproven = False
    unproven = []
    for check in report["checks"]:
        cid = check["id"]
        meta = CHECKS.get(cid, {})
        coverage = meta.get("coverage", "required")
        if coverage in ("traffic", "observational"):
            # Traffic-slice and observational rows (getmempoolinfo;
            # getbestblockhash, getnetworkinfo.observe) are honest but
            # never gate the bark verdict: bark c419159 does not call
            # these methods. Mirror probe.compute_verdict exactly.
            if check["behavior"] in ("unknown", "not_run"):
                unproven.append(cid)
            continue
        if check["availability"] == "failed":
            failed_kinds.add(check.get("fail_kind") or "shape")
        if check["behavior"] == "failed":
            failed_kinds.add(check.get("fail_kind") or "shape")
        if check["behavior"] in ("unknown", "not_run"):
            unproven.append(cid)
        if coverage in ("required", "encoding", "protocol"):
            if check["availability"] != "passed":
                surface_unproven = True
            elif check["behavior"] not in ("passed", "unknown"):
                surface_unproven = True
            elif (check["behavior"] == "unknown"
                    and not meta.get("unknown_behavior_ok")):
                surface_unproven = True

    if failed_kinds:
        expect_status = "incompatible"
    elif surface_unproven:
        expect_status = "unproven"
    else:
        expect_status = "surface_compatible"
    if verdict["status"] != expect_status:
        raise SchemaError(
            "verdict/check disagreement: status %r but the checks imply %r "
            "(failed kinds: %s)" % (verdict["status"], expect_status,
                                    sorted(failed_kinds)))
    if verdict["funded_state"] != VERDICT_RULES["funded_state"]:
        raise SchemaError("funded_state must be %r in read-only mode"
                          % VERDICT_RULES["funded_state"])
    if verdict["full_acceptance"] is not False:
        raise SchemaError("full_acceptance must be false in read-only mode")
    if "funded-unproven" not in verdict["reasons"]:
        raise SchemaError("reasons must contain the permanent "
                          "'funded-unproven' entry")
    has_surface_unproven = "surface-unproven" in verdict["reasons"]
    if has_surface_unproven != (expect_status == "unproven"):
        raise SchemaError("'surface-unproven' reason must appear exactly "
                          "when the status is 'unproven'")
    # D7: re-derive the FULL reason list from the actual failed kinds
    # (after the traffic/observational skips) and the derived status.
    # A fabricated reason, a missing reason, or a mis-ordered one are
    # all rejected; the report may not claim more or fewer failures
    # than its own check rows imply.
    order = VERDICT_RULES["reason_order"]
    expected_reasons = []
    for reason in order:
        if reason == "funded-unproven":
            expected_reasons.append(reason)
        elif reason == "surface-unproven":
            if expect_status == "unproven":
                expected_reasons.append(reason)
        elif reason.startswith("fail-") and reason[len("fail-"):] in failed_kinds:
            expected_reasons.append(reason)
    if verdict["reasons"] != expected_reasons:
        raise SchemaError(
            "reasons %r do not match the reasons implied by the check "
            "rows (%r; failed kinds: %s): reasons must be re-derived "
            "from the actual failed kinds and coverage, never "
            "hand-assembled" % (verdict["reasons"], expected_reasons,
                                sorted(failed_kinds)))
    if verdict.get("unproven_checks") != unproven:
        raise SchemaError("unproven_checks %r does not match the checks "
                          "whose behavior is unknown/not_run (%d entries)"
                          % (verdict.get("unproven_checks"), len(unproven)))


def dump_schema() -> str:
    return json.dumps(SCHEMA, indent=2, sort_keys=False) + "\n"
