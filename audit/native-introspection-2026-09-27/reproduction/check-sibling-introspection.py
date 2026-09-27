from pathlib import Path
import subprocess,os,json,hashlib
root=Path.cwd();out=root/'target/hopper/native-sdk-audit-2026-09-27';out.mkdir(exist_ok=True)
env=os.environ.copy();env.update(CARGO_BUILD_JOBS='2',CARGO_INCREMENTAL='0',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0');records=[]
def run(name,cmd,e=env):
 p=out/(name+'.log')
 with p.open('w',encoding='utf-8') as f:rc=subprocess.run(cmd,env=e,stdout=f,stderr=subprocess.STDOUT).returncode
 records.append(dict(name=name,command=cmd,exitCode=rc));(out/'checks.json').write_text(json.dumps(records,indent=2)+'\n')
 if rc:print(p.read_text(encoding='utf-8')[-6000:]);raise SystemExit(rc)
 print(name+': passed',flush=True)
run('fmt',['cargo','+1.96.0','fmt','-p','hopper-native','-p','hopper-runtime','-p','hopper-sibling-introspection-fixture','-p','hopper-framework-verifier'])
run('lock',['cargo','+1.96.0','check','-p','hopper-sibling-introspection-fixture','--offline'])
run('native',['cargo','+1.96.0','test','-p','hopper-native','--locked','--offline'])
run('runtime',['cargo','+1.96.0','test','-p','hopper-runtime','--features','thread-local-registry','--locked','--offline'])
run('clippy',['cargo','+1.96.0','clippy','-p','hopper-native','-p','hopper-runtime','-p','hopper-sibling-introspection-fixture','--all-targets','--locked','--offline','--','-Dwarnings'])
run('miri-introspection',['cargo','+nightly','miri','test','-p','hopper-native','--lib','introspect::tests','--locked','--offline'])
for arch in ['v0','v3']:
 dest=out/arch;dest.mkdir(exist_ok=True);e=env.copy();e.update(RUSTUP_HOME=str(root/'target/hopper/sbf-rustup-2026-09-23'),CARGO_TARGET_DIR=str(root/'target/hopper/canonical-pda-release-2026-09-23/build'))
 run('build-'+arch,['cargo-build-sbf','--arch',arch,'--tools-version','v1.54','--manifest-path',str(root/'bench/sibling-introspection/program/Cargo.toml'),'--sbf-out-dir',str(dest),'--','--locked','--offline'],e)
 elf=dest/'hopper_sibling_introspection_fixture.so';e=env.copy();e['HOPPER_SIBLING_SBF']=str(elf)
 run('svm-'+arch,['cargo','+1.96.0','test','-p','hopper-framework-verifier','--test','sibling_introspection_sbf','--locked','--offline','--','--ignored','--nocapture'],e)
 records.append(dict(arch=arch,elfSha256=hashlib.sha256(elf.read_bytes()).hexdigest(),bytes=elf.stat().st_size));(out/'checks.json').write_text(json.dumps(records,indent=2)+'\n')
