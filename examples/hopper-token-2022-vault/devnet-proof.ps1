param(
    [string]$RpcUrl = "https://api.devnet.solana.com",
    [switch]$SkipDeploy,
    [string]$DeployReceipt = "examples/hopper-token-2022-vault/devnet-deploy.json",
    [string]$KeypairPath
)

$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\.." )).Path
$manifestPath = "@examples/hopper-token-2022-vault/hopper.manifest.json"
$loweredPath = "examples/hopper-token-2022-vault/lowered.rs"

function Invoke-HopperCli {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    $stdoutPath = [System.IO.Path]::GetTempFileName()
    $stderrPath = [System.IO.Path]::GetTempFileName()

    try {
        $process = Start-Process -FilePath "cargo" `
            -ArgumentList (@("run", "-q", "-p", "hopper-cli", "--") + $Arguments) `
            -WorkingDirectory $repoRoot `
            -NoNewWindow `
            -Wait `
            -PassThru `
            -RedirectStandardOutput $stdoutPath `
            -RedirectStandardError $stderrPath

        $stdout = if ((Get-Item -LiteralPath $stdoutPath).Length -gt 0) {
            Get-Content -LiteralPath $stdoutPath -Raw
        } else {
            ""
        }
        $stderr = if ((Get-Item -LiteralPath $stderrPath).Length -gt 0) {
            Get-Content -LiteralPath $stderrPath -Raw
        } else {
            ""
        }

        if ($process.ExitCode -ne 0) {
            $message = ($stdout + $stderr).Trim()
            if ([string]::IsNullOrWhiteSpace($message)) {
                $message = "hopper-cli exited with code $($process.ExitCode)"
            }
            throw $message
        }

        if (-not [string]::IsNullOrWhiteSpace($stderr)) {
            Write-Host $stderr.TrimEnd()
        }

        return $stdout.TrimEnd()
    }
    finally {
        Remove-Item -LiteralPath $stdoutPath, $stderrPath -Force -ErrorAction SilentlyContinue
    }
}

function Invoke-Step {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Description,

        [Parameter(Mandatory = $true)]
        [scriptblock]$Action
    )

    Write-Host "==> $Description" -ForegroundColor Cyan
    & $Action
}

Push-Location $repoRoot
try {
    Invoke-Step -Description "Emit lowered Rust preview for hopper-token-2022-vault" -Action {
        Invoke-HopperCli -Arguments @("compile", "--emit", "rust", "--package", "hopper-token-2022-vault", "--out", $loweredPath, "--force")
    }

    Invoke-Step -Description "Explain the typed context surface from the local manifest" -Action {
        Invoke-HopperCli -Arguments @("explain", "context", $manifestPath)
    }

    Invoke-Step -Description "Build the SBF artifact through hopper-cli" -Action {
        Invoke-HopperCli -Arguments @("build", "-p", "hopper-token-2022-vault")
    }

    if ($SkipDeploy) {
        Write-Host "Skipping deploy. Local proof flow completed." -ForegroundColor Yellow
        return
    }

    if (-not (Get-Command solana -ErrorAction SilentlyContinue)) {
        throw "solana CLI was not found on PATH. Install the Solana CLI or rerun with -SkipDeploy."
    }

    Invoke-Step -Description "Deploy hopper-token-2022-vault to devnet through hopper-cli" -Action {
        $deployArgs = @("deploy", "--no-build", "-p", "hopper-token-2022-vault", "--url", $RpcUrl, "--output", "json")
        if (-not [string]::IsNullOrWhiteSpace($KeypairPath)) {
            $deployArgs += @("--keypair", $KeypairPath)
        }

        $deployOutput = Invoke-HopperCli -Arguments $deployArgs
        $deployText = ($deployOutput | Out-String).Trim()

        $receiptPath = if ([System.IO.Path]::IsPathRooted($DeployReceipt)) {
            $DeployReceipt
        } else {
            Join-Path $repoRoot $DeployReceipt
        }
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $receiptPath) | Out-Null
        Set-Content -LiteralPath $receiptPath -Value $deployText

        $receipt = $deployText | ConvertFrom-Json
        if ($null -ne $receipt.programId) {
            Write-Host "Program ID: $($receipt.programId)" -ForegroundColor Green
            & solana program show $receipt.programId --url $RpcUrl
        }
    }
}
finally {
    Pop-Location
}