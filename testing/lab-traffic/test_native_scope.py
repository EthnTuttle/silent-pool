"""Public synthetic opaque-marker fixtures, never actual private run tokens."""
import copy
import hashlib
import io
import json
from pathlib import Path
import unittest
from unittest.mock import patch

from native_scope import (MAX_ENV, MAX_MARKER, expected_digest, marker_digest,
    environment_digest, process_digest, recorded_digest, private_scope, wallet_scope)
from native_observer import bind_connection
from oracle import Invalid, Inconclusive
from test_native_audit import fixture


def rendered(scope):
    return ('[ark.native_audit]\n'+''.join(k+' = '+json.dumps(v)+'\n' for k,v in scope.items())).encode()


def scope_fixture(marker='a7'*32):
    _,_,bindings,_,_=fixture();binding=bindings[('miner','connection')]
    instance=copy.deepcopy(binding['wallet']);digest=marker_digest(marker)
    scope={k:instance[k] for k in ('run_id','identity_id','pool_id','front_id','collector_pubkey','pool_pubkey')}
    scope.update(run_token_hash=digest.hex(),collector_socket='/unit/private/collector.sock')
    raw=rendered(scope);observed=copy.deepcopy(instance['process'])
    observed.update(run_token_sha256=list(digest),config_sha256=list(hashlib.sha256(raw).digest()))
    instance['process']=copy.deepcopy(observed)
    manifest={'run_id':'run','processes':[{'role':role,'run_token':marker} for role in ('wallet','miner')]}
    return scope,raw,instance,observed,manifest


class NativeScopeTests(unittest.TestCase):
    def test_opaque_hex_marker_is_hashed_as_text_not_decoded(self):
        marker='a7'*32;actual=marker_digest(marker)
        self.assertEqual(actual,hashlib.sha256(marker.encode()).digest())
        self.assertNotEqual(actual,hashlib.sha256(bytes.fromhex(marker)).digest())
        self.assertEqual(environment_digest(b'SPC_LAB_RUN_TOKEN='+marker.encode()+b'\0',actual),actual)

    def test_literal_unicode_and_exact_marker_bytes_have_no_prefix_coercion(self):
        for marker in ('literal-run:owner','opaque-without-run-prefix',' public marker ','\u00e9'*512):
            self.assertEqual(marker_digest(marker),hashlib.sha256(marker.encode('utf-8')).digest())
        self.assertNotEqual(marker_digest('ab'*32),marker_digest('AB'*32))
        for marker in ('','\u00e9'*513,'x'*(MAX_MARKER+1),None,42,'\ud800'):
            with self.subTest(kind=type(marker).__name__),self.assertRaises(Invalid):marker_digest(marker)

    def test_expected_digest_is_required_canonical_and_nonzero(self):
        self.assertEqual(expected_digest('ab'*32),bytes.fromhex('ab'*32))
        for value in ('00'*32,'AB'*32,'a'*63,'g'*64,' '+'ab'*32,bytes(32),None,0):
            with self.subTest(kind=type(value).__name__),self.assertRaises(Invalid):expected_digest(value)

    def test_environment_census_bound_and_foreign_digest_fail_closed(self):
        token=b'public-only';expected=hashlib.sha256(token).digest();entry=b'SPC_LAB_RUN_TOKEN='+token+b'\0'
        self.assertEqual(environment_digest(entry+b'X=' + b'x'*(MAX_ENV-len(entry)-2),expected),expected)
        for raw in (b'',b'SPC_LAB_RUN_TOKEN=\0',entry+entry,entry+b'x'*(MAX_ENV-len(entry)+1),
                    b'SPC_LAB_RUN_TOKEN='+b'x'*(MAX_MARKER+1),b'NOT_SPC_LAB_RUN_TOKEN='+token):
            with self.assertRaises(Invalid):environment_digest(raw,expected)
        with self.assertRaises(Invalid):environment_digest(entry,hashlib.sha256(b'foreign').digest())
        with self.assertRaises(Invalid):environment_digest(entry,bytes(32))

    def test_proc_reader_has_actual_one_MiB_plus_one_read_limit(self):
        marker='public-only';raw=b'SPC_LAB_RUN_TOKEN='+marker.encode()+b'\0';stream=io.BytesIO(raw)
        with patch.object(Path,'open',return_value=stream):
            self.assertEqual(process_digest(Path('/not-real-proc'),{'run_token':marker}),marker_digest(marker))
        class BoundedStream(io.BytesIO):
            def read(self,size=-1):
                self.asserted_size=size
                return super().read(size)
        stream=BoundedStream(raw+b'x'*MAX_ENV)
        with patch.object(Path,'open',return_value=stream),self.assertRaises(Invalid):process_digest(Path('/not-real-proc'),{'run_token':marker})
        self.assertEqual(stream.asserted_size,MAX_ENV+1)

    def test_actual_eight_private_fields_and_no_missing_hash_or_unknown_fallback(self):
        scope,raw,_,_,_=scope_fixture();self.assertEqual(private_scope(raw),scope)
        for change in ('missing','unknown','nested','zero','upper','old-name'):
            bad=copy.deepcopy(scope)
            if change=='missing':del bad['run_token_hash']
            elif change=='unknown':bad['token']='public-only'
            elif change=='nested':bad['run_token_hash']={}
            elif change=='zero':bad['run_token_hash']='00'*32
            elif change=='upper':bad['run_token_hash']=bad['run_token_hash'].upper()
            else:bad['run_token_sha256']=bad.pop('run_token_hash')
            error=Inconclusive if change in ('missing','unknown','old-name') else Invalid
            with self.subTest(change=change),self.assertRaises(error):private_scope(rendered(bad))
        with self.assertRaises(Invalid):private_scope(raw+b'run_token_hash = "'+b'11'*32+b'"\n')

    def test_recorded_census_is_nonempty_consistent_and_bound_to_actual_run(self):
        marker='public-literal:owner';manifest={'run_id':'run','processes':[{'run_token':marker}]*4}
        self.assertEqual(recorded_digest(manifest,'run'),marker_digest(marker))
        for records in ([],[{}],[{'run_token':''}],[{'run_token':marker},{'run_token':'other'}]):
            with self.assertRaises(Invalid):recorded_digest({'run_id':'run','processes':records},'run')
        with self.assertRaises(Invalid):recorded_digest(manifest,'foreign')

    def test_config_instance_observation_and_recorded_digest_join(self):
        for marker in ('a7'*32,'run:public-owner'):
            scope,raw,instance,observed,manifest=scope_fixture(marker)
            result=wallet_scope(raw,instance,observed,manifest,'run')
            self.assertEqual(result['run_token_hash'],scope['run_token_hash']);self.assertEqual(result['recorded_processes'],2)
            self.assertNotIn(marker,json.dumps(result))

    def test_stale_config_instance_or_recorded_marker_cannot_be_adopted(self):
        for change in ('expected','instance','census','config','pid','identity','pool','front','run'):
            scope,raw,instance,observed,manifest=scope_fixture()
            if change=='expected':
                scope['run_token_hash']='19'*32;raw=rendered(scope)
                observed['config_sha256']=list(hashlib.sha256(raw).digest());instance['process']=copy.deepcopy(observed)
            elif change=='instance':instance['process']['run_token_sha256']=[23]*32
            elif change=='census':manifest['processes'][0]['run_token']='foreign'
            elif change=='config':raw+=b'\n'
            elif change=='pid':instance['process']['pid']+=1
            elif change=='identity':instance['identity_id']='bob'
            elif change=='pool':instance['pool_id']='pool-b'
            elif change=='front':instance['front_id']='front-b'
            else:manifest['run_id']='foreign'
            with self.subTest(change=change),self.assertRaises(Invalid):wallet_scope(raw,instance,observed,manifest,'run')

    def test_matching_token_does_not_replace_complete_registry_authority(self):
        _,_,bindings,_,_=fixture();b=bindings[('miner','connection')]
        for change in ('foreign-shared-peer-token','identity','config_path','executable','uid','boot_id','start_time_ticks'):
            value=copy.deepcopy(b);value['observed_miner']=copy.deepcopy(value['observed_miner']);peer=value['registry']['peers'][0]
            if change=='foreign-shared-peer-token':
                peer['process']['run_token_sha256']=[23]*32;value['observed_miner']['run_token_sha256']=[23]*32
            elif change=='identity':peer['identity']='foreign'
            elif change=='config_path':peer['config_path']='/unit/private/other.toml'
            elif change=='executable':peer['process']['executable_sha256']=[23]*32
            elif change=='uid':value['observed_miner']['uid']+=1
            elif change=='boot_id':value['observed_miner']['boot_id']='11234567-89ab-cdef-0123-456789abcdef'
            else:value['observed_miner']['start_time_ticks']+=1
            with self.subTest(change=change),self.assertRaises(Invalid):bind_connection(value['config'],value['wallet'],value['registry'],value['observed_miner'],value['route'])


if __name__=='__main__':unittest.main()
