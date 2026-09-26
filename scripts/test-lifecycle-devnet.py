#!/usr/bin/env python3
"""Finalized public-devnet lifecycle checks with complete account snapshots."""
from pathlib import Path
import argparse,base64,copy,hashlib,json,re,runpy,struct
ROOT=Path(__file__).resolve().parents[1]
H=runpy.run_path(str(ROOT/'scripts/test-runtime-gate-devnet.py'))
run,rpc,finalize=(H[k] for k in ('run','rpc','finalize'))

def main():
 p=argparse.ArgumentParser(description=__doc__)
 for k in ['native','runtime','native-elf','runtime-elf','payer','hopper','out']:p.add_argument('--'+k,required=True)
 a=p.parse_args();out=Path(a.out).resolve();assert out.is_relative_to(ROOT/'target')
 assert rpc('getGenesisHash',[])==H['GENESIS'];assert not run(['git','status','--porcelain']).strip()
 source=run(['git','rev-parse','HEAD']).strip();out.mkdir(parents=True,exist_ok=False)
 keys=out/'keys';keys.mkdir();records=[];payer=run(['solana-keygen','pubkey',a.payer]).strip()
 def write(name,value):(out/name).write_text(json.dumps(value,indent=2)+'\n')
 def key(name):
  path=keys/(name+'.json');run(['solana-keygen','new','--silent','--no-bip39-passphrase','--outfile',str(path)])
  return path,run(['solana-keygen','pubkey',str(path)]).strip()
 def send(name,program,metas,data,signers=(),error=None):
  cmd=[a.hopper,'tx','send','--program',program,'--keypair',a.payer,'--rpc',H['RPC'],'--data',data.hex()]
  for meta in metas:cmd+=['--account',meta]
  for signer in signers:cmd+=['--signer',str(signer)]
  if error:cmd+=['--allow-failure']
  output=run(cmd);(out/(name+'.log')).write_text(output)
  match=re.search(r'^signature\s*:\s*(\w+)',output,re.MULTILINE);assert match,'inspect submission before retrying'
  tx=finalize(match[1]);write(name+'.transaction.json',tx)
  assert tx['meta']['err']==({'InstructionError':[0,error]} if error else None),(name,tx['meta']['err'])
  r=dict(name=name,signature=match[1],slot=tx['slot'],error=tx['meta']['err'],fee=tx['meta']['fee'],computeUnits=tx['meta'].get('computeUnitsConsumed'))
  records.append(r);write('progress.json',records);print(name+': finalized',flush=True);return r
 def dumps(phase):
  for kind in ['native','runtime']:
   path=out/(phase+'-'+kind+'.so');run(['solana','program','dump',getattr(a,kind),str(path),'--url',H['RPC'],'--keypair',a.payer,'--commitment','finalized'])
   assert path.read_bytes()==Path(getattr(a,kind+'_elf')).read_bytes()
 dumps('before')
 _,recipient=key('recipient');fundkey,funder=key('funder')
 for name,address,amount in [('recipient',recipient,1_000_000),('funder',funder,10_000_000)]:
  send('fund-'+name,H['SYSTEM'],['payer:sw',address+':w'],struct.pack('<IQ',2,amount))
 rent={n:rpc('getMinimumBalanceForRentExemption',[n]) for n in [8,16,32]}
 for kind in ['runtime','native']:
  program=getattr(a,kind);statekey,state=key(kind+'-state')
  send('create-'+kind,H['SYSTEM'],['payer:sw',state+':sw'],struct.pack('<IQQ',0,rent[16],16)+H['pubkey_bytes'](program),[statekey])
  addresses=[payer,recipient,funder,state,program,H['SYSTEM']]
  def snapshot():return rpc('getMultipleAccounts',[addresses,dict(encoding='base64',commitment='finalized',minContextSlot=records[-1]['slot'])])['value']
  if kind=='runtime':cases=[(str(t),bytes([t]),None) for t in [0,1,2,3,4,6,5]]
  else:cases=[(str(t),bytes([t]),None) for t in [0,1,2,3,7,8]]+[
   ('missing-signer',bytes([5,32,0]),'MissingRequiredSignature'),
   ('oversize',bytes([5,255,255]),'InvalidRealloc'),
   ('9',bytes([9]),None),('shrink',bytes([5,8,0]),None),('transfer',bytes([4]),None),('regrow',bytes([5,32,0]),None),('close',bytes([6]),None)]
  for label,data,error in cases:
   before=snapshot();tag=data[0];metas=[state+':w',recipient+('' if kind=='native' and tag==3 else ':w')];signers=[]
   if kind=='native':
    signer=label!='missing-signer';metas+=[funder+(':sw' if signer else ':w')];signers=[fundkey] if signer else []
   metas+=[H['SYSTEM']]
   record=send(kind+'-'+label,program,metas,data,signers,error);expected=copy.deepcopy(before);expected[0]['lamports']-=record['fee']
   if error is None:
    account=expected[3];raw=bytearray(base64.b64decode(account['data'][0]))
    if kind=='runtime' and tag==2:raw[:]=bytes([7])*8+bytes([9])*8
    if kind=='native' and tag==8:raw[8:16]=bytes([3])*8
    if kind=='native' and tag in [5,9]:
     length=32 if tag==9 else int.from_bytes(data[1:],'little');delta=max(0,rent[length]-account['lamports']);account['lamports']+=delta;expected[2]['lamports']-=delta
     raw=raw[:length]+bytes(max(0,length-len(raw)));account['space']=length
    if kind=='native' and tag==4:account['lamports']-=100;expected[1]['lamports']+=100
    account['data'][0]=base64.b64encode(raw).decode()
    if (kind=='native' and tag==6) or (kind=='runtime' and tag==5):
     expected[1]['lamports']+=account['lamports'];expected[3]=None
   after=snapshot();write(kind+'-'+label+'.snapshots.json',dict(addresses=addresses,before=before,expected=expected,after=after));assert after==expected,(kind,label)
   record['expectedStateVerified']=True;write('progress.json',records)
 dumps('after');assert source==run(['git','rev-parse','HEAD']).strip() and not run(['git','status','--porcelain']).strip()
 write('receipt.json',dict(schema='hopper.lifecycle-devnet.v1',sourceCommit=source,genesisHash=H['GENESIS'],programs={k:getattr(a,k) for k in ['native','runtime']},elfSha256={k:hashlib.sha256(Path(getattr(a,k+'_elf')).read_bytes()).hexdigest() for k in ['native','runtime']},transactions=records,sourceUnchangedAndClean=True,deployedElfsMatchBeforeAndAfter=True,allPassed=True))
if __name__=='__main__':main()
