"""Closed spc-lab-launch-profile/1 input and RFC 8785 comparison.

argv excludes the executable. Spans are [start, end), indexing raw tokens.
The six-field comparison is PRIVATE (it contains the ownership token).
Only its digest and closed metadata may enter a public report.
"""
from __future__ import annotations

import copy
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re

SAFE_INTEGER = 2**53 - 1
TRANSPORT = "validated_lab_transport"
RAW_FIELDS = {"profile_version", "binary_sha256", "argv", "env", "config_files", "substitutions"}
CLASSES = {"datadir", "rpc-listen", "cookie-file", "api-log", "max-run-secs", "p2p-isolation"}
FLAGS = {
    "--network": 1, "--datadir": 1, "--rpc": 0, "--rpc-listen": 1,
    "--rpc-cookie-file": 1, "--regtest-pow-bits": 1, "--api-log": 1,
    "--log-level": 1, "--max-run-secs": 1, "--config": 1,
    "--no-listen": 0, "--no-seeds": 0, "--no-discover": 0,
    "--max-outbound": 1, "--listen": 1, "--connect": 1,
    "--rpc-compat-profile": 1, "--rpc-compat-profile=bark-v31": 0,
}
P2P = {"--no-listen", "--no-seeds", "--no-discover", "--max-outbound", "--listen", "--connect"}
LAUNCH_PROFILE_CONTRACT = {
    "schema": "spc-lab-launch-profile/1",
    "raw_fields": sorted(RAW_FIELDS),
    "comparison_fields": ["profile_version", "binary_sha256", "argv", "env", "config_files", "p2p_isolation"],
    "argv": "executable excluded; exact ordered string tokens; duplicate/unknown flags rejected",
    "flag_value_counts": FLAGS,
    "rpc_compat_policy": {
        "forms": [["--rpc-compat-profile", "bark-v31"], ["--rpc-compat-profile=bark-v31"]],
        "default": "absent (off)", "network": "regtest",
        "comparison": "every token remains ordered literal policy; never a substitution or P2P group member",
        "duplicates": "one selector maximum across both spellings; mixed or repeated selectors reject",
        "dialect_env": "when selected, RBITCOIN_RPC_PACKAGE_DIALECT is absent, empty, 1 or ASCII-case-insensitive true; off/invalid reject; original env bytes remain bound",
        "source": "AP1b RpcCompatProfile::parse and NodeConfig::resolve_rpc_env; CLI tests accept split and equals forms",
    },
    "substitution_classes": sorted(CLASSES),
    "indices": "u32 raw argv indices; spans [start,end); config indices remapped once and sorted",
    "api_log_span": ["--api-log", "absolute-path", "--log-level", "error|warn|info|debug|trace"],
    "isolated": ["--no-listen", "--no-seeds", "--no-discover", "--max-outbound", "0|1"],
    "namespace": ["--no-seeds", "--no-discover", "--max-outbound", "1", "--listen", "owned-IPv4:port", "--connect", "owned-IPv4:port"],
    "p2p_isolation": TRANSPORT,
    "environment": "complete finalized string map, no inheritance; same per-run SPC_LAB_RUN_TOKEN; private",
    "config_files": "argv_index of --config value plus lowercase sha256 of exact file bytes; path is literal",
    "encoding": "RFC8785 UTF-8 JSON, UTF-16 key order; safe integers only; duplicate keys/controls/surrogates rejected",
    "ownership": "absolute hash-bound spc-lab actor-exec, controller and probe PID/start ticks, PDEATHSIG SIGKILL both levels",
}


class LaunchProfileError(ValueError):
    def __init__(self, reason):
        # Never interpolate untrusted profile values (environment contains secrets).
        super().__init__("traffic_launch_profile_unsupported: " + reason)


def require(ok, reason):
    if not ok:
        raise LaunchProfileError(reason)


def pairs(items):
    out = {}
    for key, value in items:
        require(key not in out, "duplicate JSON key")
        out[key] = value
    return out


def _invalid_number(_text):
    raise LaunchProfileError("non-integer JSON number")


def _tree(value):
    if isinstance(value, str):
        require(not any(ord(c) < 32 or 127 <= ord(c) <= 159 or 0xD800 <= ord(c) <= 0xDFFF for c in value),
                "control character or surrogate")
    elif type(value) is int:
        require(abs(value) <= SAFE_INTEGER, "unsafe JSON integer")
    elif isinstance(value, list):
        for child in value:
            _tree(child)
    elif isinstance(value, dict):
        for key, child in value.items():
            require(isinstance(key, str), "non-string key")
            _tree(key)
            _tree(child)
    else:
        raise LaunchProfileError("unsupported JSON type")


def loads(raw):
    try:
        value = json.loads(raw, object_pairs_hook=pairs, parse_float=_invalid_number,
                           parse_constant=_invalid_number)
        _tree(value)
        return value
    except (UnicodeError, ValueError) as exc:
        if isinstance(exc, LaunchProfileError):
            raise
        raise LaunchProfileError("invalid JSON") from None


def canonical_bytes(value):
    """RFC8785 for our restricted domain: strings, safe integers, arrays, objects.

    No float serializer is needed or permitted. UTF-16 code-unit key sorting
    (not Python Unicode code-point sorting), UTF-8, no ASCII escaping, no
    whitespace. json.dumps supplies RFC8259 string escaping. Inputs reject
    lone surrogates and controls before serialization.
    """
    _tree(value)
    def ordered(v):
        if isinstance(v, dict):
            return {k: ordered(v[k]) for k in sorted(v, key=lambda k: k.encode("utf-16-be"))}
        if isinstance(v, list):
            return [ordered(x) for x in v]
        return v
    return json.dumps(ordered(value), ensure_ascii=False, separators=(",", ":"),
                      allow_nan=False).encode("utf-8")


def digest(value):
    return hashlib.sha256(canonical_bytes(value)).hexdigest()


def _closed(value, keys):
    require(type(value) is dict and set(value) == set(keys), "missing or unknown fields")


def _hash(value):
    require(isinstance(value, str) and re.fullmatch("[0-9a-f]{64}", value), "invalid sha256")


def _index(value, length):
    require(type(value) is int and 0 <= value < min(length, 2**32), "invalid argv index")


def _path(value):
    require(isinstance(value, str) and value.startswith("/") and
            all(p not in ("", ".", "..") for p in value.split("/")[1:]), "non-canonical absolute path")


def endpoint(value):
    require(isinstance(value, str) and value.count(":") == 1, "expected IPv4 host:port")
    host, port = value.split(":")
    try:
        ip = ipaddress.IPv4Address(host)
    except ValueError:
        raise LaunchProfileError("invalid IPv4 endpoint") from None
    require(re.fullmatch("[1-9][0-9]*", port) and 0 < int(port) <= 65535,
            "invalid endpoint port")
    return ip


def normalize(raw, *, owned_endpoints=(), read_bytes=lambda path: Path(path).read_bytes()):
    """Validate first, then map once. Caller binds endpoints to its owned map.

    No effects other than reading explicitly listed config bytes. No environment
    inheritance, shell expansion, wildcard flags or partial P2P groups.
    """
    _tree(raw)
    _closed(raw, RAW_FIELDS)
    require(type(raw["profile_version"]) is int and raw["profile_version"] == 1, "unsupported version")
    _hash(raw["binary_sha256"])
    argv, env = raw["argv"], raw["env"]
    require(type(argv) is list and argv and all(isinstance(x, str) for x in argv), "invalid argv")
    require(type(env) is dict and all(re.fullmatch("[A-Za-z_][A-Za-z0-9_]*", k) and
            isinstance(v, str) for k, v in env.items()), "invalid finalized env")
    require(bool(re.fullmatch("[A-Za-z0-9_-]{16,128}", env.get("SPC_LAB_RUN_TOKEN", ""))),
            "missing or invalid ownership token")
    flags, i = {}, 0
    while i < len(argv):
        flag = argv[i]
        name = "--rpc-compat-profile" if flag == "--rpc-compat-profile=bark-v31" else flag
        require(flag in FLAGS and name not in flags, "unknown or duplicate flag")
        width = FLAGS[flag] + 1
        require(i + width <= len(argv), "missing flag value")
        value = "bark-v31" if flag == "--rpc-compat-profile=bark-v31" else (argv[i + 1] if width == 2 else None)
        flags[name] = (i, value)
        i += width
    require(all(f in flags for f in ("--network", "--rpc", "--datadir", "--rpc-listen", "--rpc-cookie-file")),
            "missing owned regtest flags")
    require(flags["--network"][1] == "regtest", "non-regtest network")
    if "--rpc-compat-profile" in flags:
        require(flags["--rpc-compat-profile"][1] == "bark-v31", "unsupported RPC compatibility profile")
        # AP1b env_value/ env_flag_opt: empty is unset, ASCII true is on.
        # Do not rewrite the finalized environment or synthesize a dialect.
        dialect = env.get("RBITCOIN_RPC_PACKAGE_DIALECT", "")
        require(dialect in ("", "1") or (dialect.isascii() and dialect.lower() == "true"),
                "RPC compatibility profile conflicts with package dialect environment")
    for flag in ("--datadir", "--rpc-cookie-file", "--api-log", "--config"):
        if flag in flags:
            _path(flags[flag][1])
    rpc = flags["--rpc-listen"][1]
    ip = endpoint(rpc)
    require(ip.is_loopback or rpc in owned_endpoints, "RPC endpoint not owned")
    if "--regtest-pow-bits" in flags:
        require(bool(re.fullmatch("(?:0x)?[0-9a-f]{8}", flags["--regtest-pow-bits"][1])), "invalid pow bits")
    if "--max-run-secs" in flags:
        value = flags["--max-run-secs"][1]
        require(bool(re.fullmatch("[1-9][0-9]*", value)) and int(value) <= 86400, "invalid secondary deadline")
    if "--log-level" in flags:
        require(flags["--log-level"][1] in ("error", "warn", "info", "debug", "trace"), "invalid log level")
    present = set(flags) & P2P
    common = {"--no-seeds", "--no-discover", "--max-outbound"}
    if present == common | {"--no-listen"}:
        require(flags["--max-outbound"][1] in ("0", "1"), "invalid isolated outbound limit")
    else:
        require(present == common | {"--listen", "--connect"}, "incomplete/conflicting P2P group")
        require(flags["--max-outbound"][1] == "1", "invalid namespace outbound limit")
        for flag in ("--listen", "--connect"):
            value = flags[flag][1]
            endpoint(value)
            require(value in owned_endpoints, "P2P endpoint not in owned map")
        require(flags["--listen"][1] != flags["--connect"][1], "self peer")
    p2p_indices = {j for f in present for j in range(flags[f][0], flags[f][0] + FLAGS[f] + 1)}
    require(type(raw["substitutions"]) is list, "invalid substitutions")
    substitutions, claimed, p2p_claimed, classes = {}, set(), set(), set()
    value_classes = {"datadir": "--datadir", "rpc-listen": "--rpc-listen",
                     "cookie-file": "--rpc-cookie-file", "max-run-secs": "--max-run-secs"}
    for sub in raw["substitutions"]:
        require(type(sub) is dict and set(sub) in ({"class", "argv_index"}, {"class", "span"}), "invalid substitution")
        cls = sub["class"]
        require(isinstance(cls, str) and cls in CLASSES, "unknown substitution class")
        if "argv_index" in sub:
            start = sub["argv_index"]
            _index(start, len(argv))
            end = start + 1
        else:
            span = sub["span"]
            require(type(span) is list and len(span) == 2, "invalid span")
            start, end = span
            _index(start, len(argv))
            require(type(end) is int and start < end <= len(argv), "invalid span end")
        indices = set(range(start, end))
        require(not claimed & indices, "overlapping substitutions")
        claimed |= indices
        if cls == "p2p-isolation":
            require("span" in sub and indices <= p2p_indices, "invalid P2P span")
            p2p_claimed |= indices
            continue
        require(cls not in classes, "duplicate substitution class")
        classes.add(cls)
        if cls in value_classes:
            flag = value_classes[cls]
            require(flag in flags and "argv_index" in sub and start == flags[flag][0] + 1, "wrong substitution locus")
        elif cls == "api-log":
            require("span" in sub and end - start == 4 and argv[start] == "--api-log" and
                    argv[start + 2] == "--log-level", "api-log must span path and level flags")
        substitutions[start] = (end, cls)
    required_classes = {c for c, f in value_classes.items() if f in flags}
    if "--api-log" in flags or "--log-level" in flags:
        required_classes.add("api-log")
    require(classes == required_classes and p2p_claimed == p2p_indices, "undeclared substitution or partial P2P group")
    remaining, mapping, i = [], {}, 0
    while i < len(argv):
        if i in p2p_indices:
            i += 1
            continue
        mapping[i] = len(remaining)
        if i in substitutions:
            end, cls = substitutions[i]
            remaining.append({"class": cls})
            i = end
        else:
            remaining.append({"literal": argv[i]})
            i += 1
    require(type(raw["config_files"]) is list, "invalid config files")
    configs, config_seen = [], set()
    for item in raw["config_files"]:
        _closed(item, {"argv_index", "sha256"})
        index = item["argv_index"]
        _index(index, len(argv))
        _hash(item["sha256"])
        require("--config" in flags and index == flags["--config"][0] + 1 and index not in config_seen,
                "invalid config index")
        config_seen.add(index)
        require(hashlib.sha256(read_bytes(argv[index])).hexdigest() == item["sha256"], "config bytes mismatch")
        configs.append({"argv_index": mapping[index], "sha256": item["sha256"]})
    require(bool(configs) == ("--config" in flags), "undeclared config file")
    return {"profile_version": 1, "binary_sha256": raw["binary_sha256"],
            "argv": remaining, "env": dict(env),
            "config_files": sorted(configs, key=lambda c: c["argv_index"]), "p2p_isolation": TRANSPORT}


def isolated_profile(raw, *, datadir, rpc_listen, cookie_file, api_log, owned_endpoints=()):
    """Produce probe effective raw input; require comparison equality before use."""
    expected = normalize(raw, owned_endpoints=owned_endpoints)
    result = copy.deepcopy(raw)
    replacements = {"datadir": datadir, "rpc-listen": rpc_listen, "cookie-file": cookie_file}
    remove = set()
    for sub in raw["substitutions"]:
        if sub["class"] == "p2p-isolation":
            remove.update(range(*sub["span"]))
    argv, mapping = [], {}
    for i, token in enumerate(raw["argv"]):
        if i not in remove:
            mapping[i] = len(argv)
            argv.append(token)
    subs = []
    for sub in raw["substitutions"]:
        cls = sub["class"]
        if cls == "p2p-isolation":
            continue
        if "argv_index" in sub:
            index = mapping[sub["argv_index"]]
            subs.append({"class": cls, "argv_index": index})
            if cls in replacements:
                argv[index] = replacements[cls]
        else:
            start, end = sub["span"]
            start, end = mapping[start], mapping[end - 1] + 1
            subs.append({"class": cls, "span": [start, end]})
            argv[start + 1] = api_log
    start = len(argv)
    # The sealed L2 node requires max_outbound >= 1 even with no discovery
    # or peers. The complete isolation group still prevents peer dialing.
    argv.extend(["--no-listen", "--no-seeds", "--no-discover", "--max-outbound", "1"])
    subs.append({"class": "p2p-isolation", "span": [start, len(argv)]})
    result.update(argv=argv, substitutions=subs,
                  config_files=[{"argv_index": mapping[c["argv_index"]], "sha256": c["sha256"]} for c in raw["config_files"]])
    require(normalize(result) == expected, "isolated profile comparison mismatch")
    return result
