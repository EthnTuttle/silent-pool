"""Fresh r25 input pins must select the actual reviewed query bytes."""
import hashlib
import unittest

from campaign import load_controller
from decision_observations import CONTRACT
from oracle import Invalid
from storage import query_spec


class SQLInputBindingTests(unittest.TestCase):
    def test_fullrail_current_renderer_requires_exact_input_query_pin(self):
        query,query_id,schema,tables=query_spec(CONTRACT)
        digest=hashlib.sha256(query.encode()).hexdigest()
        self.assertEqual(digest,'16c8d72f970d1aed7a776e2b7a1f1269277d3f39dddb6cf6b78306f3d3802a1b')
        self.assertEqual(len(query.encode()),8699);self.assertEqual(len(tables),22)
        c=object.__new__(load_controller().Controller);c.results={}
        metadata={'sql_contract':CONTRACT,'query_id':query_id,'schema':schema,'sha256':digest,'bytes':len(query.encode()),'table_count':len(tables)}
        c.binding={'source_feed':{'contract':'unused-for-this-explicit-selector','sql_contract':CONTRACT,'sql_query':metadata}}
        c.verify_sql_query_binding();self.assertEqual(c.results['sql_query_binding']['status'],'PASSED')
        for key in metadata:
            c.binding['source_feed']['sql_query']={**metadata,key:None}
            with self.subTest(field=key),self.assertRaises(Invalid):c.verify_sql_query_binding()
        c.binding['source_feed']['sql_query']={**metadata,'sha256':'dbf0473d9747316f6852fc760f8b7604414c2e7d43a305e9520441063a25f1f0'}
        with self.assertRaises(Invalid):c.verify_sql_query_binding()
        del c.binding['source_feed']['sql_query']
        with self.assertRaises(Invalid):c.verify_sql_query_binding()


if __name__=='__main__':unittest.main()
