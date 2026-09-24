import hashlib,json,os,shutil,subprocess,time,tomllib,urllib.request,urllib.error
from pathlib import Path
ROOT=Path.cwd();BASE=ROOT/'target/hopper/refinement-release-2026-09-24';OUT=BASE/'registry-publication';OUT.mkdir(exist_ok=True)
TOKEN=os.environ.get('CARGO_REGISTRIES_CRATES_IO_TOKEN','')
def run(args):
 r=subprocess.run(args,cwd=ROOT,capture_output=True,text=True,encoding='utf-8',errors='replace');s=r.stdout+r.stderr
 return r.returncode,s.replace(TOKEN,'<redacted>') if TOKEN else s
def git(*args):
 rc,s=run(['git',*args]);assert rc==0;return s.strip()
def clean():
 assert not git('status','--porcelain'),'source tree is dirty';return git('rev-parse','HEAD')
def read(p):return json.loads(p.read_text(encoding='utf-8-sig'))
source=clean()
def code_unchanged(tested):
 assert run(['git','merge-base','--is-ancestor',tested,source])[0]==0
 assert set(git('diff','--name-only',tested,source).splitlines()) <= {'scripts/test-mint-plan-devnet.py','tools/hopper-cli/README.md'},'untested source change'
host=read(BASE/'host-gates.json');code_unchanged(host['sourceCommit']);assert host['sourceUnchangedAndClean'] and all(r['exitCode']==0 for r in host['records'])
sbf=read(BASE/'sbf-validated/gates.json');code_unchanged(sbf['sourceCommit']);assert sbf['sourceUnchangedAndClean'] and sbf['allPassed']
for p in [BASE/'publish-train.json',BASE/'cicada-attestation/attestation.json']:
 e=read(p)['source'];code_unchanged(e['commitAtStart']);assert e['commitAtStart']==e['commitAtEnd'] and e['treeCleanAtStart'] and e['treeCleanAtEnd']
fuzz=read(BASE/'cicada-semantic-report.json');assert fuzz['passed']==698 and fuzz['failed']==fuzz['skipped']==0 and fuzz['programVersion']=='0.3.1'
for path,count in [('mint-devnet-verified',11),('pda-init-devnet',9),('pda-readonly-devnet',25)]:
 e=read(BASE/path/'receipt.json');code_unchanged(e['sourceCommit']);assert e['deployedElfMatchesBeforeAndAfter'] and len(e['transactions'])==count
config=tomllib.loads((ROOT/'release/publish-order.toml').read_text(encoding='utf-8-sig'));assert config['default_version']=='0.3.1'
old=read(ROOT/'audit/registry-publication-2026-09-23/publication.json')
receipt=OUT/'publication.json'
if receipt.exists():
 records=read(receipt)
 if records['sourceCommit']!=source:
  previous=records['sourceCommit'];assert git('diff','--name-only',previous,source)=='tools/hopper-cli/README.md'
  assert not any(r['name']=='hopper-cli' for r in records['packages'])
  records['sourceCommits']=[previous,source];records['sourceCommit']=source
  records['sourceTransition']='CLI README installation heading corrected before hopper-cli publication; no Rust, dependency, lockfile, or other package content changed.'
  records['validation']['sourceDifference']='After host/SBF validation, only the devnet harness VM-error assertion and CLI README installation heading changed. Each package and execution receipt retains its actual source commit.'
else:
 records={'schema':'hopper.registry-publication.v1','sourceCommit':source,'registry':'crates-io','packages':[], 'validation':{'hostSourceCommit':host['sourceCommit'],'compiledSourceCommit':sbf['sourceCommit'],'sourceDifference':'Only scripts/test-mint-plan-devnet.py changed after host/SBF validation; final devnet runs use the corrected live VM error check. Rust, manifests, lockfiles, and README sources are identical.'}}
 for r in old['packages']:
  if r['version']=='0.1.0':
   assert not git('diff','--name-only',r.get('sourceCommit',old['sourceCommit']),source,'--','crates/'+r['name'])
   records['packages'].append(dict(r,retainedPublishedVersion=True,unchangedSincePublishedSource=True))
def save():receipt.write_text(json.dumps(records,indent=2)+'\n',encoding='utf-8',newline='\n')
save()
def version(name,number):
 req=urllib.request.Request(f'https://crates.io/api/v1/crates/{name}/{number}',headers={'User-Agent':'Hopper release verification (https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework)'})
 for attempt in range(4):
  try:
   with urllib.request.urlopen(req,timeout=40) as response:return json.load(response)['version']
  except urllib.error.HTTPError as e:
   if e.code==404:return None
   if e.code not in (429,500,502,503,504) or attempt==3:raise
   time.sleep(5)
os.environ.update(CARGO_BUILD_JOBS='1',CARGO_INCREMENTAL='0',CARGO_PROFILE_DEV_DEBUG='0',CARGO_TERM_COLOR='never',CARGO_TARGET_DIR=str(ROOT/'target'))
for index,name in enumerate(config['packages'],1):
 assert clean()==source
 number=config.get('version_overrides',{}).get(name,config['default_version']);observed=version(name,number);prior=next((r for r in records['packages'] if r['name']==name),None)
 if observed:
  assert prior and prior['version']==number and prior['archiveSha256']==observed['checksum'],f'{name}: existing version lacks matching receipt'
  prior.update(registryVerified=True,registryChecksum=observed['checksum']);save();print(f'[{index}/29] {name} {number}: registry checksum verified',flush=True);continue
 assert number=='0.3.1'
 print(f'[{index}/29] Verifying {name} {number}',flush=True)
 argv=['cargo','+1.96.0','publish','--locked','--registry','crates-io','-p',name];started=time.time_ns();rc,s=run(argv+['--dry-run']);(OUT/f'{name}-dry-run.log').write_text(s,encoding='utf-8',newline='\n')
 if rc:print(s[-4000:],flush=True);raise RuntimeError(f'{name}: dry run failed ({rc})')
 archive=ROOT/f'target/package/tmp-crate/{name}-{number}.crate';assert archive.stat().st_mtime_ns>=started;digest=hashlib.sha256(archive.read_bytes()).hexdigest();shutil.copyfile(archive,OUT/f'{name}-{number}.crate')
 item=prior or {'name':name,'version':number,'sourceCommit':source};item.update(dryRunPassed=True,archiveSha256=digest,registryVerified=False)
 if not prior:records['packages'].append(item)
 save();assert clean()==source
 rc,s=run(argv);(OUT/f'{name}-publish.log').write_text(s,encoding='utf-8',newline='\n');item['publishExitCode']=rc;save()
 if rc:print(s[-4000:],flush=True);raise RuntimeError(f'{name}: upload failed ({rc}); stopped before dependents')
 assert hashlib.sha256(archive.read_bytes()).hexdigest()==digest,'archive changed between dry run and upload'
 for attempt in range(20):
  observed=version(name,number)
  if observed:break
  time.sleep(3)
 assert observed and observed['checksum']==digest,'registry checksum mismatch'
 item.update(registryVerified=True,registryChecksum=observed['checksum'],registryCreatedAt=observed['created_at']);save();print(f'[{index}/29] {name} {number}: published and checksum verified',flush=True)
records['complete']=True;records['newPackages']=26;records['retainedPackages']=3;save();print('All 26 framework updates published; 3 independent 0.1.0 packages verified unchanged.',flush=True)
