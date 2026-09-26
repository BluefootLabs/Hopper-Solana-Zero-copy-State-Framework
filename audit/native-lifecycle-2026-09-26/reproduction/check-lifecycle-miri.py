from pathlib import Path
import os,subprocess,json
out=Path('target/hopper/native-replacement-2026-09-26');env=os.environ.copy();env.update(CARGO_BUILD_JOBS='2',CARGO_INCREMENTAL='0',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0')
records=[]
for test in ['mapped_borrows','lifecycle_preflight']:
 cmd=['cargo','+nightly','miri','test','-p','hopper-native','--test',test,'--locked','--offline']
 log=out/('miri-'+test+'.log')
 with log.open('w',encoding='utf-8') as f:rc=subprocess.run(cmd,env=env,stdout=f,stderr=subprocess.STDOUT).returncode
 records.append(dict(test=test,command=cmd,exitCode=rc))
 if rc:print(log.read_text(encoding='utf-8')[-4500:]);raise SystemExit(rc)
 print('miri '+test+': passed',flush=True)
(out/'miri-receipt.json').write_text(json.dumps(records,indent=2)+'\n')
