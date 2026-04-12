<#
.SYNOPSIS
    Run the Hopper primitive benchmark lab against a Docker-managed Solana
    test validator.

.DESCRIPTION
    1. Starts a Solana test validator container via Docker Compose.
    2. Waits until the RPC endpoint at http://127.0.0.1:8899 reports healthy.
    3. Delegates to `hopper profile bench` with the validator URL and a local
       bench keypair (generated automatically on first run).
    4. Always stops the container in the finally block.

.PARAMETER BenchArgs
    Additional arguments forwarded verbatim to `hopper profile bench`.
    For example: --out-dir bench\results --fail-on-regression 10

.EXAMPLE
    # Full run with defaults
    .\run-bench-docker.ps1

    # Skip rebuild, write output elsewhere
    .\run-bench-docker.ps1 --no-build --out-dir C:\bench-output

    # Override Solana validator version via environment
    $env:SOLANA_IMAGE = "anzaxyz/agave:v2.3.13"
    .\run-bench-docker.ps1
#>
[CmdletBinding()]
param(
    [Parameter(ValueFromRemainingArguments)]
    [string[]]$BenchArgs
)

$ErrorActionPreference = "Stop"

$ScriptDir   = Split-Path -Parent $MyInvocation.MyCommand.Path
$RootDir     = Split-Path -Parent $ScriptDir
$Compose     = Join-Path $ScriptDir "docker\docker-compose.yml"
$FixturesDir = Join-Path $ScriptDir "fixtures"
$KeypairPath = Join-Path $FixturesDir "bench-keypair.json"
$RpcUrl      = "http://127.0.0.1:8899"
$ValidatorReadyTimeout = 60   # seconds

# ── Prerequisites ─────────────────────────────────────────────────────────────

if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
    Write-Error "docker is not installed or not on PATH. Install Docker Desktop and retry."
    exit 1
}

# ── Bench keypair ─────────────────────────────────────────────────────────────
# A dedicated keypair for benchmark transactions. The test validator's built-in
# airdrop faucet funds it automatically (no real SOL involved).

if (-not (Test-Path $KeypairPath)) {
    Write-Host "Generating bench keypair at $KeypairPath ..."
    New-Item -ItemType Directory -Force -Path $FixturesDir | Out-Null

    if (-not (Get-Command solana-keygen -ErrorAction SilentlyContinue)) {
        Write-Error "solana-keygen is not on PATH. Install the Solana CLI toolchain."
        exit 1
    }

    & solana-keygen new --no-bip39-passphrase --outfile $KeypairPath
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

# ── Start validator ───────────────────────────────────────────────────────────

Write-Host "Starting Solana test validator (Docker Compose) ..."
& docker compose -f $Compose up -d validator
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$ExitCode = 1
try {
    # ── Wait for healthy ──────────────────────────────────────────────────────
    Write-Host "Waiting for validator at $RpcUrl ..."
    $Deadline = [DateTime]::Now.AddSeconds($ValidatorReadyTimeout)
    $Ready    = $false

    while ([DateTime]::Now -lt $Deadline) {
        try {
            # /health returns {"detail":"ok"} on HTTP 200 when the validator is
            # ready to serve. Invoke-RestMethod throws on non-2xx.
            $resp = Invoke-RestMethod -Uri "$RpcUrl/health" -TimeoutSec 2 -ErrorAction Stop
            # Accept both plain-string "ok" and object {"detail":"ok"}.
            if ($resp -eq "ok" -or $resp.detail -eq "ok") {
                $Ready = $true
                break
            }
        }
        catch { }
        Start-Sleep -Milliseconds 500
    }

    if (-not $Ready) {
        Write-Error "Solana test validator did not become healthy within $ValidatorReadyTimeout seconds."
        exit 1
    }

    Write-Host "Validator ready."

    # ── Run benchmark ─────────────────────────────────────────────────────────
    $CargoArgs = @(
        "run", "-p", "hopper-cli", "--",
        "profile", "bench",
        "--rpc",     $RpcUrl,
        "--keypair", $KeypairPath
    ) + $BenchArgs

    Set-Location $RootDir
    & cargo @CargoArgs
    $ExitCode = $LASTEXITCODE
}
finally {
    Write-Host "Stopping Solana test validator ..."
    & docker compose -f $Compose down
}

exit $ExitCode
