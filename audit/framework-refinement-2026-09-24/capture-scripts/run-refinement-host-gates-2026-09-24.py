from pathlib import Path
import os,subprocess,json,datetime
root=Path.cwd();out=root/'target/hopper/refinement-release-2026-09-24';out.mkdir(exist_ok=True)
env=os.environ.copy();env.update(CARGO_BUILD_JOBS='1',CARGO_INCREMENTAL='0',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0',CARGO_TARGET_DIR=str(root/'target'),CARGO_TERM_COLOR='never')
def git(*args):return subprocess.check_output(['git',*args],text=True,encoding='utf-8').strip()
source=git('rev-parse','HEAD');assert not git('status','--porcelain')
commands=[('fmt',['cargo','+1.96.0','fmt','--all','--check']),('host',['cargo','+1.96.0','test','--workspace','--locked','--offline','--no-fail-fast']),('clippy',['cargo','+1.96.0','clippy','--workspace','--all-targets','--locked','--offline','--','-D','warnings']),('unsafe',['py','-X','utf8','scripts/check-unsafe-safety-comments.py']),('provenance',['py','-X','utf8','scripts/tests/test_release_provenance.py'])]
records=[]
for name,argv in commands:
 print('Running '+name,flush=True)
 with (out/f'{name}.log').open('w',encoding='utf-8') as log: rc=subprocess.run(argv,cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT).returncode
 records.append({'name':name,'argv':argv,'exitCode':rc});(out/f'{name}.exit').write_text(str(rc)+'\n')
 (out/'host-gates.json').write_text(json.dumps({'sourceCommit':source,'records':records},indent=2)+'\n',encoding='utf-8')
 if rc:print((out/f'{name}.log').read_text(encoding='utf-8')[-7000:],flush=True);raise SystemExit(rc)
 print(name+' passed',flush=True)
assert source==git('rev-parse','HEAD') and not git('status','--porcelain')
(out/'host-gates.json').write_text(json.dumps({'sourceCommit':source,'sourceUnchangedAndClean':True,'records':records},indent=2)+'\n',encoding='utf-8')
