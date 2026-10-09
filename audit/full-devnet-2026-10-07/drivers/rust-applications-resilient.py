from pathlib import Path
import json,hashlib,os,runpy,subprocess,sys
root=Path.cwd();base=root/'target/hopper/full-devnet-2026-10-07';h=runpy.run_path(str(base/'devnet-harness-resilient.py'));source=h['SOURCE'];out=source/'target/verification';env=h['ENV'].copy();rows=[]
builds=json.loads((out/'builds.json').read_text())
def elf(name):return source/next(b for b in builds if b['label']==name)['elfs'][0]['path']
for package,suffix in [('hopper-compact-vault','COMPACT_VAULT'),('hopper-migration','MIGRATION'),('hopper-orderbook','ORDERBOOK'),('hopper-token-2022-vault','TOKEN_2022_VAULT')]:
 folder,record=h['deploy'](package,elf(package));h['dump'](folder,record,'before')
 testenv=env.copy();testenv.update(HOPPER_DEVNET='1',HOPPER_REQUIRE_DEVNET='1',HOPPER_KEYPAIR=str(h['PAYER']),SOLANA_RPC_URL=h['RPC'],HOPPER_DEVNET_RECEIPT=str(folder/'receipt.json'));testenv['HOPPER_'+suffix+'_PROGRAM_ID']=record['programId']
 argv=['cargo','test','-j','1','-p',package,'--test','devnet','--locked','--offline','--','--nocapture']
 with (folder/'test.log').open('xb') as log:code=subprocess.run(argv,cwd=source,env=testenv,stdout=log,stderr=subprocess.STDOUT).returncode
 h['dump'](folder,record,'after');receipt=json.loads((folder/'receipt.json').read_text()) if (folder/'receipt.json').is_file() else None
 rows.append(dict(package=package,exitCode=code,receiptPresent=receipt is not None,deployment=record));h['write'](out/'rust-applications.json',rows);print(package,'devnet',code,flush=True)
 if code or receipt is None:raise SystemExit('Inspect '+str(folder/'test.log'))
# Direct runtime audit client covers contexts, dynamic tails, segment policies and field guards.
package='hopper-devnet-audit';folder,record=h['deploy'](package,elf(package));h['dump'](folder,record,'before')
runenv=env.copy();runenv.update(HOPPER_DEVNET='1',HOPPER_DEVNET_RECEIPT=str(folder/'receipt.json'))
argv=['cargo','run','-j','1','-p',package,'--features','devnet-client','--bin','devnet_audit','--locked','--offline','--','--program-id',record['programId'],'--keypair',str(h['PAYER']),'--rpc',h['RPC']]
with (folder/'test.log').open('xb') as log:code=subprocess.run(argv,cwd=source,env=runenv,stdout=log,stderr=subprocess.STDOUT).returncode
h['dump'](folder,record,'after');rows.append(dict(package=package,exitCode=code,receiptPresent=(folder/'receipt.json').is_file(),deployment=record));h['write'](out/'rust-applications.json',rows);print(package,'devnet',code,flush=True)
if code:raise SystemExit('Inspect '+str(folder/'test.log'))
