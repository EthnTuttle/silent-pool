"""Retained-proof byte/provenance tests; fixtures are NOT funded/crypto proof."""
import copy
import base64
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import ap5_retained_board as proof
from case_contract import canonical_json
from oracle import Inconclusive, Invalid, decode_transaction
from test_oracle import tx_bytes
from test_oracle import policy

BASE=None
OWNER="0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
SERVER="02c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"


def compact(value):return json.dumps(value,ensure_ascii=False,separators=(",",":"),allow_nan=False).encode()
def sha(value):return hashlib.sha256(value).hexdigest()


def fixture():
    request={"schema":proof.REQUEST_SCHEMA,"operation_id":"original-board","action":"board_float","net_target_sats":115000000,"max_fee_sats":5000000}
    anchor=tx_bytes(outputs=[(115000100,bytes.fromhex("5120"+OWNER[2:]))]);txid=decode_transaction(anchor)["txid"]
    board={"gross_board_sats":115000100,"net_board_sats":115000000,"board_fee_sats":100,"onchain_fee_sats":100,
           "actual_fee_sats":200,"expiry_height":500,"required_confirmations":3,"server_pubkey":SERVER,
           "vtxo_exit_delta":10,"min_board_sats":20000,"max_vtxo_sats":None,"vtxo_id":"11"*32+":0"}
    context={"schema":proof.RESULT_SCHEMA,"operation_id":"original-board","state":"confirmed","onchain_trusted_sats":120000000,
             "float_spendable_sats":0,"observed_at_unix_ms":2**53+7,"observed_tip_height":105,"error_code":None,"funding_txid":txid,"board":board}
    context_raw=compact(context)
    opaque=b"offline-envelope-binding-only-not-a-VTXO-proof"
    original={"schema":proof.PROOF_SCHEMA,"original_board_operation_id":"original-board","board_request":request,
        "board_request_sha256":sha(compact(request)),"original_vtxo_id":board["vtxo_id"],"original_full_vtxo_hex":opaque.hex(),
        "original_full_vtxo_sha256":sha(opaque),"vtxo_encoding":"ark-lib/ProtocolEncoding/c419159","raw_anchor_tx_hex":anchor.hex(),
        "anchor_txid":txid,"anchor_vout":0,"owner_pubkey":OWNER,"owner_policy_hex":"00"+OWNER,"owner_policy_kind":"pubkey",
        "server_pubkey":SERVER,"board":board,"checked_at_unix_ms":1000,"checked_tip_height":105,"funding_confirmations":3,
        "stock_checkpoint_ref":f"board.{txid}.0","terminal_receipt_context_schema":proof.CONTEXT_SCHEMA,
        "terminal_receipt_context_canonical_json_hex":context_raw.hex(),"terminal_receipt_context_sha256":sha(context_raw)}
    raw=compact(original)
    envelope={"schema":proof.ENVELOPE_SCHEMA,"proof_sha256":sha(raw),"canonical_json_hex":raw.hex(),"original":original,
        "lineage_state":"recorded_partial","lineage_complete":False,"round_imports":[{
            "schema":"spc.lab.ap5.pool-wallet-round-import/v1","stock_movement_id":12,"input_vtxo_ids":[board["vtxo_id"]],
            "output_vtxo_ids":["22"*32+":0"],"round_funding_txid":"33"*32,"offchain_fee_sats":13,"completed_at_unix_ms":2000}],
        "lineage_error_code":None}
    read={"schema":proof.REQUEST_SCHEMA,"operation_id":"read-original","action":"board_proof_v1","original_board_operation_id":"original-board"}
    reply={"schema":proof.RESULT_SCHEMA,"operation_id":"read-original","state":"proof_available","onchain_trusted_sats":None,
           "float_spendable_sats":None,"observed_at_unix_ms":None,"observed_tip_height":None,"error_code":None,
           "funding_txid":None,"board":None,"proof":envelope}
    confirmed={**copy.deepcopy(context),"retained_proof_sha256":envelope["proof_sha256"]}
    return read,reply,copy.deepcopy(request),confirmed


def repack(reply,confirmed=None):
    p=reply["proof"];raw=compact(p["original"])
    p.update(canonical_json_hex=raw.hex(),proof_sha256=sha(raw))
    if confirmed is not None:confirmed["retained_proof_sha256"]=p["proof_sha256"]


def unavailable(reply,state="proof_missing",code="proof_not_retained"):
    value={k:copy.deepcopy(v) for k,v in reply.items() if k!="proof"};value.update(state=state,error_code=code)
    return value


def events(root,request_results):
    records=[];executor={"path":"/unit/pool-wallet-executor","sha256":"44"*32}
    def add(label,value):
        seq=len(records)+1;row={"seq":seq,"file":f"{seq:06d}-{label}.json","sha256":sha(canonical_json(value)+b"\n")}
        records.append((label,copy.deepcopy(value),row));return row
    for req,results,pool in request_results:
        row=add("pool-wallet-request",req)
        argv=[executor["path"],"--config",str(root/"run/private"/("pool-ark-"+pool)/"pool.toml"),"--request",str(root/row["file"])]
        for result in results:
            add("tool-intent",{"executable":executor["path"],"executable_sha256":executor["sha256"],"argv_sha256":sha(canonical_json(argv))})
            # Independent literal fixture order matches the producer's serde stdout.
            add("tool-exit",{"exit":0,"timeout":False,"stdout_sha256":sha(compact(result)+b"\n")})
            add("pool-wallet-result",result)
    return records,executor


class RetainedBoardTests(unittest.TestCase):
    def test_canonical_original_and_terminal_projection_bind_without_current_balance_claim(self):
        read,reply,req,confirmed=fixture()
        original=proof.bind(read,reply,req,confirmed)
        self.assertEqual(original["original_vtxo_id"],confirmed["board"]["vtxo_id"])
        self.assertNotEqual(original["original_vtxo_id"],original["anchor_txid"]+":0")
        self.assertEqual(confirmed["observed_at_unix_ms"],2**53+7)
        self.assertIsNone(reply["float_spendable_sats"])
        self.assertNotIn("crypto_verified",original)

    def test_read_id_money_and_original_binding_are_closed(self):
        read,reply,req,confirmed=fixture()
        for key,value in (("operation_id",req["operation_id"]),("net_target_sats",1),("max_fee_sats",None)):
            with self.assertRaises(Invalid):proof.request({**read,key:value})
        with self.assertRaises(Invalid):proof.bind({**read,"original_board_operation_id":"another"},reply,req,confirmed)

    def test_historical_read_cannot_report_current_observation_or_effect(self):
        read,reply,_,_=fixture()
        for key in ("onchain_trusted_sats","float_spendable_sats","observed_at_unix_ms","observed_tip_height","funding_txid","board"):
            with self.subTest(key=key),self.assertRaises(Invalid):proof.result({**reply,key:0},read)

    def test_legacy_digest_missing_is_inconclusive_not_retained_proof(self):
        read,reply,req,confirmed=fixture();del confirmed["retained_proof_sha256"]
        proof.result(confirmed,req)
        with self.assertRaises(Inconclusive):proof.bind(read,reply,req,confirmed)
        with self.assertRaises(Invalid):proof.result({**unavailable(reply),"retained_proof_sha256":"11"*32},read)

    def test_unavailable_original_or_lineage_never_reconstructs_remote_proof(self):
        read,reply,req,confirmed=fixture()
        for state,code in (("proof_missing","proof_not_retained"),("proof_unavailable","proof_digest_mismatch")):
            result=unavailable(reply,state,code);proof.result(result,read)
            with self.assertRaises(Inconclusive):proof.bind(read,result,req,confirmed)
        reply["proof"].update(lineage_state="unavailable",round_imports=[],lineage_error_code="no_retained_round_imports")
        self.assertEqual(proof.bind(read,reply,req,confirmed)["original_board_operation_id"],req["operation_id"])

    def test_canonical_order_hash_tree_and_unknown_fields_are_checked(self):
        for kind in ("order","digest","tree","field"):
            read,reply,req,confirmed=fixture();p=reply["proof"]
            if kind=="order":
                raw=canonical_json(p["original"]);p.update(canonical_json_hex=raw.hex(),proof_sha256=sha(raw));confirmed["retained_proof_sha256"]=sha(raw)
            elif kind=="digest":p["proof_sha256"]="00"*32
            elif kind=="tree":p["original"]["checked_at_unix_ms"]+=1
            else:p["original"]["unexpected"]=0
            with self.subTest(kind=kind),self.assertRaises(Invalid):proof.bind(read,reply,req,confirmed)

    def test_context_omits_only_the_one_self_referential_digest(self):
        read,reply,req,confirmed=fixture()
        for key in ("onchain_trusted_sats","float_spendable_sats","observed_at_unix_ms","observed_tip_height"):
            altered={**confirmed,key:confirmed[key]+1}
            with self.assertRaisesRegex(Invalid,"ONLY"):proof.bind(read,reply,req,altered)
        for action in ("remove","cycle"):
            r=copy.deepcopy(reply);o=r["proof"]["original"]
            ctx=json.loads(bytes.fromhex(o["terminal_receipt_context_canonical_json_hex"]))
            if action=="remove":del ctx["observed_at_unix_ms"]
            else:ctx["retained_proof_sha256"]=r["proof"]["proof_sha256"]
            raw=compact(ctx);o.update(terminal_receipt_context_canonical_json_hex=raw.hex(),terminal_receipt_context_sha256=sha(raw));repack(r)
            with self.assertRaises(Invalid):proof.result(r,read)

    def test_full_owner_policy_anchor_and_exact_u64_facts_not_amount_matching(self):
        for key,value in (("owner_policy_hex","09"+OWNER),("server_pubkey",OWNER),("anchor_vout",1),
                          ("funding_confirmations",True),("stock_checkpoint_ref","board.other.0")):
            read,reply,_,_=fixture();reply["proof"]["original"][key]=value;repack(reply)
            with self.subTest(key=key),self.assertRaises(Invalid):proof.result(reply,read)
        read,reply,_,_=fixture();reply["proof"]["original"]["board"]["net_board_sats"]=115000000.0;repack(reply)
        with self.assertRaises(Invalid):proof.result(reply,read)

    def test_partial_lineage_rejects_complete_unrelated_conflicting_or_excessive_claims(self):
        for kind in ("complete","unrelated","duplicate","capacity","unsorted"):
            read,reply,_,_=fixture();p=reply["proof"]
            if kind=="complete":p["lineage_complete"]=True
            elif kind=="unrelated":p["round_imports"][0]["input_vtxo_ids"]=["99"*32+":0"]
            elif kind=="duplicate":p["round_imports"]*=2
            elif kind=="capacity":p["round_imports"]*=257
            else:p["round_imports"][0]["output_vtxo_ids"]=["44"*32+":0","22"*32+":0"]
            with self.subTest(kind=kind),self.assertRaises(Invalid):proof.result(reply,read)

    def test_sealed_events_bind_same_wallet_invocation_stdout_and_component_copies(self):
        read,reply,req,confirmed=fixture()
        with tempfile.TemporaryDirectory(prefix="t3-retained-events-",dir=BASE) as tmp:
            root=Path(tmp);records,exe=events(root,[(req,[confirmed],"pool-a"),(read,[reply],"pool-a")])
            parsed=proof.PoolEvents(records,root,exe)
            component={"id":"board-pool-a","board_request":req,"confirmed_receipt":confirmed,"proof_read_request":read,"proof_read_result":reply}
            parsed.bind_component(component)
            changed=copy.deepcopy(component);changed["confirmed_receipt"]["observed_tip_height"]+=1
            with self.assertRaises(Invalid):parsed.bind_component(changed)
            records,exe=events(root,[(req,[confirmed],"pool-a"),(read,[reply],"pool-b")])
            with self.assertRaisesRegex(Invalid,"switched wallet"):proof.PoolEvents(records,root,exe)

    def test_missing_request_tool_or_changed_stdout_cannot_support_read_result(self):
        read,reply,req,confirmed=fixture()
        with tempfile.TemporaryDirectory(prefix="t3-retained-source-",dir=BASE) as tmp:
            root=Path(tmp);records,exe=events(root,[(req,[confirmed],"pool-a"),(read,[reply],"pool-a")])
            for kind in ("request","tool","stdout"):
                altered=copy.deepcopy(records)
                if kind=="request":altered.pop(0)
                elif kind=="tool":altered=[r for r in altered if r[0] not in {"tool-intent","tool-exit"}]
                else:next(r[1] for r in reversed(altered) if r[0]=="tool-exit")["stdout_sha256"]="00"*32
                with self.subTest(kind=kind),self.assertRaises((Invalid,Inconclusive)):proof.PoolEvents(altered,root,exe)

    def test_read_terminal_is_immutable_and_retry_gets_new_read_not_board_id(self):
        read,reply,req,confirmed=fixture();queued=unavailable(reply,"queued",None);missing=unavailable(reply)
        with tempfile.TemporaryDirectory(prefix="t3-retained-retry-",dir=BASE) as tmp:
            root=Path(tmp);records,exe=events(root,[(req,[confirmed],"pool-a"),(read,[queued,queued,missing,reply],"pool-a")])
            with self.assertRaisesRegex(Invalid,"terminal"):proof.PoolEvents(records,root,exe)
            next_read={**read,"operation_id":"read-after-missing"};next_reply={**reply,"operation_id":next_read["operation_id"]}
            records,exe=events(root,[(req,[confirmed],"pool-a"),(read,[queued,missing],"pool-a"),(next_read,[next_reply],"pool-a")])
            self.assertEqual(proof.PoolEvents(records,root,exe).operations[next_read["operation_id"]]["request"]["original_board_operation_id"],req["operation_id"])

    def test_legacy_terminal_cannot_be_rewritten_to_add_retention(self):
        _,_,req,confirmed=fixture();legacy={k:v for k,v in confirmed.items() if k!="retained_proof_sha256"}
        with tempfile.TemporaryDirectory(prefix="t3-retained-legacy-",dir=BASE) as tmp:
            root=Path(tmp);records,exe=events(root,[(req,[legacy,confirmed],"pool-a")])
            with self.assertRaisesRegex(Invalid,"terminal"):proof.PoolEvents(records,root,exe)

    def test_historical_proof_read_cannot_substitute_for_current_float_snapshot(self):
        read,reply,req,confirmed=fixture()
        with tempfile.TemporaryDirectory(prefix="t3-retained-float-",dir=BASE) as tmp:
            root=Path(tmp);records,exe=events(root,[(req,[confirmed],"pool-a"),(read,[reply],"pool-a")])
            component={"id":"float-pool-a","board_request":req,"board_result":confirmed,"fresh_snapshot":reply}
            with self.assertRaisesRegex(Invalid,"snapshot"):proof.PoolEvents(records,root,exe).bind_component(component)

    def test_board_component_requires_exact_retained_bytes_and_owner_even_when_amount_matches(self):
        from ap5_v2 import replay_component
        read,reply,req,confirmed=fixture();original=reply["proof"]["original"]
        info={"network":"regtest","serverPubkey":base64.b64encode(bytes.fromhex(SERVER)).decode(),
              "mailboxPubkey":base64.b64encode(bytes.fromhex(OWNER)).decode(),"vtxoExitDelta":10,"maxVtxoExitDepth":10,
              "requiredBoardConfirmations":3,"minBoardAmount":"20000","fees":{"board":{"minFeeSat":"0","baseFeeSat":"100","ppm":"0"}}}
        anchor=[{"raw":original["raw_anchor_tx_hex"]},{"raw":original["raw_anchor_tx_hex"]}]
        data={"schema":"spc.lab.ap5.raw-component/v2","id":"board-pool-a","maturity":100,
              "ark_info":{"raw_response_hex":compact(info).hex(),"ark_info":info,
                          "bound":{"server_pubkey":SERVER,"mailbox_pubkey":OWNER,"fees_sha256":sha(canonical_json(info["fees"]))}},
              "transactions":[anchor],"vtxos":[{"raw_hex":original["original_full_vtxo_hex"],"anchor_views":copy.deepcopy(anchor),
                  "expected":{"user_pubkey":OWNER,"server_pubkey":SERVER,"amount_sats":115000000,"expiry_height":500,"exit_delta":10,"max_exit_depth":10}}],
              "board":copy.deepcopy(original["board"]),"board_request":req,"confirmed_receipt":confirmed,"proof_read_request":read,"proof_read_result":reply}
        # Raw Bitcoin validation has its own tests; isolate this new linkage.
        with patch("ap5_v2.raw_transaction",return_value={"txid":original["anchor_txid"],"height":103,"fee_sats":100}):
            self.assertEqual(replay_component(data,policy())["status"],"INCONCLUSIVE")
            for field in ("proof","anchor","owner"):
                altered=copy.deepcopy(data)
                if field=="proof":altered["vtxos"][0]["raw_hex"]="aabb"
                elif field=="anchor":altered["transactions"][0][0]["raw"]="00"
                else:altered["vtxos"][0]["expected"]["user_pubkey"]=SERVER
                with self.subTest(field=field),self.assertRaises(Invalid):replay_component(altered,policy())
            # Unit-only crypto metadata mock: the new join must compare both
            # exact original id and full anchor, not merely equal amounts.
            from ap5_verifier import CompiledVerifier
            verifier=object.__new__(CompiledVerifier)
            crypto={"id":original["original_vtxo_id"],"chain_anchor":original["anchor_txid"]+":0"}
            with patch.object(CompiledVerifier,"verify",return_value=crypto):
                self.assertFalse(replay_component(data,policy(),verifier)["raw_checks"]["full_VTXO_crypto_still_required"])
                for field,value in (("id","99"*32+":0"),("chain_anchor",original["anchor_txid"]+":1")):
                    with patch.object(CompiledVerifier,"verify",return_value={**crypto,field:value}),self.assertRaises(Invalid):
                        replay_component(data,policy(),verifier)


if __name__=="__main__":unittest.main()
