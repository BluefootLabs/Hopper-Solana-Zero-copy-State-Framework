from pathlib import Path
import subprocess,os,json,hashlib,shutil
root=Path.cwd();out=root/'target/hopper/native-replacement-2026-09-26';out.mkdir(exist_ok=True)
env=os.environ.copy();env.update(CARGO_BUILD_JOBS='2',CARGO_INCREMENTAL='0',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0')
sbf=env.copy();sbf.update(RUSTUP_HOME=str(root/'target/hopper/sbf-rustup-2026-09-23'),CARGO_TARGET_DIR=str(root/'target/hopper/canonical-pda-release-2026-09-23/build'))
records=[]
def run(name,cmd,e=env,fail=False):
 p=out/(name+'.log')
 with p.open('w',encoding='utf-8') as f:rc=subprocess.run(cmd,env=e,stdout=f,stderr=subprocess.STDOUT).returncode
 records.append(dict(name=name,command=cmd,exitCode=rc,expectedFailure=fail))
 if (rc!=0)!=fail:print(p.read_text(encoding='utf-8')[-6500:]);raise SystemExit(1)
 print(name+': '+('expected failure' if fail else 'passed'),flush=True)
run('fmt',['cargo','+1.96.0','fmt','-p','hopper-native','-p','hopper-runtime','-p','hopper-native-lifecycle-fixture','-p','hopper-runtime-lifecycle-fixture','-p','hopper-framework-verifier'])
run('lock',['cargo','+1.96.0','check','-p','hopper-native-lifecycle-fixture','-p','hopper-runtime-lifecycle-fixture','--offline'])
run('native',['cargo','+1.96.0','test','-p','hopper-native','--features','cpi','--locked','--offline'])
run('runtime',['cargo','+1.96.0','test','-p','hopper-runtime','--features','thread-local-registry','--locked','--offline'])
run('clippy',['cargo','+1.96.0','clippy','-p','hopper-native','-p','hopper-runtime','-p','hopper-native-lifecycle-fixture','-p','hopper-runtime-lifecycle-fixture','--all-targets','--locked','--offline','--','-Dwarnings'])
baseline=out/'baseline-segments';baseline.mkdir(exist_ok=True)
shutil.copytree(root/'bench/lifecycle/runtime/src',baseline/'src',dirs_exist_ok=True)
(baseline/'Cargo.toml').write_text('[package]\nname="hopper-runtime-lifecycle-fixture"\nversion="0.0.0"\nedition="2021"\n[workspace]\n[lib]\ncrate-type=["cdylib"]\n[dependencies]\nhopper-runtime="=0.4.1"\n')
for kind,arch in [('runtime','baseline'),('runtime','v0'),('native','v0'),('runtime','v3'),('native','v3')]:
 dest=out/(kind+'-'+arch);dest.mkdir(exist_ok=True)
 manifest=baseline/'Cargo.toml' if arch=='baseline' else root/f'bench/lifecycle/{kind}/Cargo.toml'
 buildenv=sbf.copy()
 if arch=='baseline':buildenv['CARGO_TARGET_DIR']=str(out/'baseline-build')
 run('build-'+kind+'-'+arch,['cargo-build-sbf','--arch','v0' if arch=='baseline' else arch,'--tools-version','v1.54','--manifest-path',str(manifest),'--sbf-out-dir',str(dest),'--','--offline'],buildenv)
 elf=dest/f'hopper_{kind}_lifecycle_fixture.so';test=env.copy();test.update(HOPPER_LIFECYCLE_KIND=kind,HOPPER_LIFECYCLE_SBF=str(elf))
 run('test-'+kind+'-'+arch,['cargo','+1.96.0','test','-p','hopper-framework-verifier','--test','lifecycle_sbf','--locked','--offline','--','--ignored','--nocapture'],test,arch=='baseline')
 if arch=='baseline':
  log=(out/'test-runtime-baseline.log').read_text();assert 'Custom(900)' in log and 'runtime [0]' in log
 records.append(dict(kind=kind,arch=arch,elfSha256=hashlib.sha256(elf.read_bytes()).hexdigest(),bytes=elf.stat().st_size))
(out/'checks.json').write_text(json.dumps(records,indent=2)+'\n')
