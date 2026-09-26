#!/usr/bin/env python3
"""Run funded escrow custody/settlement checks against an exact ELF on devnet."""
from pathlib import Path
import argparse, base64, copy, hashlib, json, re, runpy, struct
ROOT=Path(__file__).resolve().parents[1]
H=runpy.run_path(str(ROOT/'scripts/test-runtime-gate-devnet.py'))
run,rpc,finalize,pubkey_bytes=(H[k] for k in ('run','rpc','finalize','pubkey_bytes'))
RPC,GENESIS,SYSTEM=(H[k] for k in ('RPC','GENESIS','SYSTEM'))
TOKEN='TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA'
def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('program','payer','hopper','elf','out','header-hex'):
        parser.add_argument('--'+name,required=True)
    args=parser.parse_args()
    header=bytes.fromhex(args.header_hex)
    assert len(header)==16 and header[:2]==bytes([2,2])
    out=Path(args.out).resolve()
    assert out.is_relative_to(ROOT/'target')
    assert rpc('getGenesisHash',[])==GENESIS
    assert not run(['git','status','--porcelain']).strip(),'commit source first'
    source=run(['git','rev-parse','HEAD']).strip()
    out.mkdir(parents=True,exist_ok=False);(out/'keys').mkdir()
    elf=Path(args.elf).read_bytes()
    addresses={'payer':run(['solana-keygen','pubkey',args.payer]).strip()}
    keys={}
    def write(name,data):
        (out/name).write_text(json.dumps(data,indent=2)+'\n',encoding='utf-8')
    for name in ('maker','taker','outsider','mint_a','mint_b','maker_a','maker_b','taker_a','taker_b','escrow_take','vault_take','escrow_cancel','vault_cancel'):
        path=out/'keys'/(name+'.json')
        run(['solana-keygen','new','--no-bip39-passphrase','--silent','--outfile',str(path)])
        addresses[name]=run(['solana-keygen','pubkey',str(path)]).strip();keys[name]=path
    for lane in ('take','cancel'):
        value=json.loads(run(['solana','find-program-derived-address',args.program,'string:escrow-vault','pubkey:'+addresses['escrow_'+lane],'--output','json','--url',RPC,'--keypair',args.payer]))
        addresses['authority_'+lane]=value['address']
    addresses.update(token=TOKEN,system=SYSTEM)
    names=list(addresses);public=list(addresses.values())
    assert len(set(public))==len(public)
    write('accounts.json',addresses)
    records=[]
    def snapshot():
        slot=records[-1]['slot'] if records else 0
        values=rpc('getMultipleAccounts',[public,dict(encoding='base64',commitment='finalized',minContextSlot=slot)])['value']
        return dict(zip(names,values))
    def fresh(owner,rent,data):
        return dict(owner=owner,lamports=rent,data=[base64.b64encode(data).decode(),'base64'],executable=False,rentEpoch=(1<<64)-1,space=len(data))
    def data(account):return bytearray(base64.b64decode(account['data'][0]))
    def setdata(account,value):account['data'][0]=base64.b64encode(value).decode()
    def delta(account,n):
        value=data(account);old=struct.unpack_from('<Q',value,64)[0]
        struct.pack_into('<Q',value,64,old+n);setdata(account,value)
    def send(name,program,metas,payload,signers,change=None,error=None):
        before=snapshot()
        command=[args.hopper,'tx','send','--program',program,'--keypair',args.payer,'--rpc',RPC,'--data',payload.hex()]
        for key,flags in metas:command+=['--account',addresses[key]+(':'+flags if flags else '')]
        for signer in signers:command+=['--signer',str(keys[signer])]
        if error is not None:command+=['--allow-failure']
        output=run(command);(out/(name+'.log')).write_text(output,encoding='utf-8')
        match=re.search(r'^signature\s*:\s*(\w+)',output,re.MULTILINE)
        assert match,'no signature: inspect before retrying'
        tx=finalize(match[1]);write(name+'.transaction.json',tx)
        actual=tx['meta']['err']
        assert actual==(None if error is None else {'InstructionError':[0,error]}),(name,actual,error)
        record=dict(name=name,signature=match[1],slot=tx['slot'],error=actual,computeUnits=tx['meta'].get('computeUnitsConsumed'),fee=tx['meta']['fee'])
        records.append(record)
        expected=copy.deepcopy(before)
        if change:change(expected)
        expected['payer']['lamports']-=record['fee']
        after=snapshot()
        write(name+'.snapshots.json',dict(before=before,expected=expected,after=after))
        assert after==expected,(name,'unexpected account state')
        record['expectedStateVerified']=True;write('progress.json',records)
        print(f"{name}: finalized, {record['computeUnits']} CU, exact state verified",flush=True)
    def dump(phase):
        path=out/(phase+'-onchain.so')
        run(['solana','program','dump',args.program,str(path),'--url',RPC,'--keypair',args.payer,'--commitment','finalized'])
        assert path.read_bytes()==elf,'deployed ELF differs'
    dump('before')
    for key,n in [('maker',20_000_000),('taker',2_000_000),('outsider',1_000_000)]:
        def fund(e,key=key,n=n):
            assert e[key] is None;e['payer']['lamports']-=n;e[key]=fresh(SYSTEM,n,b'')
        send('fund-'+key,SYSTEM,[('payer','sw'),(key,'w')],struct.pack('<IQ',2,n),[],fund)
    rents={size:rpc('getMinimumBalanceForRentExemption',[size,{'commitment':'finalized'}]) for size in (82,165,192)}
    def create(key,size):
        def change(e):
            assert e[key] is None;e['payer']['lamports']-=rents[size];e[key]=fresh(TOKEN,rents[size],bytes(size))
        send('create-'+key,SYSTEM,[('payer','sw'),(key,'sw')],struct.pack('<IQQ',0,rents[size],size)+pubkey_bytes(TOKEN),[key],change)
    for mint in ('mint_a','mint_b'):
        create(mint,82)
        def init(e,mint=mint):
            value=data(e[mint]);struct.pack_into('<I',value,0,1);value[4:36]=pubkey_bytes(addresses['payer']);value[44:46]=bytes([6,1]);setdata(e[mint],value)
        send('initialize-'+mint,TOKEN,[(mint,'w')],bytes([20,6])+pubkey_bytes(addresses['payer'])+bytes([0]),[],init)
    for token,mint,owner in [('maker_a','mint_a','maker'),('maker_b','mint_b','maker'),('taker_a','mint_a','taker'),('taker_b','mint_b','taker')]:
        create(token,165)
        def init(e,token=token,mint=mint,owner=owner):
            value=data(e[token]);value[:32]=pubkey_bytes(addresses[mint]);value[32:64]=pubkey_bytes(addresses[owner]);value[108]=1;setdata(e[token],value)
        send('initialize-'+token,TOKEN,[(token,'w'),(mint,'')],bytes([18])+pubkey_bytes(addresses[owner]),[],init)
    for token,mint in [('maker_a','mint_a'),('taker_b','mint_b')]:
        def issue(e,token=token,mint=mint):
            value=data(e[mint]);struct.pack_into('<Q',value,36,5000);setdata(e[mint],value);delta(e[token],5000)
        send('mint-'+token,TOKEN,[(mint,'w'),(token,'w'),('payer','s')],bytes([14])+struct.pack('<Q',5000)+bytes([6]),[],issue)
    def make_metas(lane):
        return [('maker','sw'),('escrow_'+lane,'sw'),('authority_'+lane,''),('vault_'+lane,'sw'),('mint_a',''),('mint_b',''),('maker_a','w'),('maker_b',''),('token',''),('system','')]
    make_data=bytes([0])+struct.pack('<QQ',1000,2000)
    send('make-insufficient-funding',args.program,make_metas('take'),bytes([0])+struct.pack('<QQ',6000,2000),['maker','escrow_take','vault_take'],error={'Custom':1})
    send('make-zero',args.program,make_metas('take'),bytes([0])+struct.pack('<QQ',0,2000),['maker','escrow_take','vault_take'],error={'Custom':6104})
    for lane in ('take','cancel'):
        escrow,vault='escrow_'+lane,'vault_'+lane
        def make(e,escrow=escrow,vault=vault,lane=lane):
            assert e[escrow] is None and e[vault] is None
            e['maker']['lamports']-=rents[192]+rents[165]
            state=header+b''.join(pubkey_bytes(addresses[k]) for k in ['maker','maker_b','mint_a','mint_b',vault])+struct.pack('<QQ',1000,2000)
            e[escrow]=fresh(args.program,rents[192],state)
            value=bytearray(165);value[:32]=pubkey_bytes(addresses['mint_a']);value[32:64]=pubkey_bytes(addresses['authority_'+lane]);value[108]=1;struct.pack_into('<Q',value,64,1000)
            e[vault]=fresh(TOKEN,rents[165],value);delta(e['maker_a'],-1000)
        send('make-'+lane,args.program,make_metas(lane),make_data,['maker',escrow,vault],make)
    take=[('taker','s'),('escrow_take','w'),('maker','w'),('authority_take',''),('vault_take','w'),('mint_a',''),('mint_b',''),('taker_a','w'),('taker_b','w'),('maker_b','w'),('maker_a','w'),('token','')]
    take_data=bytes([1])+struct.pack('<QQ',1000,2000)
    cancel=[('maker','sw'),('escrow_cancel','w'),('authority_cancel',''),('vault_cancel','w'),('mint_a',''),('maker_a','w'),('token','')]
    send('reinitialize',args.program,make_metas('take'),make_data,['maker','escrow_take','vault_take'],error='AccountAlreadyInitialized')
    send('stale-quote',args.program,take,bytes([1])+struct.pack('<QQ',1000,1),['taker'],error={'Custom':6101})
    send('unsigned-take',args.program,[('taker','')]+take[1:],take_data,[],error='MissingRequiredSignature')
    send('wrong-cancel-maker',args.program,[('outsider','sw')]+cancel[1:],bytes([2]),['outsider'],error='InvalidAccountData')
    send('cancel-trailing-data',args.program,cancel,bytes([2,0]),['maker'],error='InvalidInstructionData')
    bad=take.copy();bad[3]=('authority_cancel','')
    send('wrong-vault-authority',args.program,bad,take_data,['taker'],error='InvalidSeeds')
    bad=take.copy();bad[-1]=('system','')
    send('wrong-token-program',args.program,bad,take_data,['taker'],error='InvalidArgument')
    bad=take.copy();bad[9]=('taker_b','w')
    send('wrong-payment-recipient',args.program,bad,take_data,['taker'],error='InvalidAccountData')
    for lane in ('take','cancel'):
        vault='vault_'+lane
        def donate(e,vault=vault):delta(e['maker_a'],-37);delta(e[vault],37)
        send('donate-'+lane,TOKEN,[('maker_a','w'),('mint_a',''),(vault,'w'),('maker','s')],bytes([12])+struct.pack('<Q',37)+bytes([6]),['maker'],donate)
    def settle(e):
        delta(e['taker_b'],-2000);delta(e['maker_b'],2000);delta(e['taker_a'],1000);delta(e['maker_a'],37)
        e['maker']['lamports']+=e['escrow_take']['lamports']+e['vault_take']['lamports'];e['escrow_take']=None;e['vault_take']=None
    send('take',args.program,take,take_data,['taker'],settle)
    def refund(e):
        delta(e['maker_a'],1037);e['maker']['lamports']+=e['escrow_cancel']['lamports']+e['vault_cancel']['lamports'];e['escrow_cancel']=None;e['vault_cancel']=None
    send('cancel',args.program,cancel,bytes([2]),['maker'],refund)
    dump('after')
    assert run(['git','rev-parse','HEAD']).strip()==source and not run(['git','status','--porcelain']).strip()
    write('receipt.json',dict(schema='hopper.token-escrow-devnet.v1',sourceCommit=source,genesisHash=GENESIS,programId=args.program,elfSha256=hashlib.sha256(elf).hexdigest(),deployedElfMatchesBeforeAndAfter=True,sourceUnchangedAndClean=True,addresses=addresses,transactions=records,allPassed=True))
if __name__=='__main__':main()
