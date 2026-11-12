"""Execute only the admitted, public-only stock Rust VTXO verifier.

No VTXO decoder/crypto implementation, wallet, signer or network client here.
"""
import hashlib
import json
from pathlib import Path
import re
import subprocess
import time

from case_contract import canonical_json
from oracle import Inconclusive, Invalid, integer, require, strict_json, unhex
from source_feed_codec import fields
from storage import sha_file

ARTIFACT_SHA256="8aa3f792529a46dc73983af9265f8caa23b1ac79ea522bb30f8a1028329e5d34"
ARTIFACT_CANONICAL_SHA256="6d172e169a6dd39894d0443d57756ac61f9e5bd016353e478e31b6c89ca077bd"
BINARY_SHA256="5330c3ce8eaa65a4b96076badbeae420f654c33c2a611927932fab49bb8357dd"
LOCK_SHA256="ac24ee3b09c7c2dd8731417672b9d7b072aa6e9e04127b9c96e8ad80ae842833"
CONTRACT_SHA256="9cb6915a3ff8a0500e42c0846ccf3c0642fd4061c66c0701fd36e660cd85beda"
ARK_REVISION="c419159fa54085572f8ca72d2ba1716556a9d3eb"
EXPECTED="user_pubkey server_pubkey amount_sats expiry_height exit_delta max_exit_depth".split()
REPLY="schema verified validator ark_revision raw_sha256 anchor_raw_sha256 id chain_anchor policy_kind user_pubkey server_pubkey amount_sats expiry_height exit_delta exit_depth".split()
CRYPTO_CASES={"synthetic_signed_board_positive_is_unconditional","frozen_board_round_and_arkoor_full_signatures_validate",
    "same_u_in_valid_htlc_is_not_full_pubkey_policy","foreign_owner_and_server_are_rejected",
    "first_and_terminal_input_signature_corruption_is_not_just_decode_failure","wrong_anchor_input_and_output_and_truncated_proof_reject"}
COMPILED_RECEIPTS={
    "cli-check-receipt.json":"53dba670cd52e55a0052d916c27f95afba7e6b90524c48681689861c1b16dd3f",
    "nix-binding.json":"37304659cea79eea14263c182e58615b1665fda2ce5049969eb5ba35b4ac4389",
    "nix-r1-build-full.log":"9bc673ec588cb5a3512a497629bf33189bc18a0fbdd26850e24f0e02532eb536",
    "nix-r1-receipt.json":"c3d2f7c2090c7bbdbaf77467101ed7b8d6f0ed5a7c8f82f6aff73d8a24709e2c",
    "nix-r1-source-size.json":"1aa55e288dee24d20e684d78a40da17d37d60c775748c2f4aa1cee513b73f42b",
    "r3-build-receipt.json":"d6ac595c9f487dc3ccfaf9746b9a8f416d519ac84aaefe21a44d91f2719f6b4a",
    "r3-clippy-receipt.json":"814f72d81e530d09fa427faecf5ec93ccf6fa675849b843ff24b71788fcd0792",
    "r3-fmt-receipt.json":"28f3d6a75549ec8ea571501141f47bd7a3ce85edd1b1a74dc314c3e280391954",
    "r3-test-receipt.json":"a14cd1122578cab30908203d5fd0356ea2f525d5c61418fb5a72094ca22aa8d0"}


def sha(raw):return hashlib.sha256(raw).hexdigest()


def read_sealed(path,digest,limit=16*1024*1024):
    path=Path(path)
    require(path.is_file() and not path.is_symlink() and path.stat().st_size<=limit,"verifier sealed file type/bound")
    raw=path.read_bytes();require(sha(raw)==digest,"verifier source/receipt seal changed: "+str(path))
    return raw


def point_id(value):
    require(isinstance(value,str) and re.fullmatch(r"[0-9a-f]{64}:(0|[1-9][0-9]*)",value) is not None,"verifier outpoint encoding")
    txid,index=value.split(":");return txid,integer(int(index),0,0xffffffff)


def validate_request(raw_hex,anchor_hex,expected):
    fields(expected,EXPECTED)
    for k in ("user_pubkey","server_pubkey"):
        key=unhex(expected[k],33)
        from signature_oracle import public_point
        public_point(key)
    for k,bits in (("amount_sats",64),("expiry_height",32),("exit_delta",16),("max_exit_depth",16)):
        integer(expected[k],0,(1<<bits)-1)
    raw,anchor=unhex(raw_hex),unhex(anchor_hex)
    require(raw and len(raw)<=4*1024*1024 and anchor and len(anchor)<=4*1024*1024,"verifier public proof bound")
    request={"schema":"spc.lab.ap5.vtxo-proof-request/v1","raw_vtxo_hex":raw_hex,"anchor_tx_hex":anchor_hex,"expected":expected}
    encoded=canonical_json(request)
    require(len(encoded)<=8*1024*1024,"verifier request exceeds compiled bound")
    return encoded


def validate_reply(value,raw_hex,anchor_hex,expected):
    fields(value,REPLY)
    require(value["schema"]=="spc.lab.ap5.vtxo-proof/v1" and value["verified"] is True
            and value["validator"]=="stock-ark-lib-full-chain-and-signatures" and value["ark_revision"]==ARK_REVISION
            and value["policy_kind"]=="pubkey","verifier result schema/full-policy authority")
    require(value["raw_sha256"]==sha(unhex(raw_hex)) and value["anchor_raw_sha256"]==sha(unhex(anchor_hex)),"verifier result substituted public proof bytes")
    for k in EXPECTED[:-1]:require(value[k]==expected[k] and type(value[k]) is type(expected[k]),"verifier expected field mismatch: "+k)
    integer(value["exit_depth"],0,expected["max_exit_depth"])
    # The pinned Rust verifier checks the full anchor transaction/identity and
    # VTXO chain. Actual Bitcoin inclusion/maturity belongs to raw_transaction
    # in the component consumer. Stock synthetic anchors are not live blocks.
    point_id(value["id"]);point_id(value["chain_anchor"])
    return value


class CompiledVerifier:
    def __init__(self,artifact):
        from reproduced_verifier import SCHEMA as REPRODUCED, admit
        if artifact.get('schema')==REPRODUCED:
            self.admission=admit(artifact);self.artifact=dict(artifact);self.path=Path(artifact['path']);self.executions=[]
            return
        require(sha(canonical_json(artifact))==ARTIFACT_CANONICAL_SHA256,"VTXO verifier artifact is not the admitted closed configuration")
        self.artifact=dict(artifact);self.path=Path(artifact["path"]);self.executions=[]
        require(sha_file(self.path)==BINARY_SHA256,"VTXO verifier executable changed")
        binding=strict_json(read_sealed(artifact["nix_binding"]["path"],artifact["nix_binding"]["sha256"]))
        root=Path(binding["source"]);rows={r["path"]:r["sha256"] for r in binding["source_rows"]}
        require(len(rows)==len(binding["source_rows"])==9 and root==Path(artifact["source_root"]),"verifier source census")
        require({str(p.relative_to(root)) for p in root.rglob("*") if p.is_file()}==set(rows),"verifier unlisted/missing source file")
        for name,digest in rows.items():
            require(not Path(name).is_absolute() and ".." not in Path(name).parts,"verifier source path")
            read_sealed(root/name,digest)
        package=self.path.parent.parent
        read_sealed(package/"share/ap5-verifier/Cargo.lock",LOCK_SHA256)
        deriver=subprocess.run(["nix-store","--query","--deriver",str(package)],capture_output=True,timeout=10,check=True).stdout.decode().strip()
        require(deriver==artifact["deriver"],"verifier actual deriver mismatch")
        raw=subprocess.run(["nix","--extra-experimental-features","nix-command","derivation","show",deriver],capture_output=True,timeout=10,check=True).stdout
        derivations=strict_json(raw);require(set(derivations)=={deriver},"verifier derivation census")
        drv=derivations[deriver];env=drv["env"];env=strict_json(env["__json"]) if "__json" in env else env
        require(env["src"]==str(root) and drv["outputs"]["out"]["path"]==str(package) and env["doCheck"] in (True,"1"),"verifier actual Nix source/check binding")
        receipt_root=Path(artifact["nix_binding"]["path"]).parent
        for name,digest in COMPILED_RECEIPTS.items():read_sealed(receipt_root/name,digest)
        for name in ("r3-test","r3-build","r3-fmt","r3-clippy"):
            receipt=strict_json((receipt_root/(name+"-receipt.json")).read_bytes());require(type(receipt["exit"]) is int and receipt["exit"]==0,"verifier compiled check failed")
            read_sealed(receipt_root/(name+".log"),receipt["log_sha256"])
        for name in ("r3-test.log","nix-r1-build-full.log"):
            text=(receipt_root/name).read_text()
            require(all(re.search(re.escape(case)+r" \.\.\. ok",text) for case in CRYPTO_CASES)
                    and "6 passed; 0 failed; 0 ignored" in text,"verifier unconditional crypto coverage")
        self.admission={"status":"ADMITTED_COMPILED_PUBLIC_VERIFIER","binary_sha256":BINARY_SHA256,"source_root":str(root),"source_rows":rows,
            "cargo_lock_sha256":LOCK_SHA256,"unique_crypto_cases":6,"cargo_and_Nix_runs_are_not_12_cases":True,
            "actual_derivation_sha256":sha(raw),"first_actual_funded_board_required":True}

    def verify(self,raw_hex,anchor_hex,expected):
        request=validate_request(raw_hex,anchor_hex,expected)
        require(sha_file(self.path)==BINARY_SHA256,"admitted VTXO executable bytes changed before call")
        start=time.monotonic_ns();record={"binary_sha256":BINARY_SHA256,"request_sha256":sha(request),"authority":"offline_public_proof_only"}
        try:
            result=subprocess.run([str(self.path)],input=request,capture_output=True,timeout=30,
                                  env={"LANG":"C","HOME":"/nonexistent","PATH":"/nonexistent"})
        except subprocess.TimeoutExpired as error:
            record.update(status="INCONCLUSIVE",timeout=True);self.executions.append(record)
            raise Inconclusive("compiled public VTXO verifier timed out") from error
        record.update(exit=result.returncode,elapsed_ns=time.monotonic_ns()-start,stdout_sha256=sha(result.stdout),stderr_sha256=sha(result.stderr))
        record["status"]="REJECTED" if result.returncode else "RETURNED_NOT_YET_CHECKED"
        self.executions.append(record)
        require(len(result.stdout)<=64*1024 and len(result.stderr)<=64*1024,"compiled verifier output bound")
        require(result.returncode==0,"compiled stock full VTXO/signature/policy verification rejected")
        require(result.stdout.endswith(b"\n") and result.stdout.count(b"\n")==1,"compiled verifier exact one-result framing")
        value=validate_reply(strict_json(result.stdout),raw_hex,anchor_hex,expected)
        record["result"]=value
        record["status"]="VERIFIED_PUBLIC_PROOF"
        return value
