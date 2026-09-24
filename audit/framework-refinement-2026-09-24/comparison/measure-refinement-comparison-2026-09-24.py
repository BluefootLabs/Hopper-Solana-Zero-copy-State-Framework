from pathlib import Path
import json,runpy,subprocess,hashlib,datetime
root=Path.cwd();out=root/'target/hopper/refinement-release-2026-09-24/comparison';h=runpy.run_path(str(root/'scripts/bench-framework-comparison.py'))
builds=json.loads((out/'builds.json').read_text());source=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip();assert source==builds['sourceCommit'] and builds['sourceUnchangedAndClean'];assert not subprocess.check_output(['git','status','--porcelain'],text=True).strip()
verifier=root/'target/debug/framework-verifier.exe'
result={'generated':datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%d %H:%M UTC'),'toolchain':h['tool_versions'](),'mollusk':'0.15.1','release_profile':h['RELEASE_PROFILE'],'program_ids':h['PROGRAM_IDS'],'source_commit':source,'source_tree_clean':True,'build_mode':'rebuilt','lockfile_sha256':hashlib.sha256((root/'Cargo.lock').read_bytes()).hexdigest(),'verifierSha256':hashlib.sha256(verifier.read_bytes()).hexdigest(),'verifierProfile':'dev; program ELFs use the recorded release recipe'}
for case in ['hello','counter']:
 rows=[]
 for b in builds['records']:
  if b['case']!=case:continue
  row=b['row'];elf=out/'sbf'/(row['crate']+'.so');assert hashlib.sha256(elf.read_bytes()).hexdigest()==b['sha256']
  measured=h['measure'](verifier,case,elf,row);measured['source']='measured here (cross-check)' if row['label'].startswith('Pinocchio') else 'measured here';rows.append(measured)
 rows.extend({**r,'source':'pina published'} for r in h['PINA_PUBLISHED'][case]);result[case]=rows
assert not subprocess.check_output(['git','status','--porcelain'],text=True).strip();assert source==subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
(out/'results.json').write_text(json.dumps(result,indent=2)+'\n',encoding='utf-8');(out/'RESULTS.md').write_text(h['render_markdown'](result),encoding='utf-8')
print(json.dumps({k:result[k] for k in ['hello','counter']},indent=2))
