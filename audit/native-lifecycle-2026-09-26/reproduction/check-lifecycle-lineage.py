from pathlib import Path
import json,subprocess,os,hashlib
root=Path.cwd();out=root/'target/hopper/native-replacement-2026-09-26';old=json.loads((out/'apps/receipt.json').read_text());builds=[r for r in old['checks'] if 'elfSha256' in r]
env=os.environ.copy();env.update(CARGO_BUILD_JOBS='2',CARGO_INCREMENTAL='0',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0',RUSTUP_HOME=str(root/'target/hopper/sbf-rustup-2026-09-23'),CARGO_TARGET_DIR=str(root/'target/hopper/canonical-pda-release-2026-09-23/build'))
records=[]
for r in builds:
 name=r['program'];arch=r['arch'];dest=out/'lineage'/arch;dest.mkdir(parents=True,exist_ok=True)
 manifest='bench/runtime-gate/program/Cargo.toml' if name=='hopper-runtime-gate-fixture' else f'examples/{name}/Cargo.toml'
 cmd=['cargo-build-sbf','--arch',arch,'--tools-version','v1.54','--manifest-path',manifest,'--sbf-out-dir',str(dest),'--','--locked','--offline'];log=out/'lineage'/(name+'-'+arch+'.log')
 with log.open('w') as f:rc=subprocess.run(cmd,env=env,stdout=f,stderr=subprocess.STDOUT).returncode
 assert rc==0,log.read_text()[-2000:]
 digest=hashlib.sha256((dest/(name.replace('-','_')+'.so')).read_bytes()).hexdigest();assert digest==r['elfSha256'],name
 records.append(dict(**r,rebuiltBytesIdentical=True));print(name+' '+arch+': identical',flush=True)
(out/'lineage/receipt.json').write_text(json.dumps(dict(applications=records,allIdentical=True),indent=2)+'\n')
