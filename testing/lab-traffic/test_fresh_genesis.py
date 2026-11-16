"""Source-backed initial samples, never a posthoc-ancestry substitute."""
import copy
import json
from pathlib import Path
import unittest

from fresh_genesis import verify_prephase_genesis
from oracle import Invalid, Inconclusive


class FreshGenesisTests(unittest.TestCase):
    def setUp(self):
        self.fixture=json.loads((Path(__file__).parent/'fixtures/prephase-genesis-b17.json').read_bytes())

    def verify(self,value=None):
        v=value or self.fixture
        encode=lambda rows:b''.join((json.dumps(r,separators=(',',':'))+'\n').encode() for r in rows)
        return verify_prephase_genesis(encode(v['events']),encode(v['commands']),v['scenario'],v['genesis'],v['source_binding'])

    def test_derived_b17_samples_are_before_source_proven_prephase_markers(self):
        r=self.verify();self.assertEqual({n:v['event_id'] for n,v in r['nodes'].items()},{'node-a':27,'node-b':28})
        self.assertEqual(r['prephase_marker_event_ids'],{'front-a1':31,'front-b1':32})
        self.assertEqual(r['hash_start_seq'],21);self.assertEqual(len(r['pre_hash_start_reads']),4)
        self.assertEqual(r['height'],0)

    def test_late_genesis_with_an_earlier_wall_stamp_is_not_initial_proof(self):
        v=copy.deepcopy(self.fixture);late=v['events'].pop(27);v['events'].append(late)
        for index,row in enumerate(v['events'],1):
            row['event_id']=index
            if row['source']=='operator':row['source_cursor']='operator-readiness-'+str(index-1)
        with self.assertRaisesRegex(Inconclusive,'both_nodes_prephase'):self.verify(v)

    def test_missing_marker_or_node_cannot_be_filled_from_genesis_ancestry(self):
        for index in (27,31):
            v=copy.deepcopy(self.fixture);v['events'][index]['payload']={'kind':'unknown','data':{'topic':'missing','detail':'not retained'}}
            with self.subTest(index=index),self.assertRaises(Inconclusive):self.verify(v)

    def test_wrong_node_origin_genesis_or_unreviewed_source_refuses(self):
        for change in ('actor','hash','height_type','source'):
            v=copy.deepcopy(self.fixture)
            if change=='actor':v['events'][26]['actor']['id']='foreign'
            elif change=='hash':v['events'][26]['payload']['data']['block_hash']='ab'*32
            elif change=='height_type':v['events'][26]['payload']['data']['height']=False
            else:v['source_binding']['runtime_source_sha256']='00'*32
            with self.subTest(change=change),self.assertRaises(Invalid):self.verify(v)

    def test_wrong_forward_result_or_second_hash_start_is_not_initial_proof(self):
        v=copy.deepcopy(self.fixture);v['commands'][5]['event']['response_sha256']='ab'*32
        with self.assertRaises(Invalid):self.verify(v)
        v=copy.deepcopy(self.fixture);extra=copy.deepcopy(v['commands'][-1]);extra['seq']=22;v['commands'].append(extra)
        with self.assertRaises(Invalid):self.verify(v)

    def test_sequence_gap_and_marker_wrong_front_are_not_ignored(self):
        v=copy.deepcopy(self.fixture);v['events'][10]['event_id']+=1
        with self.assertRaises(Invalid):self.verify(v)
        v=copy.deepcopy(self.fixture);detail=json.loads(v['events'][31]['payload']['data']['detail']);detail['front']='foreign'
        v['events'][31]['payload']['data']['detail']=json.dumps(detail)
        with self.assertRaises(Invalid):self.verify(v)


if __name__=='__main__':unittest.main()
