from pathlib import Path
import os,subprocess,json,hashlib,shutil
root=Path.cwd();out=root/'target/hopper/refinement-release-2026-09-24/sbf-validated';out.mkdir(parents=True,exist_ok=False)
def git(*args):return subprocess.check_output(['git',*args],text=True).strip()
source=git('rev-parse','HEAD');assert not git('status','--porcelain')
host=os.environ.copy();host.update(CARGO_BUILD_JOBS='1',CARGO_INCREMENTAL='0',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0',CARGO_TARGET_DIR=str(root/'target'),CARGO_TERM_COLOR='never')
sbf=host.copy();sbf.update(RUSTUP_HOME=str(root/'target/hopper/sbf-rustup-2026-09-23'),CARGO_TARGET_DIR=str(root/'target/hopper/canonical-pda-release-2026-09-23/build'))
records=[]
def run(name,argv,env):
 print('Running '+name,flush=True)
 with (out/f'{name}.log').open('w',encoding='utf-8') as log:rc=subprocess.run(argv,cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT).returncode
 if rc:print((out/f'{name}.log').read_text()[-6000:],flush=True);raise SystemExit(rc)
 print(name+' passed',flush=True)
for arch in ['v0','v3']:
 for name,manifest in [('mint-plan','bench/mint-plan/program/Cargo.toml'),('canonical-pda','bench/canonical-pda/program/Cargo.toml')]+([('cicada','examples/hopper-cicada/Cargo.toml'),('cicada-route','examples/hopper-cicada-route-fixture/Cargo.toml'),('cicada-canonical-route','examples/hopper-cicada-canonical-route-fixture/Cargo.toml'),('escrow','examples/hopper-escrow/Cargo.toml')] if arch=='v0' else []):
  dest=out/f'{name}-{arch}';dest.mkdir()
  argv=['cargo-build-sbf','--arch',arch,'--tools-version','v1.54','--manifest-path',manifest,'--sbf-out-dir',str(dest),'--','--locked','--offline']
  run(name+'-'+arch+'-build',argv,sbf)
  elf=next(dest.glob('*.so'));records.append({'name':name,'arch':arch,'argv':argv,'exitCode':0,'elf':str(elf.relative_to(root)),'bytes':elf.stat().st_size,'sha256':hashlib.sha256(elf.read_bytes()).hexdigest()})
for arch in ['v0','v3']:
 for name,test,var in [('mint-plan','mint_plan_sbf','HOPPER_MINT_PLAN_SBF'),('canonical-pda','canonical_pda_sbf','HOPPER_CANONICAL_PDA_SBF')]:
  env=host.copy();env[var]=str(next((out/f'{name}-{arch}').glob('*.so')))
  run(name+'-'+arch+'-tests',['cargo','+1.96.0','test','--manifest-path','bench/framework-comparison/verifier/Cargo.toml','--test',test,'--locked','--offline','--','--ignored','--nocapture'],env)
for name in ['cicada','cicada-route','cicada-canonical-route']:
 elf=next((out/f'{name}-v0').glob('*.so'));shutil.copyfile(elf,root/'target/deploy'/elf.name)
host['HOPPER_REQUIRE_CICADA_SBF']='1'
run('cicada-lifecycle',['cargo','+1.96.0','test','-p','hopper-cicada','--test','lifecycle_sbf_e2e','--locked','--offline','--','--nocapture'],host)
assert source==git('rev-parse','HEAD') and not git('status','--porcelain')
(out/'gates.json').write_text(json.dumps({'sourceCommit':source,'sourceUnchangedAndClean':True,'builds':records,'testSuites':['mint-plan-v0','canonical-pda-v0','mint-plan-v3','canonical-pda-v3','cicada-lifecycle'],'allPassed':True},indent=2)+'\n',encoding='utf-8')
