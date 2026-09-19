param(
    [Parameter(Mandatory = $true)]
    [string]$KeypairPath,

    [Parameter(Mandatory = $true)]
    [string]$ProgramKeypairPath,

    [Parameter(Mandatory = $true)]
    [string]$SbfOutDirectory,

    [Parameter(Mandatory = $true)]
    [string]$DeployReceipt,

    [Parameter(Mandatory = $true)]
    [string]$EvidenceDirectory,

    [string]$RpcUrl = 'https://api.devnet.solana.com',
    [string]$SolanaCli = 'solana',
    [string]$CargoBuildSbf = 'cargo-build-sbf',
    [string]$ExpectedSolanaVersion = 'solana-cli 4.2.1'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$DEVNET_GENESIS = 'EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG'

function Invoke-NativeText {
    param(
        [Parameter(Mandatory = $true)] [string]$FilePath,
        [Parameter(Mandatory = $true)] [string[]]$Arguments,
        [Parameter(Mandatory = $true)] [string]$Label
    )

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $FilePath
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($argument in $Arguments) {
        $startInfo.ArgumentList.Add($argument)
    }

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    if (-not $process.Start()) {
        throw "$Label could not be started"
    }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $process.WaitForExit()
    $stdout = $stdoutTask.GetAwaiter().GetResult()
    $null = $stderrTask.GetAwaiter().GetResult()
    if ($process.ExitCode -ne 0) {
        throw "$Label failed; output omitted because it may contain local paths"
    }
    return $stdout.Trim()
}

if ($RpcUrl -ne 'https://api.devnet.solana.com') {
    throw 'the release helper only permits the public Solana devnet endpoint'
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$payer = (Resolve-Path -LiteralPath $KeypairPath).Path
$programKeypair = (Resolve-Path -LiteralPath $ProgramKeypairPath).Path
$sbfOut = [System.IO.Path]::GetFullPath($SbfOutDirectory)
$deployReceiptPath = [System.IO.Path]::GetFullPath($DeployReceipt)
$evidencePath = [System.IO.Path]::GetFullPath($EvidenceDirectory)
$targetRoot = [System.IO.Path]::GetFullPath((Join-Path $repoRoot 'target'))
$targetPrefix = $targetRoot.TrimEnd([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar
foreach ($path in @($programKeypair, $sbfOut, $deployReceiptPath, $evidencePath)) {
    if (-not $path.StartsWith($targetPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'program keypair and all generated output must stay below the repository target directory'
    }
}

if (Test-Path -LiteralPath $sbfOut) {
    throw 'SbfOutDirectory must not exist before the release build'
}
if (Test-Path -LiteralPath $deployReceiptPath) {
    throw 'DeployReceipt must not exist before deployment'
}
if (Test-Path -LiteralPath $evidencePath) {
    throw 'EvidenceDirectory must not exist before evidence capture'
}

$sourceCommit = Invoke-NativeText -FilePath 'git' -Arguments @('-C', $repoRoot, 'rev-parse', 'HEAD') -Label 'git rev-parse'
if ($sourceCommit -notmatch '^[0-9a-f]{40}$') {
    throw 'source HEAD is not a full commit hash'
}
$status = Invoke-NativeText -FilePath 'git' -Arguments @('-C', $repoRoot, 'status', '--porcelain=v1', '--untracked-files=all') -Label 'git status'
if ($status.Length -ne 0) {
    throw 'the release helper requires a clean source tree'
}

$version = Invoke-NativeText -FilePath $SolanaCli -Arguments @('--version') -Label 'solana version'
if (-not $version.StartsWith($ExpectedSolanaVersion, [System.StringComparison]::Ordinal)) {
    throw "expected $ExpectedSolanaVersion but found $version"
}
$genesis = Invoke-NativeText -FilePath $SolanaCli -Arguments @('--url', $RpcUrl, '--keypair', $payer, 'genesis-hash') -Label 'solana genesis-hash'
if ($genesis -ne $DEVNET_GENESIS) {
    throw 'refusing to deploy to a cluster other than public devnet'
}

$manifestPath = Join-Path $PSScriptRoot 'Cargo.toml'
$null = Invoke-NativeText -FilePath $CargoBuildSbf -Arguments @(
    '--manifest-path', $manifestPath,
    '--sbf-out-dir', $sbfOut,
    '--', '--locked'
) -Label 'locked Token-2022 vault SBF build'
$elf = Join-Path $sbfOut 'hopper_token_2022_vault.so'
if (-not (Test-Path -LiteralPath $elf -PathType Leaf)) {
    throw 'the isolated SBF build did not produce hopper_token_2022_vault.so'
}

$programId = Invoke-NativeText -FilePath $SolanaCli -Arguments @('--keypair', $programKeypair, 'address') -Label 'program keypair address'
$deployJson = Invoke-NativeText -FilePath $SolanaCli -Arguments @(
    '--url', $RpcUrl,
    '--keypair', $payer,
    '--commitment', 'finalized',
    'program', 'deploy', $elf,
    '--program-id', $programKeypair,
    '--output', 'json'
) -Label 'Token-2022 vault deployment'
try {
    $deploy = $deployJson | ConvertFrom-Json
}
catch {
    throw 'solana program deploy did not return valid JSON'
}
if ([string]$deploy.programId -ne $programId -or [string]::IsNullOrWhiteSpace([string]$deploy.signature)) {
    throw 'deployment output does not contain the expected program id and signature'
}
$receiptParent = Split-Path -Parent $deployReceiptPath
if (-not (Test-Path -LiteralPath $receiptParent -PathType Container)) {
    $null = New-Item -ItemType Directory -Path $receiptParent
}
[System.IO.File]::WriteAllText($deployReceiptPath, "$deployJson`n", [System.Text.UTF8Encoding]::new($false))

& (Join-Path $repoRoot 'scripts\capture-devnet-program-evidence.ps1') `
    -Phase Before `
    -Example 'hopper-token-2022-vault' `
    -ProgramId $programId `
    -LocalElf $elf `
    -KeypairPath $payer `
    -DeploymentReceiptPath $deployReceiptPath `
    -OutputDirectory $evidencePath `
    -SolanaCli $SolanaCli

Write-Output "deployed hopper-token-2022-vault as $programId"
Write-Output "local ELF SHA-256: $((Get-FileHash -Algorithm SHA256 -LiteralPath $elf).Hash.ToLowerInvariant())"
Write-Output 'Run the finalized devnet test, then run capture-devnet-program-evidence.ps1 -Phase After.'
