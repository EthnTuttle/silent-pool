#!/usr/bin/env python3
"""Owned stock-Bitcoin/native-SV2 990-output component proof.

No generate RPC, synthetic credit, injected job/block, Ark server or hosted
provider is used. This is component evidence, not full funded lab acceptance.
"""
import argparse,base64,hashlib,importlib.util,json,os,pathlib,secrets,socket,subprocess,time,urllib.request

ROOT=pathlib.Path(__file__).resolve().parents[1]
# Historical local postgres instance (the ladder-completion campaign); a
# fresh machine points SPC_LAB_PG_SOCKET/PORT/USER at its own provisioned
# cluster. Portable pg provisioning is tracked as remaining work; these
# defaults are labelled local archive, not fresh-setup preconditions.
PG_SOCK=os.environ.get('SPC_LAB_PG_SOCKET','/tmp/opencode/ladder-completion-pg-r1/socket')
PG_PORT=os.environ.get('SPC_LAB_PG_PORT','55479')
PG_USER=os.environ.get('SPC_LAB_PG_USER','ethan')
def write(path,value):
    path.write_text(json.dumps(value,indent=2)+'\n' if not isinstance(value,str) else value);path.chmod(0o600)
def port():
    with socket.socket() as s:s.bind(('127.0.0.1',0));return s.getsockname()[1]
def wait(fn,timeout=60):
    deadline=time.monotonic()+timeout;last=None
    while time.monotonic()<deadline:
        try:
            value=fn()
            if value:return value
        except Exception as e:last=e
        time.sleep(.15)
    raise RuntimeError(f'bounded readiness/proof timeout: {last}')

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--run-dir',type=pathlib.Path,required=True)
    parser.add_argument('--seals',type=pathlib.Path,required=True);args=parser.parse_args()
    assert not args.run_dir.exists();args.run_dir.mkdir(mode=0o700)
    private=args.run_dir/'private';private.mkdir(mode=0o700)
    binaries={r['name']:r for r in json.loads(args.seals.read_text())}
    for row in binaries.values():assert hashlib.sha256(pathlib.Path(row['path']).read_bytes()).hexdigest()==row['sha256']
    node='/nix/store/wjhi0qih18rp4walq3k8qlx7ymnn9p29-bitcoin-core-ipc-31.0/bin/bitcoind'
    redis='/nix/store/nql1lir6650xigvf43n11zz4hh0lbw9z-redis-8.10.3/bin/redis-server'
    pg='/nix/store/vync24csmxy5saql5ah77vkn59yg2x5h-postgresql-17.11/bin'
    assert pathlib.Path(node).is_file()
    ports={name:port() for name in ['rpc','sv2','jd','api','redis']}
    password=secrets.token_hex(24);children=[];logs=[]
    os.umask(0o077)
    def start(name,cmd):
        log=(private/f'{name}.log').open('w');logs.append(log)
        process=subprocess.Popen(cmd,stdin=subprocess.PIPE,stdout=log,stderr=subprocess.STDOUT,
            env={**os.environ,'RUST_LOG':'info,spc_evidence=info'},start_new_session=True)
        children.append(process);return process
    opener=urllib.request.build_opener(urllib.request.ProxyHandler({}))
    def rpc(method,params=[]):
        assert not method.startswith('generate')
        request=urllib.request.Request(f'http://127.0.0.1:{ports["rpc"]}',data=json.dumps({'jsonrpc':'2.0','id':1,'method':method,'params':params}).encode(),
            headers={'Authorization':'Basic '+base64.b64encode(f'ladder:{password}'.encode()).decode(),'Content-Type':'application/json'})
        value=json.loads(opener.open(request,timeout=5).read())
        if value.get('error'):raise RuntimeError(value['error'])
        return value['result']
    result={'scope':'native_sv2_ladder_component','funded_ark_acceptance':False,'status':'incomplete','binaries':binaries}
    try:
        data=private/'bitcoin';data.mkdir(mode=0o700)
        start('node',[node,'-regtest',f'-datadir={data}','-server=1','-listen=0','-connect=0','-txindex=1',
            f'-rpcport={ports["rpc"]}','-rpcuser=ladder',f'-rpcpassword={password}','-fallbackfee=0.00001'])
        wait(lambda:rpc('getblockchaininfo'));assert rpc('getblockcount')==0
        start('redis',[redis,'--bind','127.0.0.1','--port',str(ports['redis']),'--requirepass',password,'--save','','--appendonly','no','--dir',str(private)])
        db='ladder_native_'+secrets.token_hex(4)
        pgsock,pgport,pguser=PG_SOCK,PG_PORT,PG_USER
        subprocess.run([pg+'/createdb','-h',pgsock,'-p',pgport,db],check=True)
        pgurl=f'postgresql://{pguser}@localhost/{db}?host={pgsock}&port={pgport}'
        identity=json.loads(subprocess.check_output([binaries['spc-scan']['path'],'gen-address','--network','regtest']))
        authority=json.loads(subprocess.check_output([binaries['spc-scan']['path'],'gen-address','--network','regtest']))
        write(private/'identity.json',identity)
        spec=importlib.util.spec_from_file_location('ladder_native_oracle',ROOT/'testing/lab-live/oracle.py');oracle=importlib.util.module_from_spec(spec);spec.loader.exec_module(oracle)
        public,secret=oracle.noise_authority_from_secp_material(bytes.fromhex(authority['b_scan']),bytes.fromhex(authority['scan_key']))
        descriptor='tr(tpubD6NzVbkrYhZ4WUjXBVxZPvGhWxTRE5veAscTgokX65xcfCoDF5nyNdtaS4xGoJ7xnZuDTxCAEAnGpxJbWo6wtTAJsESmeJWpZzybZ6dvCia/0/*)'
        write(private/'pool.toml',f'''[pool]
network="regtest"
roles=["front","payout","api"]
listen="127.0.0.1:{ports['sv2']}"
authority_public_key="{public}"
authority_secret_key="{secret}"
pool_payout_descriptor="{descriptor}"
fee_bps=100
[template]
source="rpc"
rpc_url="http://127.0.0.1:{ports['rpc']}"
rpc_user="ladder"
rpc_pass="{password}"
refresh_interval_ms=100
[sv1]
enabled=false
[accounting]
work_unit_version=2
work_reference_target="{'7f'+'ff'*31}"
share_work_multiplier=1
max_work_multiplier=4294967296
max_window_target_units="18446744073709551616"
window_rule="network_block_work"
window_n=8
pow_limit_bits=0x207fffff
max_payout_slots=1024
allow_unchecked_chain=false
[extensions]
share_accounting=false
[jd]
listen="127.0.0.1:{ports['jd']}"
[api]
listen="127.0.0.1:{ports['api']}"
[storage]
postgres="{pgurl}"
redis="redis://:{password}@127.0.0.1:{ports['redis']}"
''')
        start('pool',[binaries['spc-pool']['path'],'--config',str(private/'pool.toml')])
        wait(lambda:json.loads(opener.open(f'http://127.0.0.1:{ports["api"]}/api/pool',timeout=3).read()))
        write(private/'miner.toml',f'''config_version=1
miner_id="ladder-native"
event_queue_capacity=4096
[[connections]]
id="owned-pool"
endpoint="127.0.0.1:{ports['sv2']}"
identity="{identity['address']}"
authority_public_key="{public}"
min_extranonce_size=4
reconnect={{enabled=true,initial_backoff_ms=100,max_backoff_ms=5000,handshake_timeout_ms=10000}}
workers=[{{id="native-0",budget={{hashes_per_second=3,total_attempts=200}},ntime_policy="hold",max_ntime_roll_seconds=600,batch_size=1}}]
''')
        miner=start('miner',[binaries['spc-cpu-miner']['path'],'--config',str(private/'miner.toml')])
        wait(lambda:rpc('getblockcount')>=18,120)
        miner.stdin.close();miner.wait(timeout=10)
        tip=rpc('getblockcount');height=tip-6;blockhash=rpc('getblockhash',[height])
        def settlement():
            query=f"SELECT jsonb_build_object('status',bf.status,'height',bf.height,'window_target',ds.window_target::text,'work_unit_version',ds.work_unit_version,'window_rule',ds.window_rule,'high_water',ds.high_water,'logical_entries',jsonb_array_length(ds.entries),'key_count',jsonb_array_length(ds.entries->0->'p_x'),'owed_sats',ds.entries->0->'owed_sats','entry_sats',ds.entries->0->'sats','ledger_rows',(SELECT count(*) FROM ledger_history lh WHERE lh.block_hash=bf.block_hash),'paid_sats',(SELECT sum(paid_sats) FROM ledger_history lh WHERE lh.block_hash=bf.block_hash)) FROM block_found bf JOIN distribution_snapshot ds ON ds.fingerprint=bf.fingerprint WHERE bf.height={height} AND bf.status='confirmed'"
            output=subprocess.check_output([pg+'/psql','-h',PG_SOCK,'-p',PG_PORT,'-d',db,'-Atq','-c',query],text=True).strip()
            return json.loads(output) if output else None
        booked=wait(settlement,60)
        assert booked['logical_entries']==1 and booked['key_count']==990 and booked['ledger_rows']==1
        assert booked['owed_sats']==booked['paid_sats']==4_950_000_000
        assert booked['window_target']=='9' and booked['work_unit_version']==2 and booked['window_rule']==1
        write(args.run_dir/'settlement-check.json',booked)
        block=rpc('getblock',[blockhash,2]);raw=rpc('getblock',[blockhash,0])
        blockdir=private/'block-proof';blockdir.mkdir(mode=0o700);(blockdir/'block.bin').write_bytes(bytes.fromhex(raw))
        write(args.run_dir/'raw-block.json',{'height':height,'hash':blockhash,'raw_hex':raw})
        found=json.loads(subprocess.check_output([binaries['spc-scan']['path'],'find','--address',identity['address'],'--scan-key',identity['scan_key'],
            '--from',str(height),'--to',str(height),'--rpc',f'http://127.0.0.1:{ports["rpc"]}','--rpc-user','ladder','--rpc-pass',password]))
        write(private/'scanner.json',found)
        exported=json.loads(subprocess.check_output([binaries['spc-scan']['path'],'export-tweaks','--address',identity['address'],'--scan-key',identity['scan_key'],
            '--txid',block['tx'][0]['txid'],'--blocks',str(blockdir)]))
        write(private/'tweaks.json',exported)
        # Scanner CLI emits one outcome per block. Keep complete vector, never
        # accepting first-match recovery or replacing it with pool totals.
        matches=found['found']
        assert len(matches)==990,(type(found),len(matches))
        assert len(exported)==990
        assert sum(item['amount'] for item in matches)==4_950_000_000
        assert {item['k'] for item in matches}==set(range(990))
        assert len({item['vout'] for item in matches})==990
        result.update(status='passed',tip=tip,height=height,block_hash=blockhash,owned_outputs=990,owned_sats=4_950_000_000,
            exported_tweaks=990,logical_ledger_rows=booked['ledger_rows'],frozen_window_target=booked['window_target'],node_sha256=hashlib.sha256(pathlib.Path(node).read_bytes()).hexdigest(),
            private_postgres_database=db,proof_scope='real stock Bitcoin RPC + actual native SV2 shares; Ark not exercised')
    except Exception as error:
        result['error']=str(error);raise
    finally:
        for process in reversed(children):
            if process.poll() is None:
                process.terminate()
                try:process.wait(timeout=10)
                except subprocess.TimeoutExpired:process.kill();process.wait(timeout=5)
        for log in logs:log.close()
        write(args.run_dir/'receipt.json',result)
        print(json.dumps(result))
if __name__=='__main__':main()
