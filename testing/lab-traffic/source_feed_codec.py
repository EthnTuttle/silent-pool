"""Independent spc.source-feed/2 framing and attestation preimages.

Bound source-only audited-warmup contract edebd11e; not live approval.
Only the collector attestation key is loaded here, never a wallet scan key.
"""
import hashlib
from functools import lru_cache
import os
from pathlib import Path
import socket
import stat
import time

from case_contract import canonical_json
from oracle import Inconclusive, integer, require, strict_json, unhex
from public_derivation import _N, _P, _GX, _GY, _point_add
from signature_oracle import _multiply, public_point

CONTRACT = "edebd11e723af329117432c96a16e6a74906b7853fee38fa22bbbe1e848f47b0"
PREDECESSOR_CONTRACT = "0515a6a88686f75ccf4ed0ff768a0a0324c7a0b3ae4b95b5acd4a2c126a48a7d"
COMBINED_TESTED_CONTRACT = "419b5f061d47fe4f56c36e85fb8ee5e66ba18cb045f00800dfd3d06a8aedefa3"
FINAL_CORRECTED_CONTRACT = "2314f6e2ec154d390b476e7656349819d53004bf2144b9220d39b9cf07d5d034"
# Exact read-side compatibility: feed preimages and source/SQL schemas are
# unchanged. Corrected prior-backed empty payouts receive independent settlement
# checks. This is not a live/native campaign admission.
READ_COMPATIBLE_CONTRACTS = frozenset({CONTRACT, COMBINED_TESTED_CONTRACT, FINAL_CORRECTED_CONTRACT})
INTERFACE_SHA256 = "981e4dbf96d49e2c6544832bdbb1f860d962eae2a87c723e2c069e0f39465971"
SCHEMA = "spc.source-feed/2"
MAX_FRAME, MAX_BUNDLE, DEADLINE, MAX_LIFETIME_MS = 9*1024*1024, 4*1024*1024, 10, 30000
JOB = "job_id prev_hash version nbits ntime coinbase_prefix coinbase_suffix merkle_path share_target_be pool_extranonce miner_extranonce_len".split()
TRACE = "session seq digest signature".split()
CHECKPOINT = "attempt wire_job_digest job_id generation source_watermark history_watermark bundle_digest history_digest expires_at_unix_ms complete pool signature".split()
ACK = "attempt job_id generation bundle_digest history_digest checkpoint_digest window signature".split()
FRAMES = {"job":"attempt job", "proof":"bundle_hex checkpoint", "consumed":"ack", "committed":"attempt ack_digest signature", "unavailable":"attempt"}


def fields(value, names):
    if not isinstance(value, dict) or set(value) != set(names):
        raise Inconclusive("source feed missing/unsupported fields")


def array(value, size=None, maximum=MAX_BUNDLE):
    require(isinstance(value, list) and (len(value) == size if size is not None else len(value) <= maximum), "feed byte array length/type")
    return bytes(integer(v, 0, 255) for v in value)


def le(value, n=8):
    return integer(value, 0, (1 << (8*n))-1).to_bytes(n,"little")


def tagged(tag, data):
    t = hashlib.sha256(tag.encode()).digest()
    return hashlib.sha256(t+t+data).digest()


def blob(value):
    b = array(value)
    return le(len(b),4)+b


def received_digest(job):
    fields(job, JOB)
    require(isinstance(job["merkle_path"], list) and len(job["merkle_path"]) <= 255, "feed Merkle path bound")
    prefix = array(job["pool_extranonce"], maximum=63)
    require(len(prefix)+integer(job["miner_extranonce_len"],0,63) <= 63, "feed total extranonce bound")
    b = le(job["job_id"],4)+array(job["prev_hash"],32)
    b += b"".join(le(job[k],4) for k in ("version","nbits","ntime"))
    b += blob(job["coinbase_prefix"])+blob(job["coinbase_suffix"])+le(len(job["merkle_path"]),4)
    b += b"".join(array(h,32) for h in job["merkle_path"])+array(job["share_target_be"],32)
    b += blob(job["pool_extranonce"])+le(job["miner_extranonce_len"],4)
    return tagged("SPc/ReceivedJobV1",b)


def trace_digest(seal):
    fields(seal,TRACE)
    return tagged("SPc/SourceTraceSealV1",array(seal["session"],16)+le(seal["seq"])+array(seal["digest"],32))


def checkpoint_digest(cp):
    fields(cp,CHECKPOINT)
    require(type(cp["complete"]) is bool, "checkpoint completeness type")
    b = array(cp["attempt"],16)+array(cp["wire_job_digest"],32)
    b += b"".join(le(cp[k]) for k in ("job_id","generation","source_watermark","history_watermark","expires_at_unix_ms"))
    b += array(cp["bundle_digest"],32)+array(cp["history_digest"],32)+bytes([cp["complete"]])+trace_digest(cp["pool"])
    return tagged("SPc/SourceCheckpointV1",b)


def window_facts(target, actual):
    integer(target,1,(1<<128)-1);integer(actual,0,(1<<128)-1)
    return {"window_target":str(target),"actual_sum":str(actual),"shortfall":str(max(0,target-actual)),"underfilled":actual<target}


def validate_window(value):
    fields(value,("window_target","actual_sum","shortfall","underfilled"))
    from pool_source import decimal_uint
    target,actual,shortfall=(decimal_uint(value[k],128) for k in ("window_target","actual_sum","shortfall"))
    require(type(value["underfilled"]) is bool and value==window_facts(target,actual),"inconsistent signed window facts")
    return target,actual,shortfall


def bundle_window(bundle):
    """Derive expected ACK facts from the exact retained, verified bundle.

    Legacy full-Q payout-proof/1 bundles may omit annotation, not v2 ACK facts.
    This function is not an eligible-history authority.
    """
    value=strict_json(bundle)
    require(value["schema"]=="spc.payout-proof/1","source bundle schema")
    from pool_source import decimal_uint
    target=decimal_uint(value["window_target"],128);actual=0
    for row in value["slices"]:
        if actual<target:actual+=decimal_uint(row["difficulty"],128)
    facts=window_facts(target,actual)
    annotation=value.get("window")
    if annotation is not None:
        fields(annotation,("facts","eligible_history_complete"))
        require(annotation["eligible_history_complete"] is True,"eligible history not independently complete")
        validate_window(annotation["facts"])
        require(annotation["facts"]==facts,"bundle window annotation differs from actual selected sum")
    elif actual<target:
        raise Inconclusive("underfilled bundle lacks complete eligible-history annotation")
    return facts


def consumed_digest(ack):
    fields(ack,ACK)
    target,actual,shortfall=validate_window(ack["window"])
    return tagged("SPc/SourceConsumedV2",array(ack["attempt"],16)+le(ack["job_id"])+le(ack["generation"])
                  +b"".join(array(ack[k],32) for k in ("bundle_digest","history_digest","checkpoint_digest"))
                  +le(target,16)+le(actual,16)+le(shortfall,16)+bytes([ack["window"]["underfilled"]]))


def committed_digest(attempt, ack):
    return tagged("SPc/SourceCommittedV1",array(attempt,16)+array(ack,32))


def verify_public_evidence(value, expected_attempt, expected_job_digest, pool_key, collector_key, scan_key, now_ms):
    fields(value,("schema","checkpoint","consumed","committed"))
    require(value["schema"]==SCHEMA and pool_key!=collector_key,"v2 source evidence/key separation")
    cp,ack,commit=value["checkpoint"],value["consumed"],value["committed"]
    fields(commit,("attempt","ack_digest","signature"))
    attempt=array(expected_attempt,16);require(any(attempt),"native attempt must be nonzero")
    require(array(cp["attempt"],16)==array(ack["attempt"],16)==array(commit["attempt"],16)==attempt
            and array(cp["wire_job_digest"],32)==expected_job_digest and cp["complete"] is True
            and cp["source_watermark"]>0 and cp["history_watermark"]>0 and cp["pool"]["seq"]==cp["source_watermark"],
            "public source evidence correlation/completeness")
    require(integer(now_ms)<cp["expires_at_unix_ms"]<=now_ms+MAX_LIFETIME_MS,"public source evidence expiry/lifetime")
    require(all(ack[k]==cp[k] for k in ("job_id","generation","bundle_digest","history_digest"))
            and array(ack["checkpoint_digest"],32)==checkpoint_digest(cp)
            and array(commit["ack_digest"],32)==consumed_digest(ack),"public consumption/commit correlation")
    verify_signature(trace_digest(cp["pool"]),cp["pool"]["signature"],pool_key)
    verify_signature(checkpoint_digest(cp),cp["signature"],collector_key)
    verify_signature(consumed_digest(ack),ack["signature"],scan_key)
    verify_signature(committed_digest(commit["attempt"],commit["ack_digest"]),commit["signature"],collector_key)
    return ack["window"]


def verify_signature(digest, signature, public_key):
    require(isinstance(digest,bytes) and len(digest)==32,"source attestation digest encoding")
    require(isinstance(signature,str) and isinstance(public_key,str),"source attestation signature/key encoding")
    return _verify_signature_exact(digest,signature,public_key)


@lru_cache(maxsize=65536)
def _verify_signature_exact(digest, signature, public_key):
    # Pure successful cryptographic checks only. Prefix completeness, source
    # process/config binding, expiry and candidate freshness are never cached.
    # Re-reading an append-only source log need not redo old curve operations.
    raw, pub = unhex(signature,64), unhex(public_key,32)
    r, s = int.from_bytes(raw[:32],"big"), int.from_bytes(raw[32:],"big")
    require(r < _P and s < _N, "source attestation signature scalar range")
    point = public_point(b"\2"+pub)
    e = int.from_bytes(tagged("BIP0340/challenge",raw[:32]+pub+digest),"big") % _N
    result = _point_add(_multiply(s,(_GX,_GY)), _multiply(_N-e,point))
    require(result is not None and result[1] % 2 == 0 and result[0] == r, "source attestation signature invalid")


def _sign_digest(secret, digest):
    """Deterministic BIP340 (zero aux), used only for source attestations."""
    integer(secret,1,_N-1)
    point = _multiply(secret,(_GX,_GY)); d = secret if point[1] % 2 == 0 else _N-secret
    pub = point[0].to_bytes(32,"big")
    t = bytes(a^b for a,b in zip(d.to_bytes(32,"big"),tagged("BIP0340/aux",bytes(32))))
    nonce = int.from_bytes(tagged("BIP0340/nonce",t+pub+digest),"big") % _N
    require(nonce != 0,"zero source attestation nonce")
    r = _multiply(nonce,(_GX,_GY)); nonce = nonce if r[1] % 2 == 0 else _N-nonce
    rx = r[0].to_bytes(32,"big")
    e = int.from_bytes(tagged("BIP0340/challenge",rx+pub+digest),"big") % _N
    signature = (rx+((nonce+e*d)%_N).to_bytes(32,"big")).hex()
    verify_signature(digest,signature,pub.hex())
    return signature


class CollectorKey:
    def __init__(self, path, expected_pubkey):
        path = Path(path)
        require(path.is_absolute() and path.resolve() == path and not path.is_symlink(),"collector key path")
        info, parent = path.stat(), path.parent.stat()
        require(stat.S_ISREG(info.st_mode) and info.st_uid == parent.st_uid == os.getuid() and info.st_mode & 0o077 == parent.st_mode & 0o077 == 0
                and info.st_size <= 128,"collector key private ownership/bound")
        self._secret = int.from_bytes(unhex(path.read_text().strip(),32),"big")
        integer(self._secret,1,_N-1)
        self.public_key = _multiply(self._secret,(_GX,_GY))[0].to_bytes(32,"big").hex()
        require(self.public_key == expected_pubkey,"collector attestation key differs from pinned key")

    def checkpoint(self, cp):
        return _sign_digest(self._secret,checkpoint_digest(cp))

    def committed(self, attempt, ack_digest):
        return _sign_digest(self._secret,committed_digest(attempt,ack_digest))


def validate_frame(frame):
    kind = frame.get("kind") if isinstance(frame,dict) else None
    if kind not in FRAMES:
        raise Inconclusive("unsupported source feed frame")
    fields(frame,["kind","schema"]+FRAMES[kind].split())
    require(frame["schema"] == SCHEMA,"source feed schema")
    if kind == "job":
        require(any(array(frame["attempt"],16)),"source attempt must be nonzero"); received_digest(frame["job"])
    elif kind == "proof":
        require(isinstance(frame["bundle_hex"],str) and len(frame["bundle_hex"]) <= 2*MAX_BUNDLE,"source bundle bound")
        unhex(frame["bundle_hex"]); checkpoint_digest(frame["checkpoint"])
    elif kind == "consumed":
        consumed_digest(frame["ack"]); unhex(frame["ack"]["signature"],64)
    elif kind == "committed":
        committed_digest(frame["attempt"],frame["ack_digest"]); unhex(frame["signature"],64)
    else:
        array(frame["attempt"],16)
    return frame


def encode_frame(frame):
    raw = canonical_json(validate_frame(frame))
    require(0 < len(raw) <= MAX_FRAME,"source feed frame bound")
    return le(len(raw),4)+raw


def receive(stream, deadline):
    def take(n):
        result = bytearray()
        while len(result) < n:
            remaining = deadline-time.monotonic()
            if remaining <= 0:
                raise Inconclusive("source exchange deadline")
            stream.settimeout(remaining)
            chunk = stream.recv(n-len(result))
            if not chunk:
                raise Inconclusive("truncated source feed frame")
            result.extend(chunk)
        return bytes(result)
    size = int.from_bytes(take(4),"little")
    if not 0 < size <= MAX_FRAME:
        raise Inconclusive("source feed frame resource bound")
    try:text=take(size).decode("utf-8")
    except UnicodeError as error:raise Inconclusive("source feed frame is not strict UTF-8") from error
    return validate_frame(strict_json(text))


def send(stream, frame, deadline):
    remaining = deadline-time.monotonic()
    if remaining <= 0:
        raise Inconclusive("source exchange deadline")
    stream.settimeout(remaining); stream.sendall(encode_frame(frame))
