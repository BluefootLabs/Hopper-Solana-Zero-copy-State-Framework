from pathlib import Path
import subprocess,os,json,hashlib
root=Path.cwd();out=root/'target/hopper/cpi-outcomes-2026-09-26';out.mkdir(exist_ok=True)
env=os.environ.copy();env.update(CARGO_BUILD_JOBS='2',CARGO_INCREMENTAL='0',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0');records=[]
def run(name,cmd,e=env):
 p=out/(name+'.log')
 with p.open('w',encoding='utf-8') as f:rc=subprocess.run(cmd,env=e,stdout=f,stderr=subprocess.STDOUT).returncode
 records.append(dict(name=name,command=cmd,exitCode=rc));(out/'checks.json').write_text(json.dumps(records,indent=2)+'\n')
 if rc:print(p.read_text(encoding='utf-8')[-5000:]);raise SystemExit(rc)
 print(name+': passed',flush=True)
run('fmt',['cargo','+1.96.0','fmt','-p','hopper-runtime','-p','hopper-solana','-p','hopper-token-outcomes-fixture','-p','hopper-framework-verifier'])
run('lock',['cargo','+1.96.0','check','-p','hopper-token-outcomes-fixture','--offline'])
run('runtime',['cargo','+1.96.0','test','-p','hopper-runtime','--features','thread-local-registry','--locked','--offline'])
run('solana',['cargo','+1.96.0','test','-p','hopper-solana','--locked','--offline'])
run('clippy',['cargo','+1.96.0','clippy','-p','hopper-runtime','-p','hopper-solana','-p','hopper-token-outcomes-fixture','--all-targets','--locked','--offline','--','-Dwarnings'])
run('miri-transfer',['cargo','+nightly','miri','test','-p','hopper-solana','--test','transfer','--locked','--offline'])
run('miri-dedup',['cargo','+nightly','miri','test','-p','hopper-runtime','--lib','dedup_tests','--features','thread-local-registry','--locked','--offline'])
for arch in ['v0','v3']:
 dest=out/arch;dest.mkdir(exist_ok=True);e=env.copy();e.update(RUSTUP_HOME=str(root/'target/hopper/sbf-rustup-2026-09-23'),CARGO_TARGET_DIR=str(root/'target/hopper/canonical-pda-release-2026-09-23/build'))
 run('build-'+arch,['cargo-build-sbf','--arch',arch,'--tools-version','v1.54','--manifest-path',str(root/'bench/token-outcomes/program/Cargo.toml'),'--sbf-out-dir',str(dest),'--','--locked','--offline'],e)
 elf=dest/'hopper_token_outcomes_fixture.so';e=env.copy();e['HOPPER_TOKEN_OUTCOMES_SBF']=str(elf)
 run('svm-'+arch,['cargo','+1.96.0','test','-p','hopper-framework-verifier','--test','token_outcomes_sbf','--locked','--offline','--','--ignored','--nocapture'],e)
 records.append(dict(arch=arch,elfSha256=hashlib.sha256(elf.read_bytes()).hexdigest(),bytes=elf.stat().st_size));(out/'checks.json').write_text(json.dumps(records,indent=2)+'\n')
