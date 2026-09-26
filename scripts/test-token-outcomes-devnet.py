#!/usr/bin/env python3
"""Check real token receipts and rollback against an exact Hopper ELF on devnet."""
from pathlib import Path
import argparse,base64,copy,hashlib,json,re,runpy,struct
ROOT=Path(__file__).resolve().parents[1]
H=runpy.run_path(str(ROOT/'scripts/test-runtime-gate-devnet.py'))
run,rpc,finalize,pubkey_bytes=(H[k] for k in ('run','rpc','finalize','pubkey_bytes'))
RPC,GENESIS,SYSTEM=(H[k] for k in ('RPC','GENESIS','SYSTEM'))
TOKEN='TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA'
TOKEN2022='TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb'

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('program','payer','hopper','elf','out'):parser.add_argument('--'+name,required=True)
    args=parser.parse_args();out=Path(args.out).resolve()
    assert out.is_relative_to(ROOT/'target') and rpc('getGenesisHash',[])==GENESIS
    assert not run(['git','status','--porcelain']).strip(),'commit source first'
    source=run(['git','rev-parse','HEAD']).strip();out.mkdir(parents=True,exist_ok=False);(out/'keys').mkdir()
    elf=Path(args.elf).read_bytes();addresses={'payer':run(['solana-keygen','pubkey',args.payer]).strip()};keys={};records=[]
    def write(name,value):(out/name).write_text(json.dumps(value,indent=2)+'\n',encoding='utf-8')
    for name in ['recipient','extra']+[lane+'_'+item for lane in ('classic','fee') for item in ('mint','source','destination')]:
        path=out/'keys'/(name+'.json');run(['solana-keygen','new','--silent','--no-bip39-passphrase','--outfile',str(path)])
        addresses[name]=run(['solana-keygen','pubkey',str(path)]).strip();keys[name]=path
    write('accounts.json',addresses);names=list(addresses);public=list(addresses.values())
    schedule=rpc('getEpochSchedule',[])
    def epoch(slot):
        assert slot>=schedule['firstNormalSlot']
        return schedule['firstNormalEpoch']+(slot-schedule['firstNormalSlot'])//schedule['slotsPerEpoch']
    def snapshot():
        return dict(zip(names,rpc('getMultipleAccounts',[public,dict(encoding='base64',commitment='finalized',minContextSlot=records[-1]['slot'] if records else 0)])['value']))
    def fresh(owner,rent,data):return dict(owner=owner,lamports=rent,data=[base64.b64encode(data).decode(),'base64'],executable=False,rentEpoch=(1<<64)-1,space=len(data))
    def data(account):return bytearray(base64.b64decode(account['data'][0]))
    def setdata(account,value):account['data'][0]=base64.b64encode(value).decode()
    def delta(account,n):
        value=data(account);old=struct.unpack_from('<Q',value,64)[0];struct.pack_into('<Q',value,64,old+n);setdata(account,value)
    def send(name,program,metas,payload,signers=(),change=None,error=None,post_cpi=False,returned=None):
        before=snapshot();command=[args.hopper,'tx','send','--program',program,'--keypair',args.payer,'--rpc',RPC,'--data',payload.hex()]
        for key,flags in metas:command+=['--account',addresses.get(key,key)+(':'+flags if flags else '')]
        for signer in signers:command+=['--signer',str(keys[signer])]
        if error is not None:command+=['--allow-failure']
        output=run(command);(out/(name+'.log')).write_text(output,encoding='utf-8');match=re.search(r'^signature\s*:\s*(\w+)',output,re.MULTILINE)
        assert match,'no signature: inspect public state before retrying'
        tx=finalize(match[1]);write(name+'.transaction.json',tx);actual=tx['meta']['err']
        assert actual==(None if error is None else {'InstructionError':[0,error]}),(name,actual,error)
        record=dict(name=name,signature=match[1],slot=tx['slot'],error=actual,computeUnits=tx['meta'].get('computeUnitsConsumed'),fee=tx['meta']['fee']);records.append(record)
        expected=copy.deepcopy(before)
        if change:change(expected,tx)
        expected['payer']['lamports']-=record['fee'];after=snapshot()
        write(name+'.snapshots.json',dict(before=before,expected=expected,after=after));assert after==expected,(name,'unexpected complete account state')
        if post_cpi:
            assert error is not None and any(line in tx['meta']['logMessages'] for line in [f'Program {TOKEN} success',f'Program {TOKEN2022} success'])
            record['rollbackAfterSuccessfulTokenCpi']=True
        if returned is not None:
            result=tx['meta']['returnData'];assert result['programId']==args.program and base64.b64decode(result['data'][0])==struct.pack('<QQ',*returned)
        record['expectedFullSnapshotsVerified']=True;write('progress.json',records)
        print(f"{name}: finalized, {record['computeUnits']} CU, complete state verified",flush=True)
    def dump(phase):
        path=out/(phase+'-onchain.so');run(['solana','program','dump',args.program,str(path),'--url',RPC,'--keypair',args.payer,'--commitment','finalized']);assert path.read_bytes()==elf
    dump('before')
    for key in ('recipient','extra'):
        def fund(e,tx,key=key):e['payer']['lamports']-=1_000_000;e[key]=fresh(SYSTEM,1_000_000,b'')
        send('fund-'+key,SYSTEM,[('payer','sw'),(key,'w')],struct.pack('<IQ',2,1_000_000),change=fund)
    def move(e,tx):e['payer']['lamports']-=100;e['recipient']['lamports']+=100
    send('dedup-reordered-extra',args.program,[('payer','sw'),('recipient','w'),('extra',''),(SYSTEM,'')],bytes([1])+struct.pack('<Q',100),change=move)
    rents={n:rpc('getMinimumBalanceForRentExemption',[n,{'commitment':'finalized'}]) for n in (82,165,178,278)}
    for lane,token,extended in [('classic',TOKEN,False),('fee',TOKEN2022,True)]:
        mint=lane+'_mint';src=lane+'_source';dst=lane+'_destination'
        for key,size in [(mint,278 if extended else 82),(src,178 if extended else 165),(dst,178 if extended else 165)]:
            def create(e,tx,key=key,size=size):e['payer']['lamports']-=rents[size];e[key]=fresh(token,rents[size],bytes(size))
            send('create-'+key,SYSTEM,[('payer','sw'),(key,'sw')],struct.pack('<IQQ',0,rents[size],size)+pubkey_bytes(token),[key],create)
        if extended:
            def fee_config(e,tx):
                value=data(e[mint]);struct.pack_into('<HH',value,166,1,108)
                for pos in (242,260):struct.pack_into('<QQH',value,pos,epoch(tx['slot']),100,100)
                setdata(e[mint],value)
            send('initialize-fee-config',token,[(mint,'w')],bytes([26,0,0,0])+struct.pack('<HQ',100,100),change=fee_config)
        def init_mint(e,tx):
            value=data(e[mint]);struct.pack_into('<I',value,0,1);value[4:36]=pubkey_bytes(addresses['payer']);value[44:46]=bytes([6,1])
            if extended:value[165]=1
            setdata(e[mint],value)
        send('initialize-'+mint,token,[(mint,'w')],bytes([20,6])+pubkey_bytes(addresses['payer'])+bytes([0]),change=init_mint)
        for key in (src,dst):
            def init_token(e,tx,key=key):
                value=data(e[key]);value[:32]=pubkey_bytes(addresses[mint]);value[32:64]=pubkey_bytes(addresses['payer']);value[108]=1
                if extended:value[165]=2;struct.pack_into('<HH',value,166,2,8)
                setdata(e[key],value)
            send('initialize-'+key,token,[(key,'w'),(mint,'')],bytes([18])+pubkey_bytes(addresses['payer']),change=init_token)
        def mint_tokens(e,tx):
            value=data(e[mint]);struct.pack_into('<Q',value,36,1000);setdata(e[mint],value);delta(e[src],1000)
        send('mint-'+src,token,[(mint,'w'),(src,'w'),('payer','s')],bytes([14])+struct.pack('<Q',1000)+bytes([6]),change=mint_tokens)
        for label,expected,minimum,actual,error,post_cpi in [
            ('receipt',100,99 if extended else 100,100,None,False),
            ('exact',100,100,100,'InvalidAccountData' if extended else None,extended),
            ('short-debit',100,1,99,'InvalidAccountData',True),
            ('excess-debit',100,1,101,'InvalidAccountData',True),
            ('no-movement',100,1,0,'InvalidAccountData',True),
            ('zero-policy',0,0,0,'InvalidArgument',False),
            ('invalid-minimum',100,101,100,'InvalidArgument',False),
        ]:
            def transfer(e,tx):
                delta(e[src],-actual);delta(e[dst],actual-int(extended))
                if extended:
                    value=data(e[dst]);withheld=struct.unpack_from('<Q',value,170)[0];struct.pack_into('<Q',value,170,withheld+1);setdata(e[dst],value)
            send(lane+'-'+label,args.program,[(src,'w'),(mint,''),(dst,'w'),('payer','s'),(token,'')],bytes([0])+struct.pack('<QQQ',expected,minimum,actual),change=transfer if error is None else None,error=error,post_cpi=post_cpi,returned=(actual,actual-int(extended)) if error is None else None)
        send(lane+'-self-transfer',args.program,[(src,'w'),(mint,''),(src,'w'),('payer','s'),(token,'')],bytes([0])+struct.pack('<QQQ',100,1,100),error='InvalidArgument')
    dump('after')
    assert run(['git','rev-parse','HEAD']).strip()==source and not run(['git','status','--porcelain']).strip()
    write('receipt.json',dict(schema='hopper.token-outcomes-devnet.v1',sourceCommit=source,rpcEndpoint=RPC,genesisHash=GENESIS,commitment='finalized',programId=args.program,elfSha256=hashlib.sha256(elf).hexdigest(),deployedElfMatchesBeforeAndAfter=True,allPassed=True,transactions=records))

if __name__=='__main__':main()
