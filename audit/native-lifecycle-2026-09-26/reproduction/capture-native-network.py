from pathlib import Path
import json,re,urllib.request,base64,datetime,struct,time
out=Path('target/hopper/native-replacement-2026-09-26/research');source=(out/'agave/feature-set/src/lib.rs').read_text()
modules=dict(re.findall(r'pub mod (\w+)\s*\{\s*solana_pubkey::declare_id!\("([^"]+)"\)',source))
selected={'0321':'provide_instruction_data_offset_in_vm_r2','0339':'increase_cpi_account_info_limit','0385':'enable_tx_v1','0449':'direct_account_pointers_in_program_input','0459':'syscall_parameter_address_restrictions','0460':'virtual_address_space_adjustments','sbpf-v3':'enable_sbpf_v3_deployment_and_execution','0500':'disable_sbpf_v0_v1_v2_deployment','0512':'enable_sha512_syscall','0049':'remaining_compute_units_syscall_enabled','0194':'deprecate_rent_exemption_threshold','direct-mapping':'account_data_direct_mapping','0326':'alpenglow',**{f'0437-{i}':f'set_lamports_per_byte_to_{n}' for i,n in enumerate([6333,5080,2575,1322,696],1)}}
for key in modules:
 if 'slot_time_to_' in key or ('depth' in key and 'cpi' in key):selected[key]=key
def rpc(url,method,params):
 for attempt in range(5):
  try:
   request=urllib.request.Request(url,json.dumps(dict(jsonrpc='2.0',id=1,method=method,params=params)).encode(),{'Content-Type':'application/json'})
   with urllib.request.urlopen(request,timeout=30) as response:value=json.load(response)
   assert 'error' not in value,value
   return value['result']
  except Exception:
   if attempt==4:raise
   time.sleep(2)
genesis={'devnet':'EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG','testnet':'4uhcVJyU9pJkvQyS88uRDiswHXSCkY3zQawwpjk2NsNY','mainnet-beta':'5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d'}
records=[]
for cluster,expected in genesis.items():
 url=f'https://api.{cluster}.solana.com';assert rpc(url,'getGenesisHash',[])==expected
 snapshot=rpc(url,'getMultipleAccounts',[[modules[m] for m in selected.values()],dict(encoding='base64',commitment='finalized')]);rows=[]
 for (name,module),account in zip(selected.items(),snapshot['value'],strict=True):
  row=dict(name=name,module=module,key=modules[module],state='absent')
  if account:
   if account['owner']!='Feature111111111111111111111111111111111111':row.update(state='unproven',reason='Not owned by Feature program',owner=account['owner'])
   else:
    raw=base64.b64decode(account['data'][0],validate=True);assert len(raw) in (1,9) and raw[0] in [0,1] and not account['executable']
    if raw[0]==1:
     assert len(raw)==9;slot=struct.unpack('<Q',raw[1:])[0];row.update(state='active' if slot<=snapshot['context']['slot'] else 'scheduled',activationSlot=slot)
    else:row['state']='pending'
  rows.append(row)
 record=dict(cluster=cluster,genesisHash=expected,observedAt=datetime.datetime.now(datetime.timezone.utc).isoformat(),observedSlot=snapshot['context']['slot'],commitment='finalized',rpc=url,features=rows,raw=snapshot)
 (out/(cluster+'-features.json')).write_text(json.dumps(record,indent=2)+'\n');records.append(record)
 print(cluster,record['observedSlot'],[(r['name'],r['state']) for r in rows],flush=True)
(out/'network-summary.json').write_text(json.dumps([{k:v for k,v in r.items() if k!='raw'} for r in records],indent=2)+'\n')
