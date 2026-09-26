from pathlib import Path
import subprocess,json,urllib.request,hashlib
out=Path('target/hopper/native-replacement-2026-09-26/research');records=[]
for repo in ['blueshift-gg/abiv2','pina-rs/pinapod','pina-rs/lootbox']:
 name=repo.split('/')[-1]
 def api(path):return json.loads(subprocess.check_output(['gh','api',path],text=True,encoding='utf-8'))
 head=api('repos/'+repo+'/commits/HEAD');sha=head['sha'];tree=api('repos/'+repo+'/git/trees/'+sha+'?recursive=1');assert not tree.get('truncated')
 dest=out/name;dest.mkdir(exist_ok=True);(dest/'tree.json').write_text(json.dumps(tree,indent=2)+'\n')
 files=[]
 for e in tree['tree']:
  path=e['path']
  if e['type']!='blob':continue
  if path in ['README.md','Cargo.toml'] or (path.endswith('.rs') and (name=='abiv2' or (name=='pinapod' and ('src/traits' in path or 'src/lib.rs' in path or 'src/compact' in path)) or (name=='lootbox' and 'src/lib.rs' in path))):
   url=f'https://raw.githubusercontent.com/{repo}/{sha}/{path}';raw=urllib.request.urlopen(url,timeout=45).read();p=dest/path;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(raw)
   files.append(dict(path=path,sha256=hashlib.sha256(raw).hexdigest(),lines=len(raw.splitlines()),url=url))
 record=dict(name=name,repository=repo,commit=sha,date=head['commit']['committer']['date'],files=files)
 records.append(record);print(name,sha,[(f['path'],f['lines']) for f in files],flush=True)
(out/'sister-sources.json').write_text(json.dumps(records,indent=2)+'\n')
