from pathlib import Path
import copy,hashlib,json,os,runpy,subprocess,sys

root=Path.cwd();base=root/'target/hopper/full-devnet-2026-10-07'
os.environ['HOPPER_MATRIX_WORKER']='1'
os.environ['HOPPER_MATRIX_PAYER']=str(base/'private-workers/application-suites.json')
h=runpy.run_path(str(base/'devnet-harness.py'));source=h['SOURCE'];out=source/'target/verification'

# Recover the completed refusal without resubmitting it.
folder=out/'suite-byte-allowance'
prior=json.loads((folder/'init-unsigned-authority.snapshots.json').read_text())
tx=json.loads((folder/'init-unsigned-book.transaction.json').read_text())
assert tx['meta']['err']=={'InstructionError':[0,'PrivilegeEscalation']}
expected=copy.deepcopy(prior['after']);expected[0]['lamports']-=tx['meta']['fee']
after=h['rpc']('getMultipleAccounts',[prior['addresses'],{'encoding':'base64','commitment':'finalized','minContextSlot':tx['slot']}])['value']
assert after==expected
h['write'](folder/'init-unsigned-book.recovery.json',dict(addresses=prior['addresses'],prior=prior['after'],expected=expected,after=after,slot=tx['slot'],fullStateMatched=True,transactionResubmitted=False))

# The executable source stays frozen; only the client's expected refusal changes.
driver=root/'scripts/test-byte-allowance-devnet.py';original=source/'scripts/test-byte-allowance-devnet.py'
old=original.read_bytes();new=driver.read_bytes()
normalized=old.replace(b'\r\n',b'\n');corrected=new.replace(b'\r\n',b'\n')
needle=b'    refuse("init-unsigned-book", ["payer:sw", f"{book}:w", SYSTEM], init_data, error="MissingRequiredSignature")'
replacement=b'    # The new account\'s signer privilege is enforced at the System CPI boundary.\n    refuse("init-unsigned-book", ["payer:sw", f"{book}:w", SYSTEM], init_data, error="PrivilegeEscalation")'
assert normalized.count(needle)==1 and normalized.replace(needle,replacement)==corrected
copy_path=base/'allowance-driver-corrected.py';copy_path.write_bytes(new)
adapter=base/'allowance-driver-adapter.py'
adapter.write_text('from pathlib import Path\n'+f'exec(compile(Path({str(copy_path)!r}).read_bytes(), {str(driver)!r}, "exec"), {{"__file__": {str(original)!r}, "__name__": "__main__"}})\n')
h['write'](out/'allowance-driver-provenance.json',dict(programSourceCommit=h['call'](['git','rev-parse','HEAD']),originalDriverSha256=hashlib.sha256(old).hexdigest(),correctedDriverSha256=hashlib.sha256(new).hexdigest(),adapterSha256=hashlib.sha256(adapter.read_bytes()).hexdigest(),change='Expect PrivilegeEscalation for an unsigned new account rejected at the System CPI boundary. All program source and state assertions unchanged.'))

code=(base/'application-suites-retry.py').read_text()
start=code.index('for name,package,script,args in singles:')
end=code.index("native=deploy('hopper-native-lifecycle-fixture')",start)
code=code[:start]+'''for name,package,script,args in singles:
 if name in ['sibling','runtime-gate','mint-plan','token-outcomes','named-vault']:
  completed=name+'-retry' if name=='runtime-gate' else name
  assert any(r['name']==completed and r['exitCode']==0 for r in rows);assert (source/'target/verification'/('suite-'+completed)/'receipt.json').is_file();continue
 if name=='byte-allowance':
  program=json.loads((work/package/'deployment.json').read_text())['programId'];name+='-retry'
  script=str(base/'allowance-driver-adapter.py')
 else:program=deploy(package)
 run(name,script,['--program',program,'--elf',elf(package),*args])
'''+code[end:]
code=code.replace("'scripts/'+script", "str(Path(script) if Path(script).is_absolute() else Path('scripts')/script)")
exec(compile(code,str(base/'application-suites-retry.py'),'exec'),{'__name__':'__main__','__file__':str(base/'application-suites-retry.py')})

