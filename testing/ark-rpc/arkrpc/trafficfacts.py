"""Traffic-only policy observations, parsed from raw JSON numeric lexemes."""
import json
import re

from .contracts import ContractError, parse_spec


class NumberLexeme(str):
    pass


def sats_exact(lexeme):
    if not isinstance(lexeme, NumberLexeme) or not re.fullmatch(r"(?:0|[1-9][0-9]*)(?:\.[0-9]{1,8})?", lexeme):
        raise ContractError("fee: expected raw nonnegative decimal lexeme, at most 8 places, no exponent")
    whole, _, fraction = lexeme.partition(".")
    sats = int(whole) * 100_000_000 + int(fraction.ljust(8, "0"))
    if sats > 2**64 - 1:
        raise ContractError("fee: u64 satoshi overflow")
    return sats


def policy_facts(record):
    facts = {"minrelaytxfee_btc_kb_lexeme": None, "minrelaytxfee_sats_kb": None,
             "maxmempool": None, "bytes": None, "usage": None, "errors": []}
    if record.error is not None or record.raw_body is None:
        facts["errors"].append("raw_policy_response_unavailable")
        return facts
    from .launchprofile import pairs
    try:
        result = json.loads(record.raw_body, parse_int=NumberLexeme,
                            parse_float=NumberLexeme, object_pairs_hook=pairs)["result"]
        value = result["minrelaytxfee"]
        facts["minrelaytxfee_sats_kb"] = sats_exact(value)
        facts["minrelaytxfee_btc_kb_lexeme"] = str(value)
    except (KeyError, TypeError, ValueError):
        facts["errors"].append("minrelaytxfee_invalid_or_missing")
    for key in ("maxmempool", "bytes", "usage"):
        try:
            facts[key] = parse_spec(record.result[key], {"kind": "u64"})
        except (KeyError, TypeError, ContractError):
            facts["errors"].append(key + "_invalid_or_missing")
    return facts


def effective_consensus(result):
    """Closed projection of the required live binding facts; absence rejects."""
    fields = {
        **{k: {"kind": "string"} for k in (
            "network", "consensus_profile_id", "profile_fingerprint", "initial_target_bits",
            "regtest_pow_bits_overlay", "pow_limit_bits", "retarget_floor_bits", "subsidy_initial_sats")},
        **{k: {"kind": "u64"} for k in ("pow_target_spacing", "pow_target_timespan",
            "difficulty_adjustment_interval", "subsidy_halving_interval", "coinbase_maturity")},
        **{k: {"kind": "bool"} for k in ("no_pow_retargeting", "allow_min_difficulty_blocks")},
    }
    value = parse_spec(result.get("consensus"), {"kind": "object", "fields": fields})
    if value["network"] != "regtest":
        raise ContractError("consensus network is not regtest")
    for key in ("initial_target_bits", "regtest_pow_bits_overlay", "pow_limit_bits", "retarget_floor_bits"):
        if not re.fullmatch(r"(?:0x)?[0-9a-fA-F]{8}", value[key]):
            raise ContractError("invalid consensus bits")
    if not re.fullmatch(r"[0-9a-f]{64}", value["profile_fingerprint"]):
        raise ContractError("invalid consensus fingerprint")
    if not re.fullmatch(r"(?:0|[1-9][0-9]*)", value["subsidy_initial_sats"]) or int(value["subsidy_initial_sats"]) > 2**64 - 1:
        raise ContractError("invalid subsidy sats")
    return value
