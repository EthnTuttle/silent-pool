#!/usr/bin/env python3
"""Closed test-only namespace SQL helper. No caller command or SQL accepted."""
import argparse
import hashlib
import json
from pathlib import Path
import sys

from oracle import Inconclusive, Invalid, require, strict_json
from storage import (StorageExecutionError, load_plan, observe_local,
                      private_bytes, sha_file, query_spec, CONTRACT_SOURCE_FILES)
from storage_queries import QUERY_SHA256


def execute_request(path):
    path = Path(path)
    # Its directory contract is exact, established by the collector.
    require(path.parent.name == "acceptance-sql" and path.parent.parent.name == "private", "SQL helper request path")
    root = path.parent.parent.parent
    request = strict_json(private_bytes(path, root))
    base = {"schema", "run_dir", "database_id", "binding", "query_sha256", "psql",
            "helper_sha256", "storage_sha256", "queries_source_sha256", "oracle_sha256"}
    require(set(request) in (base, base | {"query_contract", "contract_sources"}),
            "SQL helper request fields")
    require(request["schema"] == "spc-lab-sql-helper-request/1" and request["run_dir"] == str(root), "SQL helper request run")
    contract = request.get("query_contract")
    if contract is not None:
        require(set(request["contract_sources"]) == set(CONTRACT_SOURCE_FILES),
                "SQL contract source census")
        for name, digest in request["contract_sources"].items():
            require(sha_file(Path(__file__).with_name(name)) == digest, "SQL contract source changed")
    require(request["query_sha256"] == hashlib.sha256(query_spec(contract)[0].encode()).hexdigest(), "SQL helper query identity")
    for key, file in (("helper_sha256", Path(__file__)), ("storage_sha256", Path(__file__).with_name("storage.py")),
                      ("queries_source_sha256", Path(__file__).with_name("storage_queries.py")),
                      ("oracle_sha256", Path(__file__).with_name("oracle.py"))):
        require(sha_file(file) == request[key], "SQL helper source seal changed")
    require(set(request["psql"]) == {"path", "sha256"}, "SQL helper psql identity fields")
    plan = load_plan(root, request["database_id"])
    require(plan.route == "namespace_tcp" and plan.public() == request["binding"], "SQL helper storage placement binding")
    return observe_local(plan, request["psql"], contract)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--request", required=True)
    args = parser.parse_args()
    try:
        capture = execute_request(args.request)
        print(json.dumps({"status": "PASSED", "capture": capture}, default=str))
        return 0
    except StorageExecutionError as error:
        print(json.dumps({"status": "INCONCLUSIVE", "execution": error.receipt}))
        return 2
    except (OSError, ValueError, KeyError, TypeError, Invalid, Inconclusive):
        # Closed error code only. No URL, password, environment or SQL stderr.
        print('{"status":"INCONCLUSIVE","code":"sql_binding_or_execution_failed"}')
        return 2


if __name__ == "__main__":
    sys.exit(main())
