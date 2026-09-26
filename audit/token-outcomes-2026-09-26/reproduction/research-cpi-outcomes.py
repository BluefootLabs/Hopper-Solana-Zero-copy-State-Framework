from pathlib import Path
import subprocess,json,urllib.request,hashlib,datetime
from concurrent.futures import ThreadPoolExecutor
out=Path('target/hopper/cpi-outcomes-2026-09-26/research');out.mkdir(parents=True,exist_ok=True)
repos={'pina':('pina-rs/pina','HEAD'),'pinocchio':('anza-xyz/pinocchio','HEAD'),'quasar':('blueshift-gg/quasar','HEAD'),'anchor-v2':('otter-sec/anchor','anchor-next'),'token-2022':('solana-program/token-2022','HEAD'),'agave':('anza-xyz/agave','HEAD'),'simds':('solana-foundation/solana-improvement-documents','HEAD')}
def api(p):return json.loads(subprocess.check_output(['gh','api',p],text=True,encoding='utf-8'))
def capture(pair):
 name,(repo,branch)=pair;head=api('repos/'+repo+'/commits/'+branch);sha=head['sha'];tree=api('repos/'+repo+'/git/trees/'+sha+'?recursive=1');assert not tree.get('truncated')
 dest=out/name;dest.mkdir(exist_ok=True);(dest/'tree.json').write_text(json.dumps(tree,indent=2)+'\n');files=[]
 wanted={'pina':['crates/pina/src/token.rs'],'token-2022':['program/src/processor.rs','interface/src/extension/transfer_fee/mod.rs','interface/src/extension/mod.rs']}.get(name,[])
 for path in wanted:
  url=f'https://raw.githubusercontent.com/{repo}/{sha}/{path}';raw=urllib.request.urlopen(url,timeout=45).read();p=dest/path;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(raw);files.append(dict(path=path,sha256=hashlib.sha256(raw).hexdigest(),url=url))
 return dict(name=name,repository=repo,branch=branch,commit=sha,commitDate=head['commit']['committer']['date'],files=files)
with ThreadPoolExecutor(max_workers=4) as pool:records=list(pool.map(capture,repos.items()))
(out/'sources.json').write_text(json.dumps(dict(observedAt=datetime.datetime.now(datetime.timezone.utc).isoformat(),projects=records),indent=2)+'\n')
for r in records:print(r['name'],r['commit'])
