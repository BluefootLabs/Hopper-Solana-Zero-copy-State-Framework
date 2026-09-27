#!/usr/bin/env python3
"""Check processed-sibling APIs against a deployed, byte-matched SBF fixture."""
from pathlib import Path
import argparse,copy,hashlib,json,re,runpy
ROOT=Path(__file__).resolve().parents[1]
H=runpy.run_path(str(ROOT/'scripts/test-runtime-gate-devnet.py'))
run,rpc,finalize=(H[k] for k in ('run','rpc','finalize'))

def main():
    p=argparse.ArgumentParser(description=__doc__)
    for k in ['program','elf','payer','hopper','out']:p.add_argument('--'+k,required=True)
    a=p.parse_args();out=Path(a.out).resolve();assert out.is_relative_to(ROOT/'target')
    assert rpc('getGenesisHash',[])==H['GENESIS']
    assert not run(['git','status','--porcelain']).strip()
    source=run(['git','rev-parse','HEAD']).strip();out.mkdir(parents=True,exist_ok=False)
    payer=run(['solana-keygen','pubkey',a.payer]).strip()
    addresses=[payer,a.program];records=[];elf=Path(a.elf).read_bytes()
    def write(name,value):(out/name).write_text(json.dumps(value,indent=2)+'\n')
    def snapshot():
        return rpc('getMultipleAccounts',[addresses,dict(encoding='base64',commitment='finalized',minContextSlot=records[-1]['slot'] if records else 0)])['value']
    def dump(phase):
        path=out/(phase+'.so')
        run(['solana','program','dump',a.program,str(path),'--url',H['RPC'],'--keypair',a.payer,'--commitment','finalized'])
        assert path.read_bytes()==elf
    dump('before')
    names=['native-owned','runtime-owned','runtime-bounded','caller-buffers','short-data','missing','empty','exclude-child','large-cpi-data','reverse-order','short-metas']
    for case,name in enumerate(names,1):
        before=snapshot();error='AccountDataTooSmall' if case in [5,11] else None
        cmd=[a.hopper,'tx','send','--program',a.program,'--keypair',a.payer,'--rpc',H['RPC'],'--data',bytes([20,case]).hex(),'--account',payer+':sw','--account',a.program]
        if error:cmd+=['--allow-failure']
        output=run(cmd);(out/(name+'.log')).write_text(output)
        match=re.search(r'^signature\s*:\s*(\w+)',output,re.MULTILINE);assert match,'inspect submission before retrying'
        tx=finalize(match[1]);write(name+'.transaction.json',tx)
        expected_error={'InstructionError':[0,error]} if error else None
        assert tx['meta']['err']==expected_error,(name,tx['meta']['err'])
        logs=tx['meta']['logMessages'];assert f'Program {a.program} invoke [2]' in logs
        if case==8:assert f'Program {a.program} invoke [3]' in logs
        assert len(tx['meta']['innerInstructions'])==1
        record=dict(name=name,case=case,signature=match[1],slot=tx['slot'],error=tx['meta']['err'],fee=tx['meta']['fee'],computeUnits=tx['meta'].get('computeUnitsConsumed'))
        records.append(record);after=snapshot();expected=copy.deepcopy(before);expected[0]['lamports']-=record['fee']
        write(name+'.snapshots.json',dict(addresses=addresses,before=before,expected=expected,after=after))
        assert after==expected;record['expectedStateVerified']=True;write('progress.json',records)
        print(f"{name}: finalized; {record['computeUnits']} CU; expected result and full snapshots verified",flush=True)
    dump('after');assert source==run(['git','rev-parse','HEAD']).strip() and not run(['git','status','--porcelain']).strip()
    write('receipt.json',dict(schema='hopper.sibling-introspection-devnet.v1',sourceCommit=source,genesisHash=H['GENESIS'],programId=a.program,elfSha256=hashlib.sha256(elf).hexdigest(),transactions=records,sourceUnchangedAndClean=True,deployedElfMatchesBeforeAndAfter=True,allPassed=True))
if __name__=='__main__':main()
