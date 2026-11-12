"""Negative and positive fixtures for the contract mini-language.

Each case mirrors a serde semantic of the crate types bark actually
deserializes with (see arkrpc/contracts.py source citations). The golden
vectors marked #devcheck are shared with the dev-only Rust checker
(testing/ark-rpc/devcheck/), which validates them against the actual
pinned crate types so the Python semantics are not merely
self-consistent.
"""

import copy
import decimal
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from arkrpc.contracts import CONTRACTS, ContractError, parse_spec  # noqa: E402


def parse(contract_name, value):
    return parse_spec(value, CONTRACTS[contract_name]["spec"])


class IntTests(unittest.TestCase):
    SPEC = {"kind": "int"}

    def test_ok(self):
        self.assertEqual(parse_spec(190000, self.SPEC), 190000)
        self.assertEqual(parse_spec(0, self.SPEC), 0)

    def test_bool_rejected(self):
        with self.assertRaises(ContractError):
            parse_spec(True, self.SPEC)

    def test_float_rejected(self):
        # serde_json rejects 1.0 for u64/usize targets
        with self.assertRaises(ContractError):
            parse_spec(1.0, self.SPEC)

    def test_string_rejected(self):
        with self.assertRaises(ContractError):
            parse_spec("190000", self.SPEC)


class AmountBtcTests(unittest.TestCase):
    """bitcoin::amount::serde::as_btc semantics (F4).

    The deserialization range is the u64 satoshi range
    (bitcoin-units 0.1.3 amount.rs: Amount::MAX = u64::MAX), NOT the
    21M BTC MAX_MONEY consensus policy constant: an over-supply amount
    parses fine at this layer.
    """
    SPEC = {"kind": "amount_btc"}

    def test_ok(self):
        self.assertEqual(parse_spec(50, self.SPEC), 50_0000_0000)
        self.assertEqual(parse_spec(0.00000001, self.SPEC), 1)
        self.assertEqual(parse_spec(decimal.Decimal("0.1"), self.SPEC), 10_000_000)
        self.assertEqual(parse_spec(0, self.SPEC), 0)

    def test_string_rejected(self):
        # bitcoin amount as_btc deserializes via f64, strings are rejected
        with self.assertRaises(ContractError):
            parse_spec("0.1", self.SPEC)

    def test_bool_rejected(self):
        with self.assertRaises(ContractError):
            parse_spec(True, self.SPEC)

    def test_negative_rejected(self):
        with self.assertRaises(ContractError):
            parse_spec(-0.001, self.SPEC)

    def test_nine_decimals_rejected(self):
        with self.assertRaises(ContractError):
            parse_spec(decimal.Decimal("0.123456789"), self.SPEC)

    def test_over_supply_accepted(self):
        # #devcheck golden vector: 30M BTC is 3e15 sats, within u64.
        # The 21M supply cap is consensus policy (MoneyRange), not a
        # serde bound.
        self.assertEqual(parse_spec(30000000, self.SPEC), 3_000_000_000_000_000)

    def test_u64_overflow_rejected(self):
        # #devcheck golden vector: Amount::MAX is u64::MAX sats =
        # 184467440737.09551615 BTC; one BTC above it must be rejected.
        self.assertEqual(parse_spec(184467440737, self.SPEC),
                         184467440737 * 100_000_000)
        with self.assertRaises(ContractError):
            parse_spec(184467440738, self.SPEC)


class HashAndHexTests(unittest.TestCase):
    def test_hash32(self):
        spec = {"kind": "hash32"}
        good = "ab" * 32
        self.assertEqual(parse_spec(good, spec), good)
        for bad in ("ab" * 31, "ab" * 33, "zz" * 32, 42, None, ""):
            with self.assertRaises(ContractError):
                parse_spec(bad, spec)

    def test_hex(self):
        spec = {"kind": "hex"}
        self.assertEqual(parse_spec("00ff", spec), "00ff")
        for bad in ("0", "", "zz", 5, None):
            with self.assertRaises(ContractError):
                parse_spec(bad, spec)

    def test_empty_hex_valid_where_serde_hex(self):
        # #devcheck golden vector: serde_hex deserializes via
        # FromHex::from_hex, which accepts "" (empty Vec). Fields whose
        # Rust type is Vec<u8> with serde_hex accept empty hex.
        spec = {"kind": "hex", "allow_empty": True}
        self.assertEqual(parse_spec("", spec), "")


class ScriptPubkeyTypeTests(unittest.TestCase):
    """F4: TWO pinned ScriptPubkeyType enums.

    bitcoincore-rpc-json 0.19.0 (lib.rs:907-918) has NO Anchor variant;
    bark's bitcoin-ext vendored copy (rpc/mod.rs:152-166) HAS it. A
    "anchor" vout type breaks every 0.19 consumer (gettxout) while the
    bark vendored consumers (getrawtransaction verbose) accept it.
    """

    def _spk(self, contract):
        return CONTRACTS[contract]["spec"]["fields"]["vout"]["items"][
            "fields"]["scriptPubKey"]["optional"]["type"]

    def test_bark_enum_accepts_anchor(self):
        # #devcheck golden vector
        spec = self._spk("getrawtransaction.verbose")
        self.assertEqual(parse_spec("anchor", spec), "anchor")
        self.assertEqual(parse_spec("pubkeyhash", spec), "pubkeyhash")
        self.assertEqual(parse_spec("witness_v1_taproot", spec),
                         "witness_v1_taproot")
        with self.assertRaises(ContractError):
            parse_spec("PubkeyHash", spec)  # case sensitive
        with self.assertRaises(ContractError):
            parse_spec("strange_type", spec)

    def test_core_019_enum_rejects_anchor(self):
        # #devcheck golden vector: GetTxOutResult comes from the 0.19
        # re-export, whose ScriptPubkeyType lacks Anchor.
        spec = CONTRACTS["gettxout"]["spec"]["of"]["fields"][
            "scriptPubKey"]["optional"]["type"]
        with self.assertRaises(ContractError):
            parse_spec("anchor", spec)
        self.assertEqual(parse_spec("pubkeyhash", spec), "pubkeyhash")


class NullRejectTests(unittest.TestCase):
    """F4: absent-vs-null for #[serde(default)] + custom deserializer.

    serde_hex::opt and deserialize_hex_array_opt call
    String::deserialize / Vec::deserialize directly, so an explicit null
    is a hard serde error even though absence is fine via serde(default).
    """

    def _vin_spec(self):
        return CONTRACTS["getrawtransaction.verbose"]["spec"]["fields"][
            "vin"]["items"]

    def test_versionhex_absent_ok_null_rejected(self):
        # #devcheck twin: devcheck v_null_reject_versionhex
        base = {
            "hash": "0f" * 32, "confirmations": 1, "height": 0,
            "version": 1, "merkleroot": "4a" + "5e" * 31,
            "time": 1, "nonce": 2, "bits": "207fffff", "difficulty": 1.0,
            "chainwork": "00" * 32, "nTx": 1,
        }
        parse("getblockheader.verbose", base)  # absent versionHex: OK
        with self.assertRaises(ContractError):
            parse("getblockheader.verbose",
                  dict(base, versionHex=None))

    def test_vin_coinbase_absent_ok_null_rejected(self):
        # #devcheck twin: devcheck v_null_reject_vin
        spec = self._vin_spec()
        parse_spec({"sequence": 0}, spec)  # absent: OK
        with self.assertRaises(ContractError):
            parse_spec({"sequence": 0, "coinbase": None}, spec)

    def test_txinwitness_absent_ok_null_rejected(self):
        # #devcheck twin: devcheck v_null_reject_vin
        spec = self._vin_spec()
        parse_spec({"sequence": 0}, spec)
        with self.assertRaises(ContractError):
            parse_spec({"sequence": 0, "txinwitness": None}, spec)

    def test_txinwitness_empty_items_valid(self):
        # #devcheck golden vector: deserialize_hex_array_opt items are
        # hex strings; FromHex::from_hex accepts "".
        # Rust twin: devcheck v_txinwitness_empty_items
        spec = self._vin_spec()
        parsed = parse_spec({"sequence": 0, "txinwitness": ["", "00"]}, spec)
        self.assertEqual(parsed["txinwitness"], ["", "00"])

    def test_scriptsig_empty_hex_valid(self):
        # #devcheck golden vector
        # Rust twin: devcheck v_scriptsig_empty_hex
        spec = self._vin_spec()
        parsed = parse_spec({"sequence": 0, "scriptSig": {"asm": "", "hex": ""}},
                            spec)
        self.assertEqual(parsed["scriptSig"]["hex"], "")

    def test_spk_addresses_null_rejected(self):
        # #devcheck twin: devcheck v_null_reject_addresses
        spec = CONTRACTS["getrawtransaction.verbose"]["spec"]["fields"][
            "vout"]["items"]["fields"]["scriptPubKey"]
        parse_spec({"asm": "", "hex": "00"}, spec)
        with self.assertRaises(ContractError):
            parse_spec({"asm": "", "hex": "00", "addresses": None}, spec)


class StringOrArrayTests(unittest.TestCase):
    SPEC = {"kind": "string_or_array"}

    def test_both_forms(self):
        self.assertEqual(parse_spec("warn", self.SPEC), ["warn"])
        self.assertEqual(parse_spec(["a", "b"], self.SPEC), ["a", "b"])
        with self.assertRaises(ContractError):
            parse_spec(3, self.SPEC)
        with self.assertRaises(ContractError):
            parse_spec(["a", 3], self.SPEC)


class GetNetworkInfoTests(unittest.TestCase):
    BASE = {
        "version": 190000,
        "subversion": "/rbitcoin:0.8.99/",
        "protocolversion": 70016,
        "relayfee": 0.00001,
        "incrementalfee": 0.00001,
        "networks": [{
            "name": "ipv4", "limited": False, "reachable": True,
            "proxy": None, "proxy_randomize_credentials": False,
        }],
    }

    def test_required_only_version(self):
        # bark deserializes ONLY version: the required contract parses a
        # minimal response with no other fields.
        parse("getnetworkinfo.required", {"version": 190000})

    def test_required_rejects_missing_version(self):
        with self.assertRaises(ContractError):
            parse("getnetworkinfo.required", {"subversion": "x"})

    def test_observe_ok_with_null_proxy(self):
        parse("getnetworkinfo.observe", self.BASE)

    def test_observe_ok_with_string_proxy(self):
        base = copy.deepcopy(self.BASE)
        base["networks"][0]["proxy"] = "127.0.0.1:9050"
        parse("getnetworkinfo.observe", base)

    def test_observe_failures_are_not_verdict_failures(self):
        # a broken observation set must not raise in the probe: it is
        # recorded as an observation, never a failed check (coverage
        # "observational" never gates the verdict).
        base = copy.deepcopy(self.BASE)
        del base["networks"][0]["reachable"]
        with self.assertRaises(ContractError):
            parse("getnetworkinfo.observe", base)


class BlockchainInfoShapeTests(unittest.TestCase):
    BASE = {
        "chain": "regtest", "blocks": 0, "headers": 0,
        "bestblockhash": "0f" * 32, "bits": "207fffff",
        "target": "7f" * 32, "difficulty": 4.656542373906925e-05,
        "time": 1296688602, "mediantime": 1296688602,
        "verificationprogress": 1.0, "initialblockdownload": True,
        "chainwork": "00" * 32, "size_on_disk": 296, "pruned": False,
        "warnings": [],
    }

    def test_async_ok(self):
        parse("getblockchaininfo.async", self.BASE)

    def test_async_requires_bits(self):
        # #devcheck twin: devcheck v_blockchaininfo_shapes (corepc v29
        # GetBlockchainInfo requires bits/target/time)
        base = dict(self.BASE)
        del base["bits"]
        with self.assertRaises(ContractError):
            parse("getblockchaininfo.async", base)

    def test_sync_ok_without_bits(self):
        # #devcheck twin: devcheck v_blockchaininfo_shapes (0.19
        # GetBlockchainInfoResult has no bits/target/time fields)
        # bitcoincore-rpc-json 0.19 GetBlockchainInfoResult has no
        # bits/target/time fields at all
        base = dict(self.BASE)
        del base["bits"]
        del base["target"]
        del base["time"]
        parse("getblockchaininfo.sync", base)

    def test_sync_accepts_string_warnings(self):
        base = dict(self.BASE, warnings="ok")
        parse("getblockchaininfo.sync", base)
        with self.assertRaises(ContractError):
            parse("getblockchaininfo.async", base)  # async requires array


class GetBlockResultTests(unittest.TestCase):
    """getblock.verbosity1: the bdk emitter's GetBlockResult (F2)."""

    BASE = {
        "hash": "0f" * 32, "confirmations": 1, "size": 200, "weight": 800,
        "height": 0, "version": 1, "merkleroot": "4a" + "5e" * 31,
        "tx": ["4a" + "5e" * 31], "time": 1296688602, "nonce": 2,
        "bits": "207fffff", "difficulty": 4.656542373906925e-05,
        "chainwork": "00" * 32, "nTx": 1,
    }

    def test_ok(self):
        parse("getblock.verbosity1", self.BASE)

    def test_ntx_required_camel_case(self):
        base = dict(self.BASE)
        del base["nTx"]
        base["n_tx"] = 1
        with self.assertRaises(ContractError):
            parse("getblock.verbosity1", base)

    def test_versionhex_null_rejected(self):
        with self.assertRaises(ContractError):
            parse("getblock.verbosity1", dict(self.BASE, versionHex=None))

    def test_versionhex_absent_ok(self):
        parse("getblock.verbosity1", self.BASE)  # no versionHex key at all


class GetBlockHeaderTests(unittest.TestCase):
    BASE = {
        "hash": "0f" * 32, "confirmations": 1, "height": 0,
        "version": 1, "merkleroot": "4a" + "5e" * 31, "time": 1296688602,
        "mediantime": 1296688602, "nonce": 2, "bits": "207fffff",
        "difficulty": 4.656542373906925e-05, "chainwork": "00" * 32,
        "nTx": 1,
    }

    def test_ok(self):
        parse("getblockheader.verbose", self.BASE)

    def test_snake_case_n_tx_rejected(self):
        # GetBlockHeaderResult uses rename_all=camelCase: the parsed key
        # is "nTx"; a node emitting "n_tx" would fail bark's tx_status.
        base = dict(self.BASE)
        del base["nTx"]
        base["n_tx"] = 1
        with self.assertRaises(ContractError):
            parse("getblockheader.verbose", base)

    def test_missing_bits(self):
        base = dict(self.BASE)
        del base["bits"]
        with self.assertRaises(ContractError):
            parse("getblockheader.verbose", base)

    def test_optional_fields(self):
        base = dict(self.BASE, versionHex="00000001",
                    previousblockhash=None, nextblockhash=None)
        parse("getblockheader.verbose", base)


class GetIndexInfoTests(unittest.TestCase):
    """F1: best_block_height is the Core-compatible field name."""

    def test_txindex_synced(self):
        parse("getindexinfo", {"txindex": {"synced": True,
                                           "best_block_height": 0}})

    def test_empty_is_valid_shape(self):
        # an empty getindexinfo result parses; the txindex check fails
        # closed on it (TxindexError::NotEnabled), but the SHAPE is fine
        parse("getindexinfo", {})


class GetMempoolInfoTests(unittest.TestCase):
    """getmempoolinfo: corepc-types 0.13.0 v30::GetMempoolInfo (traffic).

    The pinned async client (bitcoind-async-client 0.15.0, bark's tree
    compiles the v30 module only: default features 30_2 + raw_rpc,
    v29/v30 mutually exclusive) deserializes all 13 fields as required.
    """

    BASE = {
        "loaded": True, "size": 0, "bytes": 0, "usage": 0,
        "total_fee": 0.0, "maxmempool": 83886080,
        "mempoolminfee": 0.00001000, "minrelaytxfee": 0.00001000,
        "incrementalrelayfee": 0.00001000, "unbroadcastcount": 0,
        "fullrbf": True, "permitbaremultisig": True,
        "maxdatacarriersize": 165,
    }

    def test_ok(self):
        # #devcheck twin: devcheck v_getmempoolinfo_shapes (corepc v30
        # GetMempoolInfo accepts the full 13-field shape)
        parse("getmempoolinfo", self.BASE)

    def test_v24_era_response_rejected(self):
        # #devcheck twin: devcheck v_getmempoolinfo_shapes (v30 rejects
        # the v24-era response; v24 itself accepts it, but bark's tree
        # compiles the v30 client module only).
        # corepc-types v24::GetMempoolInfo (the shape the v29 client
        # module would use) lacks permitbaremultisig and
        # maxdatacarriersize; v30 (the module bark's tree compiles)
        # requires them, so a response without them fails the pinned
        # deserializer honestly.
        base = dict(self.BASE)
        del base["permitbaremultisig"]
        del base["maxdatacarriersize"]
        with self.assertRaises(ContractError):
            parse("getmempoolinfo", base)

    def test_null_maxdatacarriersize_rejected(self):
        # #devcheck twin: devcheck v_getmempoolinfo_shapes. The EXACT
        # rbitcoin r1 observation: maxdatacarriersize is JSON null where
        # Core emits an integer. Plain u64 (no Option, no serde default),
        # so the pinned v30 deserializer fails hard and the traffic
        # check fails honestly.
        base = dict(self.BASE, maxdatacarriersize=None)
        with self.assertRaises(ContractError):
            parse("getmempoolinfo", base)

    def test_missing_total_fee_rejected(self):
        base = dict(self.BASE)
        del base["total_fee"]
        with self.assertRaises(ContractError):
            parse("getmempoolinfo", base)

    def test_bool_for_int_rejected(self):
        # serde_json never accepts a JSON bool for an integer field.
        base = dict(self.BASE, size=True)
        with self.assertRaises(ContractError):
            parse("getmempoolinfo", base)

    def test_unknown_fields_ignored(self):
        # #devcheck twin: devcheck v_getmempoolinfo_shapes (rbitcoin's
        # extra fields -- limitclustercount, limitclustersize, optimal,
        # orphanage, relay_enabled -- are ignored; corepc-types does not
        # build with serde-deny-unknown-fields).
        base = dict(self.BASE, limitclustercount=64, limitclustersize=101000,
                    optimal=True, orphanage={"bytes": 0, "size": 0},
                    relay_enabled=False)
        parse("getmempoolinfo", base)


class MempoolEntryTests(unittest.TestCase):
    BASE = {
        "vsize": 100, "time": 1, "height": 0,
        "descendantcount": 1, "descendantsize": 100,
        "ancestorcount": 1, "ancestorsize": 100,
        "wtxid": "ab" * 32,
        "fees": {"base": 0.0001, "modified": 0.0001,
                 "ancestor": 0.0001, "descendant": 0.0001},
        "depends": [], "spentby": [], "bip125-replaceable": True,
    }

    def test_ok(self):
        parse("getmempoolentry.wallet", self.BASE)

    def test_size_alias_for_vsize(self):
        base = dict(self.BASE)
        del base["vsize"]
        base["size"] = 100
        parse("getmempoolentry.wallet", base)

    def test_missing_both_vsize_forms(self):
        base = dict(self.BASE)
        del base["vsize"]
        with self.assertRaises(ContractError):
            parse("getmempoolentry.wallet", base)

    def test_weight_null_ok(self):
        # weight is a plain Option (no custom deserializer): both absent
        # and explicit null deserialize as None.
        parse("getmempoolentry.wallet", dict(self.BASE, weight=None))

    def test_server_shape_ok(self):
        parse("getmempoolentry.server",
              {"chunkweight": 100, "fees": {"chunk": 0.0001}})

    def test_server_shape_missing_chunk(self):
        with self.assertRaises(ContractError):
            parse("getmempoolentry.server",
                  {"chunkweight": 100, "fees": {"base": 0.0001}})


class SubmitPackageTests(unittest.TestCase):
    BASE = {
        "package_msg": "success",
        "tx-results": {"ab" * 32: {"txid": "cd" * 32, "error": None}},
    }

    def test_ok(self):
        parse("submitpackage", self.BASE)

    def test_missing_package_msg(self):
        base = dict(self.BASE)
        del base["package_msg"]
        with self.assertRaises(ContractError):
            parse("submitpackage", base)

    def test_missing_tx_results(self):
        base = dict(self.BASE)
        del base["tx-results"]
        with self.assertRaises(ContractError):
            parse("submitpackage", base)


class GetTxOutTests(unittest.TestCase):
    def test_null(self):
        self.assertIsNone(parse("gettxout", None))

    def test_object(self):
        value = {
            "bestblock": "0f" * 32, "confirmations": 1, "value": 50.0,
            "scriptPubKey": {"asm": "x", "hex": "00"},
            "coinbase": True,
        }
        parsed = parse("gettxout", value)
        self.assertEqual(parsed["value"], 50_0000_0000)

    def test_object_missing_coinbase(self):
        value = {
            "bestblock": "0f" * 32, "confirmations": 1, "value": 50.0,
            "scriptPubKey": {"asm": "x", "hex": "00"},
        }
        with self.assertRaises(ContractError):
            parse("gettxout", value)


class EstimateSmartFeeTests(unittest.TestCase):
    def test_no_feerate_is_valid_shape(self):
        parsed = parse("estimatesmartfee",
                       {"blocks": 1,
                        "errors": ["Insufficient data or no feerate found"]})
        self.assertIsNone(parsed.get("feerate"))

    def test_feerate_null_ok(self):
        # feerate is Option + serde(default) + as_btc::opt: explicit
        # null is accepted (as_btc::opt handles Option properly).
        parsed = parse("estimatesmartfee", {"blocks": 1, "feerate": None})
        self.assertIsNone(parsed.get("feerate"))

    def test_blocks_required(self):
        with self.assertRaises(ContractError):
            parse("estimatesmartfee", {"errors": []})


class ManifestTests(unittest.TestCase):
    def test_check_manifest_covers_new_encoding_checks(self):
        # F2: every actually-used client form has an ENCODINGS entry and
        # a CHECKS entry referencing it.
        from arkrpc.contracts import CHECKS, ENCODINGS
        for cid, meta in CHECKS.items():
            enc = meta.get("encoding")
            if enc is not None:
                self.assertIn(enc, ENCODINGS,
                              "check %s references unknown encoding %s"
                              % (cid, enc))
        for enc in ENCODINGS:
            self.assertIn("method", ENCODINGS[enc])
            self.assertIn("envelope", ENCODINGS[enc])
            self.assertIn(ENCODINGS[enc]["envelope"], ("1.0", "2.0"))

    def test_gating_coverage_classes(self):
        from arkrpc.contracts import CHECKS
        for cid, meta in CHECKS.items():
            self.assertIn(meta.get("coverage"),
                          ("required", "encoding", "protocol",
                           "observational", "funded", "traffic"),
                          "check %s has an unknown coverage class" % cid)

    def test_traffic_class_names_selected_consumer_and_diagnostic(self):
        # The traffic class exists for exactly one check: the T0
        # lab-traffic-contract subset method bark itself never calls.
        from arkrpc.contracts import CHECKS
        traffic = [cid for cid, meta in CHECKS.items()
                   if meta.get("coverage") == "traffic"]
        self.assertEqual(set(traffic), {"getmempoolinfo", "getmempoolinfo.traffic-consumer",
            "getblockheader.traffic-consumer", "getrawtransaction.blockhash.traffic-consumer"})

    def test_traffic_rules_frozen_in_verdict_rules(self):
        from arkrpc.contracts import VERDICT_RULES
        self.assertIn("never gate this verdict",
                      VERDICT_RULES["traffic_coverage"].lower())

    def test_traffic_check_never_gates_verdict(self):
        # Honest rows, no bark-verdict effect in either direction:
        # a FAILED traffic check (shape or method-surface) must not
        # make the verdict incompatible, and an UNKNOWN traffic check
        # must not block surface_compatible. Only unproven bookkeeping
        # applies.
        from arkrpc.probe import compute_verdict
        profile = {"_name": "bark-c419159"}
        for row in (
            {"id": "getmempoolinfo", "method": "getmempoolinfo",
             "availability": "passed", "behavior": "failed",
             "detail": "shape: x", "fail_kind": "shape"},
            {"id": "getmempoolinfo", "method": "getmempoolinfo",
             "availability": "failed", "behavior": "unknown",
             "detail": "x", "fail_kind": "method-surface"},
            {"id": "getmempoolinfo", "method": "getmempoolinfo",
             "availability": "unknown", "behavior": "unknown",
             "detail": "x", "fail_kind": "shape"},
        ):
            verdict = compute_verdict([row], profile)
            self.assertEqual(verdict["status"], "surface_compatible", row)
            self.assertEqual(verdict["reasons"], ["funded-unproven"], row)
        # Bookkeeping: an unknown/not_run traffic row is still listed.
        verdict = compute_verdict(
            [{"id": "getmempoolinfo", "method": "getmempoolinfo",
              "availability": "unknown", "behavior": "unknown",
              "detail": "x", "fail_kind": "shape"}], profile)
        self.assertIn("getmempoolinfo", verdict["unproven_checks"])

    def test_required_failure_still_flips_verdict(self):
        # Control for the test above: a required-class failure keeps
        # its incompatible verdict.
        from arkrpc.probe import compute_verdict
        verdict = compute_verdict(
            [{"id": "getrawmempool", "method": "getrawmempool",
              "availability": "passed", "behavior": "failed",
              "detail": "x", "fail_kind": "shape"}],
            {"_name": "bark-c419159"})
        self.assertEqual(verdict["status"], "incompatible")
        self.assertIn("fail-shape", verdict["reasons"])


    def test_verdict_rules_three_statuses(self):
        from arkrpc.contracts import VERDICT_RULES
        self.assertEqual(VERDICT_RULES["status_enum"],
                         ["incompatible", "surface_compatible", "unproven"])
        self.assertIn("surface-unproven", VERDICT_RULES["reason_order"])
        self.assertEqual(VERDICT_RULES["funded_state"], "unproven")
        self.assertIs(VERDICT_RULES["full_acceptance"], False)


class EncodingFormTests(unittest.TestCase):
    """C2: the frozen ENCODINGS entries must state the wire forms the
    pinned client code actually puts on the wire (verified against the
    pinned sources), not the pre-serialization argument arrays."""

    def test_getrawtransaction_sync_form_null_stripped(self):
        # bitcoincore-rpc 0.19 client.rs:507-513 (get_raw_transaction_
        # info) and bitcoin-ext rpc/mod.rs:320-331 build args
        # [txid, true, null], but handle_defaults (client.rs:116-147)
        # strips the trailing null: the wire form is [txid, true],
        # envelope 2.0.
        from arkrpc.contracts import ENCODINGS
        self.assertIn("getrawtransaction.bool.sync", ENCODINGS)
        self.assertNotIn("getrawtransaction.bool-null.sync", ENCODINGS)
        enc = ENCODINGS["getrawtransaction.bool.sync"]
        self.assertEqual(enc["params"], "[txid, true]")
        self.assertEqual(enc["envelope"], "2.0")
        # no pinned consumer sends the raw-hex sync form [txid, false]:
        # bitcoin-ext's sync get_mempool_spending_tx has no bark caller
        # (server/src/bitcoind.rs:154-167 is fully async)
        self.assertFalse(any(e["method"] == "getrawtransaction" and
                             e["params"] == "[txid, false]"
                             for e in ENCODINGS.values()))

    def test_estimatesmartfee_sync_form_null_stripped(self):
        # bitcoincore-rpc 0.19 client.rs:1083-1089 (estimate_smart_fee
        # with mode None): args [target, null] -> wire form [target].
        from arkrpc.contracts import ENCODINGS
        enc = ENCODINGS["estimatesmartfee.sync"]
        self.assertEqual(enc["params"], "[target]")
        self.assertEqual(enc["envelope"], "2.0")

    def test_mutating_methods_are_async_envelope_1_0(self):
        # Every active bark caller of testmempoolaccept/submitpackage
        # is an async bitcoind-async-client call_raw (envelope "1.0",
        # client/mod.rs:192): server bitcoind.rs:207-221, chain.rs:685.
        # The 2.0 "sync" forms have no active bark consumer.
        from arkrpc.contracts import ENCODINGS
        for name in ("testmempoolaccept", "submitpackage"):
            self.assertIn("%s.async" % name, ENCODINGS)
            self.assertNotIn("%s.sync" % name, ENCODINGS)
            enc = ENCODINGS["%s.async" % name]
            self.assertEqual(enc["envelope"], "1.0")
        self.assertEqual(ENCODINGS["testmempoolaccept.async"]["params"],
                         "[[hex]]")
        self.assertEqual(ENCODINGS["submitpackage.async"]["params"],
                         "[[hex, ...]]")

    def test_getmempoolinfo_consumer_is_selected_modelling_consumer(self):
        # D5: the getmempoolinfo contract names a SELECTED MODELLING
        # CONSUMER (corepc v30 via the async Reader); bark does not
        # call it and T0's RpcNodeControl is not automatically that
        # consumer.
        from arkrpc.contracts import CONTRACTS, ENCODINGS
        text = CONTRACTS["getmempoolinfo"]["consumer"]
        self.assertIn("SELECTED MODELLING CONSUMER", text)
        self.assertIn("T0", text)
        self.assertIn("modelling consumer",
                      ENCODINGS["getmempoolinfo.async"]["consumer"])


class ObservationalClassTests(unittest.TestCase):
    """D4: getbestblockhash is on the pinned async client's API surface
    (Reader::get_current_timestamp, v30.rs:107-110) but bark c419159
    never calls it: coverage class observational, honest rows, never
    gating the verdict in either direction."""

    def test_getbestblockhash_is_observational(self):
        from arkrpc.contracts import CHECKS
        self.assertEqual(CHECKS["getbestblockhash"]["coverage"],
                         "observational")

    def test_observational_class_members(self):
        from arkrpc.contracts import CHECKS
        observational = sorted(
            cid for cid, meta in CHECKS.items()
            if meta.get("coverage") == "observational")
        self.assertEqual(
            observational,
            ["getbestblockhash", "getnetworkinfo.observe"])

    def test_observational_failure_never_gates_verdict(self):
        # Honest rows, no bark-verdict effect in either direction:
        # a FAILED getbestblockhash row must not make the verdict
        # incompatible, and an UNKNOWN row must not block
        # surface_compatible.
        from arkrpc.probe import compute_verdict
        profile = {"_name": "bark-c419159"}
        for row in (
            {"id": "getbestblockhash", "method": "getbestblockhash",
             "availability": "passed", "behavior": "failed",
             "detail": "shape: x", "fail_kind": "shape"},
            {"id": "getbestblockhash", "method": "getbestblockhash",
             "availability": "failed", "behavior": "unknown",
             "detail": "x", "fail_kind": "method-surface"},
            {"id": "getnetworkinfo.observe", "method": "getnetworkinfo",
             "availability": "unknown", "behavior": "unknown",
             "detail": "x", "fail_kind": "shape"},
        ):
            verdict = compute_verdict([row], profile)
            self.assertEqual(verdict["status"], "surface_compatible", row)
            self.assertEqual(verdict["reasons"], ["funded-unproven"], row)
        # Bookkeeping: an unknown/not_run observational row is listed.
        verdict = compute_verdict(
            [{"id": "getbestblockhash", "method": "getbestblockhash",
              "availability": "unknown", "behavior": "unknown",
              "detail": "x", "fail_kind": "shape"}], profile)
        self.assertIn("getbestblockhash", verdict["unproven_checks"])

    def test_observational_rules_frozen_in_verdict_rules(self):
        from arkrpc.contracts import VERDICT_RULES
        self.assertIn("observational_coverage", VERDICT_RULES)
        self.assertIn("never gate this verdict",
                      VERDICT_RULES["observational_coverage"].lower())

    def test_v30_citations_not_v29(self):
        # D4: bark's tree compiles the v30 client module only, so the
        # async-client citations must be v30.rs, not v29.rs.
        from arkrpc.contracts import CONTRACTS, ENCODINGS
        for name, contract in CONTRACTS.items():
            for src in contract.get("sources", []):
                self.assertNotIn("v29.rs", src,
                                 "%s cites v29.rs" % name)
        self.assertIn("v30.rs:107-110",
                      CONTRACTS["getbestblockhash"]["sources"][0])


if __name__ == "__main__":
    unittest.main()
