$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$arguments = @("run", "-p", "hopper-cli", "--", "profile", "bench") + $args
& cargo @arguments
exit $LASTEXITCODE
