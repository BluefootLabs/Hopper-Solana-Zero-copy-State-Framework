from pathlib import Path
import os,subprocess,json,hashlib,runpy
root=Path.cwd();out=root/'target/hopper/refinement-release-2026-09-24/comparison';out.mkdir(exist_ok=True)
h=runpy.run_path(str(root/'scripts/bench-framework-comparison.py'))
def git(*args):return subprocess.check_output(['git',*args],text=True,encoding='utf-8').strip()
source=git('rev-parse','HEAD');assert not git('status','--porcelain')
env=os.environ.copy();env.update(h['RELEASE_PROFILE']);env.update(RUSTUP_HOME=str(root/'target/hopper/sbf-rustup-2026-09-23'),CARGO_TARGET_DIR=str(root/'target/hopper/canonical-pda-release-2026-09-23/build'),CARGO_BUILD_JOBS='1',CARGO_INCREMENTAL='0')
records=[];dest=out/'sbf';dest.mkdir(exist_ok=True)
for case in ['hello','counter']:
 for row in [*h['HOPPER_FIXTURES'][case],h['REFERENCE_FIXTURES'][case]]:
  reference=row['label'].startswith('Pinocchio')
  manifest=root/'bench/framework-comparison'/('reference' if reference else 'programs')/(case if not reference else row['directory'])
  if not reference:manifest=manifest/row['directory']
  manifest=manifest/'Cargo.toml'
  argv=['cargo-build-sbf','--lto','--arch','v0','--tools-version','v1.54','--manifest-path',str(manifest),'--sbf-out-dir',str(dest),'--','--locked','--offline']
  print(f"Benchmark build: {case} {row['label']}",flush=True)
  with (out/f"{case}-{row['directory']}.log").open('w',encoding='utf-8') as log:rc=subprocess.run(argv,cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT).returncode
  if rc:print((out/f"{case}-{row['directory']}.log").read_text(encoding='utf-8')[-5000:],flush=True);raise SystemExit(rc)
  elf=dest/(row['crate']+'.so');records.append({'case':case,'row':row,'argv':argv,'bytes':elf.stat().st_size,'sha256':hashlib.sha256(elf.read_bytes()).hexdigest()})
  (out/'builds.json').write_text(json.dumps({'sourceCommit':source,'releaseProfile':h['RELEASE_PROFILE'],'records':records},indent=2)+'\n',encoding='utf-8')
assert source==git('rev-parse','HEAD') and not git('status','--porcelain')
(out/'builds.json').write_text(json.dumps({'sourceCommit':source,'sourceUnchangedAndClean':True,'releaseProfile':h['RELEASE_PROFILE'],'records':records},indent=2)+'\n',encoding='utf-8')
