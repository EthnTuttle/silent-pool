import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from oracle import Chain, Invalid, compact_size, decode_transaction, merkle, sha256d
from source_jobs import MissingComponent, branch, verify_job, verify_publication, witness_republication
from source_capture import SourceCapture
from test_oracle import block_bytes, coinbase, policy, tx_bytes
from test_pool_source import encode, parse

BASE = None
AX = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"


def fixture():
    genesis = block_bytes(bytes(32), [coinbase(10000000, tag=b"\1\0")])
    one = coinbase(10000000, tag=b"\x51\0")
    first = block_bytes(sha256d(genesis[:80]), [one], 1600000001)
    second = block_bytes(sha256d(first[:80]), [coinbase(10000000, tag=b"\x52\0")], 1600000002)
    p = policy(genesis=sha256d(genesis[:80])[::-1].hex(), regtest_pow_bits_overlay=None,
               initial_target_bits=0x207fffff, subsidy_initial_sats=10000000, coinbase_maturity=1)
    rawtx = tx_bytes([(decode_transaction(one)["txid_bytes"], 0, b"", 0xffffffff)], [(9999000, b"\x51")], [[b"offline-witness"]])
    tx = decode_transaction(rawtx)
    reserved = bytes(32)
    wr = merkle([bytes(32), tx["wtxid_bytes"]])
    commitment = sha256d(wr + reserved)
    tag = b"\x53\x20" + bytes.fromhex(AX) + bytes(2)
    cb = coinbase(outputs=[(10001000, b"\x51"), (0, bytes.fromhex("6a24aa21a9ed") + commitment)], tag=tag)
    split = 42 + len(tag) - 2
    base = {"front_id": 0, "job_id": "00000001", "generation": "1", "height": 3, "version": 1,
            "ntime": 1600000003, "prev_hash": sha256d(second[:80]).hex(), "nbits": 0x207fffff,
            "extranonce_len": 2, "a_send_x": AX, "fingerprint": "44" * 32, "entries": [],
            "share_target": "7f" + "ff" * 31, "share_target_byte_order": "big-endian unsigned 256-bit", "m_issue": 1,
            "work_unit_version": 2, "work_unit_fingerprint": "22" * 32, "fee_bps": 100, "subsidy": "10000000",
            "window_target": "2", "window_rule": 1, "window_underfilled": True, "slices": [], "high_water": {"0": "0"}}
    oldcb = coinbase(10000000, tag=tag)
    old = {**base, "coinbase_prefix": oldcb[:split].hex(), "coinbase_suffix": oldcb[split+2:].hex(),
           "merkle_branch": [], "transactions": [], "fees": "0", "witness_plan": None, "superseded_job_id": None}
    new = {**base, "job_id": "00000002", "generation": "2", "fingerprint": "45" * 32, "superseded_job_id": "00000001",
           "coinbase_prefix": cb[:split].hex(), "coinbase_suffix": cb[split+2:].hex(), "transactions": [rawtx.hex()],
           "merkle_branch": [h.hex() for h in branch([tx])], "fees": "1000",
           "witness_plan": {"reserved": reserved.hex(), "wtxid_root": wr.hex(), "commitment": commitment.hex()}}
    minedcb = coinbase(outputs=[(10001000, b"\x51"), (0, bytes.fromhex("6a24aa21a9ed") + commitment)], tag=tag, witness=[[reserved]])
    mined = block_bytes(sha256d(second[:80]), [minedcb, rawtx], 1600000003)
    chain = Chain(p, [genesis, first, second, mined])
    events = [("distribution_frozen", old), ("job_issued", {**old, "channel_id": 7}),
              ("distribution_frozen", new), ("job_issued", {**new, "channel_id": 7}),
              ("share_rejected", {"front_id": 0, "channel_id": 7, "job_id": old["job_id"], "active_job_id": new["job_id"],
                                  "generation": "2", "reason": "stale-share", "nonce": 0, "ntime": 1600000003, "share_index": 0})]
    events.append(("accounted", {"front_id": 0, "channel_id": 7, "job_id": new["job_id"], "generation": "2", "slice_id": "1",
        "share_hash": sha256d(mined[:80]).hex(), "version": 1, "ntime": 1600000003,
        "nonce": int.from_bytes(mined[76:80], "little"), "extranonce": "0000", "coinbase": cb.hex()}))
    rawwire = (b"SPCB\2" + sha256d(mined[:80]) + (3).to_bytes(4, "little") + b"\0" + bytes.fromhex(new["fingerprint"])
               + bytes.fromhex(AX) + b"\1\0\0\1" + b"0" + (2).to_bytes(16, "little") + b"\0\0\2\x20"
               + bytes.fromhex("22" * 32) + b"\1")
    events.append(("source_publication", {"stream": "spc:blocks", "stream_id": "100-0", "publication_complete": True,
        "raw_d_hex": rawwire.hex(), "front_id": 0, "block_hash": sha256d(mined[:80]).hex(),
        "coinbase_fingerprint": new["fingerprint"], "sql_snapshot_key": hashlib.sha256(b"SPc/DistributionSnapshotV2" + rawwire).hexdigest()}))
    return events, chain, p


class SourceJobTests(unittest.TestCase):
    def test_identical_coinbase_fingerprints_need_exact_SPCB3_frozen_economics(self):
        from types import SimpleNamespace
        from source_jobs import publication_job
        first=("session",0,"00000001",1);second=("session",0,"00000002",2)
        def job(key,revision):
            return {"payload":{"fingerprint":"11"*32,"height":12,"economic":{"job_id":(12<<32)|int(key[2],16),"generation":key[3],"revision":revision}}}
        source=SimpleNamespace(jobs={first:job(first,25),second:job(second,26)})
        wire={"coinbase_fingerprint":"11"*32,"height":12,"front_id":0,"economic":source.jobs[first]["payload"]["economic"]}
        self.assertEqual(publication_job(source,wire)[0],first)
        with self.assertRaisesRegex(Invalid,"ambiguous"):publication_job(source,{**wire,"economic":None})
        with self.assertRaisesRegex(Invalid,"ambiguous"):publication_job(source,{**wire,"economic":{**wire["economic"],"revision":27}})

    def test_actual_jobproof_fields_raw_fees_witness_and_SPCB_binding(self):
        events, chain, p = fixture()
        source = parse(encode(events))
        key = ("11" * 16, 0, "00000002", 2)
        checked = verify_job(source, key, {chain.tip["hash"]: chain}, p)
        self.assertEqual(checked["fees"], 1000)
        self.assertTrue(checked["witness"])
        self.assertEqual(checked["job"]["job_id"], (3 << 32) | 2)
        self.assertIsInstance(checked["job"]["coinbase_prefix"], list)
        publication = verify_publication(source, chain.tip["hash"], {chain.tip["hash"]: chain}, p)
        self.assertFalse(publication["owed_verified"])
        self.assertTrue(publication["witness"])

    def test_same_parent_refresh_requires_actual_new_issue_and_old_stale_result(self):
        events, chain, p = fixture()
        self.assertEqual(len(witness_republication(parse(encode(events)), {chain.tip["hash"]: chain}, p)), 1)
        for change in ("missing-rejection", "wrong-channel", "wrong-reason"):
            altered = copy.deepcopy(events)
            if change == "missing-rejection":
                altered.pop(4)
            else:
                altered[4][1]["channel_id" if change == "wrong-channel" else "reason"] = 9 if change == "wrong-channel" else "low-difficulty"
            with self.assertRaises(MissingComponent):
                witness_republication(parse(encode(altered)), {chain.tip["hash"]: chain}, p)

    def test_witness_or_fee_claim_cannot_override_raw_job(self):
        for change in ("fee", "reserved", "merkle", "contextual-bits"):
            events, chain, p = fixture()
            job = events[2][1]
            if change == "fee":
                job["fees"] = "999"
            elif change == "reserved":
                job["witness_plan"]["reserved"] = "01" * 32
            elif change == "merkle":
                job["merkle_branch"] = []
            else:
                job["nbits"] = 0x200fffff
            with self.subTest(change=change), self.assertRaises(Invalid):
                verify_job(parse(encode(events)), ("11" * 16, 0, "00000002", 2), {chain.tip["hash"]: chain}, p)

    def test_missing_parent_is_typed_unknown(self):
        events, _, p = fixture()
        with self.assertRaisesRegex(MissingComponent, "raw_parent"):
            verify_job(parse(encode(events)), ("11" * 16, 0, "00000002", 2), {}, p)

    def test_full_coinbase_witness_is_preserved_and_reserved_value_checked(self):
        events,chain,p=fixture();job=events[2][1]
        stripped=bytes.fromhex(job["coinbase_prefix"])+bytes(2)+bytes.fromhex(job["coinbase_suffix"])
        full=stripped[:4]+b"\0\1"+stripped[4:-4]+b"\1\x20"+bytes(32)+stripped[-4:]
        split=len(bytes.fromhex(job["coinbase_prefix"]))+2
        job["coinbase_prefix"]=full[:split].hex();job["coinbase_suffix"]=full[split+2:].hex()
        checked=verify_job(parse(encode(events)),("11"*16,0,"00000002",2),{chain.tip["hash"]:chain},p)
        self.assertEqual(checked["coinbase"]["witness"],[[bytes(32)]])
        altered=bytearray(full);altered[-5]=1
        job["coinbase_suffix"]=altered[split+2:].hex()
        with self.assertRaisesRegex(Invalid,"full coinbase witness"):
            verify_job(parse(encode(events)),("11"*16,0,"00000002",2),{chain.tip["hash"]:chain},p)

    def test_nonce_slot_cannot_move_to_zero_output_or_locktime_bytes(self):
        events,chain,p=fixture();job=events[2][1]
        raw=bytes.fromhex(job["coinbase_prefix"])+bytes(2)+bytes.fromhex(job["coinbase_suffix"])
        job["coinbase_prefix"]=raw[:-4].hex();job["coinbase_suffix"]=raw[-2:].hex()
        # The zero-nonce transaction is byte-identical; the nonce span is not.
        with self.assertRaisesRegex(Invalid,"extranonce overlaps"):
            verify_job(parse(encode(events)),("11"*16,0,"00000002",2),{chain.tip["hash"]:chain},p)

    def test_referenced_witness_job_material_carries_immutable_reserved_map(self):
        from source_feed_provider import ReplayProvider
        from types import SimpleNamespace
        events,chain,p=fixture();source=parse(encode(events));account=source.accounts[1][0]["payload"]
        account.update(sp_id="33"*32,share_index=0,d=1)
        source.slice_publications[1]=[(None,{"height":3,"front_id":0,"work_unit_version":2,"work_unit_fingerprint":"22"*32,
                                           "difficulty":1,"root":"44"*32})]
        provider=object.__new__(ReplayProvider);provider.capture=SimpleNamespace(policy=p);provider.views={chain.tip["hash"]:chain}
        material=provider._window_material(source,{"mappings":[{"slice_id":1}]})
        self.assertEqual(material["witness_reserved"],{str((3<<32)|2):[0]*32})
        self.assertEqual(material["jobs"][0]["job_id"],(3<<32)|2)

    def test_live_collector_path_keeps_tails_and_never_grants_incomplete_feed(self):
        events, chain, p = fixture()
        with tempfile.TemporaryDirectory(prefix="t3-source-vector-", dir=BASE) as tmp:
            root = Path(tmp); private = root / "private"; private.mkdir()
            scenario = {"fronts": [{"id": "front-a", "pool_id": "pool-a"}],
                        "pools": [{"id": "pool-a", "work_profile": {"reference_target": "7f" + "ff" * 31, "window_n": 1}}]}
            class Capture:
                rows = []
                def add(self, kind, value):
                    self.rows.append((kind, value))
            capture = Capture()
            collector = SourceCapture(root / "runtime", private, capture, scenario, {"path": "/unused", "sha256": "11" * 32}, p)
            path = collector.expected["front-a"]
            path.parent.mkdir(parents=True)
            raw = encode(events)
            path.write_bytes(raw[:-1])
            with patch.object(collector, "bind", return_value=None):
                collector.poll({}, {chain.tip["hash"]: chain})
                collector.poll({}, {chain.tip["hash"]: chain})
                self.assertEqual(collector.issues["front-a"]["component"], "source_log_incomplete_tail")
                self.assertEqual((collector.output / "front-a.log").read_bytes(), raw[:-1])
                path.write_bytes(raw)
                collector.poll({}, {chain.tip["hash"]: chain})
                self.assertFalse(collector.issues)
                self.assertEqual(len(collector.publications({chain.tip["hash"]: chain})["front-a"]), 1)
                old = events[0][1]
                oldcb = bytes.fromhex(old["coinbase_prefix"]) + bytes(2) + bytes.fromhex(old["coinbase_suffix"])
                oldblock = block_bytes(bytes.fromhex(old["prev_hash"]), [oldcb], old["ntime"])
                prefix = [b["raw"] + compact_size(len(b["transactions"])) + b"".join(t["raw"] for t in b["transactions"])
                          for b in chain.blocks[:3]]
                fork = Chain(p, prefix + [oldblock])
                with self.assertRaisesRegex(MissingComponent, "missing_source_publication"):
                    collector.publications({chain.tip["hash"]: chain, fork.tip["hash"]: fork})
                with self.assertRaisesRegex(MissingComponent, "RULEB"):
                    collector.for_job("front-a", ("11" * 16, 0, "00000002", 2))
                packet = json.loads(Path(collector.components[("front-a", ("11" * 16, 0, "00000002", 2))]["path"]).read_bytes())
                self.assertIn("job", packet["components"])
                self.assertFalse(packet["mining_authorized"])
                path.write_bytes(raw.replace(b"INFO", b"WARN", 1))
                with self.assertRaisesRegex(Invalid, "prefix changed"):
                    collector.poll({}, {chain.tip["hash"]: chain})


if __name__ == "__main__":
    unittest.main()
