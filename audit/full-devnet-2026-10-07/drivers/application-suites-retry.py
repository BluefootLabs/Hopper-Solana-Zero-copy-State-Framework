from pathlib import Path
import json,os,time
_status=Path(__file__).parent/'worker-status'/('application-suites.json')
if _status.is_file() and os.environ.get('HOPPER_MATRIX_WORKER')!='1':
 deadline=time.monotonic()+7200
 while True:
  state=json.loads(_status.read_text())
  if state['status']=='finished':raise SystemExit(state['exitCode'])
  if time.monotonic()>deadline:raise SystemExit('Worker did not complete; inspect its transactions before retrying')
  time.sleep(5)
from pathlib import Path
import json,runpy,subprocess,sys,shutil,re
root=Path.cwd();base=root/'target/hopper/full-devnet-2026-10-07'
h=runpy.run_path(str(base/'devnet-harness.py'));source=h['SOURCE'];work=h['WORK'];env=h['ENV']
builds=json.loads((source/'target/verification/builds.json').read_text());assert len(builds)==54 and all(b['exitCode']==0 for b in builds)
rows=json.loads((source/'target/verification/application-suites.json').read_text())
def elf(package):return source/next(b for b in builds if b['label']==package)['elfs'][0]['path']
def deploy(package,label=None):
 folder,record=h['deploy'](label or package,elf(package));return record['programId']
def run(name,script,args):
 output=source/'target/verification'/('suite-'+name);log=source/'target/verification'/(name+'.suite.log')
 command=[sys.executable,'scripts/'+script,'--payer',str(h['PAYER']),'--hopper',str(h['SENDER']),'--out',str(output),*map(str,args)]
 with log.open('xb') as stream:code=subprocess.run(command,cwd=source,env=env,stdout=stream,stderr=subprocess.STDOUT).returncode
 row=dict(name=name,exitCode=code,output=str(output.relative_to(source)),log=str(log.relative_to(source)),command=command);rows.append(row)
 (source/'target/verification/application-suites.json').write_text(json.dumps(rows,indent=2)+'\n')
 print(name,'exit',code,flush=True)
 if code:raise SystemExit('Suite failed; inspect '+str(log))
 return output
singles=[
 ('sibling','hopper-sibling-introspection-fixture','test-sibling-introspection-devnet.py',[]),
 ('runtime-gate','hopper-runtime-gate-fixture','test-runtime-gate-devnet.py',['--typed-header-hex','5201000040f1d31e9ea36a9a01000000']),
 ('mint-plan','hopper-mint-plan-fixture','test-mint-plan-devnet.py',[]),
 ('token-outcomes','hopper-token-outcomes-fixture','test-token-outcomes-devnet.py',[]),
 ('named-vault','hopper-vault','test-named-vault-devnet.py',['--header-hex','0101000011a7a81acb67013101000000']),
 ('byte-allowance','hopper-byte-allowance','test-byte-allowance-devnet.py',['--header-hex','5c0100007a6ee6c48cc8cff401000000']),
 ('tail-lab','hopper-tail-lab','test-tail-lab-devnet.py',[]),
 ('token-escrow','hopper-escrow','test-token-escrow-devnet.py',['--header-hex','02020000b2e9d63057f5904701000000']),
 ('token-lab','hopper-token-lab','test-token-lab-devnet.py',[]),
]
# Run an independent bounded suite at a time; all share the same devnet fee payer.
for name,package,script,args in singles:
 if name=='sibling':
  assert json.loads((source/'target/verification/suite-sibling/receipt.json').read_text())['allPassed'];continue
 if name=='runtime-gate':
  program=json.loads((work/package/'deployment.json').read_text())['programId'];name+='-retry'
 else:program=deploy(package)
 run(name,script,['--program',program,'--elf',elf(package),*args])
native=deploy('hopper-native-lifecycle-fixture');runtime=deploy('hopper-runtime-lifecycle-fixture')
run('lifecycle','test-lifecycle-devnet.py',['--native',native,'--runtime',runtime,'--native-elf',elf('hopper-native-lifecycle-fixture'),'--runtime-elf',elf('hopper-runtime-lifecycle-fixture')])
programs=[deploy('hopper-return-provenance-fixture','return-'+kind) for kind in ['driver','callee','nested']]
run('return-provenance','test-return-provenance-devnet.py',['--driver',programs[0],'--callee',programs[1],'--nested',programs[2],'--elf',elf('hopper-return-provenance-fixture')])
programs=[deploy(p) for p in ['hopper-treasury','hopper-bounded-multisig','hopper-checked-cpi-fixture']]
flat=source/'target/verification/governance-elfs';flat.mkdir()
for p in ['hopper-treasury','hopper-bounded-multisig','hopper-checked-cpi-fixture']:shutil.copy2(elf(p),flat/elf(p).name)
layouts=source/'target/verification/governance-layouts.json'
run('governance','test-governance-devnet.py',['--treasury',programs[0],'--multisig',programs[1],'--native',programs[2],'--elf-dir',flat,'--layouts',layouts])
print('All scheduled application suites completed',flush=True)

latest={r['name'].removesuffix('-retry'):r for r in rows}
assert len(latest)==12 and all(r['exitCode']==0 for r in latest.values())
(source/'target/verification/application-suites-final.json').write_text(json.dumps(latest,indent=2)+'\n')
