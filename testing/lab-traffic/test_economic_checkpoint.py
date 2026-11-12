"""Migration-0011 byte/SQL/journal vectors, not a claimed PG execution receipt."""
import copy
import hashlib
import json
import unittest

from economic_checkpoint import INITIAL,SCHEMA,QUERY_ID,validate_initial,query
from source_feed_codec import CONTRACT
from ruleb_sql import replay_checkpoint
from oracle import Inconclusive,Invalid
from test_ruleb_sql import fixture


def captured():
    observation,job,chain=fixture()
    observation["schema"]=SCHEMA;observation["query_id"]=QUERY_ID
    history=observation["tables"]["source_accounting_history"]
    initial=history[1]
    initial["operation"]="UPDATE";initial["before_row"]=copy.deepcopy(INITIAL)
    for record in history[1:]:record["transaction_id"]+=1
    migrations={i:hashlib.sha384(("offline-migration-"+str(i)).encode()).hexdigest() for i in range(1,11)}
    snapshot={"schema":"spc.economic-checkpoint/1","migration_version":11,"complete_from_empty":True,
        "state":INITIAL,"row_limit":1025,"history_start":{k:history[0][k] for k in ("event_id","table_name","operation","after_row")},
        "booked_balances":[],"economic_blocks":[],"chain":[],"found":[],"ledger":[],"ark_payouts":[],
        "prior_migrations":[{"version":i,"checksum":c} for i,c in migrations.items()]}
    state_raw=json.dumps(INITIAL).encode();snapshot_raw=json.dumps(snapshot).encode()
    row={"checkpoint_id":1,"schema_version":1,"migration_version":11,"classification":"fresh_empty","economic_revision":0,
         "history_watermark":1,"transaction_id":2,"snapshot_boundary":"2:3:","state_bytes":"\\x"+state_raw.hex(),
         "snapshot_bytes":"\\x"+snapshot_raw.hex(),"snapshot_sha256":"\\x"+hashlib.sha256(snapshot_raw).hexdigest(),"recorded_at":"now"}
    history.insert(1,{"event_id":2,"transaction_id":2,"table_name":"economic_checkpoint","operation":"INSERT",
                      "before_row":None,"after_row":copy.deepcopy(row),"recorded_at":"now"})
    observation["tables"]["economic_checkpoint"]=[row]
    observation["counts"]["economic_checkpoint"]=1
    observation["counts"]["source_accounting_history"]+=1
    return observation,job,chain,migrations


class InitialCheckpointTests(unittest.TestCase):
    def test_actual_checkpoint_INSERT_supplies_exact_missing_seed_without_history_rewrite(self):
        observation,job,chain,migrations=captured();history=copy.deepcopy(observation["tables"]["source_accounting_history"])
        seed=validate_initial(observation,migration_checksums=migrations,source_contract=CONTRACT)
        self.assertEqual(seed.preceding_watermark,1)
        self.assertEqual(seed.replay_seed()["economic_state"][(1,)],INITIAL)
        result=replay_checkpoint(observation,job,chain,source_contract=CONTRACT,migration_checksums=migrations)
        self.assertEqual(result["available_from_replay"],{"33"*32:100000})
        self.assertEqual(history,observation["tables"]["source_accounting_history"])
        self.assertIn('public."economic_checkpoint"',query(source_contract=CONTRACT))

    def test_missing_or_nonempty_compatibility_checkpoint_never_seeds_zero(self):
        observation,_,_,migrations=captured()
        observation["tables"]["economic_checkpoint"]=[]
        with self.assertRaises(Inconclusive):validate_initial(observation,migration_checksums=migrations,source_contract=CONTRACT)
        observation,_,_,migrations=captured()
        observation["tables"]["economic_checkpoint"][0]["classification"]="unverified_existing"
        with self.assertRaises(Inconclusive):validate_initial(observation,migration_checksums=migrations,source_contract=CONTRACT)

    def test_changed_snapshot_state_journal_or_migration_hash_is_rejected(self):
        for change in ("snapshot","state","journal","migration"):
            observation,_,_,migrations=captured();row=observation["tables"]["economic_checkpoint"][0]
            if change=="snapshot":row["snapshot_bytes"]="\\x7b7d"
            elif change=="state":row["state_bytes"]="\\x7b7d"
            elif change=="journal":observation["tables"]["source_accounting_history"][1]["after_row"]["history_watermark"]=0
            else:migrations[9]="00"*48
            with self.subTest(change=change),self.assertRaises((Invalid,Inconclusive)):
                validate_initial(observation,migration_checksums=migrations,source_contract=CONTRACT)

    def test_checkpoint_transaction_and_preceding_watermark_are_not_current_DB_defaults(self):
        for field,value in (("history_watermark",3),("snapshot_boundary","1:2:1"),("transaction_id",99)):
            observation,_,_,migrations=captured();row=observation["tables"]["economic_checkpoint"][0]
            row[field]=value;observation["tables"]["source_accounting_history"][1]["after_row"]=copy.deepcopy(row)
            with self.assertRaises((Invalid,Inconclusive)):
                validate_initial(observation,migration_checksums=migrations,source_contract=CONTRACT)


if __name__ == "__main__":unittest.main()
