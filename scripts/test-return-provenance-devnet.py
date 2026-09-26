#!/usr/bin/env python3
"""Check direct/nested CPI return provenance against three deployed fixture ELFs."""
from pathlib import Path
import argparse,base64,copy,hashlib,json,re,runpy
ROOT=Path(__file__).resolve().parents[1]
H=runpy.run_path(str(ROOT/'scripts/test-runtime-gate-devnet.py'))
run,rpc,finalize=(H[k] for k in ('run','rpc','finalize'))

def main():
    p=argparse.ArgumentParser(description=__doc__)
    for k in ['driver','callee','nested','elf','payer','hopper','out']:p.add_argument('--'+k,required=True)
    a=p.parse_args();out=Path(a.out).resolve();assert out.is_relative_to(ROOT/'target')
    assert rpc('getGenesisHash',[])==H['GENESIS']
    assert not run(['git','status','--porcelain']).strip()
    source=run(['git','rev-parse','HEAD']).strip();out.mkdir(parents=True,exist_ok=False)
    payer=run(['solana-keygen','pubkey',a.payer]).strip()
    addresses=[payer,a.driver,a.callee,a.nested];records=[];elf=Path(a.elf).read_bytes()
    def write(name,value):(out/name).write_text(json.dumps(value,indent=2)+'\n')
    def snapshot():
        return rpc('getMultipleAccounts',[addresses,dict(encoding='base64',commitment='finalized',minContextSlot=records[-1]['slot'] if records else 0)])['value']
    def dumps(phase):
        for name in ['driver','callee','nested']:
            path=out/(phase+'-'+name+'.so')
            run(['solana','program','dump',getattr(a,name),str(path),'--url',H['RPC'],'--keypair',a.payer,'--commitment','finalized'])
            assert path.read_bytes()==elf
    dumps('before')
    for tag,name,error in [(4,'direct',None),(5,'short','AccountDataTooSmall'),(6,'empty','InvalidAccountData'),(7,'nested','IncorrectProgramId')]:
        before=snapshot();cmd=[a.hopper,'tx','send','--program',a.driver,'--keypair',a.payer,'--rpc',H['RPC'],'--data',bytes([tag]).hex(),'--account',a.callee]
        if tag==7:cmd+=['--account',a.nested]
        if error:cmd+=['--allow-failure']
        output=run(cmd);(out/(name+'.log')).write_text(output)
        match=re.search(r'^signature\s*:\s*(\w+)',output,re.MULTILINE);assert match,'inspect submission before retrying'
        tx=finalize(match[1]);write(name+'.transaction.json',tx)
        expected_error={'InstructionError':[0,error]} if error else None
        assert tx['meta']['err']==expected_error,(name,tx['meta']['err'])
        if tag==4:
            returned=tx['meta']['returnData'];assert returned['programId']==a.callee
            assert base64.b64decode(returned['data'][0])==(42).to_bytes(8,'little')
        if tag==7:
            assert f'Program {a.callee} invoke [2]' in tx['meta']['logMessages']
            assert f'Program {a.nested} invoke [3]' in tx['meta']['logMessages']
        record=dict(name=name,signature=match[1],slot=tx['slot'],error=tx['meta']['err'],fee=tx['meta']['fee'],computeUnits=tx['meta'].get('computeUnitsConsumed'))
        records.append(record);after=snapshot();expected=copy.deepcopy(before);expected[0]['lamports']-=record['fee']
        write(name+'.snapshots.json',dict(addresses=addresses,before=before,expected=expected,after=after))
        assert after==expected;record['expectedStateVerified']=True;write('progress.json',records)
        print(f"{name}: finalized; {record['computeUnits']} CU; producer/type/state verified",flush=True)
    dumps('after');assert source==run(['git','rev-parse','HEAD']).strip() and not run(['git','status','--porcelain']).strip()
    write('receipt.json',dict(schema='hopper.return-provenance-devnet.v1',sourceCommit=source,genesisHash=H['GENESIS'],programs={k:getattr(a,k) for k in ['driver','callee','nested']},elfSha256=hashlib.sha256(elf).hexdigest(),transactions=records,sourceUnchangedAndClean=True,deployedElfsMatchBeforeAndAfter=True,allPassed=True))
if __name__=='__main__':main()
