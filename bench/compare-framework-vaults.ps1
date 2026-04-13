param(
    [Parameter(Mandatory = $true)]
    [string]$QuasarRoot,

    [string]$OutDir = "bench\results\framework-vaults",

    [switch]$NoBuild
)

$ErrorActionPreference = "Stop"

$hopperRoot = Split-Path -Parent $PSScriptRoot
$resolvedQuasarRoot = (Resolve-Path -LiteralPath $QuasarRoot).Path
$resolvedOutDir = if ([System.IO.Path]::IsPathRooted($OutDir)) {
    $OutDir
} else {
    Join-Path $hopperRoot $OutDir
}

if (-not (Test-Path -LiteralPath (Join-Path $resolvedQuasarRoot "examples\vault\Cargo.toml"))) {
    throw "Quasar root does not look valid: missing examples\vault\Cargo.toml"
}

if (-not (Test-Path -LiteralPath (Join-Path $resolvedQuasarRoot "examples\pinocchio-vault\Cargo.toml"))) {
    throw "Quasar root does not include examples\pinocchio-vault; cannot benchmark the Pinocchio-style vault target"
}

New-Item -ItemType Directory -Force -Path $resolvedOutDir | Out-Null

function Invoke-CargoCapture {
    param(
        [Parameter(Mandatory = $true)]
        [string]$WorkingDirectory,

        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    Write-Host "==> cargo $($Arguments -join ' ')" -ForegroundColor Cyan

    $stdoutPath = [System.IO.Path]::GetTempFileName()
    $stderrPath = [System.IO.Path]::GetTempFileName()
    try {
        $startProcessArgs = @{
            FilePath = "cargo"
            ArgumentList = $Arguments
            WorkingDirectory = $WorkingDirectory
            NoNewWindow = $true
            Wait = $true
            PassThru = $true
            RedirectStandardOutput = $stdoutPath
            RedirectStandardError = $stderrPath
        }
        $process = Start-Process @startProcessArgs

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

        $text = ($stdout + $stderr).TrimEnd()

        if ($process.ExitCode -ne 0) {
            throw "cargo $($Arguments -join ' ') failed in $WorkingDirectory`n$text"
        }

        return $text
    }
    finally {
        Remove-Item -LiteralPath $stdoutPath, $stderrPath -Force -ErrorAction SilentlyContinue
    }
}

if (-not $NoBuild) {
    Invoke-CargoCapture -WorkingDirectory $hopperRoot -Arguments @("build-sbf", "--manifest-path", "examples/hopper-parity-vault/Cargo.toml") | Out-Null
    Invoke-CargoCapture -WorkingDirectory $resolvedQuasarRoot -Arguments @("build-sbf", "--manifest-path", "examples/vault/Cargo.toml") | Out-Null
    Invoke-CargoCapture -WorkingDirectory $resolvedQuasarRoot -Arguments @("build-sbf", "--manifest-path", "examples/pinocchio-vault/Cargo.toml") | Out-Null
}

$output = Invoke-CargoCapture -WorkingDirectory $hopperRoot -Arguments @(
    "run",
    "-p",
    "framework-vault-bench",
    "--",
    "--quasar-root",
    $resolvedQuasarRoot,
    "--out-dir",
    $resolvedOutDir
)

Write-Host ""
Write-Host $output