from pathlib import Path
import subprocess,json,urllib.request,hashlib,datetime
from concurrent.futures import ThreadPoolExecutor
out=Path('target/hopper/native-replacement-2026-09-26/research');out.mkdir(parents=True,exist_ok=True)
repos={'pinocchio':('anza-xyz/pinocchio','HEAD'),'sdk':('anza-xyz/solana-sdk','HEAD'),'pina':('pina-rs/pina','HEAD'),'quasar':('blueshift-gg/quasar','HEAD'),'beethoven':('blueshift-gg/beethoven','HEAD'),'anchor-v2':('otter-sec/anchor','anchor-next'),'agave':('anza-xyz/agave','HEAD'),'simds':('solana-foundation/solana-improvement-documents','HEAD')}
def api(path):return json.loads(subprocess.check_output(['gh','api',path],text=True,encoding='utf-8'))
def capture(pair):
 name,(repo,branch)=pair;head=api('repos/'+repo+'/commits/'+branch);sha=head['sha'];tree=api('repos/'+repo+'/git/trees/'+sha+'?recursive=1');assert not tree.get('truncated')
 dest=out/name;dest.mkdir(exist_ok=True);(dest/'tree.json').write_text(json.dumps(tree,indent=2)+'\n')
 # Pin complete file inventories first. Fetch implementation files after scope review.
 wanted=[x['path'] for x in tree['tree'] if x['type']=='blob' and x['path'] in ['Cargo.toml','README.md','sdk/Cargo.toml','sdk/src/lib.rs','feature-set/src/lib.rs']]
 files=[]
 for path in wanted:
  url=f'https://raw.githubusercontent.com/{repo}/{sha}/{path}';raw=urllib.request.urlopen(url,timeout=45).read();p=dest/path;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(raw);files.append(dict(path=path,sha256=hashlib.sha256(raw).hexdigest(),url=url))
 return dict(name=name,repository=repo,branch=branch,commit=sha,commitDate=head['commit']['committer']['date'],files=files,inventoryBlobs=sum(x['type']=='blob' for x in tree['tree']))
with ThreadPoolExecutor(max_workers=4) as pool:records=list(pool.map(capture,repos.items()))
(out/'sources.json').write_text(json.dumps(dict(observedAt=datetime.datetime.now(datetime.timezone.utc).isoformat(),projects=records),indent=2)+'\n')
for r in records:print({k:v for k,v in r.items() if k!='files'},flush=True)
for org in ['blueshift-gg','pina-rs']:
 result=api('orgs/'+org+'/repos?per_page=100&type=public');(out/(org+'-repos.json')).write_text(json.dumps([dict(name=r['name'],description=r['description'],url=r['html_url'],archived=r['archived'],updatedAt=r['updated_at']) for r in result],indent=2)+'\n')
 print(org,[(r['name'],r['description']) for r in result if not r['archived']],flush=True)
