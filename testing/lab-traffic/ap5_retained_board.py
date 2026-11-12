"""Independent read-only parser for the tested pool-control retained-proof DTO.

Pins describe source/encoding, not funded or grant-union approval. Envelope
binding is not VTXO cryptographic validation or current spendable-float proof.
"""
from copy import deepcopy
import hashlib
import json
from pathlib import Path
import re

from case_contract import canonical_json
from oracle import Inconclusive, decode_transaction, integer, require, strict_json
from signature_oracle import public_point

CONTROL_HANDOFF_SHA256="3b341bfc0b000994d02083cb98244bc180afc29ca08b75b08101cfddb4905531"
CONTROL_SOURCE="dd9f0c37d7e79474a56132170272fcd6509eb3ebdc501aaa93372e8dd8a4102b"
CONTROL_CODE="8ec102f55081c58f2ff29aaadb5a78c56c9531a1245a74ffa77b484e077e0e42"
REQUEST_SCHEMA="spc.lab.ap5.pool-wallet-request/v1"
RESULT_SCHEMA="spc.lab.ap5.pool-wallet-result/v1"
PROOF_SCHEMA="spc.lab.ap5.pool-wallet-board-proof/v1"
ENVELOPE_SCHEMA="spc.lab.ap5.pool-wallet-board-proof-envelope/v1"
CONTEXT_SCHEMA="spc.lab.ap5.pool-wallet-terminal-receipt-context/v1"
REQUEST_ORDER="schema operation_id action net_target_sats max_fee_sats original_board_operation_id".split()
RESULT_ORDER="schema operation_id state onchain_trusted_sats float_spendable_sats observed_at_unix_ms observed_tip_height error_code funding_txid board".split()
BOARD_ORDER="gross_board_sats net_board_sats board_fee_sats onchain_fee_sats actual_fee_sats expiry_height required_confirmations server_pubkey vtxo_exit_delta min_board_sats max_vtxo_sats vtxo_id".split()
ORIGINAL_ORDER=("schema original_board_operation_id board_request board_request_sha256 original_vtxo_id original_full_vtxo_hex "
    "original_full_vtxo_sha256 vtxo_encoding raw_anchor_tx_hex anchor_txid anchor_vout owner_pubkey owner_policy_hex "
    "owner_policy_kind server_pubkey board checked_at_unix_ms checked_tip_height funding_confirmations stock_checkpoint_ref "
    "terminal_receipt_context_schema terminal_receipt_context_canonical_json_hex terminal_receipt_context_sha256").split()
ENVELOPE_ORDER="schema proof_sha256 canonical_json_hex original lineage_state lineage_complete round_imports lineage_error_code".split()
IMPORT_ORDER="schema stock_movement_id input_vtxo_ids output_vtxo_ids round_funding_txid offchain_fee_sats completed_at_unix_ms".split()
PROOF_STATES={"proof_available","proof_missing","proof_unavailable"}
STATES={"queued","observed","rejected","already_at_target","unknown","submitted","confirmed"}|PROOF_STATES
TERMINAL={"observed","rejected","already_at_target","confirmed"}|PROOF_STATES
PROOF_ERRORS={"original_operation_missing","original_board_unresolved","original_operation_unavailable",
    "original_operation_mismatch","original_board_not_confirmed","proof_not_retained","proof_unavailable",
    "proof_digest_mismatch","proof_binding_mismatch"}


def sha(raw):return hashlib.sha256(raw).hexdigest()


def closed(value,names):
    require(isinstance(value,dict) and set(value)==set(names),"retained-board field set mismatch")
    return {name:value[name] for name in names}


def compact(value):
    # Rust struct order is supplied explicitly; sorting here would be wrong.
    return json.dumps(value,separators=(",",":"),ensure_ascii=False,allow_nan=False).encode("utf-8")


def hex_bytes(value,size=None,maximum=4*1024*1024):
    require(isinstance(value,str) and 0<len(value)<=2*maximum and len(value)%2==0
            and re.fullmatch(r"[0-9a-f]+",value) is not None,"retained-board canonical hex/bound")
    raw=bytes.fromhex(value)
    require(size is None or len(raw)==size,"retained-board byte length")
    return raw


def operation_id(value):
    require(isinstance(value,str) and 0<len(value)<=128 and all(33<=ord(c)<=126 for c in value),"retained-board operation id")


def outpoint(value):
    require(isinstance(value,str) and re.fullmatch(r"[0-9a-f]{64}:(0|[1-9][0-9]*)",value) is not None,
            "retained-board canonical VTXO id")
    txid,index=value.split(":");integer(int(index),0,0xffffffff)
    return txid,int(index)


def request(value):
    require(isinstance(value,dict) and value.get("schema")==REQUEST_SCHEMA,"pool-control request schema")
    operation_id(value.get("operation_id"));action=value.get("action")
    extra={"snapshot":[],"board_float":["net_target_sats","max_fee_sats"],"board_proof_v1":["original_board_operation_id"]}
    require(action in extra,"unsupported pool-control action")
    closed(value,["schema","operation_id","action"]+extra[action])
    if action=="board_proof_v1":
        operation_id(value["original_board_operation_id"])
        require(value["operation_id"]!=value["original_board_operation_id"],"read id must differ from original board mutation id")
    elif action=="board_float":
        integer(value["net_target_sats"],1,2100000000000000);integer(value["max_fee_sats"],0,2100000000000000)
    return value


def request_bytes(value):
    request(value)
    return compact({k:value[k] for k in REQUEST_ORDER if k in value})


def board(value,req):
    closed(value,BOARD_ORDER)
    for name in BOARD_ORDER:
        if name in {"server_pubkey","vtxo_id","max_vtxo_sats"}:continue
        integer(value[name],0,0xffffffff if name in {"expiry_height","vtxo_exit_delta"} else 0xffffffffffffffff)
    public_point(hex_bytes(value["server_pubkey"],33))
    require(value["server_pubkey"].startswith(("02","03")),"compressed board server key")
    if value["max_vtxo_sats"] is not None:integer(value["max_vtxo_sats"])
    if value["vtxo_id"] is not None:outpoint(value["vtxo_id"])
    require(value["gross_board_sats"]-value["board_fee_sats"]==value["net_board_sats"]>0,"board gross/net/fee binding")
    require(value["board_fee_sats"]+value["onchain_fee_sats"]==value["actual_fee_sats"]<=req["max_fee_sats"],"board total fee/cap binding")
    require(value["required_confirmations"]>0 and value["gross_board_sats"]>=value["min_board_sats"],"board minimum/confirmation policy")
    require(value["max_vtxo_sats"] is None or value["net_board_sats"]<=value["max_vtxo_sats"],"board exceeds maximum VTXO")


def result(value,req):
    request(req)
    require(isinstance(value,dict) and set(RESULT_ORDER)<=set(value)<=set(RESULT_ORDER)|{"proof","retained_proof_sha256"},"pool-control result fields")
    require(value["schema"]==RESULT_SCHEMA and value["operation_id"]==req["operation_id"] and value["state"] in STATES,"pool-control result correlation/state")
    state=value["state"]
    for name in ("onchain_trusted_sats","float_spendable_sats","observed_at_unix_ms","observed_tip_height"):
        if value[name] is not None:integer(value[name],0,0xffffffff if name=="observed_tip_height" else 0xffffffffffffffff)
    code=value["error_code"]
    require(code is None or isinstance(code,str) and code.isascii() and 0<len(code)<=256,"pool-control error code")
    if "retained_proof_sha256" in value:
        require(state=="confirmed","retained digest belongs only to Confirmed")
        hex_bytes(value["retained_proof_sha256"],32)
    if req["action"]=="board_proof_v1":
        require(state in PROOF_STATES|{"queued"},"historical read has a mutation/current-observation state")
        require(all(value[k] is None for k in ("onchain_trusted_sats","float_spendable_sats","observed_at_unix_ms","observed_tip_height","funding_txid","board")),
                "historical read must not report current balances/time/tip or board effects")
        if state=="proof_available":
            require("proof" in value and code is None,"available proof missing envelope or contains error")
            envelope(value["proof"],req["original_board_operation_id"])
        else:
            require("proof" not in value,"nonavailable read contains proof")
            require(code is None if state=="queued" else code in PROOF_ERRORS,"proof-read reason")
            if state=="proof_missing":require(code in {"original_operation_missing","proof_not_retained"},"proof-missing reason")
        return value
    require(state not in PROOF_STATES and "proof" not in value,"proof state/envelope on non-read action")
    if req["action"]=="snapshot":require(state in {"queued","observed","rejected"},"snapshot cannot board")
    else:require(state!="observed","board action cannot be a snapshot")
    if state in {"unknown","submitted","confirmed"}:
        hex_bytes(value["funding_txid"],32);board(value["board"],req)
        if state=="confirmed":require(value["board"]["vtxo_id"] is not None,"Confirmed lacks VTXO identity")
    else:require(value["funding_txid"] is None and value["board"] is None,"no-effect state contains a board")
    if state in {"observed","already_at_target","confirmed"}:
        require(value["observed_at_unix_ms"] is not None and value["observed_tip_height"] is not None,"pool observation context missing")
    if state in {"observed","already_at_target"}:
        require(value["onchain_trusted_sats"] is not None and value["float_spendable_sats"] is not None,"pool balance is unknown")
    if state=="already_at_target":require(value["float_spendable_sats"]>=req["net_target_sats"],"no-op below target")
    return value


def context_bytes(value):
    ordered=closed(value,RESULT_ORDER)
    ordered["board"]=closed(ordered["board"],BOARD_ORDER)
    return compact(ordered)


def original_bytes(value):
    ordered=closed(value,ORIGINAL_ORDER)
    request(value["board_request"])
    ordered["board_request"]={k:value["board_request"][k] for k in REQUEST_ORDER if k in value["board_request"]}
    ordered["board"]=closed(value["board"],BOARD_ORDER)
    return compact(ordered)


def lineage(value,original_id):
    require(value["lineage_complete"] is False,"partial lineage cannot establish completeness/current float")
    edges=value["round_imports"]
    require(isinstance(edges,list) and len(edges)<=256,"retained round-import bound")
    if value["lineage_state"]=="unavailable":
        require(not edges and value["lineage_error_code"] in {"no_retained_round_imports","round_import_evidence_unavailable"},"unavailable lineage shape")
        return
    require(value["lineage_state"]=="recorded_partial" and edges and value["lineage_error_code"] is None,"partial lineage shape")
    moves=[];inputs_seen=set();outputs_seen=set()
    for edge in edges:
        closed(edge,IMPORT_ORDER)
        require(edge["schema"]=="spc.lab.ap5.pool-wallet-round-import/v1","round import schema")
        moves.append(integer(edge["stock_movement_id"],0,0xffffffff))
        integer(edge["offchain_fee_sats"]);integer(edge["completed_at_unix_ms"],1)
        hex_bytes(edge["round_funding_txid"],32)
        for side in ("input_vtxo_ids","output_vtxo_ids"):
            ids=edge[side]
            require(isinstance(ids,list) and 0<len(ids)<=256,"round import identity bound")
            for identifier in ids:outpoint(identifier)
            require(ids==sorted(set(ids)),"round import IDs must be sorted and unique")
        ins,outs=set(edge["input_vtxo_ids"]),set(edge["output_vtxo_ids"])
        require(not(ins&inputs_seen or outs&outputs_seen or ins&outs) and original_id not in outs,"conflicting retained lineage")
        inputs_seen|=ins;outputs_seen|=outs
    require(moves==sorted(set(moves)),"stock movement IDs must be sorted and unique")
    reached={original_id};selected=set()
    while True:
        old=len(selected)
        for index,edge in enumerate(edges):
            if reached.intersection(edge["input_vtxo_ids"]):
                selected.add(index);reached.update(edge["output_vtxo_ids"])
        if len(selected)==old:break
    require(len(selected)==len(edges),"lineage has no explicit input-ID path; amounts do not prove ancestry")


def envelope(value,board_id):
    closed(value,ENVELOPE_ORDER);require(value["schema"]==ENVELOPE_SCHEMA,"retained envelope schema")
    original=value["original"];closed(original,ORIGINAL_ORDER)
    require(original["schema"]==PROOF_SCHEMA and original["vtxo_encoding"]=="ark-lib/ProtocolEncoding/c419159","original proof schema/codec")
    req=request(original["board_request"])
    require(req["action"]=="board_float" and req["operation_id"]==original["original_board_operation_id"]==board_id,"original board operation binding")
    require(sha(request_bytes(req))==original["board_request_sha256"],"original request digest")
    require(sha(hex_bytes(original["original_full_vtxo_hex"]))==original["original_full_vtxo_sha256"],"original full VTXO byte digest")
    outpoint(original["original_vtxo_id"])
    tx=decode_transaction(hex_bytes(original["raw_anchor_tx_hex"]))
    require(not tx["coinbase"] and tx["txid"]==original["anchor_txid"],"original raw anchor identity")
    vout=integer(original["anchor_vout"],0,0xffffffff);require(vout<len(tx["outputs"]),"original anchor vout")
    for key in ("owner_pubkey","server_pubkey"):
        public_point(hex_bytes(original[key],33));require(original[key].startswith(("02","03")),"compressed original public key")
    require(original["owner_policy_kind"]=="pubkey" and original["owner_policy_hex"]=="00"+original["owner_pubkey"],"original complete Pubkey policy mismatch")
    board(original["board"],req);b=original["board"]
    require(b["vtxo_id"]==original["original_vtxo_id"] and b["server_pubkey"]==original["server_pubkey"]
            and b["gross_board_sats"]==tx["outputs"][vout]["value"],"original board/anchor/public server binding")
    integer(original["checked_at_unix_ms"],1);integer(original["checked_tip_height"],0,0xffffffff)
    require(integer(original["funding_confirmations"],1)>=b["required_confirmations"]
            and original["checked_tip_height"]<b["expiry_height"],"original historical confirmation context")
    require(original["stock_checkpoint_ref"]==f"board.{tx['txid']}.{vout}","original stock checkpoint reference")
    require(original["terminal_receipt_context_schema"]==CONTEXT_SCHEMA,"terminal context schema")
    context_raw=hex_bytes(original["terminal_receipt_context_canonical_json_hex"])
    require(sha(context_raw)==original["terminal_receipt_context_sha256"],"terminal context digest")
    context=strict_json(context_raw.decode("utf-8"));result(context,req)
    require(context_bytes(context)==context_raw,"terminal context is not canonical struct order")
    require(context["state"]=="confirmed" and context["funding_txid"]==tx["txid"] and context["board"]==b,"terminal context board binding")
    raw=hex_bytes(value["canonical_json_hex"])
    require(raw==original_bytes(original) and strict_json(raw.decode("utf-8"))==original,"original canonical bytes/tree mismatch")
    hex_bytes(value["proof_sha256"],32);require(sha(raw)==value["proof_sha256"],"retained original envelope digest")
    lineage(value,original["original_vtxo_id"])
    return original


def bind(read_request,read_result,board_request,confirmed):
    request(read_request);request(board_request)
    require(read_request["action"]=="board_proof_v1" and board_request["action"]=="board_float"
            and read_request["original_board_operation_id"]==board_request["operation_id"],"retained read targets another board")
    result(confirmed,board_request);result(read_result,read_request)
    if confirmed["state"]!="confirmed" or read_result["state"]!="proof_available":
        raise Inconclusive("original board proof unavailable; no remote reconstruction")
    proof=read_result["proof"];original=proof["original"]
    require(request_bytes(original["board_request"])==request_bytes(board_request),"retained proof differs from sealed board request")
    if "retained_proof_sha256" not in confirmed:
        raise Inconclusive("legacy Confirmed has no retained proof reference")
    require(confirmed["retained_proof_sha256"]==proof["proof_sha256"],"Confirmed retained proof digest mismatch")
    projection={k:v for k,v in confirmed.items() if k!="retained_proof_sha256"}
    require(context_bytes(projection)==hex_bytes(original["terminal_receipt_context_canonical_json_hex"]),
            "terminal context must omit ONLY retained_proof_sha256")
    return deepcopy(original)


def result_bytes(value):
    """Exact serde ResultDto stdout preimage, including nested declared order."""
    ordered={k:value[k] for k in RESULT_ORDER}
    if ordered["board"] is not None:ordered["board"]=closed(ordered["board"],BOARD_ORDER)
    if "proof" in value:
        proof=closed(value["proof"],ENVELOPE_ORDER)
        proof["original"]=strict_json(original_bytes(proof["original"]))
        proof["round_imports"]=[closed(row,IMPORT_ORDER) for row in proof["round_imports"]]
        ordered["proof"]=proof
    if "retained_proof_sha256" in value:ordered["retained_proof_sha256"]=value["retained_proof_sha256"]
    return compact(ordered)


class PoolEvents:
    """Replay separately sealed pool request/result events, plus tool preimages."""
    def __init__(self,records,root,executor):
        self.operations={};self.root=Path(root);self.executor=executor
        active=None;tool=None;ready=None
        for label,value,event in records:
            if label=="pool-wallet-request":
                request(value);key=value["operation_id"]
                require(key not in self.operations,"pool request id reused")
                if ready is not None:raise Inconclusive("successful pool tool has no retained response event")
                self.operations[key]={"request":deepcopy(value),"request_file":event["file"],"result":None,"terminal":False,"effect":None,"vtxo_id":None,"pool":None}
                active=key
            elif label=="tool-intent":
                if tool is not None:raise Inconclusive("overlapping tool intents lack correlation")
                is_pool=value["executable_sha256"]==executor["sha256"]
                if is_pool:
                    require(active is not None,"pool tool has no sealed request")
                    require(Path(value["executable"]).resolve()==Path(executor["path"]).resolve(),"pool executor path differs from binding")
                    op=self.operations[active];matches=[]
                    for pool in ("pool-a","pool-b"):
                        argv=[value["executable"],"--config",str(self.root/"run/private"/("pool-ark-"+pool)/"pool.toml"),
                              "--request",str(self.root/op["request_file"])]
                        if sha(canonical_json(argv))==value["argv_sha256"]:matches.append(pool)
                    require(len(matches)==1 and op["pool"] in (None,matches[0]),"same request changed wallet/config/path invocation")
                    op["pool"]=matches[0]
                tool={"pool":is_pool,"operation":active}
            elif label=="tool-exit":
                if tool is None:raise Inconclusive("tool exit lacks preceding intent")
                if tool["pool"]:
                    require(type(value["exit"]) is int,"pool tool exit type")
                    if value["exit"]==0 and value.get("interrupted") is not True and value.get("timeout") is not True:
                        if ready is not None:raise Inconclusive("pool response event omitted before next completion")
                        ready=(tool["operation"],value["stdout_sha256"])
                tool=None
            elif label=="pool-wallet-result":
                key=value["operation_id"]
                require(key in self.operations and ready is not None and ready[0]==key,"pool result lacks correlated successful tool/request")
                op=self.operations[key];result(value,op["request"])
                require(sha(result_bytes(value)+b"\n")==ready[1],"pool result differs from actual executor stdout")
                ready=None
                if op["terminal"]:require(canonical_json(value)==canonical_json(op["result"]),"terminal pool receipt was rewritten")
                if value["funding_txid"] is not None:
                    proposal={k:v for k,v in value["board"].items() if k!="vtxo_id"}
                    effect=canonical_json([value["funding_txid"],proposal])
                    require(op["effect"] in (None,effect),"same operation changed original board/policy")
                    op["effect"]=effect
                    learned=value["board"]["vtxo_id"]
                    if learned is not None:
                        require(op["vtxo_id"] in (None,learned),"same board operation changed original VTXO")
                        op["vtxo_id"]=learned
                if op["effect"] is not None:require(value["state"] not in {"observed","rejected","already_at_target"},"effect-bearing board reverted to no-effect")
                if value["state"]=="proof_available":
                    old=self.operations.get(op["request"]["original_board_operation_id"])
                    require(old is not None and old["result"] is not None and old["pool"]==op["pool"],"original board receipt absent or read switched wallet")
                    bind(op["request"],value,old["request"],old["result"])
                op["result"]=deepcopy(value);op["terminal"]=value["state"] in TERMINAL
        if tool is not None or ready is not None:raise Inconclusive("pool/tool archive ends with unresolved result correlation")

    def bind_component(self,data):
        pairs=[];pool=None
        if data["id"].startswith("board-"):
            pool=data["id"].removeprefix("board-")
            pairs=[(data["board_request"],data["confirmed_receipt"]),(data["proof_read_request"],data["proof_read_result"])]
        elif data["id"].startswith("float-"):
            pool=data["id"].removeprefix("float-")
            pairs=[(data["board_request"],data["board_result"])]
            snapshot=data["fresh_snapshot"];op=self.operations.get(snapshot["operation_id"])
            require(op is not None and op["request"]["action"]=="snapshot" and op["pool"]==pool
                    and canonical_json(op["result"])==canonical_json(snapshot),"float snapshot lacks independent sealed observation")
        for req,res in pairs:
            op=self.operations.get(req["operation_id"])
            require(op is not None and op["pool"]==pool and canonical_json(op["request"])==canonical_json(req)
                    and canonical_json(op["result"])==canonical_json(res),"component copies differ from sealed same-wallet request/result events")
