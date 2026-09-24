from pathlib import Path
import os,subprocess,json,tomllib,hashlib,shutil
root=Path.cwd();base=root/'target/hopper/refinement-release-2026-09-24';out=base/'registry-consumer';out.mkdir(exist_ok=True)
publication=json.loads((base/'registry-publication/publication.json').read_text());assert publication['complete'];versions={r['name']:r for r in publication['packages']}
profile=(root/'Cargo.toml').read_text(encoding='utf-8').split('[profile.release]',1)[1]
env=os.environ.copy();env.update(CARGO_BUILD_JOBS='1',CARGO_INCREMENTAL='0',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0');env.pop('CARGO_REGISTRIES_CRATES_IO_TOKEN',None);env.pop('CARGO_REGISTRY_TOKEN',None)
def run(argv,path,label,environment):
 r=subprocess.run(argv,cwd=path,env=environment,capture_output=True,text=True,encoding='utf-8',errors='replace');(path/f'{label}.log').write_text(r.stdout+r.stderr,encoding='utf-8',newline='\n')
 if r.returncode:print((r.stdout+r.stderr)[-5000:],flush=True);raise RuntimeError(label+' failed')
 return r
records=[]
for name,fixture,test,var,count in [('mint','mint-plan','mint_plan_sbf','HOPPER_MINT_PLAN_SBF',5),('pda','canonical-pda','canonical_pda_sbf','HOPPER_CANONICAL_PDA_SBF',3)]:
 p=out/name;p.mkdir(exist_ok=True);(p/'src').mkdir(exist_ok=True);hashes={}
 for f in (root/'bench'/fixture/'program/src').glob('*.rs'):
  shutil.copyfile(f,p/'src'/f.name);hashes[f.name]=hashlib.sha256(f.read_bytes()).hexdigest()
 (p/'Cargo.toml').write_text(f'''[workspace]
[package]
name = "hopper-registry-{name}-consumer"
version = "0.0.0"
edition = "2021"
publish = false
[lib]
crate-type = ["cdylib"]
[dependencies]
hopper = {{ package = "hopper-lang", version = "=0.3.1", features = ["proc-macros"] }}
[lints.rust]
unexpected_cfgs = {{ level = "allow", check-cfg = ['cfg(target_os, values("solana"))'] }}
[profile.release]'''+profile,encoding='utf-8',newline='\n')
 print('Resolving registry-only '+name+' program',flush=True)
 result=run(['cargo','+1.96.0','metadata','--format-version','1'],p,'registry-resolution',env);metadata=json.loads(result.stdout)
 for package in metadata['packages']:
  if package['name']!=f'hopper-registry-{name}-consumer':assert package['source']=='registry+https://github.com/rust-lang/crates.io-index'
 lock=tomllib.loads((p/'Cargo.lock').read_text());resolved=[]
 for package in lock['package']:
  if package['name'] in versions:
   expected=versions[package['name']];assert package['version']==expected['version'] and package['checksum']==expected['registryChecksum'];resolved.append({k:package[k] for k in ['name','version','source','checksum']})
 sbf=dict(env,RUSTUP_HOME=str(root/'target/hopper/sbf-rustup-2026-09-23'),CARGO_TARGET_DIR=str(out/'build'))
 run(['cargo-build-sbf','--manifest-path',str(p/'Cargo.toml'),'--tools-version','v1.54','--arch','v0','--sbf-out-dir',str(p/'sbf'),'--','--locked'],p,'registry-sbf-build',sbf)
 elf=p/f'sbf/hopper_registry_{name}_consumer.so'
 verifier=max((root/'target/debug/deps').glob(test+'-*.exe'),key=lambda q:q.stat().st_mtime)
 result=run([str(verifier),'--ignored','--nocapture'],p,'registry-sbf-execution',dict(env,**{var:str(elf)}));assert f'{count} passed; 0 failed; 0 ignored' in result.stdout
 record={'name':name,'allDependenciesFromCratesIo':True,'fixtureSourceSha256':hashes,'releaseDependencies':resolved,'elfBytes':elf.stat().st_size,'elfSha256':hashlib.sha256(elf.read_bytes()).hexdigest(),'testsPassed':count,'lockfileSha256':hashlib.sha256((p/'Cargo.lock').read_bytes()).hexdigest()};records.append(record);print(name+' registry-only compiled tests passed',flush=True)
(out/'receipt.json').write_text(json.dumps({'schema':'hopper.registry-consumer.v1','publicationSourceCommit':publication['sourceCommit'],'build':{'platformTools':'v1.54','arch':'v0','locked':True},'records':records,'allPassed':True},indent=2)+'\n',encoding='utf-8',newline='\n')
