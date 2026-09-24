from pathlib import Path
import hashlib,json,urllib.request,tarfile,io,time
root=Path.cwd();base=root/'target/hopper/refinement-release-2026-09-24';pub=json.loads((base/'registry-publication/publication.json').read_text());assert pub['complete'];out=base/'registry-downloads';out.mkdir(exist_ok=True);rows=[]
for item in pub['packages']:
 name=item['name'];version=item['version'];url=f'https://static.crates.io/crates/{name}/{name}-{version}.crate';request=urllib.request.Request(url,headers={'User-Agent':'Hopper release archive verification'})
 with urllib.request.urlopen(request,timeout=45) as response:data=response.read()
 digest=hashlib.sha256(data).hexdigest();assert digest==item['registryChecksum']==item['archiveSha256']
 with tarfile.open(fileobj=io.BytesIO(data),mode='r:gz') as tar:
  vcs=json.load(tar.extractfile(f'{name}-{version}/.cargo_vcs_info.json'))
 assert vcs['git']['sha1']==item['sourceCommit'] and not vcs['git'].get('dirty',False),(name,vcs)
 (out/f'{name}-{version}.crate').write_bytes(data);rows.append({'name':name,'version':version,'url':url,'sha256':digest,'bytes':len(data),'vcs':vcs})
 print(name+' '+version+': downloaded archive and clean VCS commit verified',flush=True)
(out/'receipt.json').write_text(json.dumps({'schema':'hopper.registry-download-verification.v1','allPassed':True,'packages':rows},indent=2)+'\n',encoding='utf-8',newline='\n')
