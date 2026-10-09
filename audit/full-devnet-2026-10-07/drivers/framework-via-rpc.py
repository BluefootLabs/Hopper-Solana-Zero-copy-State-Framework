from pathlib import Path
import hashlib,json,os,runpy,sys
base=Path(__file__).resolve().parent;h=runpy.run_path(str(base/'devnet-harness.py'));os.environ.update(h['ENV']);script=h['SOURCE']/'scripts/test-framework-fixtures-devnet.py';module=runpy.run_path(str(script));original=module['run']
def run(argv):
 if argv[:3]==['solana','program','deploy']:argv=[*argv,'--use-rpc']
 return original(argv)
module['main'].__globals__['run']=run
module['main']()
h['write'](h['SOURCE']/'target/verification/framework-transport.json',dict(driverSha256=hashlib.sha256(script.read_bytes()).hexdigest(),adapterSha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),addedDeploymentFlag='--use-rpc',transactionPreflightEnabled=True))
