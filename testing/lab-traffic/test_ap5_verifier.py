"""Consumer framing/linkage units with mocked crypto; actual CLI checked separately."""
import base64
import copy
import hashlib
import json
from pathlib import Path
import subprocess
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import ap5_v2
from ap5_verifier import CompiledVerifier, BINARY_SHA256, ARK_REVISION, validate_request, validate_reply
from base_network import reject_known_host_network_failure
from runtime_bootstrap import FIXED_CLI_SHA256
from case_contract import canonical_json
from oracle import Inconclusive, Invalid, address_script, decode_transaction
from test_oracle import tx_bytes, policy
from test_source_feed import pub

OWNER="02"+pub(3);SERVER="02"+pub(7)


def data_fixture(name="receive-alice",amounts=(10000,)):
    address="mipcBbFg9gMiCh81Kj8tqqdgoZub1ZJRfn";script=address_script(address)
    raw=tx_bytes(outputs=[(9000,script)]).hex();txid=decode_transaction(bytes.fromhex(raw))["txid"]
    anchor={"txid":txid,"wtxid":txid,"height":100,"block_hash":"33"*32,"vsize":100,"fee_sats":100}
    info={"network":"regtest","serverPubkey":base64.b64encode(bytes.fromhex(SERVER)).decode(),
          "mailboxPubkey":base64.b64encode(bytes.fromhex(OWNER)).decode(),"vtxoExitDelta":2016,"maxVtxoExitDepth":100,"fees":{}}
    data={"schema":"spc.lab.ap5.raw-component/v2","id":name,"maturity":100,
        "ark_info":{"raw_response_hex":canonical_json(info).hex(),"ark_info":info,
            "bound":{"server_pubkey":SERVER,"mailbox_pubkey":OWNER,"fees_sha256":hashlib.sha256(b"{}").hexdigest()}},"vtxos":[]}
    replies={}
    for index,amount in enumerate(amounts,1):
        opaque=bytes([index]).hex();expected={"user_pubkey":OWNER,"server_pubkey":SERVER,"amount_sats":amount,"expiry_height":101010,"exit_delta":2016,"max_exit_depth":100}
        data["vtxos"].append({"raw_hex":opaque,"anchor_views":[{"raw":raw},{"raw":raw}],"expected":expected})
        replies[opaque]={"schema":"spc.lab.ap5.vtxo-proof/v1","verified":True,"validator":"stock-ark-lib-full-chain-and-signatures","ark_revision":ARK_REVISION,
            "raw_sha256":hashlib.sha256(bytes.fromhex(opaque)).hexdigest(),"anchor_raw_sha256":hashlib.sha256(bytes.fromhex(raw)).hexdigest(),
            "id":f"{index:064x}:0","chain_anchor":txid+":0","policy_kind":"pubkey",**{k:v for k,v in expected.items() if k!="max_exit_depth"},"exit_depth":1}
    return data,replies,anchor,address,script


def mocked_verifier():
    # Deliberately a unit-only method mock; this is not an executable admission.
    verifier=object.__new__(CompiledVerifier);verifier.path=Path("/unit/never-executed");verifier.executions=[]
    return verifier


class AP5VerifierTests(unittest.TestCase):
    def test_unadmitted_artifact_is_rejected_before_any_process_or_source_access(self):
        with patch("ap5_verifier.subprocess.run") as run:
            with self.assertRaises(Invalid):CompiledVerifier({"path":"/unit/foreign","sha256":BINARY_SHA256,"compiled":True})
            run.assert_not_called()

    def test_request_reply_closed_types_input_hashes_and_full_policy(self):
        data,replies,_,_,_=data_fixture();p=data["vtxos"][0];reply=replies[p["raw_hex"]]
        validate_request(p["raw_hex"],p["anchor_views"][0]["raw"],p["expected"])
        validate_reply(reply,p["raw_hex"],p["anchor_views"][0]["raw"],p["expected"])
        for key,value in (("verified",1),("policy_kind","htlc"),("raw_sha256","00"*32),("anchor_raw_sha256","00"*32),
                          ("amount_sats",10000.0),("exit_depth",101),("id","01:0"),("unexpected",True)):
            with self.subTest(key=key),self.assertRaises((Invalid,Inconclusive)):
                validate_reply({**reply,key:value},p["raw_hex"],p["anchor_views"][0]["raw"],p["expected"])
        with self.assertRaises(Invalid):validate_request(p["raw_hex"],p["anchor_views"][0]["raw"],{**p["expected"],"exit_delta":65536})

    def test_compiled_call_framing_failure_timeout_and_receipt_are_fail_closed(self):
        data,replies,_,_,_=data_fixture();p=data["vtxos"][0];v=mocked_verifier();reply=canonical_json(replies[p["raw_hex"]])+b"\n"
        with patch("ap5_verifier.sha_file",return_value=BINARY_SHA256),patch("ap5_verifier.subprocess.run") as run:
            run.return_value=SimpleNamespace(returncode=0,stdout=reply,stderr=b"")
            self.assertEqual(v.verify(p["raw_hex"],p["anchor_views"][0]["raw"],p["expected"])["id"],replies[p["raw_hex"]]["id"])
            self.assertEqual(run.call_args.args[0],[str(v.path)])
            self.assertEqual(v.executions[-1]["status"],"VERIFIED_PUBLIC_PROOF")
            for response in (SimpleNamespace(returncode=1,stdout=b"",stderr=b"rejected"),SimpleNamespace(returncode=0,stdout=reply+reply,stderr=b"")):
                run.return_value=response
                with self.assertRaises(Invalid):v.verify(p["raw_hex"],p["anchor_views"][0]["raw"],p["expected"])
            run.side_effect=subprocess.TimeoutExpired([str(v.path)],30)
            with self.assertRaises(Inconclusive):v.verify(p["raw_hex"],p["anchor_views"][0]["raw"],p["expected"])

    def test_receive_compiled_identity_not_equal_amount_or_sent_label(self):
        data,replies,anchor,_,_=data_fixture();data["payout_attribution"]=[{"status":"sent","vtxo_id":ap5_v2.stock_bytes(next(iter(replies.values()))["id"])}]
        with patch("ap5_v2.raw_transaction",return_value=anchor),patch.object(CompiledVerifier,"verify",side_effect=lambda raw,*args:replies[raw]):
            result=ap5_v2.replay_component(data,policy(),mocked_verifier())
            self.assertEqual(result["raw_checks"]["full_vtxo_crypto_verified"],1)
            self.assertEqual(result["missing_components"],["same_AP5_run_raw_canonical_ancestry_and_observation_binding"])
            self.assertEqual(result["status"],"INCONCLUSIVE")
            data["payout_attribution"][0]["vtxo_id"]="aa"*36
            with self.assertRaisesRegex(Invalid,"identities"):ap5_v2.replay_component(data,policy(),mocked_verifier())

    def test_current_float_rows_must_match_verified_id_value_and_expiry(self):
        data,replies,anchor,_,_=data_fixture("float-pool-a",(115000000,));v=next(iter(replies.values()))
        data.update(target_sats=115000000,already_at_target=True,board_request={"operation_id":"board","action":"board_float","net_target_sats":115000000},
            board_result={"operation_id":"board","state":"already_at_target","funding_txid":None,"board":None},
            fresh_snapshot={"operation_id":"fresh","state":"observed","float_spendable_sats":115000000,"observed_tip_height":100},
            float_rows=[{"id":v["id"],"state":"Spendable","amount_sats":115000000,"expiry_height":101010}])
        with patch("ap5_v2.raw_transaction",return_value=anchor),patch.object(CompiledVerifier,"verify",side_effect=lambda raw,*args:replies[raw]):
            self.assertEqual(ap5_v2.replay_component(data,policy(),mocked_verifier())["raw_checks"]["unverified_float_rows"],0)
            for field,value in (("id","22"*32+":0"),("amount_sats",115000001),("expiry_height",101011)):
                changed=copy.deepcopy(data);changed["float_rows"][0][field]=value
                with self.assertRaises(Invalid):ap5_v2.replay_component(changed,policy(),mocked_verifier())

    def test_round_and_offboard_values_are_joined_to_compiled_proof_identities(self):
        data,replies,anchor,_,_=data_fixture("sp-refresh-alice",(10000,9000));values=list(replies.values())
        inputs=[ap5_v2.stock_bytes(values[0]["id"])];outputs=[values[1]["id"]]
        cp={"version":2,"input_ids":[list(bytes.fromhex(inputs[0]))],"outputs":[{}],"unlock_hash":[3]*32,
            "funding":list(bytes.fromhex(data["vtxos"][0]["anchor_views"][0]["raw"])),"locked":[[1]],"released":[[2]]}
        cp["proof_binding"]=list(hashlib.sha256(json.dumps([cp[k] for k in ("version","input_ids","outputs","unlock_hash","funding","locked")],separators=(",",":")).encode()).digest())
        data["round"]={"checkpoint":cp,"checkpoint_raw":json.dumps(cp,separators=(",",":")),"inputs":inputs,"outputs":outputs,"funding_txid":anchor["txid"],
            "value":{"input_sats":10000,"output_sats":9000,"user_paid_fee_sats":1000}}
        with patch("ap5_v2.raw_transaction",return_value=anchor),patch.object(CompiledVerifier,"verify",side_effect=lambda raw,*args:replies[raw]):
            ap5_v2.replay_component(data,policy(),mocked_verifier())
            data["round"]["value"]["user_paid_fee_sats"]=0
            with self.assertRaises(Invalid):ap5_v2.replay_component(data,policy(),mocked_verifier())
        data,replies,anchor,address,script=data_fixture("stock-offboard-alice");v=next(iter(replies.values()));raw=data["vtxos"][0]["anchor_views"][0]["raw"]
        op={"id":"offboard","payload":json.dumps({"operation_id":"offboard","action":{"kind":"offboard","address":address},"vtxo_ids":[ap5_v2.stock_bytes(v["id"])]}),
            "result":"offboard_broadcast:"+anchor["txid"]}
        data.update(transactions=[[{"raw":raw},{"raw":raw}]],offboard={"operation":op,"inputs":[v["id"]],"destination_address":address,"destination_script":script.hex(),
            "value":{**anchor,"ark_input_sats":10000,"onchain_received_sats":9000,"user_paid_fee_sats":1000,"destination_script":script.hex()}})
        with patch("ap5_v2.raw_transaction",return_value=anchor),patch.object(CompiledVerifier,"verify",side_effect=lambda raw,*args:replies[raw]):
            ap5_v2.replay_component(data,policy(),mocked_verifier())
            data["offboard"]["inputs"]=["ff"*32+":0"]
            with self.assertRaises(Invalid):ap5_v2.replay_component(data,policy(),mocked_verifier())

    def test_phase_fix_admission_does_not_authorize_repeating_known_failed_host_network(self):
        with self.assertRaisesRegex(Inconclusive,"host_P2P"):reject_known_host_network_failure({"sha256":FIXED_CLI_SHA256})

    def test_automatic_renewal_previous_expiry_is_bound_to_compiled_input_proofs(self):
        data,replies,anchor,_,_=data_fixture("second-renewal-alice",(10000,9000));values=list(replies.values())
        values[1]["expiry_height"]=101020;data["vtxos"][1]["expected"]["expiry_height"]=101020
        stock=ap5_v2.stock_bytes(values[0]["id"]);height=100
        operation=hashlib.sha256(b"SPc/StockAutoRefresh/v1"+bytes.fromhex(stock)+height.to_bytes(4,"big")).hexdigest()
        payload={"operation_id":operation,"action":{"kind":"refresh"},"vtxo_ids":[stock]}
        data["renewal"]={"automatic":True,"operations":[{"id":operation,"payload":json.dumps(payload),"verified_auto_trigger_height":height,
            "stage":"stock_submitted","result":"round_submitted:"+anchor["txid"]}],"manual_operation_ids":[],
            "inputs":[stock],"outputs":[values[1]["id"]],"round_ids":[anchor["txid"]],"previous_expiry":101010}
        with patch("ap5_v2.raw_transaction",return_value=anchor),patch.object(CompiledVerifier,"verify",side_effect=lambda raw,*args:replies[raw]):
            ap5_v2.replay_component(data,policy(),mocked_verifier())
            data["renewal"]["previous_expiry"]=1
            with self.assertRaises(Invalid):ap5_v2.replay_component(data,policy(),mocked_verifier())


if __name__=="__main__":unittest.main()
