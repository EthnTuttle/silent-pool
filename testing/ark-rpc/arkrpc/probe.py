"""Check orchestration: the read-only probe itself.

Every check is reported on two axes:

- availability: did the node produce a usable RPC exchange for the
  method (or, for checks whose expected outcome IS a domain error, the
  expected error that proves the method is routed)?
- behavior: does the observed exchange satisfy the pinned-consumer
  contract?

Statuses are passed / failed / unknown / not_run. `unknown` means the
probe ran the exchange but the result does not prove the property (for
example, fee estimates on an empty mempool, or a mempool-entry shape
that needs a funded transaction). `not_run` means the path was
deliberately not exercised: the funded broadcast paths
(testmempoolaccept / submitpackage / sendrawtransaction with a valid
funded transaction) are classified not_run unless an explicit funded
fixture is supplied by a separate opt-in packet. An invalid sample
proves method recognition only, never funded behavior.

JSON-RPC protocol errors (-32700 / -32600) and id mismatches are
protocol failures: they never count as evidence for or against method
support (F3/F8).

Verdict logic (manifest v2; mirrored by schemadef and by
contracts.VERDICT_RULES):

- any failed check -> status "incompatible", with reasons ordered:
  fail-version-gate, fail-method-surface, fail-encoding,
  fail-protocol, fail-network, fail-genesis, fail-shape, then the
  permanent "funded-unproven" reason;
- no failures but some gating check (coverage required/encoding/
  protocol) not proven available -> status "unproven" with the
  "surface-unproven" reason: protocol-level, auth, or transport
  failures must never be laundered into surface_compatible;
- no failures and every gating check proven -> status
  "surface_compatible".

Checks with coverage class "traffic" (the T0 lab-traffic-contract
subset: getmempoolinfo) never gate the verdict in either direction:
bark c419159 does not consume the method, so their honest
passed/failed rows are traffic-slice information only.

Read-only mode never offers a full-compat outcome: funded_state is
permanently "unproven" and full_acceptance permanently false, because
funded broadcast/mempool behavior is unobservable here. A passing
version gate plus every read-only check yields surface_compatible --
surface and encoding compatibility -- never full acceptance.
"""

from __future__ import annotations

import hashlib

from .client import CallRecord, JsonRpcClient
from .contracts import CONTRACTS, ContractError, parse_spec

# Core regtest genesis coinbase txid (the regtest genesis block commits
# to the same coinbase transaction as mainnet).
GENESIS_COINBASE_TXID = (
    "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b"
)

# Valid hex, not a valid transaction: proves the method is routed without
# exercising any funded (mutating) behavior.
INVALID_TX_HEX = "00"

PASSED = "passed"
FAILED = "failed"
UNKNOWN = "unknown"
NOT_RUN = "not_run"

FAIL_KIND_ORDER = [
    "version-gate",
    "method-surface",
    "encoding",
    "protocol",
    "network",
    "genesis",
    "shape",
]


def _sha256d(b: bytes) -> bytes:
    return hashlib.sha256(hashlib.sha256(b).digest()).digest()


def hash_display(b: bytes) -> str:
    """Core display form of a double-SHA256 digest (bytes reversed)."""
    return _sha256d(b)[::-1].hex()


def _mk(cid, method, availability, behavior, detail, fail_kind=None,
        evidence=None):
    rec = {
        "id": cid,
        "method": method,
        "availability": availability,
        "behavior": behavior,
        "detail": detail,
    }
    if fail_kind:
        rec["fail_kind"] = fail_kind
    if evidence:
        rec["evidence"] = evidence
    return rec


class ProbeSession:
    """Collects checks, observations, and raw exchanges for one probe run."""

    def __init__(self, client: JsonRpcClient, profile: dict,
                 expected_network: str, expected_genesis: str):
        self.client = client
        self.profile = profile
        self.expected_network = expected_network
        self.expected_genesis = expected_genesis
        self.checks = []
        self.observations = {}
        self.exchanges = {}  # exchange key -> CallRecord.as_dict()
        self.id_match_count = 0
        self.id_mismatch_count = 0

    def call(self, method, params, envelope="1.0") -> CallRecord:
        rec = self.client.call(method, params, envelope=envelope)
        key = "%s %s" % (method, _params_key(params))
        if envelope != "1.0":
            key += " [jsonrpc %s]" % envelope
        self.exchanges[key] = rec.as_dict()
        # C1: every receipt is counted, on the 200 path and the
        # HTTP-error path alike, regardless of how the call was
        # categorized afterwards. A receipt with id_match False is a
        # mismatch even when the client classified the call as a
        # server error or method_missing behind the broken echo.
        if rec.receipt is not None:
            if rec.receipt["id_match"]:
                self.id_match_count += 1
            else:
                self.id_mismatch_count += 1
        return rec

    def add(self, check):
        self.checks.append(check)

    def observe(self, key, value):
        self.observations[key] = value


def _params_key(params) -> str:
    import json
    return json.dumps(params, sort_keys=True, default=str)


def _err(rec: CallRecord) -> str:
    return str(rec.error)


def _is_protocol_level(rec: CallRecord) -> bool:
    """True when the failure says nothing about method support.

    Transport, timeout, auth, parse, bad envelope, protocol errors
    (-32700/-32600) and id mismatches are protocol-level: they cannot
    prove a method missing any more than they can prove it present.
    """
    return rec.error is not None and rec.error.category in (
        "transport", "timeout", "auth", "parse", "bad_jsonrpc",
        "protocol_error", "id_mismatch", "oversized")


def _failure_axes(rec: CallRecord, fail_kind: str):
    """Availability + fail_kind for a check whose upstream call failed.

    - protocol-level failure: availability UNKNOWN (the node's surface
      was not proven either way; the verdict becomes "unproven");
    - method not found (-32601): availability FAILED under
      fail-kind method-surface;
    - any other routed error: availability FAILED under the check's own
      fail kind.
    """
    if _is_protocol_level(rec):
        return UNKNOWN, fail_kind
    if rec.error.category == "method_missing":
        return FAILED, "method-surface"
    return FAILED, fail_kind


def _shape_check(session, cid, method, result, contract_name,
                 fail_kind="shape"):
    """Parse `result` against a named contract; append a behavior check."""
    contract = CONTRACTS[contract_name]
    try:
        parse_spec(result, contract["spec"])
        session.add(_mk(cid, method, PASSED, PASSED,
                        "parsed against %s (%s)" % (
                            contract_name, _first_source(contract)),
                        fail_kind=fail_kind))
        return True
    except ContractError as exc:
        session.add(_mk(cid, method, PASSED, FAILED, "shape: %s" % exc,
                        fail_kind=fail_kind))
        return False


def _first_source(contract) -> str:
    srcs = contract.get("sources") or []
    return srcs[0] if srcs else "see contracts.py"


def _call_failed_rows(session, cid_list, method, rec):
    """Append rows for checks that could not run because a call failed."""
    for cid, fail_kind in cid_list:
        availability, fk = _failure_axes(rec, fail_kind)
        session.add(_mk(cid, method, availability, UNKNOWN,
                        "not evaluated: %s call failed: %s" % (method, _err(rec)),
                        fail_kind=fk))


# ---------------------------------------------------------------------------
# Check groups
# ---------------------------------------------------------------------------


def _check_getnetworkinfo(session):
    rec = session.call("getnetworkinfo", [], envelope="1.0")
    gates = [
        ("getnetworkinfo.gate.server", "version-gate"),
        ("getnetworkinfo.gate.wallet", "version-gate"),
    ]
    if rec.error is not None:
        availability, fk = _failure_axes(rec, "shape")
        session.add(_mk("getnetworkinfo.version-shape", "getnetworkinfo",
                        availability, UNKNOWN,
                        "not evaluated: call failed: %s" % _err(rec),
                        fail_kind=fk))
        # Observational coverage never gates the verdict (F4): when the
        # call fails, record unknown rather than failed.
        session.add(_mk("getnetworkinfo.observe", "getnetworkinfo",
                        UNKNOWN, UNKNOWN,
                        "not evaluated: call failed: %s" % _err(rec)))
        _call_failed_rows(session, gates, "getnetworkinfo", rec)
        _sync20_getnetworkinfo(session, rec)
        return

    result = rec.result
    _shape_check(session, "getnetworkinfo.version-shape", "getnetworkinfo",
                 result, "getnetworkinfo.required")

    # Observational superset: recorded, never a verdict failure (F4).
    try:
        obs = parse_spec(result, CONTRACTS["getnetworkinfo.observe"]["spec"])
        session.add(_mk("getnetworkinfo.observe", "getnetworkinfo",
                        PASSED, PASSED,
                        "observation set parsed (not a bark requirement; "
                        "never gates the verdict)"))
        session.observe("networkinfo_observed", {
            k: obs.get(k) for k in ("subversion", "protocolversion",
                                    "relayfee", "incrementalfee")})
    except ContractError as exc:
        session.add(_mk("getnetworkinfo.observe", "getnetworkinfo",
                        PASSED, PASSED,
                        "observation set did not parse (%s); recorded as an "
                        "observation, not a failure: no bark consumer "
                        "deserializes these fields" % exc))

    version = result.get("version") if isinstance(result, dict) else None
    session.observe("rpc_version", version)
    if isinstance(result, dict):
        session.observe("subversion", result.get("subversion"))
        session.observe("protocolversion", result.get("protocolversion"))

    for (cid, fail_kind), key, minimum in (
        (gates[0], "server", session.profile["server_min_rpc_version"]),
        (gates[1], "wallet", session.profile["wallet_min_rpc_version"]),
    ):
        if isinstance(version, bool) or not isinstance(version, int):
            session.add(_mk(cid, "getnetworkinfo", PASSED, UNKNOWN,
                            "version field is not a usable integer "
                            "(observed %r); gate cannot be evaluated" % (version,),
                            fail_kind=fail_kind))
        elif version >= minimum:
            session.add(_mk(cid, "getnetworkinfo", PASSED, PASSED,
                            "version %d >= %s minimum %d" % (version, key, minimum),
                            fail_kind=fail_kind))
        else:
            session.add(_mk(cid, "getnetworkinfo", PASSED, FAILED,
                            "version %d < %s minimum %d: the %s gate fails "
                            "honestly (bark refuses to start against this node)"
                            % (version, key, minimum, key),
                            fail_kind=fail_kind,
                            evidence={"observed": version, "minimum": minimum}))

    _sync20_getnetworkinfo(session, rec)


def _sync20_getnetworkinfo(session, upstream_rec):
    """Encoding check: the sync client's exact form (envelope 2.0,
    numeric nonce id starting at 1), used by bitcoincore-rpc 0.19
    version() and bark's sync BitcoinRpcClient."""
    rec = session.call("getnetworkinfo", [], envelope="2.0")
    if rec.error is not None:
        availability, fk = _failure_axes(rec, "encoding")
        session.add(_mk("protocol.envelope20.getnetworkinfo", "getnetworkinfo",
                        availability, UNKNOWN,
                        "sync-client form (envelope 2.0, numeric id) failed: "
                        "%s" % _err(rec), fail_kind=fk))
        return
    try:
        parse_spec(rec.result, CONTRACTS["getnetworkinfo.required"]["spec"])
        session.add(_mk("protocol.envelope20.getnetworkinfo", "getnetworkinfo",
                        PASSED, PASSED,
                        "sync-client form accepted (envelope 2.0, numeric id "
                        "%d echoed exactly, HTTP %s)"
                        % (rec.request_id, rec.receipt["http_status"]),
                        fail_kind="encoding",
                        evidence={"receipt": rec.receipt}))
    except ContractError as exc:
        session.add(_mk("protocol.envelope20.getnetworkinfo", "getnetworkinfo",
                        PASSED, FAILED, "shape: %s" % exc,
                        fail_kind="encoding"))


def _check_getblockchaininfo(session):
    rec = session.call("getblockchaininfo", [], envelope="1.0")
    if rec.error is not None:
        for cid, fk0 in (("getblockchaininfo.network", "network"),
                        ("getblockchaininfo.shape.async", "shape"),
                        ("getblockchaininfo.shape.sync", "shape")):
            availability, fk = _failure_axes(rec, fk0)
            session.add(_mk(cid, "getblockchaininfo", availability, UNKNOWN,
                            "not evaluated: call failed: %s" % _err(rec),
                            fail_kind=fk))
        return

    result = rec.result
    if isinstance(result, dict):
        session.observe("chain", result.get("chain"))
        for obs_key in ("blocks", "headers", "bestblockhash", "bits", "target",
                        "initialblockdownload", "pruned"):
            if obs_key in result:
                session.observe("blockchaininfo.%s" % obs_key, result[obs_key])

    chain = result.get("chain") if isinstance(result, dict) else None
    if chain == session.expected_network:
        session.add(_mk("getblockchaininfo.network", "getblockchaininfo",
                        PASSED, PASSED,
                        "chain %r matches expected network" % chain,
                        fail_kind="network"))
    else:
        session.add(_mk("getblockchaininfo.network", "getblockchaininfo",
                        PASSED, FAILED,
                        "chain %r != expected network %r; no fallback chain "
                        "is attempted" % (chain, session.expected_network),
                        fail_kind="network", evidence={"observed": chain}))

    # Async client shape: corepc-types 0.13.0 v29::GetBlockchainInfo
    # (requires bits/target/time; re-exported as v30 by the client bark
    # actually uses). This is the shape bark's server network() needs.
    _shape_check(session, "getblockchaininfo.shape.async", "getblockchaininfo",
                 result, "getblockchaininfo.async")
    # Sync companion shape: bitcoincore-rpc-json 0.19 GetBlockchainInfoResult
    # (does NOT require bits/target/time; warnings may be string or array).
    _shape_check(session, "getblockchaininfo.shape.sync", "getblockchaininfo",
                 result, "getblockchaininfo.sync")


def _check_getindexinfo(session):
    rec = session.call("getindexinfo", [], envelope="1.0")
    if rec.error is not None:
        availability, fk = _failure_axes(rec, "shape")
        session.add(_mk("getindexinfo.txindex", "getindexinfo", availability,
                        UNKNOWN,
                        "not evaluated: call failed: %s" % _err(rec),
                        fail_kind=fk))
        return
    result = rec.result
    try:
        parse_spec(result, CONTRACTS["getindexinfo"]["spec"])
    except ContractError as exc:
        session.add(_mk("getindexinfo.txindex", "getindexinfo", PASSED, FAILED,
                        "shape: %s" % exc, fail_kind="shape"))
        return
    txindex = result.get("txindex") if isinstance(result, dict) else None
    if isinstance(txindex, dict):
        session.observe("txindex", txindex)
    if txindex is None:
        session.add(_mk("getindexinfo.txindex", "getindexinfo", PASSED, FAILED,
                        "txindex absent: bark require_txindex fails closed "
                        "(TxindexError::NotEnabled)", fail_kind="shape"))
    elif txindex.get("synced") is True:
        session.add(_mk("getindexinfo.txindex", "getindexinfo", PASSED, PASSED,
                        "txindex present and synced (best_block_height=%r; "
                        "the same field name Bitcoin Core and corepc emit)"
                        % txindex.get("best_block_height"), fail_kind="shape"))
    else:
        session.add(_mk("getindexinfo.txindex", "getindexinfo", PASSED, FAILED,
                        "txindex present but synced=%r (bark requires a "
                        "usable index; probe is stricter than bark here)"
                        % txindex.get("synced"), fail_kind="shape"))


def _check_tips(session):
    # getblockcount
    rec = session.call("getblockcount", [], envelope="1.0")
    if rec.error is not None:
        availability, fk = _failure_axes(rec, "shape")
        session.add(_mk("getblockcount", "getblockcount", availability, UNKNOWN,
                        "not evaluated: call failed: %s" % _err(rec),
                        fail_kind=fk))
    elif isinstance(rec.result, bool) or not isinstance(rec.result, int):
        session.add(_mk("getblockcount", "getblockcount", PASSED, FAILED,
                        "expected JSON integer, got %r" % (rec.result,),
                        fail_kind="shape"))
    else:
        session.observe("tip_height", rec.result)
        session.add(_mk("getblockcount", "getblockcount", PASSED, PASSED,
                        "integer tip height %d" % rec.result,
                        fail_kind="shape"))

    # getbestblockhash: async client API surface (D4). The pinned
    # bitcoind-async-client 0.15.0 v30 Reader exposes it via
    # get_current_timestamp (v30.rs:107-110), but bark c419159 has no
    # getbestblockhash/get_current_timestamp caller, so this check is
    # coverage class "observational": an honest row that never gates
    # the bark verdict in either direction.
    rec = session.call("getbestblockhash", [], envelope="1.0")
    if rec.error is not None:
        availability, fk = _failure_axes(rec, "shape")
        session.add(_mk("getbestblockhash", "getbestblockhash",
                        availability, UNKNOWN,
                        "not evaluated: call failed: %s" % _err(rec),
                        fail_kind=fk))
    else:
        try:
            parse_spec(rec.result, CONTRACTS["getbestblockhash"]["spec"])
            session.add(_mk("getbestblockhash", "getbestblockhash",
                            PASSED, PASSED,
                            "hash32 tip (async client "
                            "get_current_timestamp form; observational: "
                            "bark c419159 never calls it)",
                            fail_kind="shape"))
        except ContractError as exc:
            session.add(_mk("getbestblockhash", "getbestblockhash",
                            PASSED, FAILED, "shape: %s" % exc,
                            fail_kind="shape"))


def _check_genesis_walk(session):
    """Walk the genesis block: hash, header, coinbase tx, txout."""
    rec = session.call("getblockhash", [0], envelope="1.0")
    dependent = [
        ("getblock.verbosity0", "shape"),
        ("getblock.verbosity0.rehash", "shape"),
        ("getblock.verbosity1", "shape"),
        ("getblock.singleparam", "encoding"),
        ("getblockheader.verbose", "shape"),
        ("getblockheader.verbose.genesis", "genesis"),
        ("getblockheader.encoding.sync20", "encoding"),
        ("getblockheader.nonverbose", "encoding"),
        ("getrawtransaction.verbosity0", "shape"),
        ("getrawtransaction.verbosity1", "shape"),
        ("getrawtransaction.bool-verbose", "encoding"),
        ("getrawtransaction.bool-verbose.async", "encoding"),
        ("gettxout.mempool-true", "shape"),
        ("gettxout.mempool-false", "shape"),
        ("getblockheader.traffic-consumer", "shape"),
        ("getrawtransaction.blockhash.traffic-consumer", "shape"),
    ]
    if rec.error is not None:
        availability, fk = _failure_axes(rec, "genesis")
        session.add(_mk("getblockhash.genesis", "getblockhash", availability,
                        UNKNOWN,
                        "not evaluated: call failed: %s" % _err(rec),
                        fail_kind=fk))
        _call_failed_rows(session, dependent, "getblockhash", rec)
        return

    if rec.result == session.expected_genesis:
        session.add(_mk("getblockhash.genesis", "getblockhash", PASSED, PASSED,
                        "genesis hash matches the Core regtest genesis",
                        fail_kind="genesis"))
    else:
        session.add(_mk("getblockhash.genesis", "getblockhash", PASSED, FAILED,
                        "hash at height 0 is %r, expected the Core regtest "
                        "genesis %s" % (rec.result, session.expected_genesis),
                        fail_kind="genesis", evidence={"observed": rec.result}))
    _walk_getblock(session, rec.result)
    _walk_getblockheader(session, rec.result)
    header = session.call("getblockheader", [rec.result], envelope="1.0")
    if header.error is not None:
        _call_failed_rows(session, [("getblockheader.traffic-consumer", "shape")],
                          "getblockheader", header)
    else:
        _shape_check(session, "getblockheader.traffic-consumer", "getblockheader",
                     header.result, "getblockheader.traffic-consumer")
    _walk_getrawtransaction(session, rec.result)
    _walk_gettxout(session)


def _walk_getblock(session, block_hash):
    # verbosity 0, async form [hash, 0] envelope 1.0
    rec = session.call("getblock", [block_hash, 0], envelope="1.0")
    if rec.error is not None:
        availability, fk = _failure_axes(rec, "shape")
        session.add(_mk("getblock.verbosity0", "getblock", availability, UNKNOWN,
                        "not evaluated: call failed: %s" % _err(rec),
                        fail_kind=fk))
        session.add(_mk("getblock.verbosity0.rehash", "getblock", availability,
                        UNKNOWN,
                        "not evaluated: upstream getblock failed",
                        fail_kind=fk))
    else:
        try:
            parse_spec(rec.result, CONTRACTS["getblock.verbosity0"]["spec"])
            session.add(_mk("getblock.verbosity0", "getblock", PASSED, PASSED,
                            "genesis block served as hex (verbosity 0, "
                            "async form [hash, 0])", fail_kind="shape"))
        except ContractError as exc:
            session.add(_mk("getblock.verbosity0", "getblock", PASSED, FAILED,
                            "shape: %s" % exc, fail_kind="shape"))
        try:
            raw = bytes.fromhex(rec.result)
            if len(raw) < 80:
                raise ValueError("block hex is shorter than an 80-byte header")
            header_hash = hash_display(raw[:80])
        except (ValueError, TypeError) as exc:
            session.add(_mk("getblock.verbosity0.rehash", "getblock", PASSED,
                            FAILED, "cannot re-hash header: %s" % exc,
                            fail_kind="shape"))
        else:
            if header_hash == block_hash:
                session.add(_mk("getblock.verbosity0.rehash", "getblock",
                                PASSED, PASSED,
                                "double-SHA256 of the 80-byte header equals "
                                "the requested block hash (integrity proven)",
                                fail_kind="shape"))
            else:
                session.add(_mk("getblock.verbosity0.rehash", "getblock",
                                PASSED, FAILED,
                                "re-hashed header %s != requested hash %s"
                                % (header_hash, block_hash), fail_kind="shape",
                                evidence={"rehashed": header_hash}))

    # verbosity 1: the bdk emitter's get_block_info form, sent EXACTLY
    # as the sync client sends it: [hash, 1] with envelope 2.0 and a
    # numeric nonce id (F2). GetBlockResult contract.
    rec = session.call("getblock", [block_hash, 1], envelope="2.0")
    if rec.error is not None:
        availability, fk = _failure_axes(rec, "shape")
        session.add(_mk("getblock.verbosity1", "getblock", availability, UNKNOWN,
                        "not evaluated: call failed: %s" % _err(rec),
                        fail_kind=fk))
    else:
        ok = _shape_check(session, "getblock.verbosity1", "getblock",
                          rec.result, "getblock.verbosity1")
        if ok and isinstance(rec.result, dict):
            problems = []
            if rec.result.get("hash") != block_hash:
                problems.append("hash %r != requested" % rec.result.get("hash"))
            if rec.result.get("height") != 0:
                problems.append("height %r != 0" % rec.result.get("height"))
            if problems:
                session.checks[-1] = _mk(
                    "getblock.verbosity1", "getblock", PASSED, FAILED,
                    "shape parsed but inconsistent: %s" % "; ".join(problems),
                    fail_kind="shape")

    # single-param form: async get_block_height calls getblock [hash]
    # and relies on the server defaulting verbosity to 1 (F2).
    rec = session.call("getblock", [block_hash], envelope="1.0")
    if rec.error is not None:
        availability, fk = _failure_axes(rec, "encoding")
        session.add(_mk("getblock.singleparam", "getblock", availability,
                        UNKNOWN,
                        "not evaluated: call failed: %s" % _err(rec),
                        fail_kind=fk))
    else:
        try:
            parse_spec(rec.result, CONTRACTS["getblock.verbosity1"]["spec"])
            session.add(_mk("getblock.singleparam", "getblock", PASSED, PASSED,
                            "single-param form accepted ([hash]; verbosity "
                            "defaults to 1 server-side, async "
                            "get_block_height form)", fail_kind="encoding"))
        except ContractError as exc:
            session.add(_mk("getblock.singleparam", "getblock", PASSED, FAILED,
                            "shape: %s" % exc, fail_kind="encoding"))


def _walk_getblockheader(session, block_hash):
    # verbose: async/server call_raw form [hash, true] envelope 1.0
    rec = session.call("getblockheader", [block_hash, True], envelope="1.0")
    if rec.error is not None:
        availability, fk = _failure_axes(rec, "shape")
        session.add(_mk("getblockheader.verbose", "getblockheader",
                        availability,
                        UNKNOWN, "not evaluated: call failed: %s" % _err(rec),
                        fail_kind=fk))
        g_availability, g_fk = _failure_axes(rec, "genesis")
        session.add(_mk("getblockheader.verbose.genesis", "getblockheader",
                        g_availability,
                        UNKNOWN,
                        "not evaluated: upstream getblockheader failed",
                        fail_kind=g_fk))
    else:
        parsed_ok = False
        try:
            parsed = parse_spec(rec.result,
                                CONTRACTS["getblockheader.verbose"]["spec"])
            parsed_ok = True
            session.add(_mk("getblockheader.verbose", "getblockheader",
                            PASSED, PASSED,
                            "parsed against bitcoincore-rpc-json 0.19 "
                            "GetBlockHeaderResult (bark server tx_status "
                            "type; async form [hash, true])",
                            fail_kind="shape"))
        except ContractError as exc:
            session.add(_mk("getblockheader.verbose", "getblockheader",
                            PASSED, FAILED, "shape: %s" % exc,
                            fail_kind="shape"))
        if parsed_ok:
            problems = []
            if parsed.get("hash") != block_hash:
                problems.append("returned header hash %r != requested hash %s"
                                % (parsed.get("hash"), block_hash))
            if parsed.get("hash") != session.expected_genesis:
                problems.append("hash %r != genesis" % parsed.get("hash"))
            if parsed.get("height") != 0:
                problems.append("height %r != 0" % parsed.get("height"))
            if not isinstance(parsed.get("nTx"), int) or parsed.get("nTx") < 1:
                problems.append("nTx %r < 1" % parsed.get("nTx"))
            if problems:
                session.add(_mk("getblockheader.verbose.genesis",
                                "getblockheader", PASSED, FAILED,
                                "; ".join(problems), fail_kind="genesis"))
            else:
                session.add(_mk("getblockheader.verbose.genesis",
                                "getblockheader", PASSED, PASSED,
                                "genesis header: hash/height/nTx consistent "
                                "(nTx=%d, confirmations=%r)"
                                % (parsed["nTx"], parsed.get("confirmations")),
                                fail_kind="genesis"))
        else:
            session.add(_mk("getblockheader.verbose.genesis", "getblockheader",
                            PASSED, UNKNOWN, "not evaluated: verbose shape "
                            "failed", fail_kind="genesis"))

    # sync form [hash, true] envelope 2.0 (encoding check, F2/F3)
    rec = session.call("getblockheader", [block_hash, True], envelope="2.0")
    if rec.error is not None:
        availability, fk = _failure_axes(rec, "encoding")
        session.add(_mk("getblockheader.encoding.sync20", "getblockheader",
                        availability, UNKNOWN,
                        "not evaluated: call failed: %s" % _err(rec),
                        fail_kind=fk))
    else:
        try:
            parse_spec(rec.result, CONTRACTS["getblockheader.verbose"]["spec"])
            session.add(_mk("getblockheader.encoding.sync20", "getblockheader",
                            PASSED, PASSED,
                            "sync form accepted ([hash, true], envelope 2.0, "
                            "numeric id %d echoed)" % rec.request_id,
                            fail_kind="encoding"))
        except ContractError as exc:
            session.add(_mk("getblockheader.encoding.sync20", "getblockheader",
                            PASSED, FAILED, "shape: %s" % exc,
                            fail_kind="encoding"))

    # sync hex form [hash, false] envelope 2.0 (encoding check, F2)
    rec = session.call("getblockheader", [block_hash, False], envelope="2.0")
    if rec.error is not None:
        availability, fk = _failure_axes(rec, "encoding")
        session.add(_mk("getblockheader.nonverbose", "getblockheader",
                        availability, UNKNOWN,
                        "not evaluated: call failed: %s" % _err(rec),
                        fail_kind=fk))
    else:
        try:
            parse_spec(rec.result,
                       CONTRACTS["getblockheader.nonverbose"]["spec"])
            raw = bytes.fromhex(rec.result)
            if len(raw) < 80:
                raise ValueError("header hex shorter than 80 bytes")
            if hash_display(raw[:80]) != block_hash:
                raise ValueError("re-hashed header != requested hash")
            session.add(_mk("getblockheader.nonverbose", "getblockheader",
                            PASSED, PASSED,
                            "sync hex form accepted ([hash, false], envelope "
                            "2.0); 80-byte header re-hash matches",
                            fail_kind="encoding"))
        except (ContractError, ValueError, TypeError) as exc:
            session.add(_mk("getblockheader.nonverbose", "getblockheader",
                            PASSED, FAILED, "shape: %s" % exc,
                            fail_kind="encoding"))


def _walk_getrawtransaction(session, genesis_hash):
    forms = (
        # (check id, params, envelope, form description)
        ("getrawtransaction.verbosity0", [GENESIS_COINBASE_TXID, 0], "1.0",
         "async form [txid, 0]"),
        ("getrawtransaction.blockhash.traffic-consumer",
         [GENESIS_COINBASE_TXID, 0, genesis_hash], "1.0",
         "traffic confirmed lookup [txid, 0, blockhash]; not canonicality proof"),
        ("getrawtransaction.verbosity1", [GENESIS_COINBASE_TXID, 1], "1.0",
         "async form [txid, 1]"),
        ("getrawtransaction.bool-verbose",
         [GENESIS_COINBASE_TXID, True], "2.0",
         "sync form [txid, true] (bitcoincore-rpc 0.19 "
         "get_raw_transaction_info and bitcoin-ext "
         "custom_get_raw_transaction_info: handle_defaults strips the "
         "trailing null, so the wire form is two params, envelope 2.0)"),
        ("getrawtransaction.bool-verbose.async",
         [GENESIS_COINBASE_TXID, True], "1.0",
         "bark call_raw form [txid, true]"),
    )
    for cid, params, envelope, form in forms:
        rec = session.call("getrawtransaction", params, envelope=envelope)
        if rec.error is not None:
            if rec.error.code == -5:
                # Core-identical behavior (F8): Bitcoin Core also does
                # not serve the genesis coinbase via getrawtransaction
                # (code -5); this is not an rbitcoin quirk.
                session.add(_mk(cid, "getrawtransaction", PASSED, UNKNOWN,
                                "node does not serve the genesis coinbase via "
                                "getrawtransaction (code -5) -- Bitcoin Core "
                                "behaves identically; %s routed and reached "
                                "the domain layer; shape unverified without a "
                                "funded fixture" % form,
                                fail_kind="encoding" if cid.startswith(
                                    "getrawtransaction.bool") else "shape"))
            elif _is_protocol_level(rec):
                session.add(_mk(cid, "getrawtransaction", UNKNOWN, UNKNOWN,
                                "could not evaluate (%s): %s" % (form, _err(rec)),
                                fail_kind="encoding" if cid.startswith(
                                    "getrawtransaction.bool") else "shape"))
            else:
                fk = ("method-surface" if rec.error.category == "method_missing"
                      else ("encoding" if cid.startswith("getrawtransaction.bool")
                            else "shape"))
                session.add(_mk(cid, "getrawtransaction", FAILED, UNKNOWN,
                                "not evaluated (%s): call failed: %s"
                                % (form, _err(rec)),
                                fail_kind=fk))
            continue
        fail_kind = ("encoding" if cid.startswith("getrawtransaction.bool")
                     else "shape")
        if cid in ("getrawtransaction.verbosity0", "getrawtransaction.blockhash.traffic-consumer"):
            try:
                raw = bytes.fromhex(rec.result)
                txid = hash_display(raw)
            except (ValueError, TypeError):
                session.add(_mk(cid, "getrawtransaction", PASSED, FAILED,
                                "verbosity 0 result is not valid tx hex",
                                fail_kind=fail_kind))
                continue
            if txid == GENESIS_COINBASE_TXID:
                session.add(_mk(cid, "getrawtransaction", PASSED, PASSED,
                                "legacy coinbase served as hex; recomputed "
                                "txid matches (dSHA256 of raw, no witness)",
                                fail_kind=fail_kind))
            else:
                session.add(_mk(cid, "getrawtransaction", PASSED, FAILED,
                                "recomputed txid %s != expected %s"
                                % (txid, GENESIS_COINBASE_TXID),
                                fail_kind=fail_kind))
            continue
        try:
            parsed = parse_spec(
                rec.result, CONTRACTS["getrawtransaction.verbose"]["spec"])
        except ContractError as exc:
            session.add(_mk(cid, "getrawtransaction", PASSED, FAILED,
                            "shape: %s" % exc, fail_kind=fail_kind))
            continue
        problems = []
        if parsed.get("txid") != GENESIS_COINBASE_TXID:
            problems.append("txid %r != genesis coinbase txid"
                            % parsed.get("txid"))
        blockhash = parsed.get("blockhash")
        if blockhash is None:
            problems.append("blockhash absent (bark bitcoind_tx_status "
                            "would treat a confirmed tx as unconfirmed)")
        elif blockhash != genesis_hash:
            problems.append("blockhash %r != genesis block hash" % blockhash)
        if problems:
            session.add(_mk(cid, "getrawtransaction", PASSED, FAILED,
                            "; ".join(problems), fail_kind=fail_kind))
        else:
            session.add(_mk(cid, "getrawtransaction", PASSED, PASSED,
                            "verbose coinbase parses (bark "
                            "GetRawTransactionResult); %s; txid and blockhash "
                            "consistent" % form, fail_kind=fail_kind))


def _walk_gettxout(session):
    for cid, include_mempool in (("gettxout.mempool-true", True),
                                  ("gettxout.mempool-false", False)):
        rec = session.call("gettxout",
                           [GENESIS_COINBASE_TXID, 0, include_mempool],
                           envelope="1.0")
        if rec.error is not None:
            availability, fk = _failure_axes(rec, "shape")
            session.add(_mk(cid, "gettxout", availability, UNKNOWN,
                            "not evaluated: call failed: %s" % _err(rec),
                            fail_kind=fk))
            continue
        try:
            parsed = parse_spec(rec.result, CONTRACTS["gettxout"]["spec"])
        except ContractError as exc:
            session.add(_mk(cid, "gettxout", PASSED, FAILED,
                            "shape: %s" % exc, fail_kind="shape"))
            continue
        if parsed is None:
            session.add(_mk(cid, "gettxout", PASSED, PASSED,
                            "null for the genesis coinbase -- Core-identical "
                            "(the genesis coinbase output is not in the UTXO "
                            "set on Bitcoin Core either; bark try_get_tx_out "
                            "maps this to None). Note: the non-null "
                            "GetTxOutResult shape remains unverified without "
                            "a funded fixture", fail_kind="shape"))
        else:
            session.add(_mk(cid, "gettxout", PASSED, PASSED,
                            "non-null UTXO parsed against GetTxOutResult",
                            fail_kind="shape"))


def _check_mempool_surface(session):
    # --- getrawmempool, async form [] -------------------------------------
    rec = session.call("getrawmempool", [], envelope="1.0")
    if rec.error is not None:
        availability, fk = _failure_axes(rec, "shape")
        session.add(_mk("getrawmempool", "getrawmempool", availability, UNKNOWN,
                        "not evaluated: call failed: %s" % _err(rec),
                        fail_kind=fk))
    else:
        try:
            entries = parse_spec(rec.result, CONTRACTS["getrawmempool"]["spec"])
            session.add(_mk("getrawmempool", "getrawmempool", PASSED, PASSED,
                            "array of txids (%d entries; async form [])"
                            % len(entries), fail_kind="shape"))
            session.observe("mempool_size", len(entries))
        except ContractError as exc:
            session.add(_mk("getrawmempool", "getrawmempool", PASSED, FAILED,
                            "shape: %s" % exc, fail_kind="shape"))

    # --- getrawmempool, bitcoin-ext sync form [false] envelope 2.0 --------
    rec = session.call("getrawmempool", [False], envelope="2.0")
    if rec.error is not None:
        availability, fk = _failure_axes(rec, "encoding")
        session.add(_mk("getrawmempool.bool-form", "getrawmempool",
                        availability, UNKNOWN,
                        "not evaluated: call failed: %s" % _err(rec),
                        fail_kind=fk))
    else:
        try:
            parse_spec(rec.result, CONTRACTS["getrawmempool"]["spec"])
            session.add(_mk("getrawmempool.bool-form", "getrawmempool",
                            PASSED, PASSED,
                            "sync form accepted ([false], envelope 2.0; "
                            "bitcoin-ext get_mempool_spending_tx)",
                            fail_kind="encoding"))
        except ContractError as exc:
            session.add(_mk("getrawmempool.bool-form", "getrawmempool",
                            PASSED, FAILED, "shape: %s" % exc,
                            fail_kind="encoding"))

    # --- getmempoolentry (expect -5 for a txid not in the mempool) --------
    rec = session.call("getmempoolentry", [GENESIS_COINBASE_TXID],
                       envelope="1.0")
    # `recognized` reflects whether the JSON-RPC layer routed the method:
    # only a JSON-RPC error response that is NOT a protocol error
    # (-32700/-32600) or an id mismatch proves routing; auth, transport,
    # timeout, parse and protocol failures prove nothing about it.
    recognized_availability = UNKNOWN
    if rec.error is not None:
        if _is_protocol_level(rec):
            session.add(_mk("getmempoolentry.recognized", "getmempoolentry",
                            UNKNOWN, UNKNOWN,
                            "protocol-level failure, not method evidence: %s"
                            % _err(rec), fail_kind="shape"))
        elif rec.error.code == -5:
            recognized_availability = PASSED
            session.add(_mk("getmempoolentry.recognized", "getmempoolentry",
                            PASSED, UNKNOWN,
                            "txid not in mempool (expected on an unfunded "
                            "node): method routed and returns the Core -5 "
                            "code bark maps to None; entry shape unverified "
                            "without a funded fixture", fail_kind="shape"))
        elif rec.error.category == "method_missing":
            recognized_availability = FAILED
            session.add(_mk("getmempoolentry.recognized", "getmempoolentry",
                            FAILED, NOT_RUN,
                            "method not found (-32601): getmempoolentry is "
                            "absent from the RPC surface",
                            fail_kind="method-surface"))
        elif rec.error.category.startswith("server_error_"):
            recognized_availability = PASSED
            session.add(_mk("getmempoolentry.recognized", "getmempoolentry",
                            PASSED, FAILED,
                            "expected Core error -5 for a txid not in the "
                            "mempool; got %s (bark maps -5 to None and "
                            "propagates other errors)" % _err(rec),
                            fail_kind="shape"))
        else:
            session.add(_mk("getmempoolentry.recognized", "getmempoolentry",
                            UNKNOWN, UNKNOWN,
                            "could not evaluate: %s" % _err(rec),
                            fail_kind="shape"))
    else:
        recognized_availability = PASSED
        session.add(_mk("getmempoolentry.recognized", "getmempoolentry",
                        PASSED, FAILED,
                        "unexpected success for a txid that cannot be in the "
                        "mempool of a fresh node (result %r)" % (rec.result,),
                        fail_kind="shape"))
    for cid, contract_name in (
        ("getmempoolentry.shape.server", "getmempoolentry.server"),
        ("getmempoolentry.shape.wallet", "getmempoolentry.wallet"),
    ):
        session.add(_mk(cid, "getmempoolentry", recognized_availability, UNKNOWN,
                        "requires a funded mempool entry (separate opt-in "
                        "packet); %s shape unverified on an unfunded node"
                        % contract_name, fail_kind="method-surface"))

    # --- estimatesmartfee (targets from the profile) ----------------------
    # Sent EXACTLY as bark wallet fetch_fee_rates sends it:
    # estimatesmartfee [target, "ECONOMICAL"] (EstimateMode serializes
    # UPPERCASE) over envelope 1.0 via the async client call_raw.
    fee_present = {}
    for target in session.profile["fee_targets"]:
        rec = session.call("estimatesmartfee",
                           [target, session.profile["fee_estimate_mode"]],
                           envelope="1.0")
        cid = "estimatesmartfee.t%d.shape" % target
        if rec.error is not None:
            availability, fk = _failure_axes(rec, "shape")
            session.add(_mk(cid, "estimatesmartfee", availability, UNKNOWN,
                            "not evaluated: call failed: %s" % _err(rec),
                            fail_kind=fk))
            fee_present[target] = None
            continue
        try:
            parsed = parse_spec(rec.result, CONTRACTS["estimatesmartfee"]["spec"])
            session.add(_mk(cid, "estimatesmartfee", PASSED, PASSED,
                            "parsed (blocks=%r, feerate=%r; mode literal "
                            "\"ECONOMICAL\" as bark serializes it)"
                            % (parsed.get("blocks"), parsed.get("feerate")),
                            fail_kind="shape"))
        except ContractError as exc:
            session.add(_mk(cid, "estimatesmartfee", PASSED, FAILED,
                            "shape: %s" % exc, fail_kind="shape"))
        fee_present[target] = isinstance(rec.result, dict) and "feerate" in rec.result
        if fee_present[target]:
            session.observe("feerate.t%d" % target, rec.result.get("feerate"))
    missing = [t for t, ok in fee_present.items() if not ok]
    if missing:
        session.add(_mk("estimatesmartfee.fee-data", "estimatesmartfee",
                        PASSED, UNKNOWN,
                        "no feerate for targets %s (empty mempool: the node "
                        "returns errors + blocks only). Missing fee data is "
                        "unknown, not pass: bark falls back to its configured "
                        "fallback fee" % missing))
    else:
        session.add(_mk("estimatesmartfee.fee-data", "estimatesmartfee",
                        PASSED, PASSED,
                        "feerates present for all profile targets %s"
                        % sorted(fee_present)))


def _check_getmempoolinfo(session):
    """Traffic-slice check (coverage class "traffic"), never gating the
    bark verdict.

    Grounding (D5): the consumer of record here is a SELECTED
    MODELLING CONSUMER -- corepc-types 0.13.0 v30::GetMempoolInfo via
    bitcoind-async-client 0.15.0 Reader::get_mempool_info (v30.rs:128,
    getmempoolinfo [] over envelope 1.0; bark's tree compiles the v30
    module: default features 30_2 + raw_rpc, no 29_0). bark c419159
    itself never calls getmempoolinfo, and the T0 lab-traffic-contract
    RpcNodeControl consumer is NOT automatically this consumer --
    whether the T0 side requires the full v30 superset is the T0
    owner's statement to make. This row records the honest observation
    (including the real shape failure when a required v30 field is JSON
    null) without coercion and without claiming a universal
    traffic-surface verdict; it changes no bark-verdict outcome, and
    the traffic consumer reads the row directly.
    """
    rec = session.call("getmempoolinfo", [], envelope="1.0")
    from .trafficfacts import policy_facts
    session.observe("traffic_fee_policy", policy_facts(rec))
    if rec.error is not None:
        _call_failed_rows(session, [("getmempoolinfo.traffic-consumer", "shape")],
                          "getmempoolinfo", rec)
        availability, fk = _failure_axes(rec, "shape")
        session.add(_mk("getmempoolinfo", "getmempoolinfo", availability,
                        UNKNOWN,
                        "not evaluated: call failed: %s" % _err(rec),
                        fail_kind=fk))
        return
    _shape_check(session, "getmempoolinfo", "getmempoolinfo", rec.result,
                 "getmempoolinfo")
    _shape_check(session, "getmempoolinfo.traffic-consumer", "getmempoolinfo",
                 rec.result, "getmempoolinfo.traffic-consumer")
    if isinstance(rec.result, dict):
        for key in ("loaded", "size", "bytes", "usage", "maxmempool",
                    "fullrbf"):
            if key in rec.result:
                session.observe("mempoolinfo.%s" % key, rec.result[key])


def _check_mutating_recognition(session):
    """Method recognition only, with an invalid sample.

    These are mutating RPC paths. The probe deliberately does NOT
    exercise them with funded transactions (separate opt-in packet), so
    their behavior axis is not_run. An invalid sample (valid hex, not a
    transaction) can only prove that the method is routed, because both
    Core and rbitcoin reject it before any state changes. Protocol
    errors (-32700/-32600) and id mismatches are NOT routing evidence.
    """
    samples = (
        # C2: every active bark caller of these three methods is an
        # async bitcoind-async-client call_raw (envelope 1.0): server
        # bitcoind.rs test_mempool_accept / submit_package and chain.rs
        # broadcast, wallet send_raw_transaction. The 2.0 sync forms
        # have no active bark consumer and are not sent.
        ("testmempoolaccept", [[INVALID_TX_HEX]], "1.0"),
        ("submitpackage", [[INVALID_TX_HEX]], "1.0"),
        ("sendrawtransaction", [INVALID_TX_HEX], "1.0"),
    )
    for method, params, envelope in samples:
        rec = session.call(method, params, envelope=envelope)
        cid = "%s.recognized" % method
        if rec.error is None:
            session.add(_mk(cid, method, PASSED, NOT_RUN,
                            "method routed (returned a result for an invalid "
                            "sample, which is unexpected but not a funded "
                            "path); funded behavior deliberately not exercised",
                            fail_kind="method-surface"))
        elif _is_protocol_level(rec):
            session.add(_mk(cid, method, UNKNOWN, NOT_RUN,
                            "protocol-level failure, not method evidence: %s"
                            % _err(rec), fail_kind="method-surface"))
        elif rec.error.category == "method_missing":
            session.add(_mk(cid, method, FAILED, NOT_RUN,
                            "method not found (-32601): %s is absent from the "
                            "RPC surface" % method, fail_kind="method-surface"))
        elif (rec.error.category.startswith("server_error_")
                or rec.error.category == "invalid_params"):
            # A JSON-RPC error object (routed and rejected) proves the
            # method exists on the surface. Transport-level and protocol
            # failures prove nothing and stay unknown.
            session.add(_mk(cid, method, PASSED, NOT_RUN,
                            "method routed: rejected the invalid sample with "
                            "%s (funded broadcast path deliberately not "
                            "exercised; invalid samples prove recognition "
                            "only)" % _err(rec), fail_kind="method-surface"))
        else:
            session.add(_mk(cid, method, UNKNOWN, NOT_RUN,
                            "could not determine whether the method is "
                            "routed: %s" % _err(rec), fail_kind="method-surface"))


def _check_id_echo(session):
    """F3: every exchange must echo the numeric request id exactly.

    The async client's Response.id is declared u64: a string echo
    breaks every pinned async consumer. The client already fails such
    calls individually on BOTH the 200 and the HTTP-error path (C1);
    this check aggregates the receipts so a report cannot bury an id
    mismatch inside unrelated unknowns. The aggregate counts every
    receipt the run produced, including HTTP 500/404 error-path
    receipts (protocol.id-echo is emitted after all calls, including
    the ZMQ observation).
    """
    total = session.id_match_count + session.id_mismatch_count
    if session.id_mismatch_count:
        session.add(_mk("protocol.id-echo", None, FAILED, FAILED,
                        "%d of %d exchanges echoed the request id "
                        "incorrectly (string/absent/mismatched/"
                        "non-numeric), including HTTP error-path "
                        "receipts; the pinned async client (Response.id: "
                        "u64) would fail to deserialize these responses"
                        % (session.id_mismatch_count, total),
                        fail_kind="protocol"))
    elif session.id_match_count == 0:
        session.add(_mk("protocol.id-echo", None, UNKNOWN, UNKNOWN,
                        "no exchange produced a parseable receipt; id echo "
                        "unverified", fail_kind="protocol"))
    else:
        session.add(_mk("protocol.id-echo", None, PASSED, PASSED,
                        "all %d exchanges (200 and HTTP-error paths "
                        "alike) echoed the numeric request id exactly"
                        % session.id_match_count,
                        fail_kind="protocol"))


def _observe_zmq(session):
    rec = session.call("getzmqnotifications", [], envelope="1.0")
    if rec.error is not None:
        if rec.error.category == "method_missing":
            session.observe("getzmqnotifications",
                            "method not implemented (no ZMQ support)")
        else:
            session.observe("getzmqnotifications", "error: %s" % _err(rec))
    else:
        session.observe("getzmqnotifications", rec.result)
    session.observe(
        "zmq_impact",
        "bark c419159 does not use ZMQ: it polls the node via "
        "getblockcount/getblockhash (server poll loop) and "
        "getrawmempool (bdk emitter), so ZMQ absence is not a "
        "compatibility gap for the pinned bark.")


# ---------------------------------------------------------------------------
# Verdict
# ---------------------------------------------------------------------------


def compute_verdict(checks, profile):
    """Three-status verdict (mirrored by contracts.VERDICT_RULES and
    schemadef.validate_report_consistency):

    - incompatible: any check failed on either axis;
    - unproven: no failures, but some gating check (coverage
      required/encoding/protocol) has availability != passed -- the
      surface was not proven, so surface_compatible must not be claimed
      (e.g. auth/transport/protocol-level failures);
    - surface_compatible: no failures and every gating check passed on
      the availability axis, with behavior passed or legitimately
      unknown (unknown_behavior_ok: genesis-coinbase -5, empty-mempool
      fee data, unfunded mempool entry).

    Checks with coverage class "traffic" (getmempoolinfo) never gate
    the verdict in either direction: bark c419159 does not consume the
    method, so their honest passed/failed rows are traffic-slice
    information for the T0 consumer only.

    Checks with coverage class "observational" (getbestblockhash,
    getnetworkinfo.observe) are likewise honest rows that never gate
    the verdict: getbestblockhash is on the pinned async client's API
    surface (Reader::get_current_timestamp) but bark c419159 has no
    caller for it, so a failure there is lab information, not a bark
    incompatibility (D4).

    funded_state is permanently "unproven" and full_acceptance
    permanently false in read-only mode.
    """
    from .contracts import CHECKS

    fail_kinds = set()
    surface_unproven = False
    unproven = []
    for check in checks:
        cid = check["id"]
        meta = CHECKS.get(cid, {})
        coverage = meta.get("coverage", "required")
        if coverage in ("traffic", "observational"):
            # Traffic-slice and observational rows are honest (their
            # consumers read them directly) but NEVER gate the bark
            # verdict: bark c419159 does not call these methods
            # (getmempoolinfo; getbestblockhash), so a failure is not
            # a bark incompatibility and a pass proves nothing about
            # bark compatibility. Only unproven bookkeeping applies.
            if check["behavior"] in (UNKNOWN, NOT_RUN):
                unproven.append(cid)
            continue
        if check["availability"] == FAILED:
            fail_kinds.add(check.get("fail_kind") or "shape")
        if check["behavior"] == FAILED:
            fail_kinds.add(check.get("fail_kind") or "shape")
        if check["behavior"] in (UNKNOWN, NOT_RUN):
            unproven.append(cid)
        # Gating coverage classes decide surface_compatible: the check
        # must be availability=passed, and its behavior must be passed
        # or legitimately unknown.
        if coverage in ("required", "encoding", "protocol"):
            if check["availability"] != PASSED:
                surface_unproven = True
            elif check["behavior"] not in (PASSED, UNKNOWN):
                surface_unproven = True
            elif (check["behavior"] == UNKNOWN
                    and not meta.get("unknown_behavior_ok")):
                surface_unproven = True

    if fail_kinds:
        status = "incompatible"
    elif surface_unproven:
        status = "unproven"
    else:
        status = "surface_compatible"
    reasons = ["fail-%s" % k for k in FAIL_KIND_ORDER if k in fail_kinds]
    if status == "unproven":
        reasons.append("surface-unproven")
    # Permanent in read-only mode: funded behavior is unproven here, so
    # no read-only run can ever claim full acceptance (F5).
    reasons.append("funded-unproven")
    return {
        "status": status,
        "reasons": reasons,
        "funded_state": "unproven",
        "full_acceptance": False,
        "unproven_checks": unproven,
        "summary": _verdict_summary(status, fail_kinds, profile),
    }


def _verdict_summary(status, fail_kinds, profile):
    name = profile.get("_name", "bark-c419159")
    if status == "surface_compatible":
        return ("surface and encoding compatibility: every read-only check "
                "passed against the %s contract; funded broadcast/mempool "
                "behavior remains unproven (read-only mode can never grant "
                "full acceptance)" % name)
    if status == "unproven":
        return ("unproven: no check failed, but the RPC surface was not "
                "proven (protocol-level, auth, or transport failures left "
                "gating checks at availability unknown); funded "
                "broadcast/mempool behavior also remains unproven")
    parts = []
    if "version-gate" in fail_kinds:
        parts.append("the RPC version gate fails honestly "
                     "(bark server needs >= 310000, wallet >= 290000)")
    if "method-surface" in fail_kinds:
        parts.append("one or more required methods are missing or unreachable")
    if "encoding" in fail_kinds:
        parts.append("one or more exact client-call encodings "
                     "(params/envelope/id forms the pinned consumers send) "
                     "are not accepted")
    if "protocol" in fail_kinds:
        parts.append("the JSON-RPC protocol layer misbehaves "
                     "(numeric id echo or envelope handling)")
    if "network" in fail_kinds:
        parts.append("the node is not on the expected network")
    if "genesis" in fail_kinds:
        parts.append("genesis/block-data checks failed")
    if "shape" in fail_kinds:
        parts.append("one or more response shapes do not parse against the "
                     "bark-consuming crate types")
    parts.append("funded broadcast/mempool behavior additionally remains "
                 "unproven (read-only mode)")
    return "%s: %s" % (status, "; ".join(parts))


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


def run_probe(client, profile, *, expected_network=None, expected_genesis=None):
    """Run every read-only check; returns the full probe result dict."""
    if expected_network is None:
        expected_network = profile["expected_network"]
    if expected_genesis is None:
        expected_genesis = profile["expected_genesis"]
    session = ProbeSession(client, profile, expected_network, expected_genesis)
    _check_getnetworkinfo(session)
    _check_getblockchaininfo(session)
    _check_getindexinfo(session)
    _check_tips(session)
    _check_genesis_walk(session)
    _check_mempool_surface(session)
    _check_getmempoolinfo(session)
    _check_mutating_recognition(session)
    _observe_zmq(session)
    # C1: the id-echo aggregate must cover every exchange the run made
    # (including the ZMQ observation call), so it is computed after all
    # calls rather than over a silent subset.
    _check_id_echo(session)
    verdict = compute_verdict(session.checks, profile)
    return {
        "profile": dict(profile),
        "checks": session.checks,
        "observations": session.observations,
        "verdict": verdict,
        "exchanges": session.exchanges,
    }
