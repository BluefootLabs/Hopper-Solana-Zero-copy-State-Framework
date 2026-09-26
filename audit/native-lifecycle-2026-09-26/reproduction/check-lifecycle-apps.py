from pathlib import Path
import os,subprocess,json,hashlib
root=Path.cwd();out=root/'target/hopper/native-replacement-2026-09-26/apps';out.mkdir(exist_ok=True)
env=os.environ.copy();env.update(CARGO_BUILD_JOBS='2',CARGO_INCREMENTAL='0',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0')
sbf=env.copy();sbf.update(RUSTUP_HOME=str(root/'target/hopper/sbf-rustup-2026-09-23'),CARGO_TARGET_DIR=str(root/'target/hopper/canonical-pda-release-2026-09-23/build'))
records=[]
def run(name,cmd,e=env):
 p=out/(name+'.log')
 with p.open('w',encoding='utf-8') as f:rc=subprocess.run(cmd,env=e,stdout=f,stderr=subprocess.STDOUT).returncode
 records.append(dict(name=name,command=cmd,exitCode=rc))
 if rc:print(p.read_text(encoding='utf-8')[-6000:]);raise SystemExit(rc)
 print(name+': passed',flush=True)
run('host',['cargo','+1.96.0','test','-p','hopper-lang','--features','proc-macros','--lib','--tests','--locked','--offline'])
run('core',['cargo','+1.96.0','test','-p','hopper-systems','--lib','--locked','--offline'])
run('unsafe',['python','scripts/check-unsafe-safety-comments.py','--inventory-out',str(out/'unsafe-inventory.md')])
programs=[('hopper-treasury','examples/hopper-treasury','HOPPER_TREASURY_SBF'),('hopper-bounded-multisig','examples/hopper-bounded-multisig','HOPPER_MULTISIG_SBF'),('hopper-escrow','examples/hopper-escrow','HOPPER_ESCROW_SBF'),('hopper-byte-allowance','examples/hopper-byte-allowance','HOPPER_BYTE_ALLOWANCE_SBF'),('hopper-runtime-gate-fixture','bench/runtime-gate/program','HOPPER_GATE_SBF')]
for arch in ['v0','v3']:
 dest=out/arch;dest.mkdir(exist_ok=True);test=env.copy()
 for name,path,key in programs:
  run('build-'+name+'-'+arch,['cargo-build-sbf','--arch',arch,'--tools-version','v1.54','--manifest-path',path+'/Cargo.toml','--sbf-out-dir',str(dest),'--','--locked','--offline'],sbf)
  elf=dest/(name.replace('-','_')+'.so');test[key]=str(elf)
  records.append(dict(program=name,arch=arch,elfSha256=hashlib.sha256(elf.read_bytes()).hexdigest()))
 for fixture in ['governance_sbf','token_escrow_sbf','byte_allowance_sbf','ambient_gate_sbf']:
  run(fixture+'-'+arch,['cargo','+1.96.0','test','-p','hopper-framework-verifier','--test',fixture,'--locked','--offline','--','--ignored','--nocapture'],test)
(out/'receipt.json').write_text(json.dumps(dict(checks=records,allPassed=True),indent=2)+'\n')
