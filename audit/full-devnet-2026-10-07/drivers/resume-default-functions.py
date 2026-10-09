from pathlib import Path
import hashlib,json,re,runpy,sys
root=Path.cwd();base=root/'target/hopper/full-devnet-2026-10-07';h=runpy.run_path(str(base/'devnet-harness.py'));source=h['SOURCE'];live=source/'target/verification';out=live/'devnet-hopper-function-lab';program=json.loads((live/'hopper-function-lab.deployment.json').read_text())['programId'];elf=live/'sbf/hopper-function-lab/hopper_function_lab.so'
assert not (out/'summary.json').exists() and not list((out/'keys').glob('*.json')),'This adapter resumes only the stateless prefix, before state creation'
original=json.loads((out/'progress.json').read_text());h['write'](out/'pre-resumption-progress.json',original)
known={case['name'] for case in runpy.run_path(str(source/'scripts/function-lab-cases.py'))['cases']()};assert all(row['name'] in known for row in original)
logs={p.stem:p for p in out.glob('*.log') if p.stem in known};assert len(logs)==70
alphabet='123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'
def decode(value):
 number=0
 for char in value:number=number*58+alphabet.index(char)
 return bytes(len(value)-len(value.lstrip('1')))+number.to_bytes((number.bit_length()+7)//8,'big')
globals_={};replayed=[];sent_new=[]
def resume_or_send(folder,name,argv,target,data):
 if name not in logs:
  sent_new.append(name);return globals_['run'](argv)
 assert target==program and name in known
 saved=logs[name].read_text(encoding='utf-8');signature=re.search(r'^signature\s*:\s*(\w+)',saved,re.M)[1]
 tx=json.loads(logs[name].with_suffix('.transaction.json').read_text());assert tx['transaction']['signatures'][0]==signature
 message=tx['transaction']['message'];keys=message['accountKeys'];instructions=[ix for ix in message['instructions'] if keys[ix['programIdIndex']]==target]
 assert len(instructions)==1 and decode(instructions[0]['data'])==data and instructions[0]['accounts']==[],'Saved instruction differs from the requested stateless case'
 replayed.append(dict(name=name,signature=signature,slot=tx['slot']));return saved
script=source/'scripts/test-function-lab-devnet.py';code=script.read_text(encoding='utf-8')
assert code.count('sent = run(argv)')==1
code=code.replace('sent = run(argv)','sent = _resume_or_send(out, name, argv, target, data)').replace('out.mkdir(parents=True, exist_ok=False)','out.mkdir(parents=True, exist_ok=True)').replace('keys.mkdir()','keys.mkdir(exist_ok=True)').replace('dump = out / f"{phase}-onchain.so"','dump = out / f"{phase}-resumed-onchain.so"')
sys.argv=[str(script),'--program',program,'--payer',str(h['PAYER']),'--hopper',str(h['SENDER']),'--elf',str(elf),'--source-snapshot',str(live/'source.json'),'--out',str(out)]
import os
os.environ.update(h['ENV'])
globals_.update(__name__='__main__',__file__=str(script),_resume_or_send=resume_or_send)
try:exec(compile(code,str(script),'exec'),globals_)
finally:h['write'](out/'resumption.json',dict(adapterSha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),originalDriverSha256=hashlib.sha256(script.read_bytes()).hexdigest(),replayedReadOnlyCases=replayed,newlySubmittedCases=sent_new,scope='Only stateless known-answer calls are replayed from verified finalized transactions; none is resubmitted. State checks use fresh accounts.'))
