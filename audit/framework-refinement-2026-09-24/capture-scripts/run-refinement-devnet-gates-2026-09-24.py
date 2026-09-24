from pathlib import Path
import subprocess,json,runpy
root=Path.cwd();base=root/'target/hopper/refinement-release-2026-09-24';out=base/'deployment'
h=runpy.run_path(str(root/'scripts/test-runtime-gate-devnet.py'));assert h['rpc']('getGenesisHash',[])==h['GENESIS']
main_payer=root/'target/hopper/devnet-release/payer.json';mint_payer=out/'mint-test-payer-v2.json'
def run(name,args):
 with (base/f'{name}.log').open('w',encoding='utf-8') as log:rc=subprocess.run(args,cwd=root,stdout=log,stderr=subprocess.STDOUT).returncode
 if rc:print((base/f'{name}.log').read_text()[-3500:],flush=True);raise SystemExit(rc)
 print(name+' passed',flush=True)
run('mint-test-keygen',['solana-keygen','new','--no-bip39-passphrase','--silent','--outfile',str(mint_payer)])
address=subprocess.check_output(['solana-keygen','pubkey',str(mint_payer)],text=True).strip()
run('mint-test-funding',['solana','transfer',address,'0.03','--allow-unfunded-recipient','--keypair',str(main_payer),'--url',h['RPC'],'--commitment','finalized','--output','json'])
print('Running finalized mint cases with a dedicated test payer.',flush=True)
run('mint-devnet-verified',['py','-X','utf8','scripts/test-mint-plan-devnet.py','--program','GJHAaGgruxJiToRou1HE7Y8csHNunAuPthgc8RVSwACi','--payer',str(mint_payer),'--hopper','target/debug/hopper.exe','--elf',str(base/'sbf-validated/mint-plan-v0/hopper_mint_plan_fixture.so'),'--out',str(base/'mint-devnet-verified')])
for name,script in [('pda-init-devnet','test-pda-initialization-devnet.py'),('pda-readonly-devnet','test-canonical-pda-devnet.py')]:
 run(name,['py','-X','utf8','scripts/'+script,'--payer',str(main_payer),'--hopper','target/debug/hopper.exe','--elf',str(out/'hopper_canonical_pda_fixture.padded.so'),'--out',str(base/name)])
print('All 45 focused devnet transactions passed.',flush=True)
