from pathlib import Path
import datetime,hashlib,json,os,re,runpy,subprocess

ROOT=Path(__file__).resolve().parents[3]
SOURCE=ROOT/'target/hopper/full-devnet-2026-10-07/validation-v6/source'
WORK=SOURCE/'target/verification/matrix'
CLI=ROOT/'target/hopper/token-review-2026-09-30/tooling/agave-4.3.0'
PAYER=Path(os.environ.get('HOPPER_MATRIX_PAYER',str(ROOT/'target/hopper/devnet-release/payer.json'))).resolve()
assert PAYER.is_relative_to(ROOT/'target/hopper')
SENDER=ROOT/'target/hopper/dispatch-2026-10-07/sender/hopper.exe'
ENV=os.environ.copy();ENV['PATH']=str(CLI)+os.pathsep+ENV['PATH'];ENV['CARGO_TARGET_DIR']=str(ROOT/'target')
H=runpy.run_path(str(SOURCE/'scripts/test-runtime-gate-devnet.py'));rpc=H['rpc'];RPC=H['RPC']

def call(argv):return subprocess.check_output(list(map(str,argv)),cwd=SOURCE,env=ENV,text=True,encoding='utf-8',stderr=subprocess.PIPE).strip()
def write(path,value):path.write_text(json.dumps(value,indent=2)+'\n',encoding='utf-8')

def deploy(label,elf,source_record=None,features=False):
 assert rpc('getGenesisHash',[])==H['GENESIS']
 assert not call(['git','status','--porcelain'])
 folder=WORK/label;folder.mkdir(parents=True,exist_ok=False);private=folder/'private';private.mkdir()
 keys=[private/'program.json',private/'buffer.json']
 for key in keys:call([CLI/'solana-keygen.exe','new','--silent','--no-bip39-passphrase','--outfile',key])
 ids=[call([CLI/'solana-keygen.exe','pubkey',key]) for key in keys]
 before=rpc('getMultipleAccounts',[ids,{'encoding':'base64','commitment':'finalized'}]);assert before['value']==[None,None]
 balance=rpc('getBalance',[call([CLI/'solana-keygen.exe','pubkey',PAYER]),{'commitment':'finalized'}])['value']
 rent=rpc('getMinimumBalanceForRentExemption',[elf.stat().st_size+45]);assert balance>rent*2+20_000_000,'Insufficient devnet SOL for this deployment'
 command=[CLI/'solana.exe','program','deploy',elf,'--program-id',keys[0],'--buffer',keys[1],'--keypair',PAYER,'--url',RPC,'--use-rpc','--max-sign-attempts','10','--with-compute-unit-price','1000','--max-len',str(elf.stat().st_size),'--output','json']
 if features:command.append('--skip-feature-verify')
 with (private/'deploy.stdout').open('x',encoding='utf-8') as stdout,(private/'deploy.stderr').open('x',encoding='utf-8') as stderr:code=subprocess.run(list(map(str,command)),cwd=SOURCE,env=ENV,stdout=stdout,stderr=stderr).returncode
 record=dict(label=label,programId=ids[0],elfPath=str(elf),elfBytes=elf.stat().st_size,elfSha256=hashlib.sha256(elf.read_bytes()).hexdigest(),source=source_record,sourceCommit=call(['git','rev-parse','HEAD']) if source_record is None else None,freshAtSlot=before['context']['slot'],localFeatureSelectionOverride=features,transactionPreflightEnabled=True,exitCode=code,observedAt=datetime.datetime.now(datetime.timezone.utc).isoformat())
 write(folder/'deployment.json',record)
 print('deployed',label,code,ids[0],flush=True)
 if code:
  for line in (private/'deploy.stderr').read_text(encoding='utf-8').splitlines():
   if any(word in line for word in ['Error:','failed','syscall','simulation']):print(line,flush=True)
  raise RuntimeError('Deployment failed; inspect state before retrying')
 return folder,record

def dump(folder,record,phase):
 path=folder/(phase+'.so');call([CLI/'solana.exe','program','dump',record['programId'],path,'--url',RPC,'--keypair',PAYER,'--commitment','finalized'])
 assert hashlib.sha256(path.read_bytes()).hexdigest()==record['elfSha256']

def send(folder,record,name,data=b'',error=None):
 command=[SENDER,'tx','send','--program',record['programId'],'--keypair',PAYER,'--rpc',RPC,'--data',data.hex()]
 if error:command.append('--allow-failure')
 sent=call(command);(folder/(name+'.log')).write_text(sent,encoding='utf-8');sig=re.search(r'^signature\s*:\s*(\w+)',sent,re.M)[1];tx=H['finalize'](sig)
 write(folder/(name+'.transaction.json'),tx);assert tx['meta']['err']==(None if error is None else {'InstructionError':[0,error]}),tx['meta']['err'];return tx
