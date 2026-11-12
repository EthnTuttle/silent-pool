"""Signed EconomicChanged trace hints are not accounting checkpoints."""
import hashlib
import unittest

from audited_window import rewards
from economic_checkpoint import validate_initial
from oracle import Inconclusive, Invalid
from pool_source import PoolSource
from source_feed_codec import COMBINED_TESTED_CONTRACT
from test_audited_window import encode_events
from test_source_feed import pub


def hint():return {"economic_revision":"29","anchor_hash":"12"*32,"coherent_snapshot_reload_required":True}


def parse(events):
    raw=encode_events(events)
    return PoolSource(raw,independently_captured_log_sha256=hashlib.sha256(raw).hexdigest(),
                      source_contract=COMBINED_TESTED_CONTRACT,pool_pubkey=pub(7))


class EconomicReloadTests(unittest.TestCase):
    def test_signed_known_hint_advances_complete_trace_without_changing_money(self):
        events=[("economic_change_observed",hint()),("economic_change_observed",{**hint(),"economic_revision":str(2**64-1)})]
        source=parse(events)
        self.assertEqual(source.heads["01"*16][0],2)
        self.assertEqual(source.jobs,{})
        self.assertEqual(source.accounts,{})
        self.assertEqual(source.block_publications,{})
        with self.assertRaises(Inconclusive):rewards({"selected":[]},5000000000,0,100)
        with self.assertRaises(Inconclusive):validate_initial({"tables":{}},migration_checksums={},source_contract=COMBINED_TESTED_CONTRACT)

    def test_hint_u64_is_canonical_decimal_string_and_reload_flag_is_true_bool(self):
        for value in (29,True,"029","-1","1.0",str(2**64)," 29","٢٩"):
            with self.subTest(value=value),self.assertRaises(Invalid):parse([("economic_change_observed",{**hint(),"economic_revision":value})])
        for value in (False,1,"true",None):
            with self.subTest(value=value),self.assertRaises(Invalid):parse([("economic_change_observed",{**hint(),"coherent_snapshot_reload_required":value})])

    def test_hint_anchor_and_payload_fields_are_closed(self):
        for value in ("12"*31,"12"*33,"zz"*32,[18]*32,None):
            with self.subTest(value=value),self.assertRaises(Invalid):parse([("economic_change_observed",{**hint(),"anchor_hash":value})])
        for value in ({k:v for k,v in hint().items() if k!="anchor_hash"},{**hint(),"pending_sats":"1"}):
            with self.assertRaises(Invalid):parse([("economic_change_observed",value)])

    def test_hint_cannot_be_dropped_or_unknown_events_whitelisted(self):
        raw=encode_events([("economic_change_observed",hint()),("economic_change_observed",hint())])
        missing=raw.splitlines(keepends=True)[1]
        with self.assertRaises(Inconclusive):PoolSource(missing,independently_captured_log_sha256=hashlib.sha256(missing).hexdigest(),source_contract=COMBINED_TESTED_CONTRACT,pool_pubkey=pub(7))
        with self.assertRaisesRegex(Inconclusive,"unrecognized"):parse([("economic_balance_trusted",hint())])
        changed=raw.replace(b'"economic_revision":"29"',b'"economic_revision":"30"',1)
        with self.assertRaises(Invalid):PoolSource(changed,independently_captured_log_sha256=hashlib.sha256(changed).hexdigest(),source_contract=COMBINED_TESTED_CONTRACT,pool_pubkey=pub(7))


if __name__=="__main__":unittest.main()
