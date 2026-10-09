from pathlib import Path
import base64,copy,hashlib,json,os,re,runpy,struct,subprocess
root=Path.cwd();base=root/'target/hopper/full-devnet-2026-10-07'
os.environ['HOPPER_MATRIX_PAYER']=str(base/'private-workers/application-suites.json')
h=runpy.run_path(str(base/'devnet-harness.py'));folder=h['WORK']/'hopper-token-lab';record=json.loads((folder/'deployment.json').read_text())
assert record['exitCode']!=0,'Only recover a completed failed attempt'
elf=Path(record['elfPath']).read_bytes();assert hashlib.sha256(elf).hexdigest()==record['elfSha256']
loader='BPFLoaderUpgradeab1e11111111111111111111111'
program=h['call']([h['CLI']/'solana-keygen.exe','pubkey',folder/'private/program.json'])
buffer=h['call']([h['CLI']/'solana-keygen.exe','pubkey',folder/'private/buffer.json'])
payer=h['call']([h['CLI']/'solana-keygen.exe','pubkey',h['PAYER']])
assert program==record['programId'] and h['rpc']('getGenesisHash',[])==h['H']['GENESIS']
out=folder/'rpc-write-recovery';out.mkdir(exist_ok=False)
def snapshot(slot=0):
 values=h['rpc']('getMultipleAccounts',[[program,buffer],dict(encoding='base64',commitment='finalized',minContextSlot=slot)])['value']
 assert values[0] is None,'Program already exists; inspect rather than upload'
 account=values[1];assert account is not None and account['owner']==loader and not account['executable']
 data=base64.b64decode(account['data'][0]);assert data[:5]==struct.pack('<IB',1,1)
 assert data[5:37]==h['H']['pubkey_bytes'](payer) and len(data)==37+len(elf)
 return account,data
initial,data=snapshot();h['write'](out/'initial-buffer.json',initial);records=[]
for offset in range(0,len(elf),900):
 chunk=elf[offset:offset+900]
 if data[37+offset:37+offset+len(chunk)]==chunk:continue
 before,data=snapshot(records[-1]['slot'] if records else 0)
 payload=struct.pack('<IIQ',1,offset,len(chunk))+chunk
 sent=h['call']([h['SENDER'],'tx','send','--program',loader,'--keypair',h['PAYER'],'--rpc',h['RPC'],'--data',payload.hex(),'--account',buffer+':w','--account','payer:s'])
 (out/f'write-{offset}.log').write_text(sent,encoding='utf-8')
 signature=re.search(r'^signature\s*:\s*(\w+)',sent,re.M)[1];tx=h['H']['finalize'](signature)
 h['write'](out/f'write-{offset}.transaction.json',tx);assert tx['meta']['err'] is None
 expected=bytearray(data);expected[37+offset:37+offset+len(chunk)]=chunk
 after,data=snapshot(tx['slot']);expected_account=copy.deepcopy(before);expected_account['data'][0]=base64.b64encode(expected).decode()
 assert after==expected_account,'Unexpected buffer state'
 records.append(dict(offset=offset,bytes=len(chunk),signature=signature,slot=tx['slot'],completeBufferStateMatched=True))
 h['write'](out/'progress.json',records);print('finalized buffer write',offset,len(chunk),flush=True)
assert data[37:]==elf
h['write'](out/'complete-buffer.json',dict(account=after if records else initial,elfSha256=hashlib.sha256(data[37:]).hexdigest(),transactions=records))
command=[h['CLI']/'solana.exe','program','deploy',record['elfPath'],'--program-id',folder/'private/program.json','--buffer',folder/'private/buffer.json','--keypair',h['PAYER'],'--url',h['RPC'],'--use-rpc','--max-sign-attempts','2','--with-compute-unit-price','1000','--max-len',len(elf),'--output','json']
with (folder/'private/recovery.stdout').open('x') as stdout,(folder/'private/recovery.stderr').open('x') as stderr:
 code=subprocess.run(list(map(str,command)),cwd=h['SOURCE'],env=h['ENV'],stdout=stdout,stderr=stderr).returncode
assert code==0,'Inspect finalization before another attempt'
h['dump'](folder,record,'recovered')
(folder/'deployment.initial.json').write_bytes((folder/'deployment.json').read_bytes())
record.update(exitCode=0,recoveredFromExistingBuffer=True,recoveryReceipt='rpc-write-recovery/complete-buffer.json',recoveryDriverSha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest())
h['write'](folder/'deployment.json',record)
print('Recovered deployment matches the exact ELF',program,flush=True)
