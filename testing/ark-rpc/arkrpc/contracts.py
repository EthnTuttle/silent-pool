"""Canonical RPC response contracts, client-call encodings, check manifest,
and verdict rules for the AP-1a probe (manifest v2).

SOURCE OF TRUTH: the pinned bark checkout `c419159` and the exact crate
versions its Cargo.lock resolves. Every contract below cites the Rust
type that a pinned consumer deserializes with; nothing here is invented
from a checklist. Where bark has two clients, both contracts are kept
separate because their required-field sets differ:

- the async client `bitcoind-async-client` 0.15.0 (envelope "1.0",
  numeric usize request ids starting at 0), deserializing
  `corepc-types` 0.13.0 shapes; and
- the sync `bitcoincore-rpc` 0.19 client used by bark's own
  `BitcoinRpcClient` and by the bdk emitter (jsonrpc 0.18 crate:
  envelope "2.0", numeric usize nonce ids starting at 1),
  deserializing `bitcoincore-rpc-json` 0.19.0 shapes; and
- bark's vendored copies in `bitcoin-ext` (GetRawTransactionResult,
  SubmitPackageResult), which differ from 0.19 where noted.

Because this probe is Python (stdlib only), it cannot link the Rust
decoders; instead it implements the serde semantics of those types
strictly, and a dev-only Rust checker (`devcheck/`) validates a shared
set of golden vectors against the actual pinned crate types so the
Python semantics are not merely self-consistent:

- a missing non-Option field is an error;
- an `Option<T>` field WITHOUT a custom deserializer may be absent or
  explicitly null (serde deserializes both as None);
- a field with `#[serde(default)]` and a custom deserializer
  (serde_hex::opt, deserialize_hex_array_opt, or a bare Vec) may be
  absent but NOT explicitly null -- the custom deserializer expects a
  string/sequence, so null is a hard error. Contracts mark these
  fields with "null": "reject";
- unknown fields are ignored (none of the pinned types build with
  serde-deny-unknown-fields);
- a JSON bool is never accepted for an integer field, and a float is
  never accepted for an integer field (serde_json rules);
- `serde_hex` fields deserialize via `FromHex::from_hex`, which
  ACCEPTS the empty string (empty Vec): empty hex is valid wherever
  the Rust type is `Vec<u8>` with serde_hex. Contracts mark these
  with "allow_empty": true. 32-byte hash fields remain exactly 64 hex
  chars;
- `bitcoin::amount::serde::as_btc` deserializes via f64 ->
  `Amount::from_btc`, which requires >= 0, at most 8 decimal places
  (in the float's shortest round-trip string), and a satoshi value
  that fits u64 (`Amount::MAX == u64::MAX` sats). The 21M BTC
  MAX_MONEY constant is consensus policy checked elsewhere
  (MoneyRange), NOT part of deserialization: a 30M BTC vout value
  parses fine at this layer and is not a contract failure;
- `ScriptPubkeyType` has TWO pinned variants: bitcoincore-rpc-json
  0.19 does NOT have an `Anchor` variant (a "anchor" type string
  fails deserialization there), while bark's bitcoin-ext copy DOES.
  Both consumer contexts are modeled separately.

Amounts are parsed with `decimal.Decimal` (json parse_float) so the 8
decimal place check is exact, not float-fuzzy.
"""

from __future__ import annotations

import decimal
import hashlib
import json
import re

MANIFEST_VERSION = 2
TRAFFIC_CONSUMER = "spc-lab-tx-rpc-consumer/1"
LEGACY_R2_CONTRACT_SHA256 = "f4fc3e043f420870675095ab24144ba14e0afba6cb9ad69d818d99b1bccb2f62"
LEGACY_TRAFFIC_CONTRACT_SHA256 = "da87e2b665f4c3427098e308fd87d5946c8fc78d1ffa64c93a1ad0f84b3b5ca8"

# ---------------------------------------------------------------------------
# Named ark profiles (no arbitrary numbers: a profile is a named constant)
# ---------------------------------------------------------------------------

# Core regtest genesis block hash (byte-reversed display form), the same
# constant rbitcoin's own test suite asserts.
REGTEST_GENESIS_HASH = "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206"

PROFILES = {
    # The only profile: the pinned bark the whole Ark vertical targets.
    "bark-c419159": {
        # server/src/bitcoind.rs:31  MIN_BITCOIND_VERSION: usize = 31_00_00
        # ("v31 added the chunk fields of getmempoolentry")
        "server_min_rpc_version": 310000,
        # bark/src/chain.rs:51  MIN_BITCOIND_VERSION: usize = 290000
        # (0-fee anchors considered standard from 29.0)
        "wallet_min_rpc_version": 290000,
        # bark check_network(): Network::from_core_arg(getblockchaininfo.chain)
        "expected_network": "regtest",
        # lab chain consistency: one canonical regtest genesis
        "expected_genesis": REGTEST_GENESIS_HASH,
        # bark/src/chain.rs fetch_fee_rates: targets 1 (fast), 3 (regular),
        # 6 (slow). The mode literal is EstimateMode::Economical which
        # serializes UPPERCASE (bitcoincore-rpc-json 0.19 EstimateMode has
        # #[serde(rename_all = "UPPERCASE")]): the wire literal is
        # "ECONOMICAL", exactly what the probe sends.
        "fee_targets": [1, 3, 6],
        "fee_estimate_mode": "ECONOMICAL",
        "source_citations": [
            "bark c419159 server/src/bitcoind.rs:30-31 (MIN_BITCOIND_VERSION = 31_00_00)",
            "bark c419159 bark/src/chain.rs:334-348 (estimatesmartfee [target, "
            "EstimateMode::Economical]); 36-38,51 (fee targets 1/3/6; MIN 290000)",
            "bitcoincore-rpc-json 0.19.0 src/lib.rs:1828-1834 (EstimateMode "
            "rename_all UPPERCASE -> wire literal \"ECONOMICAL\")",
        ],
    },
}


class ProfileError(ValueError):
    pass


def get_profile(name: str) -> dict:
    try:
        return PROFILES[name]
    except KeyError:
        raise ProfileError(
            "unknown ark profile %r (known: %s); profiles are named constants, "
            "not free-form configuration" % (name, ", ".join(sorted(PROFILES)))
        ) from None


# ---------------------------------------------------------------------------
# Spec mini language
# ---------------------------------------------------------------------------


class ContractError(ValueError):
    """A response did not parse against the documented Rust contract."""


_HEX_RE = re.compile(r"^[0-9a-fA-F]+$")
# bitcoin-units 0.1.3 amount.rs:875  pub const MAX: Amount = Amount(u64::MAX)
# Deserialization range is the u64 satoshi range, NOT the 21M MAX_MONEY
# consensus policy constant (that is MoneyRange, a separate semantic check).
_MAX_SATS = 2**64 - 1


def _fail(path: str, why: str) -> None:
    raise ContractError("%s: %s" % (path or "<root>", why))


def parse_spec(value, spec: dict, path: str = ""):
    """Validate `value` against `spec`, mirroring serde semantics.

    Returns a normalized copy (amounts become exact integer sats).
    Raises ContractError with a field-addressed message on mismatch.
    """
    kind = spec["kind"]

    if kind == "nullable":
        if value is None:
            return None
        return parse_spec(value, spec["of"], path)

    if kind == "object":
        if not isinstance(value, dict):
            _fail(path, "expected JSON object, got %s" % _json_type(value))
        out = {}
        fields = dict(spec.get("fields", {}))
        aliases = spec.get("aliases", {})  # field -> [alternate json names]
        for name, fspec in fields.items():
            candidates = [name] + list(aliases.get(name, []))
            present = next((k for k in candidates if k in value), None)
            if present is None:
                _fail(_p(path, name), "required field missing")
            out[name] = parse_spec(value[present], fspec, _p(path, present))
        for name, fspec in spec.get("optional", {}).items():
            candidates = [name] + list(aliases.get(name, []))
            present = next((k for k in candidates if k in value), None)
            if present is None:
                # serde: an absent field deserializes as None for both
                # plain Option<T> and #[serde(default)] fields.
                out[name] = None
                continue
            if value[present] is None:
                if spec.get("null_reject") or fspec.get("null") == "reject":
                    # #[serde(default)] + custom deserializer (serde_hex::opt,
                    # deserialize_hex_array_opt, bare Vec): the custom
                    # deserializer expects a string/sequence, so an explicit
                    # null is a hard serde error even though absence is fine.
                    _fail(_p(path, present),
                          "explicit null rejected by the pinned custom "
                          "deserializer (absence would be accepted)")
                out[name] = None
            else:
                out[name] = parse_spec(value[present], fspec, _p(path, present))
        if spec.get("deny_unknown"):
            known = set()
            for name in list(fields) + list(spec.get("optional", {})):
                known.add(name)
                known.update(aliases.get(name, []))
            extra = [k for k in value if k not in known]
            if extra:
                _fail(path, "unknown fields %s (deny_unknown)" % sorted(extra))
        return out

    if kind == "u64":
        if type(value) is not int or not 0 <= value <= 2**64 - 1:
            _fail(path, "expected JSON integer in u64 range")
        return value

    if kind == "int":
        if isinstance(value, bool) or not isinstance(value, int):
            _fail(path, "expected JSON integer, got %s" % _json_type(value))
        return value

    if kind == "float":
        if isinstance(value, bool) or not isinstance(value, (int, float, decimal.Decimal)):
            _fail(path, "expected JSON number, got %s" % _json_type(value))
        return float(value)

    if kind == "bool":
        if not isinstance(value, bool):
            _fail(path, "expected JSON boolean, got %s" % _json_type(value))
        return value

    if kind == "string":
        if not isinstance(value, str):
            _fail(path, "expected JSON string, got %s" % _json_type(value))
        return value

    if kind == "hex":
        if not isinstance(value, str) or len(value) % 2:
            _fail(path, "expected even-length hex string")
        if value and not _HEX_RE.match(value):
            _fail(path, "expected even-length hex string")
        if not value and not spec.get("allow_empty"):
            # serde_hex (FromHex::from_hex) accepts "" (empty Vec), so
            # allow_empty marks every field whose Rust type is Vec<u8>
            # with serde_hex. Fields decoded by consensus
            # encode::deserialize_hex (getblock/getrawtransaction
            # verbosity 0) still require non-empty.
            _fail(path, "expected non-empty hex string")
        return value.lower()

    if kind == "hash32":
        if not isinstance(value, str) or len(value) != 64 or not _HEX_RE.match(value):
            _fail(path, "expected 64-char hex (32-byte hash) string")
        return value.lower()

    if kind == "array":
        if not isinstance(value, list):
            _fail(path, "expected JSON array, got %s" % _json_type(value))
        return [parse_spec(v, spec["items"], "%s[%d]" % (path, i)) for i, v in enumerate(value)]

    if kind == "map":
        if not isinstance(value, dict):
            _fail(path, "expected JSON object (map), got %s" % _json_type(value))
        return {k: parse_spec(v, spec["values"], _p(path, k)) for k, v in value.items()}

    if kind == "amount_btc":
        # bitcoin::amount::serde::as_btc -> f64::deserialize -> Amount::from_btc
        # -> from_str_in (u64 satoshi range, not the 21M policy cap).
        if isinstance(value, bool) or not isinstance(value, (int, float, decimal.Decimal)):
            _fail(path, "expected JSON number (BTC amount), got %s" % _json_type(value))
        d = value if isinstance(value, decimal.Decimal) else decimal.Decimal(str(value))
        if not d.is_finite():
            _fail(path, "amount is not finite")
        if d < 0:
            _fail(path, "amount is negative (Amount::from_btc rejects)")
        if -d.as_tuple().exponent > 8:
            _fail(path, "amount has more than 8 decimal places")
        sats = int(d.scaleb(8))
        if sats > _MAX_SATS:
            _fail(path, "amount exceeds the u64 satoshi range "
                        "(Amount::MAX; the 21M BTC supply cap is consensus "
                        "policy, not a deserialization bound)")
        return sats

    if kind == "enum":
        if not isinstance(value, str) or value not in spec["values"]:
            _fail(path, "expected one of %s" % sorted(spec["values"]))
        return value

    if kind == "string_or_array":
        # bitcoincore-rpc-json 0.19 StringOrStringArray (warnings field)
        if isinstance(value, str):
            return [value]
        if isinstance(value, list) and all(isinstance(v, str) for v in value):
            return value
        _fail(path, "expected string or array of strings")

    raise AssertionError("unknown spec kind %r" % kind)


def _json_type(v) -> str:
    if v is None:
        return "null"
    if isinstance(v, bool):
        return "boolean"
    if isinstance(v, (int, float, decimal.Decimal)):
        return "number"
    if isinstance(v, str):
        return "string"
    if isinstance(v, list):
        return "array"
    if isinstance(v, dict):
        return "object"
    return type(v).__name__


def _p(path: str, name: str) -> str:
    return "%s.%s" % (path, name) if path else name


# ---------------------------------------------------------------------------
# ScriptPubkeyType: TWO pinned consumer contexts (F4)
# ---------------------------------------------------------------------------

# bitcoincore-rpc-json 0.19.0 src/lib.rs:904-918: NO Anchor variant.
# A vout whose scriptPubKey type is "anchor" FAILS deserialization for
# every 0.19 consumer (gettxout, getmempool descendants of these types).
_SCRIPT_PUBKEY_TYPE_CORE_RPC_JSON = {
    "kind": "enum",
    "values": [
        "nonstandard", "pubkey", "pubkeyhash", "scripthash",
        "multisig", "nulldata", "witness_v0_keyhash",
        "witness_v0_scripthash", "witness_v1_taproot", "witness_unknown",
    ],
    "note": "bitcoincore-rpc-json 0.19.0 has no Anchor variant: \"anchor\" "
            "fails deserialization in every 0.19 consumer",
}

# bark c419159 bitcoin-ext/src/rpc/mod.rs:152-166: Anchor IS present
# (added for CPFP anchor outputs).
_SCRIPT_PUBKEY_TYPE_BARK_BITCOIN_EXT = {
    "kind": "enum",
    "values": [
        "nonstandard", "anchor", "pubkey", "pubkeyhash", "scripthash",
        "multisig", "nulldata", "witness_v0_keyhash",
        "witness_v0_scripthash", "witness_v1_taproot", "witness_unknown",
    ],
    "note": "bark bitcoin-ext vendored copy includes Anchor",
}

# getrawtransaction verbose uses bitcoin-ext's own vendored types
# (bark server custom_get_raw_transaction_info / wallet
# bitcoind_tx_status deserialize rpc::GetRawTransactionResult).
_SPK_BARK = {
    "kind": "object",
    "fields": {"asm": {"kind": "string"},
               "hex": {"kind": "hex", "allow_empty": True}},
    "optional": {
        "reqSigs": {"kind": "int"},
        "type": _SCRIPT_PUBKEY_TYPE_BARK_BITCOIN_EXT,
        # addresses: Vec<Address> with #[serde(default)]: absent OK,
        # explicit null REJECTED (Vec deserializer expects a sequence).
        "addresses": {"kind": "array", "items": {"kind": "string"},
                      "null": "reject"},
        "address": {"kind": "string"},
    },
}

# gettxout uses bitcoincore-rpc-json 0.19.0 GetTxOutResult (via bark
# bitcoin-ext try_get_tx_out, which types json::GetTxOutResult from the
# 0.19 re-export): its ScriptPubkeyType REJECTS "anchor".
_SPK_CORE = {
    "kind": "object",
    "fields": {"asm": {"kind": "string"},
               "hex": {"kind": "hex", "allow_empty": True}},
    "optional": {
        "reqSigs": {"kind": "int"},
        "type": _SCRIPT_PUBKEY_TYPE_CORE_RPC_JSON,
        "addresses": {"kind": "array", "items": {"kind": "string"},
                      "null": "reject"},
        "address": {"kind": "string"},
    },
}


# ---------------------------------------------------------------------------
# Contracts, each citing the Rust type that consumes it
# ---------------------------------------------------------------------------

CONTRACTS = {
    "getmempoolinfo.traffic-consumer": {
        "spec": {"kind": "object", "fields": {"size": {"kind": "u64"}}},
        "consumer": TRAFFIC_CONSUMER,
        "sources": ["traffic-runtime-wiring Section 1: size-only backlog consumer"],
        "gates": [],
    },
    "getblockheader.traffic-consumer": {
        "spec": {"kind": "object", "fields": {
            "height": {"kind": "u64"}, "confirmations": {"kind": "int"}}},
        "consumer": TRAFFIC_CONSUMER,
        "sources": ["traffic-runtime-wiring Section 1: read-only chain observation"],
        "gates": [],
    },
    # --- getnetworkinfo, bark-required core --------------------------------
    # bark deserializes ONLY `version` from getnetworkinfo:
    # server/src/bitcoind.rs require_version Response{version: usize} and
    # bark/src/chain.rs NetworkInfo{version: usize}. This is the required
    # contract; the richer field set is observational (below) and never
    # gates the verdict.
    "getnetworkinfo.required": {
        "spec": {
            "kind": "object",
            "fields": {"version": {"kind": "int"}},
        },
        "consumer": "bark server require_version / wallet NetworkInfo (version only)",
        "sources": [
            "bark c419159 server/src/bitcoind.rs:236-244 (require_version, Response{version})",
            "bark c419159 bark/src/chain.rs:183-195 (require_version, NetworkInfo{version})",
        ],
        "gates": ["server_min_rpc_version", "wallet_min_rpc_version"],
    },

    # --- getnetworkinfo, observational superset ----------------------------
    # NOT a bark requirement: no pinned bark consumer deserializes these
    # fields from getnetworkinfo. Recorded for lab bookkeeping only; a
    # mismatch here is an observation, never a failed check.
    "getnetworkinfo.observe": {
        "spec": {
            "kind": "object",
            "fields": {
                "version": {"kind": "int"},
                "subversion": {"kind": "string"},
                "protocolversion": {"kind": "int"},
                "relayfee": {"kind": "amount_btc"},
                "networks": {
                    "kind": "array",
                    "items": {
                        "kind": "object",
                        "fields": {
                            "name": {"kind": "string"},
                            "limited": {"kind": "bool"},
                            "reachable": {"kind": "bool"},
                            # rbitcoin emits "proxy": null when no proxy is
                            # configured; Core emits a string (often "").
                            # Observation only (bitcoincore-rpc-json 0.19
                            # GetNetworkInfoResultNetwork declares String,
                            # but no bark consumer deserializes it).
                            "proxy": {"kind": "nullable", "of": {"kind": "string"}},
                            "proxy_randomize_credentials": {"kind": "bool"},
                        },
                    },
                },
            },
            "optional": {
                "incrementalfee": {"kind": "amount_btc"},
            },
        },
        "consumer": "none (observation set; lab plan section 5.3 item 2)",
        "sources": [
            "lab plan 2026-11-07 section 5.3 item 2 (observation set)",
        ],
        "gates": [],
    },

    # --- getblockchaininfo, async client ----------------------------------
    # bitcoind-async-client 0.15.0 v30::network() deserializes
    # corepc-types 0.13.0 v29::GetBlockchainInfo (re-exported as v30).
    "getblockchaininfo.async": {
        "spec": {
            "kind": "object",
            "fields": {
                "chain": {"kind": "string"},
                "blocks": {"kind": "int"},
                "headers": {"kind": "int"},
                "bestblockhash": {"kind": "hash32"},
                "bits": {"kind": "string"},
                "target": {"kind": "string"},
                "difficulty": {"kind": "float"},
                "time": {"kind": "int"},
                "mediantime": {"kind": "int"},
                "verificationprogress": {"kind": "float"},
                "initialblockdownload": {"kind": "bool"},
                "chainwork": {"kind": "hex", "allow_empty": True},
                "size_on_disk": {"kind": "int"},
                "pruned": {"kind": "bool"},
                "warnings": {"kind": "array", "items": {"kind": "string"}},
            },
            "optional": {
                "pruneheight": {"kind": "int"},
                "automatic_pruning": {"kind": "bool"},
                "prune_target_size": {"kind": "int"},
                "signet_challenge": {"kind": "string"},
            },
        },
        "consumer": "bitcoind-async-client 0.15.0 (corepc-types 0.13.0 v29 shapes)",
        "sources": [
            "bitcoind-async-client 0.15.0 src/client/v30.rs:182-188 (network(): "
            "call::<GetBlockchainInfo>(\"getblockchaininfo\") -> .chain)",
            "corepc-types 0.13.0 src/v29/blockchain/mod.rs:279-324 (GetBlockchainInfo)",
        ],
        "gates": [],
    },

    # --- getblockchaininfo, sync companion --------------------------------
    # bitcoincore-rpc-json 0.19.0 GetBlockchainInfoResult: does NOT
    # require bits/target/time; warnings is StringOrStringArray;
    # softforks has #[serde(default)].
    "getblockchaininfo.sync": {
        "spec": {
            "kind": "object",
            "fields": {
                "chain": {"kind": "string"},
                "blocks": {"kind": "int"},
                "headers": {"kind": "int"},
                "bestblockhash": {"kind": "hash32"},
                "difficulty": {"kind": "float"},
                "mediantime": {"kind": "int"},
                "verificationprogress": {"kind": "float"},
                "initialblockdownload": {"kind": "bool"},
                "chainwork": {"kind": "hex", "allow_empty": True},
                "size_on_disk": {"kind": "int"},
                "pruned": {"kind": "bool"},
                "warnings": {"kind": "string_or_array"},
            },
            "optional": {
                "pruneheight": {"kind": "int"},
                "automatic_pruning": {"kind": "bool"},
                "prune_target_size": {"kind": "int"},
                "softforks": {"kind": "map", "values": {"kind": "object", "fields": {}}},
            },
        },
        "consumer": "bitcoincore-rpc 0.19 sync client (bdk emitter companion)",
        "sources": [
            "bitcoincore-rpc-json 0.19.0 src/lib.rs:1017-1057 (GetBlockchainInfoResult)",
            "bark c419159 bitcoin-ext/src/rpc/mod.rs:5 (re-exports "
            "bdk_bitcoind_rpc::bitcoincore_rpc)",
        ],
        "gates": [],
    },

    # --- getindexinfo ------------------------------------------------------
    # bark bitcoin-ext require_txindex deserializes bitcoincore-rpc-json
    # 0.19.0 GetIndexInfoResult{txindex: Option<IndexStatus{synced,
    # best_block_height}>} and fails closed when txindex is absent.
    # best_block_height is what BOTH Bitcoin Core and corepc-types emit
    # (corepc-types 0.13.0 src/v21/util.rs:25 defines GetIndexInfoName
    # with best_block_height and re-exports it through every version
    # module including v29/v30); it is the Core-compatible baseline, not
    # a divergence.
    "getindexinfo": {
        "spec": {
            "kind": "object",
            "fields": {},
            "optional": {
                "txindex": {
                    "kind": "object",
                    "fields": {
                        "synced": {"kind": "bool"},
                        "best_block_height": {"kind": "int"},
                    },
                },
                "coinstatsindex": {
                    "kind": "object",
                    "fields": {
                        "synced": {"kind": "bool"},
                        "best_block_height": {"kind": "int"},
                    },
                },
                "basic block filter index": {
                    "kind": "object",
                    "fields": {
                        "synced": {"kind": "bool"},
                        "best_block_height": {"kind": "int"},
                    },
                },
            },
        },
        "consumer": "bark bitcoin-ext require_txindex (0.19 GetIndexInfoResult)",
        "sources": [
            "bark c419159 bitcoin-ext/src/rpc/mod.rs:431-481 (require_txindex, TxindexError)",
            "bitcoincore-rpc-json 0.19.0 src/lib.rs:2141-2153 (IndexStatus, GetIndexInfoResult)",
            "corepc-types 0.13.0 src/v21/util.rs:18-26 (GetIndexInfoName: synced, "
            "best_block_height -- the same field names Bitcoin Core itself emits)",
        ],
        "gates": [],
    },

    # --- getblockcount -----------------------------------------------------
    "getblockcount": {
        "spec": {"kind": "int"},
        "consumer": "bitcoind-async-client 0.15.0 get_block_count",
        "sources": [
            "bark c419159 server/src/bitcoind.rs:77-81 (tip via get_block_count)",
            "bark c419159 bark/src/chain.rs:384-394 (fetch_tip via get_block_count)",
            "bitcoind-async-client 0.15.0 src/client/v30.rs:90-91 (getblockcount [])",
        ],
        "gates": [],
    },

    # --- getbestblockhash --------------------------------------------------
    # D4: the pinned async client's v30 Reader exposes getbestblockhash
    # only inside get_current_timestamp, and bark c419159 has NO caller
    # for either (grep over bark/, server/, bark-cli/ src at c419159).
    # The check is therefore coverage class "observational": an honest
    # row for the async client API surface that never gates the bark
    # verdict in either direction.
    "getbestblockhash": {
        "spec": {"kind": "hash32"},
        "consumer": "bitcoind-async-client 0.15.0 get_current_timestamp "
                    "(async client API; no bark c419159 caller)",
        "sources": [
            "bitcoind-async-client 0.15.0 src/client/v30.rs:107-110 "
            "(get_current_timestamp: getbestblockhash [] then get_block)",
        ],
        "gates": [],
    },

    # --- getblockhash ------------------------------------------------------
    "getblockhash": {
        "spec": {"kind": "hash32"},
        "consumer": "bitcoind-async-client 0.15.0 get_block_hash",
        "sources": [
            "bark c419159 server/src/bitcoind.rs:77-95 (tip/deep_tip/block_ref)",
            "bitcoind-async-client 0.15.0 src/client/v30.rs:94-96 (getblockhash [height])",
        ],
        "gates": [],
    },

    # --- getblock verbosity 0 (hex) ----------------------------------------
    # bitcoind-async-client Reader::get_block and the sync client both
    # request verbosity 0 and consensus-decode the hex string into a
    # bitcoin Block. The probe additionally re-hashes the 80-byte header
    # and requires it to equal the requested hash. Non-empty hex is
    # required here (consensus decode of "" fails below the JSON layer).
    "getblock.verbosity0": {
        "spec": {"kind": "hex"},
        "consumer": "bitcoind-async-client get_block / bitcoincore-rpc 0.19 get_block",
        "sources": [
            "bitcoind-async-client 0.15.0 src/client/v30.rs:60-66 (getblock [hash, 0])",
            "bitcoincore-rpc 0.19.0 src/client.rs:335-342 (getblock [hash, 0])",
            "bark c419159 bark/src/chain.rs:468-482 (block())",
        ],
        "gates": [],
    },

    # --- getblock verbosity 1 (GetBlockResult) ------------------------------
    # bitcoincore-rpc-json 0.19.0 GetBlockResult, the type the bdk
    # emitter's get_block_info call deserializes (bdk_bitcoind_rpc 0.22.0
    # src/lib.rs calls bitcoincore_rpc get_block_info -> getblock
    # [hash, 1]). camelCase; nTx; versionHex is Option + serde(default)
    # + serde_hex::opt (absent OK, explicit null REJECTED);
    # strippedsize/mediantime/previousblockhash/nextblockhash are plain
    # Options (absent or null both fine).
    "getblock.verbosity1": {
        "spec": {
            "kind": "object",
            "fields": {
                "hash": {"kind": "hash32"},
                "confirmations": {"kind": "int"},
                "size": {"kind": "int"},
                "weight": {"kind": "int"},
                "height": {"kind": "int"},
                "version": {"kind": "int"},
                "merkleroot": {"kind": "hash32"},
                "tx": {"kind": "array", "items": {"kind": "hash32"}},
                "time": {"kind": "int"},
                "nonce": {"kind": "int"},
                "bits": {"kind": "string"},
                "difficulty": {"kind": "float"},
                "chainwork": {"kind": "hex", "allow_empty": True},
                "nTx": {"kind": "int"},
            },
            "optional": {
                "strippedsize": {"kind": "int"},
                "versionHex": {"kind": "hex", "allow_empty": True,
                               "null": "reject"},
                "mediantime": {"kind": "int"},
                "previousblockhash": {"kind": "hash32"},
                "nextblockhash": {"kind": "hash32"},
            },
        },
        "consumer": "bdk_bitcoind_rpc 0.22.0 emitter via bitcoincore-rpc 0.19 get_block_info",
        "sources": [
            "bitcoincore-rpc-json 0.19.0 src/lib.rs:195-219 (GetBlockResult)",
            "bitcoincore-rpc 0.19.0 src/client.rs:344-346 (getblock [hash, 1])",
            "bdk_bitcoind_rpc 0.22.0 src/lib.rs:203 (emitter get_block_info poll path)",
        ],
        "gates": [],
    },

    # --- getblockheader verbose --------------------------------------------
    # bitcoincore-rpc-json 0.19.0 GetBlockHeaderResult. bark server
    # tx_status() (async call_raw, envelope 1.0) and the sync client
    # (getblockheader [hash, true], envelope 2.0) both deserialize it.
    "getblockheader.verbose": {
        "spec": {
            "kind": "object",
            "fields": {
                "hash": {"kind": "hash32"},
                "confirmations": {"kind": "int"},
                "height": {"kind": "int"},
                "version": {"kind": "int"},
                "merkleroot": {"kind": "hash32"},
                "time": {"kind": "int"},
                "nonce": {"kind": "int"},
                "bits": {"kind": "string"},
                "difficulty": {"kind": "float"},
                "chainwork": {"kind": "hex", "allow_empty": True},
                # GetBlockHeaderResult has #[serde(rename_all =
                # "camelCase")], so the parsed key is "nTx" (Core style).
                "nTx": {"kind": "int"},
            },
            "optional": {
                # versionHex: Option + serde(default) + serde_hex::opt:
                # absent OK, explicit null REJECTED.
                "versionHex": {"kind": "hex", "allow_empty": True,
                               "null": "reject"},
                "mediantime": {"kind": "int"},
                "previousblockhash": {"kind": "hash32"},
                "nextblockhash": {"kind": "hash32"},
            },
        },
        "consumer": "bark server tx_status (async) / bitcoincore-rpc 0.19 sync",
        "sources": [
            "bitcoincore-rpc-json 0.19.0 src/lib.rs:221-245 (GetBlockHeaderResult)",
            "bark c419159 server/src/bitcoind.rs:104-124 (getblockheader [hash, true])",
            "bitcoincore-rpc 0.19.0 src/client.rs:354-359 (getblockheader [hash, bool])",
        ],
        "gates": [],
    },

    # --- getblockheader non-verbose (hex) -----------------------------------
    # bitcoincore-rpc 0.19 get_block_header requests
    # getblockheader [hash, false] (JSON bool) and consensus-decodes the
    # hex string.
    "getblockheader.nonverbose": {
        "spec": {"kind": "hex"},
        "consumer": "bitcoincore-rpc 0.19 sync get_block_header",
        "sources": [
            "bitcoincore-rpc 0.19.0 src/client.rs:349-352 (getblockheader [hash, false])",
        ],
        "gates": [],
    },

    # --- getrawtransaction verbosity 1, bark bitcoin-ext vendored type ------
    # bitcoin-ext/src/rpc/mod.rs GetRawTransactionResult (camelCase),
    # parsed by server custom_get_raw_transaction_info and wallet
    # bitcoind_tx_status (blockhash is the field both read). Its
    # ScriptPubkeyType INCLUDES Anchor. hex is serde_hex (empty valid);
    # coinbase is serde(default) + serde_hex::opt (absent OK, null
    # REJECTED); txinwitness is serde(default) +
    # deserialize_hex_array_opt (absent OK, null REJECTED, items are
    # hex strings and empty items are valid).
    "getrawtransaction.verbose": {
        "spec": {
            "kind": "object",
            "fields": {
                "hex": {"kind": "hex", "allow_empty": True},
                "txid": {"kind": "hash32"},
                "hash": {"kind": "hash32"},
                "size": {"kind": "int"},
                "vsize": {"kind": "int"},
                "version": {"kind": "int"},
                "locktime": {"kind": "int"},
                "vin": {
                    "kind": "array",
                    "items": {
                        "kind": "object",
                        "fields": {"sequence": {"kind": "int"}},
                        "optional": {
                            "coinbase": {"kind": "hex", "allow_empty": True,
                                         "null": "reject"},
                            "txid": {"kind": "hash32"},
                            "vout": {"kind": "int"},
                            "scriptSig": {
                                "kind": "object",
                                "fields": {
                                    "asm": {"kind": "string"},
                                    "hex": {"kind": "hex", "allow_empty": True},
                                },
                            },
                            "txinwitness": {
                                "kind": "array",
                                "items": {"kind": "hex", "allow_empty": True},
                                "null": "reject",
                            },
                        },
                    },
                },
                "vout": {
                    "kind": "array",
                    "items": {
                        "kind": "object",
                        "fields": {
                            "value": {"kind": "amount_btc"},
                            "n": {"kind": "int"},
                            "scriptPubKey": _SPK_BARK,
                        },
                    },
                },
            },
            "optional": {
                "in_active_chain": {"kind": "bool"},
                "blockhash": {"kind": "hash32"},
                "confirmations": {"kind": "int"},
                "time": {"kind": "int"},
                "blocktime": {"kind": "int"},
            },
        },
        "consumer": "bark bitcoin-ext GetRawTransactionResult (server + wallet)",
        "sources": [
            "bark c419159 bitcoin-ext/src/rpc/mod.rs:118-203 (GetRawTransactionResult, "
            "ScriptPubkeyType with Anchor, serde_hex, deserialize_hex_array_opt)",
            "bark c419159 server/src/bitcoind.rs:137-151 (custom_get_raw_transaction_info)",
            "bark c419159 bark/src/chain.rs:862-883 (bitcoind_tx_status)",
        ],
        "gates": [],
    },

    # --- getrawtransaction verbosity 0 --------------------------------------
    "getrawtransaction.verbosity0": {
        "spec": {"kind": "hex"},
        "consumer": "bitcoind-async-client get_raw_transaction_verbosity_zero",
        "sources": [
            "bitcoind-async-client 0.15.0 src/client/v30.rs:134-143 "
            "(getrawtransaction [txid, 0])",
            "bark c419159 server/src/bitcoind.rs:97-102 (get_raw_txout)",
        ],
        "gates": [],
    },

    # --- gettxout -----------------------------------------------------------
    # bitcoin-ext try_get_tx_out types the response as the 0.19
    # GetTxOutResult (via the bitcoincore_rpc::json re-export), whose
    # ScriptPubkeyType REJECTS "anchor". A spent/unknown output is JSON
    # null -> Ok(None); the genesis coinbase is null on Core too (not an
    # rbitcoin quirk).
    "gettxout": {
        "spec": {
            "kind": "nullable",
            "of": {
                "kind": "object",
                "fields": {
                    "bestblock": {"kind": "hash32"},
                    "confirmations": {"kind": "int"},
                    "value": {"kind": "amount_btc"},
                    "scriptPubKey": _SPK_CORE,
                    "coinbase": {"kind": "bool"},
                },
            },
        },
        "consumer": "bark bitcoin-ext try_get_tx_out (0.19 GetTxOutResult)",
        "sources": [
            "bark c419159 bitcoin-ext/src/rpc/mod.rs:483-498 (try_get_tx_out: "
            "gettxout [txid, vout, bool])",
            "bitcoincore-rpc-json 0.19.0 src/lib.rs:739-748 (GetTxOutResult)",
            "bark c419159 bark/src/chain.rs:782-801 (outpoint_spent_confirmed)",
        ],
        "gates": [],
    },

    # --- getrawmempool ------------------------------------------------------
    "getrawmempool": {
        "spec": {"kind": "array", "items": {"kind": "hash32"}},
        "consumer": "bitcoind-async-client get_raw_mempool / bitcoin-ext sync",
        "sources": [
            "bitcoind-async-client 0.15.0 src/client/v30.rs:113-117 (getrawmempool [])",
            "bark c419159 bitcoin-ext/src/rpc/mod.rs:388-393 (getrawmempool [false])",
            "bark c419159 server/src/bitcoind.rs:153-167 (get_mempool_spending_tx)",
        ],
        "gates": [],
    },

    # --- getmempoolinfo ------------------------------------------------------
    # TRAFFIC-SLICE surface (T0 lab-traffic-contract), NOT a bark
    # requirement: bark c419159 itself never calls getmempoolinfo. The
    # consumer is the pinned async client's Reader trait, which bark
    # depends on (bitcoind-async-client 0.15.0, features ["raw_rpc"] +
    # default "30_2"): the v29/v30 client modules are mutually
    # exclusive (src/client/mod.rs:21-30) and bark's tree compiles the
    # v30 module only, so the single deserializer in the pinned tree is
    # corepc-types 0.13.0 v30::GetMempoolInfo. All 13 fields are
    # required (no Option, no serde(default)); total_fee is a plain
    # f64 (not an Amount). Coverage class "traffic": honest rows, never
    # gating the bark verdict.
    "getmempoolinfo": {
        "spec": {
            "kind": "object",
            "fields": {
                "loaded": {"kind": "bool"},
                "size": {"kind": "int"},
                "bytes": {"kind": "int"},
                "usage": {"kind": "int"},
                "total_fee": {"kind": "float"},
                "maxmempool": {"kind": "int"},
                "mempoolminfee": {"kind": "float"},
                "minrelaytxfee": {"kind": "float"},
                "incrementalrelayfee": {"kind": "float"},
                "unbroadcastcount": {"kind": "int"},
                "fullrbf": {"kind": "bool"},
                "permitbaremultisig": {"kind": "bool"},
                "maxdatacarriersize": {"kind": "int"},
            },
        },
        "consumer": "SELECTED MODELLING CONSUMER: corepc-types 0.13.0 "
                    "v30::GetMempoolInfo via bitcoind-async-client 0.15.0 "
                    "Reader::get_mempool_info (v30 module; bark pins default "
                    "features 30_2 + raw_rpc). bark c419159 itself never "
                    "calls getmempoolinfo, and the T0 lab-traffic-contract "
                    "RpcNodeControl consumer is NOT automatically this "
                    "consumer -- whether the T0 side requires the full v30 "
                    "superset is the T0 owner's statement. Coverage class "
                    "traffic: an honest, uncoerced row that never gates the "
                    "bark verdict in either direction",
        "sources": [
            "bitcoind-async-client 0.15.0 src/client/v30.rs:128-131 "
            "(get_mempool_info: call::<GetMempoolInfo>(\"getmempoolinfo\", &[]))",
            "bitcoind-async-client 0.15.0 src/client/mod.rs:21-30 (v29/v30 "
            "modules mutually exclusive via features; default 30_2)",
            "bark c419159 Cargo.toml:114 (bitcoind-async-client 0.15.0, "
            "features [\"raw_rpc\"], defaults on -> 30_2; no 29_0 anywhere "
            "in the tree)",
            "corepc-types 0.13.0 src/v30/blockchain/mod.rs:20-75 "
            "(GetMempoolInfo: 13 required fields, no Option; total_fee f64)",
        ],
        "gates": [],
    },

    # --- getmempoolentry, server shape --------------------------------------
    # bark server/src/bitcoind.rs MempoolEntry{chunk_weight: u64,
    # fees{chunk: Amount as_btc}} (chunk_fee_rate). A txid not in the
    # mempool is an RPC error with code -5 (RPC_INVALID_ADDRESS_OR_KEY)
    # which bark maps to Ok(None).
    "getmempoolentry.server": {
        "spec": {
            "kind": "object",
            "fields": {
                "chunkweight": {"kind": "int"},
                "fees": {
                    "kind": "object",
                    "fields": {"chunk": {"kind": "amount_btc"}},
                },
            },
        },
        "consumer": "bark server chunk_fee_rate",
        "sources": [
            "bark c419159 server/src/bitcoind.rs:169-205 (MempoolEntry, chunk_fee_rate)",
        ],
        "gates": [],
    },

    # --- getmempoolentry, wallet shape --------------------------------------
    # bitcoincore-rpc-json 0.19.0 GetMempoolEntryResult as consumed by
    # bark/src/chain.rs mempool_ancestor_info (fees.ancestor, weight,
    # ancestor_size). vsize has serde alias "size"; weight and
    # unbroadcast are plain Options (absent or null both fine).
    "getmempoolentry.wallet": {
        "spec": {
            "kind": "object",
            "aliases": {"vsize": ["size"]},
            "fields": {
                "vsize": {"kind": "int"},
                "time": {"kind": "int"},
                "height": {"kind": "int"},
                "descendantcount": {"kind": "int"},
                "descendantsize": {"kind": "int"},
                "ancestorcount": {"kind": "int"},
                "ancestorsize": {"kind": "int"},
                "wtxid": {"kind": "hash32"},
                "fees": {
                    "kind": "object",
                    "fields": {
                        "base": {"kind": "amount_btc"},
                        "modified": {"kind": "amount_btc"},
                        "ancestor": {"kind": "amount_btc"},
                        "descendant": {"kind": "amount_btc"},
                    },
                },
                "depends": {"kind": "array", "items": {"kind": "hash32"}},
                "spentby": {"kind": "array", "items": {"kind": "hash32"}},
                "bip125-replaceable": {"kind": "bool"},
            },
            "optional": {
                "weight": {"kind": "int"},
                "unbroadcast": {"kind": "bool"},
            },
        },
        "consumer": "bark wallet mempool_ancestor_info (0.19 GetMempoolEntryResult)",
        "sources": [
            "bitcoincore-rpc-json 0.19.0 src/lib.rs:1099-1155 (GetMempoolEntryResult)",
            "bark c419159 bark/src/chain.rs:486-502 (mempool_ancestor_info)",
        ],
        "gates": [],
    },

    # --- estimatesmartfee ---------------------------------------------------
    # bitcoincore-rpc-json 0.19.0 EstimateSmartFeeResult as consumed by
    # bark/src/chain.rs fetch_fee_rates: estimatesmartfee
    # [target, "ECONOMICAL"] (EstimateMode serializes UPPERCASE).
    # feerate is Option + serde(default) + as_btc::opt (null OK);
    # errors is a plain Option; on an empty mempool Core-shaped nodes
    # return errors + blocks with no feerate, and bark falls back to the
    # configured fallback fee.
    "estimatesmartfee": {
        "spec": {
            "kind": "object",
            "fields": {"blocks": {"kind": "int"}},
            "optional": {
                "feerate": {"kind": "amount_btc"},
                "errors": {"kind": "array", "items": {"kind": "string"}},
            },
        },
        "consumer": "bark wallet fetch_fee_rates (0.19 EstimateSmartFeeResult)",
        "sources": [
            "bitcoincore-rpc-json 0.19.0 src/lib.rs:1483-1497 (EstimateSmartFeeResult)",
            "bark c419159 bark/src/chain.rs:334-348 (estimatesmartfee [target, Economical])",
        ],
        "gates": [],
    },

    # --- testmempoolaccept ---------------------------------------------------
    # bitcoincore-rpc-json 0.19.0 TestMempoolAcceptResult as consumed by
    # bark server test_mempool_accept.
    "testmempoolaccept": {
        "spec": {
            "kind": "array",
            "items": {
                "kind": "object",
                "fields": {"txid": {"kind": "hash32"}, "allowed": {"kind": "bool"}},
                "optional": {
                    "reject-reason": {"kind": "string"},
                    "vsize": {"kind": "int"},
                    "fees": {
                        "kind": "object",
                        "fields": {"base": {"kind": "amount_btc"}},
                    },
                },
            },
        },
        "consumer": "bark server test_mempool_accept (0.19 TestMempoolAcceptResult)",
        "sources": [
            "bitcoincore-rpc-json 0.19.0 src/lib.rs:834-853 (TestMempoolAcceptResult)",
            "bark c419159 server/src/bitcoind.rs:216-221 (test_mempool_accept)",
        ],
        "gates": [],
    },

    # --- submitpackage --------------------------------------------------------
    # bitcoin-ext/src/rpc/mod.rs SubmitPackageResult{package_msg,
    # tx-results: HashMap<Wtxid, {txid, error}>} as consumed by bark
    # submit_package and wallet broadcast_package (success requires
    # package_msg == "success"). Called as submitpackage [hexes].
    "submitpackage": {
        "spec": {
            "kind": "object",
            "fields": {
                "package_msg": {"kind": "string"},
                "tx-results": {
                    "kind": "map",
                    "values": {
                        "kind": "object",
                        "fields": {"txid": {"kind": "hash32"}},
                        "optional": {"error": {"kind": "string"}},
                    },
                },
            },
        },
        "consumer": "bark bitcoin-ext SubmitPackageResult (server + wallet)",
        "sources": [
            "bark c419159 bitcoin-ext/src/rpc/mod.rs:205-218,370-383 (SubmitPackageResult; "
            "submitpackage [hexes])",
            "bark c419159 server/src/bitcoind.rs:207-214 (submit_package)",
            "bark c419159 bark/src/chain.rs:675-713 (broadcast_package)",
        ],
        "gates": [],
    },

    # --- sendrawtransaction ----------------------------------------------------
    "sendrawtransaction": {
        "spec": {"kind": "hash32"},
        "consumer": "bitcoind-async-client send_raw_transaction / sync 0.19",
        "sources": [
            "bark c419159 server/src/bitcoind.rs:129-135 (broadcast_tx)",
            "bark c419159 bark/src/chain.rs:658-673 (broadcast_tx)",
            "bitcoind-async-client 0.15.0 src/client/v30.rs:192-202 "
            "(sendrawtransaction [hex]; options None -> single param)",
        ],
        "gates": [],
    },
}


# ---------------------------------------------------------------------------
# Client-call encodings (F2/F3): every form a pinned consumer actually sends
# ---------------------------------------------------------------------------

# envelope "1.0": bitcoind-async-client 0.15.0 (client/mod.rs:189-196:
# json!({"jsonrpc": "1.0", "id": <usize counter from 0>, ...})).
# envelope "2.0": bitcoincore-rpc 0.19 -> jsonrpc 0.18 (client.rs:44-59:
# nonce AtomicUsize starting at 1, id: Value::from(nonce), jsonrpc "2.0").
# Both use NUMERIC request ids; a response that echoes the id as a JSON
# string breaks the async client (its Response.id is u64).

ENCODINGS = {
    "getblockheader.traffic-consumer": {
        "method": "getblockheader", "params": "[hash]", "envelope": "1.0",
        "consumer": TRAFFIC_CONSUMER},
    "getrawtransaction.blockhash.traffic-consumer": {
        "method": "getrawtransaction", "params": "[txid, 0, blockhash]",
        "envelope": "1.0", "consumer": TRAFFIC_CONSUMER},
    "getnetworkinfo.async": {"method": "getnetworkinfo", "params": "[]",
                             "envelope": "1.0",
                             "consumer": "bitcoind-async-client / server call_raw"},
    "getnetworkinfo.sync": {"method": "getnetworkinfo", "params": "[]",
                            "envelope": "2.0",
                            "consumer": "bitcoincore-rpc 0.19 version()"},
    "getblockchaininfo.async": {"method": "getblockchaininfo", "params": "[]",
                                "envelope": "1.0",
                                "consumer": "async v30 network()"},
    "getindexinfo.async": {"method": "getindexinfo", "params": "[]",
                           "envelope": "1.0",
                           "consumer": "bitcoin-ext require_txindex (call_raw)"},
    "getblockcount.async": {"method": "getblockcount", "params": "[]",
                            "envelope": "1.0", "consumer": "async get_block_count"},
    "getbestblockhash.async": {"method": "getbestblockhash", "params": "[]",
                               "envelope": "1.0",
                               "consumer": "async get_current_timestamp"},
    "getblockhash.async": {"method": "getblockhash", "params": "[height]",
                           "envelope": "1.0", "consumer": "async get_block_hash"},
    "getblock.verbosity0.async": {"method": "getblock", "params": "[hash, 0]",
                                  "envelope": "1.0", "consumer": "async get_block"},
    "getblock.verbosity0.sync": {"method": "getblock", "params": "[hash, 0]",
                                 "envelope": "2.0",
                                 "consumer": "sync get_block / get_block_hex"},
    "getblock.verbosity1.sync": {"method": "getblock", "params": "[hash, 1]",
                                 "envelope": "2.0",
                                 "consumer": "sync get_block_info (bdk emitter)"},
    "getblock.verbosity1.async": {"method": "getblock", "params": "[hash, 1]",
                                  "envelope": "1.0",
                                  "consumer": "async get_block_verbose_one"},
    "getblock.singleparam.async": {"method": "getblock", "params": "[hash]",
                                   "envelope": "1.0",
                                   "consumer": "async get_block_height "
                                               "(verbosity defaults to 1 server-side)"},
    "getblockheader.verbose.async": {"method": "getblockheader",
                                     "params": "[hash, true]",
                                     "envelope": "1.0",
                                     "consumer": "bark server tx_status (call_raw)"},
    "getblockheader.verbose.sync": {"method": "getblockheader",
                                    "params": "[hash, true]",
                                    "envelope": "2.0",
                                    "consumer": "sync get_block_header_info"},
    "getblockheader.nonverbose.sync": {"method": "getblockheader",
                                       "params": "[hash, false]",
                                       "envelope": "2.0",
                                       "consumer": "sync get_block_header (hex)"},
    "getrawtransaction.verbosity0.async": {
        "method": "getrawtransaction", "params": "[txid, 0]", "envelope": "1.0",
        "consumer": "async get_raw_transaction_verbosity_zero"},
    "getrawtransaction.verbosity1.async": {
        "method": "getrawtransaction", "params": "[txid, 1]", "envelope": "1.0",
        "consumer": "async get_raw_transaction_verbosity_one"},
    "getrawtransaction.bool.async": {
        "method": "getrawtransaction", "params": "[txid, true]", "envelope": "1.0",
        "consumer": "bark server custom_get_raw_transaction_info / wallet "
                    "bitcoind_tx_status (call_raw)"},
    "getrawtransaction.bool.sync": {
        "method": "getrawtransaction", "params": "[txid, true]",
        "envelope": "2.0",
        "consumer": "bitcoincore-rpc 0.19 get_raw_transaction_info and "
                    "bitcoin-ext custom_get_raw_transaction_info "
                    "(client.rs:507-513 / rpc/mod.rs:320-331: args are "
                    "[txid, true, null] but handle_defaults "
                    "(client.rs:116-147) STRIPS the trailing null, so the "
                    "wire form is [txid, true])"},
    "getrawtransaction.bool-blockhash.sync": {
        "method": "getrawtransaction", "params": "[txid, true, blockhash]",
        "envelope": "2.0",
        "consumer": "bitcoincore-rpc 0.19 get_raw_transaction_info with "
                    "Some(block_hash)"},
    "gettxout.async": {"method": "gettxout", "params": "[txid, vout, bool]",
                       "envelope": "1.0",
                       "consumer": "bitcoin-ext try_get_tx_out (call_raw)"},
    "gettxout.sync": {"method": "gettxout", "params": "[txid, vout, bool]",
                      "envelope": "2.0", "consumer": "sync 0.19 get_tx_out"},
    "getrawmempool.async": {"method": "getrawmempool", "params": "[]",
                            "envelope": "1.0", "consumer": "async get_raw_mempool"},
    "getrawmempool.bool.sync": {"method": "getrawmempool", "params": "[false]",
                                "envelope": "2.0",
                                "consumer": "bitcoin-ext get_mempool_spending_tx"},
    "getmempoolinfo.async": {"method": "getmempoolinfo", "params": "[]",
                             "envelope": "1.0",
                             "consumer": "selected modelling consumer: "
                                         "async Reader::get_mempool_info "
                                         "(corepc v30; bark itself does "
                                         "not call it, and the T0 "
                                         "RpcNodeControl consumer is not "
                                         "automatically this consumer)"},
    "getmempoolentry.async": {"method": "getmempoolentry", "params": "[txid]",
                              "envelope": "1.0",
                              "consumer": "bark server / wallet (call_raw)"},
    "getmempoolentry.sync": {"method": "getmempoolentry", "params": "[txid]",
                             "envelope": "2.0", "consumer": "sync 0.19"},
    "estimatesmartfee.async": {"method": "estimatesmartfee",
                               "params": "[target, \"ECONOMICAL\"]",
                               "envelope": "1.0",
                               "consumer": "bark wallet fetch_fee_rates (call_raw); "
                                           "EstimateMode serializes UPPERCASE"},
    "estimatesmartfee.sync": {"method": "estimatesmartfee",
                              "params": "[target]",
                              "envelope": "2.0",
                              "consumer": "sync 0.19 estimate_smart_fee "
                                          "(client.rs:1083-1089: args are "
                                          "[target, null] but handle_defaults "
                                          "strips the trailing null; "
                                          "Some(mode) appends the mode "
                                          "literal, e.g. [target, "
                                          "\"ECONOMICAL\"])"},
    "testmempoolaccept.async": {"method": "testmempoolaccept",
                                "params": "[[hex]]",
                                "envelope": "1.0",
                                "consumer": "bark server bitcoind.rs:216-221 "
                                            "test_mempool_accept (async "
                                            "call_raw; captaind, offboards "
                                            "callers)"},
    "submitpackage.async": {"method": "submitpackage", "params": "[[hex, ...]]",
                            "envelope": "1.0",
                            "consumer": "bark chain.rs:685 and server "
                                        "bitcoind.rs:207-214 submit_package "
                                        "(async call_raw; every active bark "
                                        "caller is envelope 1.0)"},
    "sendrawtransaction.async": {"method": "sendrawtransaction", "params": "[hex]",
                                 "envelope": "1.0",
                                 "consumer": "async send_raw_transaction "
                                             "(options None -> single param)"},
    "sendrawtransaction.sync": {"method": "sendrawtransaction", "params": "[hex]",
                                "envelope": "2.0", "consumer": "sync 0.19"},
}


# ---------------------------------------------------------------------------
# Check manifest (F5): every check the probe can emit, with coverage class
# ---------------------------------------------------------------------------

# coverage classes:
#   required     gates the verdict (a behavior/availability failure makes
#                the node incompatible)
#   encoding     exact client-form acceptance (params + envelope + numeric
#                id echo); gates the verdict under fail-kind "encoding"
#   protocol     JSON-RPC protocol behavior (id echo, envelope handling);
#                gates the verdict under fail-kind "protocol"
#   observational recorded for lab bookkeeping; NEVER fails the verdict
#                (traffic-like skip: honest rows, no fail-kind
#                accumulation, no gating). Used for methods on the
#                pinned async client's API surface that bark c419159
#                itself never calls (getbestblockhash via
#                get_current_timestamp, v30.rs:107-110) and for the
#                getnetworkinfo observation superset.
#   funded       funded-path behavior; in read-only mode the behavior axis
#                is permanently unknown/not_run (funded_state "unproven")
#   traffic      traffic-slice surface (T0 lab-traffic-contract subset):
#                methods the pinned async client exposes but bark c419159
#                itself never calls. Rows are HONEST (passed/failed/
#                unknown, real fail_kind) for the traffic consumer, but
#                they NEVER gate the bark verdict in either direction:
#                no fail_kind is accumulated and no surface_compatible
#                requirement applies, because the verdict answers "is
#                this node compatible with the pinned bark?" and bark
#                does not consume the method. A traffic check that fails
#                is traffic-surface information, not a bark verdict
#                change; a traffic check that passes proves nothing
#                about bark compatibility either.
#
# unknown_behavior_ok: the check's behavior axis may legitimately be
# unknown on a healthy unfunded node (genesis coinbase -5, empty
# mempool fee data, unfunded mempool entry) without blocking
# surface_compatible.

CHECKS = {
    "getmempoolinfo.traffic-consumer": {
        "method": "getmempoolinfo", "coverage": "traffic",
        "contract": "getmempoolinfo.traffic-consumer",
        "encoding": "getmempoolinfo.async", "emit": "always",
        "unknown_behavior_ok": False},
    "getblockheader.traffic-consumer": {
        "method": "getblockheader", "coverage": "traffic",
        "contract": "getblockheader.traffic-consumer",
        "encoding": "getblockheader.traffic-consumer", "emit": "always",
        "unknown_behavior_ok": False},
    "getrawtransaction.blockhash.traffic-consumer": {
        "method": "getrawtransaction", "coverage": "traffic",
        "contract": "getrawtransaction.verbosity0",
        "encoding": "getrawtransaction.blockhash.traffic-consumer", "emit": "always",
        "unknown_behavior_ok": True},
    "getnetworkinfo.version-shape": {
        "method": "getnetworkinfo", "coverage": "required",
        "contract": "getnetworkinfo.required",
        "emit": "always"},
    "getnetworkinfo.gate.server": {
        "method": "getnetworkinfo", "coverage": "required",
        "contract": None, "emit": "always",
        "unknown_behavior_ok": True},
    "getnetworkinfo.gate.wallet": {
        "method": "getnetworkinfo", "coverage": "required",
        "contract": None, "emit": "always",
        "unknown_behavior_ok": True},
    "getnetworkinfo.observe": {
        "method": "getnetworkinfo", "coverage": "observational",
        "contract": "getnetworkinfo.observe",
        "emit": "always (mismatches are recorded as observations, not failures)"},
    "protocol.envelope20.getnetworkinfo": {
        "method": "getnetworkinfo", "coverage": "encoding",
        "contract": "getnetworkinfo.required",
        "encoding": "getnetworkinfo.sync",
        "emit": "always (sync-client form: envelope 2.0, numeric id)"},
    "getblockchaininfo.network": {
        "method": "getblockchaininfo", "coverage": "required",
        "contract": None, "emit": "always"},
    "getblockchaininfo.shape.async": {
        "method": "getblockchaininfo", "coverage": "required",
        "contract": "getblockchaininfo.async", "emit": "always"},
    "getblockchaininfo.shape.sync": {
        "method": "getblockchaininfo", "coverage": "required",
        "contract": "getblockchaininfo.sync", "emit": "always"},
    "getindexinfo.txindex": {
        "method": "getindexinfo", "coverage": "required",
        "contract": "getindexinfo", "emit": "always"},
    "getblockcount": {
        "method": "getblockcount", "coverage": "required",
        "contract": "getblockcount", "emit": "always"},
    "getbestblockhash": {
        "method": "getbestblockhash", "coverage": "observational",
        "contract": "getbestblockhash", "emit": "always (async client API "
        "surface via get_current_timestamp; bark c419159 has no caller, "
        "so an honest failure never gates the bark verdict)"},
    "getblockhash.genesis": {
        "method": "getblockhash", "coverage": "required",
        "contract": "getblockhash", "emit": "always"},
    "getblock.verbosity0": {
        "method": "getblock", "coverage": "required",
        "contract": "getblock.verbosity0",
        "encoding": "getblock.verbosity0.async", "emit": "always"},
    "getblock.verbosity0.rehash": {
        "method": "getblock", "coverage": "required",
        "contract": None, "emit": "always (header re-hash integrity)"},
    "getblock.verbosity1": {
        "method": "getblock", "coverage": "required",
        "contract": "getblock.verbosity1",
        "encoding": "getblock.verbosity1.sync",
        "emit": "always (bdk emitter form: [hash, 1], envelope 2.0)"},
    "getblock.singleparam": {
        "method": "getblock", "coverage": "encoding",
        "contract": "getblock.verbosity1",
        "encoding": "getblock.singleparam.async",
        "emit": "always (async get_block_height form: [hash], verbosity "
                "defaults to 1 server-side)"},
    "getblockheader.verbose": {
        "method": "getblockheader", "coverage": "required",
        "contract": "getblockheader.verbose",
        "encoding": "getblockheader.verbose.async", "emit": "always"},
    "getblockheader.verbose.genesis": {
        "method": "getblockheader", "coverage": "required",
        "contract": None, "emit": "always (genesis consistency)"},
    "getblockheader.encoding.sync20": {
        "method": "getblockheader", "coverage": "encoding",
        "contract": "getblockheader.verbose",
        "encoding": "getblockheader.verbose.sync",
        "emit": "always (sync form: [hash, true], envelope 2.0)"},
    "getblockheader.nonverbose": {
        "method": "getblockheader", "coverage": "encoding",
        "contract": "getblockheader.nonverbose",
        "encoding": "getblockheader.nonverbose.sync",
        "emit": "always (sync hex form: [hash, false], envelope 2.0)"},
    "getrawtransaction.verbosity0": {
        "method": "getrawtransaction", "coverage": "required",
        "contract": "getrawtransaction.verbosity0",
        "encoding": "getrawtransaction.verbosity0.async", "emit": "always",
        "unknown_behavior_ok": True},
    "getrawtransaction.verbosity1": {
        "method": "getrawtransaction", "coverage": "required",
        "contract": "getrawtransaction.verbose",
        "encoding": "getrawtransaction.verbosity1.async", "emit": "always",
        "unknown_behavior_ok": True},
    "getrawtransaction.bool-verbose": {
        "method": "getrawtransaction", "coverage": "encoding",
        "contract": "getrawtransaction.verbose",
        "encoding": "getrawtransaction.bool.sync",
        "emit": "always (sync form: [txid, true], envelope 2.0; "
                "handle_defaults strips the trailing null)",
        "unknown_behavior_ok": True},
    "getrawtransaction.bool-verbose.async": {
        "method": "getrawtransaction", "coverage": "encoding",
        "contract": "getrawtransaction.verbose",
        "encoding": "getrawtransaction.bool.async",
        "emit": "always (bark call_raw form: [txid, true], envelope 1.0)",
        "unknown_behavior_ok": True},
    "gettxout.mempool-true": {
        "method": "gettxout", "coverage": "required",
        "contract": "gettxout", "encoding": "gettxout.async", "emit": "always"},
    "gettxout.mempool-false": {
        "method": "gettxout", "coverage": "required",
        "contract": "gettxout", "encoding": "gettxout.async", "emit": "always"},
    "getrawmempool": {
        "method": "getrawmempool", "coverage": "required",
        "contract": "getrawmempool", "encoding": "getrawmempool.async",
        "emit": "always"},
    "getrawmempool.bool-form": {
        "method": "getrawmempool", "coverage": "encoding",
        "contract": "getrawmempool", "encoding": "getrawmempool.bool.sync",
        "emit": "always (bitcoin-ext sync form: [false], envelope 2.0)"},
    "getmempoolinfo": {
        "method": "getmempoolinfo", "coverage": "traffic",
        "contract": "getmempoolinfo", "encoding": "getmempoolinfo.async",
        "emit": "always (traffic-slice surface of the pinned async client; "
                "bark c419159 itself never calls getmempoolinfo, so this "
                "check never gates the bark verdict in either direction)"},
    "getmempoolentry.recognized": {
        "method": "getmempoolentry", "coverage": "required",
        "contract": None, "emit": "always",
        "unknown_behavior_ok": True},
    "getmempoolentry.shape.server": {
        "method": "getmempoolentry", "coverage": "funded",
        "contract": "getmempoolentry.server",
        "emit": "always (behavior not_run without a funded fixture)"},
    "getmempoolentry.shape.wallet": {
        "method": "getmempoolentry", "coverage": "funded",
        "contract": "getmempoolentry.wallet",
        "emit": "always (behavior not_run without a funded fixture)"},
    "estimatesmartfee.t1.shape": {
        "method": "estimatesmartfee", "coverage": "required",
        "contract": "estimatesmartfee", "emit": "profile target 1"},
    "estimatesmartfee.t3.shape": {
        "method": "estimatesmartfee", "coverage": "required",
        "contract": "estimatesmartfee", "emit": "profile target 3"},
    "estimatesmartfee.t6.shape": {
        "method": "estimatesmartfee", "coverage": "required",
        "contract": "estimatesmartfee", "emit": "profile target 6"},
    "estimatesmartfee.fee-data": {
        "method": "estimatesmartfee", "coverage": "required",
        "contract": None,
        "emit": "always (unknown, never pass, when no feerate is returned)",
        "unknown_behavior_ok": True},
    "protocol.id-echo": {
        "method": None, "coverage": "protocol", "contract": None,
        "emit": "always (every exchange must echo the numeric request id "
                "exactly; a string echo breaks the async client whose "
                "Response.id is u64)"},
    "testmempoolaccept.recognized": {
        "method": "testmempoolaccept", "coverage": "funded",
        "contract": "testmempoolaccept",
        "encoding": "testmempoolaccept.async",
        "emit": "always (invalid sample proves routing only; sent as the "
                "active bark caller form: async call_raw, envelope 1.0)"},
    "submitpackage.recognized": {
        "method": "submitpackage", "coverage": "funded",
        "contract": "submitpackage",
        "encoding": "submitpackage.async",
        "emit": "always (invalid sample proves routing only; sent as the "
                "active bark caller form: async call_raw, envelope 1.0)"},
    "sendrawtransaction.recognized": {
        "method": "sendrawtransaction", "coverage": "funded",
        "contract": "sendrawtransaction",
        "encoding": "sendrawtransaction.async",
        "emit": "always (invalid sample proves routing only)"},
}


# ---------------------------------------------------------------------------
# Verdict rules (F5): mirrored by schemadef.validate_report_consistency
# ---------------------------------------------------------------------------

VERDICT_RULES = {
    "status_enum": ["incompatible", "surface_compatible", "unproven"],
    "note": (
        "Read-only mode never offers a full-compat outcome: full "
        "acceptance additionally requires funded broadcast/mempool "
        "behavior which this mode cannot observe. funded_state is "
        "permanently \"unproven\" and full_acceptance permanently false "
        "here; a separate funded packet must prove them."
    ),
    "incompatible_if": (
        "any check has availability=failed or behavior=failed"
    ),
    "unproven_if": (
        "no check failed, but some required/encoding/protocol check has "
        "availability != passed (e.g. auth/transport/protocol-level "
        "failures): the surface was not proven, so surface_compatible "
        "must not be claimed"
    ),
    "surface_compatible_if": (
        "no check failed and every required/encoding/protocol check "
        "passed on the availability axis, with behavior passed or "
        "legitimately unknown (unknown_behavior_ok: genesis-coinbase -5, "
        "empty-mempool fee data, unfunded mempool entry)"
    ),
    "reason_order": [
        "fail-version-gate", "fail-method-surface", "fail-encoding",
        "fail-protocol", "fail-network", "fail-genesis", "fail-shape",
        "surface-unproven", "funded-unproven",
    ],
    "funded_state": "unproven",
    "full_acceptance": False,
    "traffic_coverage": (
        "checks with coverage class \"traffic\" (the T0 lab-traffic-"
        "contract subset: getmempoolinfo) are honest rows for the traffic "
        "consumer but NEVER gate this verdict in either direction: their "
        "failures add no fail-kind and their passes satisfy no "
        "surface_compatible requirement, because bark c419159 does not "
        "call the method. A traffic-check failure is traffic-surface "
        "information only; the verdict remains scoped to the pinned bark."
    ),
    "observational_coverage": (
        "checks with coverage class \"observational\" (getbestblockhash, "
        "getnetworkinfo.observe) are honest rows for the async client API "
        "surface / lab bookkeeping but NEVER gate this verdict in either "
        "direction, exactly like traffic rows: their failures add no "
        "fail-kind and their passes satisfy no surface_compatible "
        "requirement, because bark c419159 has no caller for these "
        "methods (getbestblockhash is only reachable through the async "
        "client's get_current_timestamp, which bark never calls)."
    ),
    "unproven_checks": (
        "check ids whose behavior axis is unknown or not_run; always "
        "non-empty in read-only mode (the funded paths)"
    ),
}


# ---------------------------------------------------------------------------
# Canonical JSON (the absorption interface, F5)
# ---------------------------------------------------------------------------


def canonical_contract_json() -> str:
    """Frozen JSON form of contracts, encodings, checks, and verdict rules.

    This is the absorption interface for AP-1b: it must contain the
    check IDs, the required/observational coverage classification, the
    exact client-call encodings, and the verdict rules, so a report can
    be validated against the same rules the probe applied.
    """
    from .launchprofile import LAUNCH_PROFILE_CONTRACT
    payload = {
        "launch_profile_contract": LAUNCH_PROFILE_CONTRACT,
        "probe": "ap1a-ark-rpc-probe",
        "manifest_version": MANIFEST_VERSION,
        "traffic_consumer": TRAFFIC_CONSUMER,
        "traffic_metadata": {
            "consumer_model_failures": "verbatim failed getmempoolinfo row; non-gating",
            "fee_policy": "raw decimal lexemes, exact integer sats/kB; unknown is null",
            "launch_profile": "spc-lab-launch-profile/1; RFC8785 six-field comparison; private env",
        },
        "profiles": PROFILES,
        "contracts": {
            k: {
                "spec": v["spec"],
                "consumer": v.get("consumer"),
                "sources": v.get("sources", []),
                "gates": v.get("gates", []),
            }
            for k, v in CONTRACTS.items()
        },
        "encodings": ENCODINGS,
        "checks": CHECKS,
        "verdict_rules": VERDICT_RULES,
    }
    return json.dumps(payload, sort_keys=True, indent=1, default=str)


def contract_hash() -> str:
    return hashlib.sha256(canonical_contract_json().encode()).hexdigest()
