#!/usr/bin/env python3
"""Verify exact deployed governance/native ELFs and public devnet account effects.

Keypairs stay under ignored target/. Refused transactions must preserve all
observed accounts except payer fees. Clock writes must lie between independently
observed finalized Clock values; every other state byte is checked exactly.
"""
from pathlib import Path
import argparse,base64,copy,hashlib,json,re,runpy,struct
ROOT=Path(__file__).resolve().parents[1]
H=runpy.run_path(str(ROOT/'scripts/test-runtime-gate-devnet.py'))
run,rpc,finalize,keybytes=(H[k] for k in ('run','rpc','finalize','pubkey_bytes'))
RPC,GENESIS,SYSTEM=(H[k] for k in ('RPC','GENESIS','SYSTEM'))
CLOCK='SysvarC1ock11111111111111111111111111111111'
def words(*v):return struct.pack('<'+'Q'*len(v),*v)
def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['treasury','multisig','native','payer','hopper','elf-dir','layouts','out']:p.add_argument('--'+name,required=True)
    a=p.parse_args();out=Path(a.out).resolve();assert out.is_relative_to(ROOT/'target')
    assert rpc('getGenesisHash',[])==GENESIS
    assert not run(['git','status','--porcelain']).strip(),'commit source first'
    source=run(['git','rev-parse','HEAD']).strip();out.mkdir(parents=True,exist_ok=False);(out/'keys').mkdir()
    layouts=json.loads(Path(a.layouts).read_text());headers={k:bytes.fromhex(v) for k,v in layouts.items() if isinstance(v,str)}
    addresses={'payer':run(['solana-keygen','pubkey',a.payer]).strip(),'system':SYSTEM};keys={}
    for name in ['alice','bob','carol','outsider','destination','multisig','treasury','cooldown','payout','stale','revoked','future','expired']:
        path=out/'keys'/(name+'.json');run(['solana-keygen','new','--silent','--no-bip39-passphrase','--outfile',str(path)])
        addresses[name]=run(['solana-keygen','pubkey',str(path)]).strip();keys[name]=path
    def write(name,value):(out/name).write_text(json.dumps(value,indent=2)+'\n',encoding='utf-8')
    write('accounts.json',addresses);records=[];names=list(addresses)
    def snapshot():
        result=rpc('getMultipleAccounts',[[addresses[k] for k in names]+[CLOCK],dict(encoding='base64',commitment='finalized',minContextSlot=records[-1]['slot'] if records else 0)])
        clock=struct.unpack_from('<q',base64.b64decode(result['value'][-1]['data'][0]),32)[0]
        return dict(zip(names,result['value'][:-1])),clock
    def fresh(owner,rent,data):return dict(owner=owner,lamports=rent,data=[base64.b64encode(data).decode(),'base64'],executable=False,rentEpoch=(1<<64)-1,space=len(data))
    def data(account):return bytearray(base64.b64decode(account['data'][0]))
    def setdata(account,value):account['data'][0]=base64.b64encode(value).decode()
    def put(account,offset,n):v=data(account);struct.pack_into('<Q',v,offset,n);setdata(account,v)
    def move(e,src,dst,n):e[src]['lamports']-=n;e[dst]['lamports']+=n
    def send(name,program,metas,payload,signers=(),change=None,refused=False,clock_write=None,native_tag=None):
        before,clock_before=snapshot()
        cmd=[a.hopper,'tx','send','--program',program,'--keypair',a.payer,'--rpc',RPC,'--data',payload.hex()]
        for key,flags in metas:cmd+=['--account',addresses[key]+(':'+flags if flags else '')]
        for key in signers:cmd+=['--signer',str(keys[key])]
        if refused:cmd+=['--allow-failure']
        output=run(cmd);(out/(name+'.log')).write_text(output,encoding='utf-8')
        match=re.search(r'^signature\s*:\s*(\w+)',output,re.MULTILINE);assert match,'inspect submission before retrying'
        tx=finalize(match[1]);write(name+'.transaction.json',tx);err=tx['meta']['err']
        assert (isinstance(err,dict) and 'InstructionError' in err) if refused else err is None,(name,err)
        record=dict(name=name,signature=match[1],slot=tx['slot'],error=err,fee=tx['meta']['fee'],computeUnits=tx['meta'].get('computeUnitsConsumed'))
        records.append(record);after,clock_after=snapshot();expected=copy.deepcopy(before)
        if change:change(expected)
        if clock_write:
            key,offset=clock_write;stamp=struct.unpack_from('<Q',data(after[key]),offset)[0]
            assert clock_before<=stamp<=clock_after,(name,clock_before,stamp,clock_after)
            put(expected[key],offset,stamp);record['clockWriteWithinObservedBounds']=True
        expected['payer']['lamports']-=record['fee']
        write(name+'.snapshots.json',dict(before=before,expected=expected,after=after,clockBefore=clock_before,clockAfter=clock_after))
        assert after==expected,(name,'account state mismatch; inspect saved snapshots')
        if native_tag is not None:
            assert not tx['meta'].get('innerInstructions'), 'preflight should not invoke a callee'
            if native_tag<2:
                returned=tx['meta']['returnData'];assert returned['programId']==program
                assert base64.b64decode(returned['data'][0])==bytes([native_tag,0xAC])
            else:assert record['computeUnits']<1000,'abort consumed excessive compute'
        record['expectedStateVerified']=True;write('progress.json',records)
        print(f"{name}: finalized; {record['computeUnits']} CU; state verified",flush=True)
    programs={'treasury':(a.treasury,'hopper_treasury.so'),'multisig':(a.multisig,'hopper_bounded_multisig.so'),'native':(a.native,'hopper_checked_cpi_fixture.so')}
    def dumps(phase):
        for name,(program,filename) in programs.items():
            dest=out/(phase+'-'+name+'.so');run(['solana','program','dump',program,str(dest),'--url',RPC,'--keypair',a.payer,'--commitment','finalized'])
            assert dest.read_bytes()==(Path(a.elf_dir)/filename).read_bytes()
    dumps('before')
    for key in ['alice','bob','carol','outsider','destination']:
        def fund(e,key=key):e['payer']['lamports']-=1_000_000;e[key]=fresh(SYSTEM,1_000_000,b'')
        send('fund-'+key,SYSTEM,[('payer','sw'),(key,'w')],struct.pack('<IQ',2,1_000_000),change=fund)
    for tag in range(4):send('native-'+str(tag),a.native,[('outsider','w'),('destination','w')],bytes([tag]),refused=tag>=2,native_tag=tag)
    rents={n:rpc('getMinimumBalanceForRentExemption',[n,{'commitment':'finalized'}]) for n in [169,layouts['multisigSize'],layouts['payoutSize']]}
    members=['alice','bob','carol'];epoch=0;threshold=2
    def multi_bytes(label='ops',old=None):
        tail=struct.pack('<H',len(label))+label.encode()+struct.pack('<H',len(members))+b''.join(keybytes(addresses[k]) for k in members)
        prefix=headers['multisig']+words(threshold,epoch)+struct.pack('<I',len(tail))+tail
        return prefix+(bytes(old[len(prefix):]) if old is not None else bytes(layouts['multisigSize']-len(prefix)))
    init=bytes([2])+words(2)+struct.pack('<H',3)+b'ops'+struct.pack('<H',3)+b''.join(keybytes(addresses[k]) for k in members)
    def create_multi(e):n=rents[layouts['multisigSize']];e['payer']['lamports']-=n;e['multisig']=fresh(a.multisig,n,multi_bytes())
    send('multisig-initialize',a.multisig,[('payer','sw'),('multisig','sw'),('system','')],init,['multisig'],create_multi)
    send('multisig-deposit',a.multisig,[('payer','sw'),('multisig','w'),('system','')],bytes([4])+words(500_000),change=lambda e:move(e,'payer','multisig',500_000))
    approvals=[('multisig','w'),('alice','s'),('bob','s')]
    send('multisig-one-approval',a.multisig,approvals[:-1],bytes([0])+b'\x03\x00dao',['alice'],refused=True)
    send('multisig-outsider',a.multisig,[('multisig','w'),('alice','s'),('outsider','s')],bytes([0])+b'\x03\x00dao',['alice','outsider'],refused=True)
    send('multisig-duplicate',a.multisig,[('multisig','w'),('alice','s'),('alice','s')],bytes([0])+b'\x03\x00dao',['alice'],refused=True)
    send('multisig-rename',a.multisig,approvals,bytes([0])+b'\x03\x00dao',['alice','bob'],lambda e:setdata(e['multisig'],multi_bytes('dao')))
    withdraw=[('multisig','w'),('destination','w'),('alice','s'),('bob','s')]
    send('multisig-withdraw',a.multisig,withdraw,bytes([3])+words(100_000),['alice','bob'],lambda e:move(e,'multisig','destination',100_000))
    send('multisig-rent-protected',a.multisig,withdraw,bytes([3])+words(400_001),['alice','bob'],refused=True)
    _,now=snapshot()
    def grant(key,start,end):
        def change(e):
            n=rents[layouts['payoutSize']];e['payer']['lamports']-=n
            value=headers['payout']+keybytes(addresses['multisig'])+keybytes(addresses['destination'])+words(epoch,50_000,start,end)+b'\0'
            assert len(value)==layouts['payoutSize'];e[key]=fresh(a.multisig,n,value)
        send('approve-'+key,a.multisig,[('multisig','w'),(key,'sw'),('payer','sw'),('destination',''),('system',''),('alice','s'),('bob','s')],bytes([5])+words(50_000,start,end),[key,'alice','bob'],change)
    grant('payout',0,now+86400);grant('stale',0,now+86400);grant('revoked',0,now+86400);grant('future',now+86400,now+172800)
    execute=lambda key:[('multisig','w'),(key,'w'),('destination','w')]
    send('payout-before-window',a.multisig,execute('future'),bytes([6]),refused=True)
    send('payout-wrong-destination',a.multisig,[('multisig','w'),('payout','w'),('outsider','w')],bytes([6]),refused=True)
    send('payout-extra-payload',a.multisig,execute('payout'),bytes([6,0]),refused=True)
    def pay(e):move(e,'multisig','destination',50_000);v=data(e['payout']);v[112]=1;setdata(e['payout'],v)
    send('payout-permissionless-execution',a.multisig,execute('payout'),bytes([6]),change=pay)
    send('payout-replay',a.multisig,execute('payout'),bytes([6]),refused=True)
    def revoke(e):e['multisig']['lamports']+=e['revoked']['lamports'];e['revoked']=None
    send('payout-revoke',a.multisig,[('multisig','w'),('revoked','w'),('alice','s'),('bob','s')],bytes([7]),['alice','bob'],revoke)
    send('payout-revoked-execution',a.multisig,execute('revoked'),bytes([6]),refused=True)
    epoch+=1;members.remove('carol')
    send('multisig-remove-member',a.multisig,approvals,bytes([9])+keybytes(addresses['carol']),['alice','bob'],lambda e:setdata(e['multisig'],multi_bytes('dao',data(e['multisig']))))
    send('payout-stale-after-removal',a.multisig,execute('stale'),bytes([6]),refused=True)
    send('multisig-removed-member',a.multisig,[('multisig','w'),('alice','s'),('carol','s')],bytes([8]),['alice','carol'],refused=True)
    epoch+=1
    send('multisig-invalidate-payouts',a.multisig,approvals,bytes([8]),['alice','bob'],lambda e:put(e['multisig'],24,epoch))
    epoch+=1;threshold=1
    send('multisig-threshold-change',a.multisig,approvals,bytes([10])+words(1),['alice','bob'],lambda e:(put(e['multisig'],16,threshold),put(e['multisig'],24,epoch)))
    for key,delay in [('treasury',0),('cooldown',86400)]:
        def create(e,key=key,delay=delay):
            n=rents[169];e['payer']['lamports']-=n
            value=headers['core']+keybytes(addresses['payer'])+words(0)+headers['permissions']+keybytes(addresses['payer'])+b'\0'+words(150_000)+headers['budget']+words(200_000,0,0,delay,0)
            assert len(value)==169;e[key]=fresh(a.treasury,n,value)
        send(key+'-initialize',a.treasury,[('payer','sw'),(key,'sw'),('system','')],bytes([0])+words(200_000,150_000,delay),[key],create)
        def deposit(e,key=key):move(e,'payer',key,300_000);put(e[key],48,300_000)
        send(key+'-deposit',a.treasury,[('payer','sw'),(key,'w'),('system','')],bytes([1])+words(300_000),change=deposit)
        def spend(e,key=key):move(e,key,'destination',100_000);put(e[key],137,100_000)
        send(key+'-withdraw',a.treasury,[('payer','s'),(key,'w'),('destination','w')],bytes([2])+words(100_000),change=spend,clock_write=(key,161))
    send('treasury-cooldown-enforced',a.treasury,[('payer','s'),('cooldown','w'),('destination','w')],bytes([2])+words(1),refused=True)
    send('treasury-budget-enforced',a.treasury,[('payer','s'),('treasury','w'),('destination','w')],bytes([2])+words(100_001),refused=True)
    send('treasury-operator-enforced',a.treasury,[('outsider','s'),('treasury','w'),('destination','w')],bytes([2])+words(1),['outsider'],refused=True)
    def freeze(e):v=data(e['treasury']);v[104]=1;setdata(e['treasury'],v)
    send('treasury-freeze',a.treasury,[('payer','s'),('treasury','w')],bytes([3,1]),change=freeze)
    send('treasury-frozen',a.treasury,[('payer','s'),('treasury','w'),('destination','w')],bytes([2])+words(1),refused=True)
    dumps('after');assert source==run(['git','rev-parse','HEAD']).strip() and not run(['git','status','--porcelain']).strip()
    write('receipt.json',dict(schema='hopper.governance-native-devnet.v1',sourceCommit=source,genesisHash=GENESIS,programs={k:dict(programId=v[0],elfSha256=hashlib.sha256((Path(a.elf_dir)/v[1]).read_bytes()).hexdigest()) for k,v in programs.items()},transactions=records,sourceUnchangedAndClean=True,deployedElfsMatchBeforeAndAfter=True,allPassed=True))
if __name__=='__main__':main()
