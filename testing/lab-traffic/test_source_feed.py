"""Real Unix framing/ACK durability, with explicitly offline attestation keys.

The protocol-only provider below is never selected by a production entrypoint
and does not represent a verified payout bundle or native funded acceptance.
"""
import copy
import hashlib
import json
import os
from pathlib import Path
import socket
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

from oracle import Inconclusive, Invalid
from signature_oracle import _multiply
from public_derivation import _GX,_GY
from source_feed_codec import (SCHEMA,CONTRACT,MAX_FRAME,CollectorKey,received_digest,trace_digest,
    checkpoint_digest,consumed_digest,committed_digest,verify_signature,_sign_digest,encode_frame,receive,send,bundle_window,window_facts,
    verify_public_evidence,validate_window,tagged)
from source_feed_provider import VerifiedEvidence,ReplayProvider,_REPLAY_TOKEN
from source_feed_server import FeedServer,Journal

BASE=None


def pub(secret):return _multiply(secret,(_GX,_GY))[0].to_bytes(32,"big").hex()


def job():
    return {"job_id":7,"prev_hash":[1]*32,"version":2,"nbits":0x207fffff,"ntime":1600000000,
        "coinbase_prefix":[2,3],"coinbase_suffix":[4,5],"merkle_path":[[6]*32],"share_target_be":[7]*32,
        "pool_extranonce":[8]*4,"miner_extranonce_len":4}


def protocol_only_provider():
    # Explicit unittest mock of the replay boundary, not a server option or
    # source evidence accepted by the production ReplayProvider constructor.
    provider=object.__new__(ReplayProvider)
    def prepare(agent,received):
        seal={"session":[1]*16,"seq":9,"digest":[2]*32,"signature":""}
        seal["signature"]=_sign_digest(7,trace_digest(seal))
        bundle=json.dumps({"schema":"spc.payout-proof/1","offline_protocol_vector":True,"window_target":"9",
                           "slices":[{"difficulty":"9"}],"window":{"facts":window_facts(9,9),"eligible_history_complete":True}}).encode()
        return VerifiedEvidence(bundle,seal,(1<<32)|7,4,10,bytes([8])*32,_REPLAY_TOKEN)
    provider.prepare=prepare
    return provider


class SourceFeedTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(prefix="t3-feed-vectors-",dir=BASE);self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name);self.root.chmod(0o700)
        key=self.root/"collector-key";key.write_text(f"{8:064x}");key.chmod(0o600)
        self.signer=CollectorKey(key,pub(8));self.clock=1000
        from storage import ProcFS
        self.agents={"alice":{"scan_pubkey":pub(9),"pool_pubkey":pub(7),"pid":os.getpid(),
                               "start_time_ticks":ProcFS().identity(os.getpid())["start_time_ticks"]}}
        self.server=FeedServer(self.root,self.signer,self.agents,protocol_only_provider(),now_ms=lambda:self.clock)
        self.addCleanup(lambda:self.server.journal.close())

    def request(self,number=1):return {"kind":"job","schema":SCHEMA,"attempt":[number]*16,"job":job()}

    def ack(self,proof):
        cp=proof["checkpoint"]
        ack={k:copy.deepcopy(cp[k]) for k in ("attempt","job_id","generation","bundle_digest","history_digest")}
        ack.update(checkpoint_digest=list(checkpoint_digest(cp)),window=bundle_window(bytes.fromhex(proof["bundle_hex"])),signature="")
        ack["signature"]=_sign_digest(9,consumed_digest(ack))
        return {"kind":"consumed","schema":SCHEMA,"ack":ack}

    def test_published_BIP340_vector_and_independent_authority_keys(self):
        signature=_sign_digest(3,bytes(32))
        self.assertEqual(signature,"e907831f80848d1069a5371b402410364bdf1c5f8307b0084c55f1ce2dca8215"+
                         "25f66a4a85ea8b71e482a74f382d2ce5ebeee8fdb2172f477df4900d310536c0")
        verify_signature(bytes(32),signature,pub(3))
        with self.assertRaises(Invalid):verify_signature(bytes(32),signature,pub(8))

    def test_received_digest_binds_every_actual_job_and_extranonce_field(self):
        original=job();baseline=received_digest(original)
        for name in original:
            altered=copy.deepcopy(original)
            if isinstance(altered[name],int):altered[name]+=1
            elif name=="merkle_path":altered[name][0][0]^=1
            else:altered[name][0]^=1
            self.assertNotEqual(received_digest(altered),baseline,name)
        with self.assertRaises(Inconclusive):received_digest({**original,"unknown":True})

    def test_actual_unix_exchange_commits_only_after_durable_scan_signed_ACK(self):
        left,right=socket.socketpair();result=[]
        worker=threading.Thread(target=lambda:result.append(self.server.exchange(right,"alice")))
        worker.start()
        with left:
            send(left,self.request(),time.monotonic()+10)
            proof=receive(left,time.monotonic()+10)
            self.assertEqual(proof["kind"],"proof")
            self.assertNotIn("consumed",[r["kind"] for r in self.server.journal.rows])
            verify_signature(checkpoint_digest(proof["checkpoint"]),proof["checkpoint"]["signature"],pub(8))
            ack=self.ack(proof);send(left,ack,time.monotonic()+10)
            committed=receive(left,time.monotonic()+10)
            verify_signature(committed_digest(committed["attempt"],committed["ack_digest"]),committed["signature"],pub(8))
            self.assertEqual(self.server.journal.rows[-2]["kind"],"consumed")
            self.assertEqual(self.server.journal.rows[-1]["kind"],"committed")
        worker.join(10);self.assertFalse(worker.is_alive())
        self.assertEqual(result[0]["status"],"COMMITTED_CONSUMPTION_ONLY")

    def test_replay_is_exact_and_restart_does_not_extend_expiry(self):
        request=self.request();proof=self.server.proof("alice",request);ack=self.ack(proof)
        committed=self.server.consume("alice",ack);count=len(self.server.journal.rows)
        self.assertEqual(self.server.proof("alice",request),proof)
        self.assertEqual(self.server.consume("alice",ack),committed)
        self.assertEqual(len(self.server.journal.rows),count)
        self.server.journal.close()
        self.server=FeedServer(self.root,self.signer,self.agents,protocol_only_provider(),now_ms=lambda:self.clock)
        self.assertEqual(self.server.proof("alice",request),proof)
        self.clock=proof["checkpoint"]["expires_at_unix_ms"]
        with self.assertRaises(Inconclusive):self.server.proof("alice",request)

    def test_wrong_scan_node_job_or_superseded_attempt_never_commits(self):
        request=self.request();proof=self.server.proof("alice",request)
        changed=copy.deepcopy(request);changed["job"]["ntime"]+=1
        with self.assertRaises(Invalid):self.server.proof("alice",changed)
        ack=self.ack(proof);ack["ack"]["signature"]=_sign_digest(7,consumed_digest(ack["ack"]))
        with self.assertRaises(Invalid):self.server.consume("alice",ack)
        ack=self.ack(proof);ack["ack"]["generation"]+=1
        with self.assertRaises(Invalid):self.server.consume("alice",ack)
        self.server.proof("alice",self.request(2))
        with self.assertRaisesRegex(Invalid,"superseded"):self.server.consume("alice",self.ack(proof))
        self.assertNotIn("committed",[r["kind"] for r in self.server.journal.rows])

    def test_fsync_failure_cannot_generate_committed_signature(self):
        proof=self.server.proof("alice",self.request());ack=self.ack(proof)
        original=self.server.journal.append
        def append(kind,value):
            if kind=="consumed":raise OSError("offline simulated durable write failure")
            return original(kind,value)
        with patch.object(self.server.journal,"append",append), patch.object(self.signer,"committed") as sign:
            with self.assertRaises(OSError):self.server.consume("alice",ack)
            sign.assert_not_called()

    def test_actual_journal_fsync_failure_poison_blocks_later_replay_grants(self):
        request=self.request();proof=self.server.proof("alice",request);ack=self.ack(proof)
        calls=[];real=os.fsync
        def fsync(fd):
            calls.append(fd)
            if len(calls)==3:raise OSError("offline consumed-journal fsync failure")
            return real(fd)
        with patch("source_feed_server.os.fsync",fsync),patch.object(self.signer,"committed") as sign:
            with self.assertRaises(OSError):self.server.consume("alice",ack)
            sign.assert_not_called()
        self.assertTrue(self.server.journal.poisoned)
        with self.assertRaisesRegex(Inconclusive,"poisoned"):self.server.proof("alice",request)

    def test_oversize_truncated_and_unavailable_frames_are_not_ACKs(self):
        utf16=json.dumps(self.request()).encode("utf-16")
        for raw in ((MAX_FRAME+1).to_bytes(4,"little"), (100).to_bytes(4,"little")+b"{}",len(utf16).to_bytes(4,"little")+utf16):
            left,right=socket.socketpair()
            with left,right:
                right.sendall(raw);right.shutdown(socket.SHUT_WR)
                with self.assertRaises(Inconclusive):receive(left,time.monotonic()+1)
        with self.assertRaises(Invalid):self.server.consume("alice",{"kind":"unavailable","schema":SCHEMA,"attempt":[1]*16})

    def test_opaque_bundle_or_failed_replay_cannot_create_a_proof(self):
        with patch.object(self.server.provider,"prepare",return_value={"complete":True,"bundle_hex":"00"}):
            with self.assertRaises(Invalid):self.server.proof("alice",self.request())
        self.assertNotIn("prepared",[r["kind"] for r in self.server.journal.rows])
        with self.assertRaises(Inconclusive):self.server.proof("alice",self.request())

    def test_pool_key_is_pinned_outside_the_response_and_provider_deadline_is_bounded(self):
        wrong={"alice":{**self.agents["alice"],"pool_pubkey":self.agents["alice"]["scan_pubkey"]}}
        with self.assertRaisesRegex(Invalid,"independently pinned"):
            FeedServer(self.root/"wrong-authority",self.signer,wrong,self.server.provider)
        self.assertFalse((self.root/"wrong-authority").exists())
        self.server.agents["alice"]["pool_pubkey"]=pub(10)
        with patch.object(self.signer,"checkpoint") as sign:
            with self.assertRaisesRegex(Invalid,"signature invalid"):
                self.server.proof("alice",self.request())
            sign.assert_not_called()
        with self.assertRaisesRegex(Inconclusive,"deadline"):
            self.server.proof("alice",self.request(2),deadline=time.monotonic()-1)

    def test_signed_trace_requires_complete_independently_captured_prefix(self):
        from pool_source import PoolSource,trace_digest as event_digest
        from test_source_jobs import fixture
        events,_,_=fixture();lines=[];previous="00"*32
        for seq,(kind,payload) in enumerate(events,1):
            if kind=="distribution_frozen":payload.update(finding_slice_id="1",finding_slice_excluded=True)
            source={"schema":"spc.source-evidence/2","session":"01"*16,"seq":str(seq),"previous":previous,"complete_prefix_required":True}
            source["digest"]=event_digest(kind,payload,source);previous=source["digest"]
            seal={"session":[1]*16,"seq":seq,"digest":list(bytes.fromhex(previous)),"signature":""}
            seal["signature"]=_sign_digest(7,trace_digest(seal));source["pool_seal"]=seal
            lines.append(('INFO spc_evidence: event="'+kind+'" evidence='+json.dumps({**payload,"source":source})+'\n').encode())
        raw=b"".join(lines)
        parsed=PoolSource(raw,independently_captured_log_sha256=hashlib.sha256(raw).hexdigest(),source_contract=CONTRACT,pool_pubkey=pub(7))
        self.assertEqual(len(parsed.events),len(events))
        tail=b"".join(lines[1:])
        with self.assertRaises(Inconclusive):
            PoolSource(tail,independently_captured_log_sha256=hashlib.sha256(tail).hexdigest(),source_contract=CONTRACT,pool_pubkey=pub(7))
        with self.assertRaises(Invalid):
            PoolSource(raw,independently_captured_log_sha256=hashlib.sha256(raw).hexdigest(),source_contract=CONTRACT,pool_pubkey=pub(10))

    def test_v1_activation_frames_and_old_signature_tag_have_no_fallback(self):
        request=self.request();request["schema"]="spc.source-feed/1"
        with self.assertRaises(Invalid):self.server.proof("alice",request)
        proof=self.server.proof("alice",self.request());ack=self.ack(proof)
        old=copy.deepcopy(ack);old["schema"]="spc.source-feed/1"
        with self.assertRaises(Invalid):self.server.consume("alice",old)
        a=ack["ack"]
        raw=bytes(a["attempt"])+a["job_id"].to_bytes(8,"little")+a["generation"].to_bytes(8,"little")
        raw+=b"".join(bytes(a[k]) for k in ("bundle_digest","history_digest","checkpoint_digest"))
        a["signature"]=_sign_digest(9,tagged("SPc/SourceConsumedV1",raw))
        with self.assertRaises(Invalid):self.server.consume("alice",ack)
        del ack["ack"]["window"]
        with self.assertRaises(Inconclusive):self.server.consume("alice",ack)

    def test_WindowFacts_are_canonical_u128_strings_in_the_binary_ACK_preimage(self):
        proof=self.server.proof("alice",self.request());ack=self.ack(proof)["ack"]
        q=2**100+7;actual=2**80+3;ack["window"]=window_facts(q,actual)
        raw=bytes(ack["attempt"])+ack["job_id"].to_bytes(8,"little")+ack["generation"].to_bytes(8,"little")
        raw+=b"".join(bytes(ack[k]) for k in ("bundle_digest","history_digest","checkpoint_digest"))
        raw+=q.to_bytes(16,"little")+actual.to_bytes(16,"little")+(q-actual).to_bytes(16,"little")+b"\1"
        tag=hashlib.sha256(b"SPc/SourceConsumedV2").digest()
        self.assertEqual(consumed_digest(ack),hashlib.sha256(tag+tag+raw).digest())
        for field,value in (("actual_sum",actual),("window_target","09"),("shortfall","0"),("underfilled",False)):
            changed=copy.deepcopy(ack["window"]);changed[field]=value
            with self.assertRaises(Invalid):validate_window(changed)

    def test_consistent_but_false_window_claim_cannot_get_collector_commit(self):
        original=self.server.provider.prepare
        def partial(agent,received):
            evidence=original(agent,received)
            body=json.loads(evidence.bundle);body["slices"]=[{"difficulty":"1"}]
            body["window"]={"facts":window_facts(9,1),"eligible_history_complete":True}
            return VerifiedEvidence(json.dumps(body).encode(),evidence.pool,evidence.job_id,evidence.generation,
                                    evidence.history_watermark,evidence.history_digest,_REPLAY_TOKEN)
        self.server.provider.prepare=partial
        proof=self.server.proof("alice",self.request());ack=self.ack(proof)
        ack["ack"]["window"]=window_facts(9,9)
        ack["ack"]["signature"]=_sign_digest(9,consumed_digest(ack["ack"]))
        with self.assertRaisesRegex(Invalid,"window differs"):self.server.consume("alice",ack)
        self.assertNotIn("committed",[r["kind"] for r in self.server.journal.rows])

    def test_public_v2_evidence_checks_all_signatures_attempt_and_signed_window(self):
        request=self.request();proof=self.server.proof("alice",request);ack=self.ack(proof)
        committed=self.server.consume("alice",ack)
        evidence={"schema":SCHEMA,"checkpoint":proof["checkpoint"],"consumed":ack["ack"],
                  "committed":{k:committed[k] for k in ("attempt","ack_digest","signature")}}
        self.assertEqual(verify_public_evidence(evidence,request["attempt"],received_digest(request["job"]),pub(7),pub(8),pub(9),1000),window_facts(9,9))
        changed=copy.deepcopy(evidence);changed["consumed"]["window"]=window_facts(9,1)
        with self.assertRaises(Invalid):verify_public_evidence(changed,request["attempt"],received_digest(request["job"]),pub(7),pub(8),pub(9),1000)
        with self.assertRaises(Invalid):verify_public_evidence(evidence,[2]*16,received_digest(request["job"]),pub(7),pub(8),pub(9),1000)

    def test_legacy_full_bundle_can_omit_annotation_but_partial_or_false_complete_cannot(self):
        body={"schema":"spc.payout-proof/1","window_target":"9","slices":[{"difficulty":"9"}]}
        self.assertEqual(bundle_window(json.dumps(body).encode()),window_facts(9,9))
        body["slices"]=[]
        with self.assertRaises(Inconclusive):bundle_window(json.dumps(body).encode())
        body["window"]={"facts":window_facts(9,0),"eligible_history_complete":False}
        with self.assertRaises(Invalid):bundle_window(json.dumps(body).encode())


if __name__ == "__main__":unittest.main()
