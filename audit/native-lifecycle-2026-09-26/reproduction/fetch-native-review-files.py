from pathlib import Path
import json,urllib.request,hashlib
from concurrent.futures import ThreadPoolExecutor
out=Path('target/hopper/native-replacement-2026-09-26/research');projects=json.loads((out/'sources.json').read_text())['projects'];items=[]
for p in projects:
 name=p['name'];tree=json.loads((out/name/'tree.json').read_text())['tree']
 for entry in tree:
  path=entry['path']
  if entry['type']!='blob':continue
  selected=(name=='sdk' and path in ['account-view/src/lib.rs','instruction-view/src/cpi.rs','instruction-view/src/lib.rs']) or (name=='pinocchio' and path in ['sdk/src/entrypoint/mod.rs','sdk/src/entrypoint/lazy.rs','sdk/src/sysvars/rent.rs']) or (name=='quasar' and path in ['lang/src/ops/close.rs','lang/src/ops/realloc.rs','lang/src/ops/init.rs','lang/src/account_init.rs','lang/src/cpi/dyn_cpi.rs','lang/src/accounts/account.rs']) or (name=='pina' and path in ['crates/pina/src/cpi.rs','crates/pina/tests/verified_transfer.rs','crates/pina/tests/compact_account.rs','examples/account_realloc_program/src/lib.rs','crates/pina/src/lib.rs']) or (name=='anchor-v2' and path in ['lang-v2/src/cpi.rs','lang-v2/tests/close_semantics.rs','lang-v2/tests/realloc_payer_target_check.rs']) or (name=='simds' and path.startswith('proposals/') and any(path.split('/')[-1].startswith(n) for n in ['0321','0326','0339','0385','0449','0459','0460','0500','0512','0558','0194','0437'])) or (name=='beethoven' and path.endswith('.rs') and ('src/lib.rs' in path or 'src/instructions' in path))
  if selected:items.append((p,path))
def fetch(item):
 p,path=item;url=f'https://raw.githubusercontent.com/{p["repository"]}/{p["commit"]}/{path}';raw=urllib.request.urlopen(url,timeout=45).read();dest=out/p['name']/path;dest.parent.mkdir(parents=True,exist_ok=True);dest.write_bytes(raw)
 return dict(project=p['name'],commit=p['commit'],path=path,sha256=hashlib.sha256(raw).hexdigest(),lines=len(raw.splitlines()),url=url)
with ThreadPoolExecutor(max_workers=6) as pool:records=list(pool.map(fetch,items))
(out/'fetched-files.json').write_text(json.dumps(records,indent=2)+'\n')
print('Fetched',len(records),'files;',sum(r['lines'] for r in records),'lines. Download is not a review-completion claim.')
for r in records:print(r['project'],r['path'],r['lines'])
