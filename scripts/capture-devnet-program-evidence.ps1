param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('Before', 'After')]
    [string]$Phase,

    [Parameter(Mandatory = $true)]
    [string]$Example,

    [Parameter(Mandatory = $true)]
    [string]$ProgramId,

    [Parameter(Mandatory = $true)]
    [string]$LocalElf,

    [Parameter(Mandatory = $true)]
    [string]$KeypairPath,

    [Parameter(Mandatory = $true)]
    [string]$OutputDirectory,

    [Parameter(Mandatory = $true)]
    [string]$DeploymentReceiptPath,

    [string]$ReceiptPath,
    [string]$SolanaCli = 'solana',
    [string]$RpcUrl = 'https://api.devnet.solana.com',
    [string]$ExpectedSolanaVersion = 'solana-cli 4.2.1',
    [string]$ExpectedGenesis = 'EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

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
        throw "$Label failed; command output omitted because it may contain local paths"
    }
    return $stdout.Trim()
}

function Get-JsonProperty {
    param(
        [Parameter(Mandatory = $true)] [object]$Value,
        [Parameter(Mandatory = $true)] [string[]]$Names
    )

    foreach ($name in $Names) {
        $property = $Value.PSObject.Properties |
            Where-Object { $_.Name -ieq $name } |
            Select-Object -First 1
        if ($null -ne $property) {
            return $property.Value
        }
    }
    return $null
}

function Require-CleanSource {
    param([Parameter(Mandatory = $true)] [string]$RepositoryRoot)

    $head = Invoke-NativeText -FilePath 'git' -Arguments @('-C', $RepositoryRoot, 'rev-parse', 'HEAD') -Label 'git rev-parse'
    if ($head -notmatch '^[0-9a-f]{40}$') {
        throw 'git HEAD is not a full 40-character commit'
    }
    $status = Invoke-NativeText -FilePath 'git' -Arguments @('-C', $RepositoryRoot, 'status', '--porcelain=v1', '--untracked-files=all') -Label 'git status'
    if ($status.Length -ne 0) {
        throw 'devnet release evidence requires a clean source tree'
    }
    return $head
}

function Get-ProgramSnapshot {
    param(
        [Parameter(Mandatory = $true)] [string]$DestinationPrefix,
        [Parameter(Mandatory = $true)] [string]$ExpectedLocalHash
    )

    $showText = Invoke-NativeText -FilePath $SolanaCli -Arguments @(
        '--url', $RpcUrl,
        '--keypair', $KeypairPath,
        'program', 'show', $ProgramId,
        '--output', 'json'
    ) -Label 'solana program show'
    try {
        $show = $showText | ConvertFrom-Json
    }
    catch {
        throw 'solana program show did not return valid JSON'
    }

    $reportedProgramId = Get-JsonProperty -Value $show -Names @('programId', 'program_id')
    if ($null -ne $reportedProgramId -and [string]$reportedProgramId -ne $ProgramId) {
        throw 'solana program show returned a different program id'
    }
    $programDataAddress = Get-JsonProperty -Value $show -Names @('programDataAddress', 'program_data_address')
    $loader = Get-JsonProperty -Value $show -Names @('owner', 'loader')
    $lastDeployedSlot = Get-JsonProperty -Value $show -Names @('lastDeployedSlot', 'lastDeploySlot', 'last_deployed_slot')
    $authority = Get-JsonProperty -Value $show -Names @('authority', 'upgradeAuthority', 'upgrade_authority')
    if ([string]::IsNullOrWhiteSpace([string]$programDataAddress)) {
        throw 'program metadata is missing the loader ProgramData address'
    }
    if ([string]$loader -ne 'BPFLoaderUpgradeab1e11111111111111111111111') {
        throw 'release evidence requires an upgradeable-loader program account'
    }
    if ($null -eq $lastDeployedSlot -or [uint64]$lastDeployedSlot -eq 0) {
        throw 'program metadata is missing a nonzero deployment slot'
    }
    if ([string]$authority -ne $expectedUpgradeAuthority) {
        throw 'the deployed program upgrade authority is not the explicit devnet signer'
    }

    $showPath = "$DestinationPrefix-program-show.json"
    [System.IO.File]::WriteAllText($showPath, "$showText`n", [System.Text.UTF8Encoding]::new($false))
    $programDataText = Invoke-NativeText -FilePath $SolanaCli -Arguments @(
        '--url', $RpcUrl,
        '--keypair', $KeypairPath,
        'account', [string]$programDataAddress,
        '--output', 'json'
    ) -Label 'solana ProgramData account'
    try {
        $null = $programDataText | ConvertFrom-Json
    }
    catch {
        throw 'solana account did not return valid ProgramData JSON'
    }
    $programDataPath = "$DestinationPrefix-program-data-account.json"
    [System.IO.File]::WriteAllText($programDataPath, "$programDataText`n", [System.Text.UTF8Encoding]::new($false))
    $dumpPath = "$DestinationPrefix-onchain.so"
    $null = Invoke-NativeText -FilePath $SolanaCli -Arguments @(
        '--url', $RpcUrl,
        '--keypair', $KeypairPath,
        'program', 'dump', $ProgramId, $dumpPath
    ) -Label 'solana program dump'
    if (-not (Test-Path -LiteralPath $dumpPath -PathType Leaf)) {
        throw 'solana program dump did not create the requested file'
    }
    $dumpHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $dumpPath).Hash.ToLowerInvariant()
    if ($dumpHash -ne $ExpectedLocalHash) {
        throw 'downloaded on-chain program bytes do not match the local release ELF'
    }

    return [ordered]@{
        programId = $ProgramId
        programDataAddress = [string]$programDataAddress
        loader = [string]$loader
        lastDeployedSlot = [uint64]$lastDeployedSlot
        upgradeAuthority = if ($null -eq $authority) { $null } else { [string]$authority }
        showSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $showPath).Hash.ToLowerInvariant()
        programDataAccountSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $programDataPath).Hash.ToLowerInvariant()
        elfSha256 = $dumpHash
    }
}

function Validate-Receipt {
    param([Parameter(Mandatory = $true)] [string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw 'the required devnet receipt does not exist'
    }
    try {
        $receipt = Get-Content -Raw -LiteralPath $Path | ConvertFrom-Json
    }
    catch {
        throw 'the devnet receipt is not valid JSON'
    }
    if ([string]$receipt.schema -ne 'hopper.devnet-evidence.v1') {
        throw 'the devnet receipt uses an unsupported schema'
    }
    if ([string]$receipt.example -ne $Example) {
        throw 'the devnet receipt example does not match this evidence capture'
    }
    if ([string]$receipt.commitment -ne 'finalized') {
        throw 'the devnet receipt is not finalized'
    }
    if ([string]$receipt.cluster.genesis_hash -ne $ExpectedGenesis) {
        throw 'the devnet receipt has the wrong genesis hash'
    }
    if ([string]::IsNullOrWhiteSpace([string]$receipt.cluster.node_version)) {
        throw 'the devnet receipt is missing the RPC node version'
    }
    if ($null -eq $receipt.transactions -or @($receipt.transactions).Count -eq 0) {
        throw 'the devnet receipt contains no transactions'
    }
    foreach ($transaction in @($receipt.transactions)) {
        $slot = Get-JsonProperty -Value $transaction -Names @('slot', 'finalizedSlot', 'finalized_slot')
        $outcome = Get-JsonProperty -Value $transaction -Names @('outcome')
        if ([string]::IsNullOrWhiteSpace([string]$transaction.signature) -or [uint64]$slot -eq 0) {
            throw 'every receipt transaction must include a signature and finalized slot'
        }
        if ([string]$outcome -notin @('succeeded', 'rejected')) {
            throw 'every receipt transaction must record a succeeded or rejected outcome'
        }
    }

    $singleProgram = Get-JsonProperty -Value $receipt -Names @('programId', 'program_id')
    $programMatch = $null -ne $singleProgram -and [string]$singleProgram -eq $ProgramId
    $programs = Get-JsonProperty -Value $receipt -Names @('programs')
    if (-not $programMatch -and $null -ne $programs) {
        $programMatch = @($programs.PSObject.Properties.Value) -contains $ProgramId
    }
    if (-not $programMatch) {
        throw 'the devnet receipt does not name the captured program id'
    }
}

if ($RpcUrl -ne 'https://api.devnet.solana.com') {
    throw 'release evidence must use the public Solana devnet RPC endpoint'
}

$repositoryRoot = Invoke-NativeText -FilePath 'git' -Arguments @('rev-parse', '--show-toplevel') -Label 'git root'
$repositoryRoot = [System.IO.Path]::GetFullPath($repositoryRoot)
$sourceCommit = Require-CleanSource -RepositoryRoot $repositoryRoot
$localElfPath = (Resolve-Path -LiteralPath $LocalElf).Path
$keypairFullPath = (Resolve-Path -LiteralPath $KeypairPath).Path
$deploymentReceiptFullPath = (Resolve-Path -LiteralPath $DeploymentReceiptPath).Path
try {
    $deploymentReceipt = Get-Content -Raw -LiteralPath $deploymentReceiptFullPath | ConvertFrom-Json
}
catch {
    throw 'the deployment receipt is not valid JSON'
}
$deploymentProgramId = Get-JsonProperty -Value $deploymentReceipt -Names @('programId', 'program_id')
$deploymentSignature = Get-JsonProperty -Value $deploymentReceipt -Names @('signature', 'transactionSignature', 'transaction_signature')
if ([string]$deploymentProgramId -ne $ProgramId -or [string]::IsNullOrWhiteSpace([string]$deploymentSignature)) {
    throw 'the deployment receipt does not bind the requested program id and signature'
}
$deploymentReceiptHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $deploymentReceiptFullPath).Hash.ToLowerInvariant()
$outputPath = [System.IO.Path]::GetFullPath($OutputDirectory)
$version = Invoke-NativeText -FilePath $SolanaCli -Arguments @('--version') -Label 'solana version'
if (-not $version.StartsWith($ExpectedSolanaVersion, [System.StringComparison]::Ordinal)) {
    throw "expected $ExpectedSolanaVersion but found $version"
}
$expectedUpgradeAuthority = Invoke-NativeText -FilePath $SolanaCli -Arguments @(
    '--keypair', $keypairFullPath,
    'address'
) -Label 'solana signer address'
$genesis = Invoke-NativeText -FilePath $SolanaCli -Arguments @('--url', $RpcUrl, '--keypair', $keypairFullPath, 'genesis-hash') -Label 'solana genesis-hash'
if ($genesis -ne $ExpectedGenesis) {
    throw 'refusing to capture evidence from a non-devnet cluster'
}
$localHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $localElfPath).Hash.ToLowerInvariant()

if ($Phase -eq 'Before') {
    if (Test-Path -LiteralPath $outputPath) {
        throw 'the evidence output directory must not exist before the Before phase'
    }
    $null = New-Item -ItemType Directory -Path $outputPath
    Copy-Item -LiteralPath $localElfPath -Destination (Join-Path $outputPath 'release-local.so')
    Copy-Item -LiteralPath $deploymentReceiptFullPath -Destination (Join-Path $outputPath 'deployment-receipt.json')
    $deploymentConfirmation = Invoke-NativeText -FilePath $SolanaCli -Arguments @(
        '--url', $RpcUrl,
        '--keypair', $keypairFullPath,
        '--commitment', 'finalized',
        'confirm', [string]$deploymentSignature,
        '--output', 'json'
    ) -Label 'finalized deployment confirmation'
    try {
        $null = $deploymentConfirmation | ConvertFrom-Json
    }
    catch {
        throw 'solana confirm did not return valid deployment JSON'
    }
    $deploymentConfirmationPath = Join-Path $outputPath 'deployment-finalized.json'
    [System.IO.File]::WriteAllText($deploymentConfirmationPath, "$deploymentConfirmation`n", [System.Text.UTF8Encoding]::new($false))
    $before = Get-ProgramSnapshot -DestinationPrefix (Join-Path $outputPath 'before') -ExpectedLocalHash $localHash
    $state = [ordered]@{
        schema = 'hopper.devnet-program-capture.v1'
        example = $Example
        sourceCommit = $sourceCommit
        genesisHash = $genesis
        solanaVersion = $version
        localElfSha256 = $localHash
        deploymentReceiptSha256 = $deploymentReceiptHash
        deploymentConfirmationSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $deploymentConfirmationPath).Hash.ToLowerInvariant()
        before = $before
    }
    $state | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $outputPath 'capture-state.json') -Encoding utf8NoBOM
    Write-Output "captured pre-test program evidence for $Example"
    exit 0
}

if (-not (Test-Path -LiteralPath $outputPath -PathType Container)) {
    throw 'the Before phase evidence directory is missing'
}
$statePath = Join-Path $outputPath 'capture-state.json'
$state = Get-Content -Raw -LiteralPath $statePath | ConvertFrom-Json
if ([string]$state.schema -ne 'hopper.devnet-program-capture.v1' -or [string]$state.example -ne $Example) {
    throw 'the Before phase state does not match this evidence capture'
}
if ([string]$state.sourceCommit -ne $sourceCommit -or [string]$state.localElfSha256 -ne $localHash) {
    throw 'source commit or local ELF changed between evidence phases'
}
if ([string]$state.deploymentReceiptSha256 -ne $deploymentReceiptHash) {
    throw 'deployment receipt changed between evidence phases'
}
$deploymentConfirmationPath = Join-Path $outputPath 'deployment-finalized.json'
if (-not (Test-Path -LiteralPath $deploymentConfirmationPath -PathType Leaf) -or
    [string]$state.deploymentConfirmationSha256 -ne (Get-FileHash -Algorithm SHA256 -LiteralPath $deploymentConfirmationPath).Hash.ToLowerInvariant()) {
    throw 'the finalized deployment confirmation is missing or changed'
}
if ([string]::IsNullOrWhiteSpace($ReceiptPath)) {
    throw 'ReceiptPath is required for the After phase'
}
Validate-Receipt -Path $ReceiptPath
$after = Get-ProgramSnapshot -DestinationPrefix (Join-Path $outputPath 'after') -ExpectedLocalHash $localHash
foreach ($field in @('programId', 'programDataAddress', 'loader', 'lastDeployedSlot', 'upgradeAuthority', 'programDataAccountSha256', 'elfSha256')) {
    if ([string]$state.before.$field -ne [string]$after.$field) {
        throw "program metadata field $field changed during the devnet test"
    }
}
Copy-Item -LiteralPath $ReceiptPath -Destination (Join-Path $outputPath 'receipt.json')

$provenance = [ordered]@{
    schema = 'hopper.devnet-program-evidence.v1'
    example = $Example
    sourceCommit = $sourceCommit
    rpcEndpoint = 'https://api.devnet.solana.com'
    genesisHash = $genesis
    solanaVersion = $version
    localElfSha256 = $localHash
    deploymentReceiptSha256 = $deploymentReceiptHash
    deploymentConfirmationSha256 = [string]$state.deploymentConfirmationSha256
    deploymentSignature = [string]$deploymentSignature
    program = $after
    receiptSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $outputPath 'receipt.json')).Hash.ToLowerInvariant()
    treeCleanBeforeAndAfter = $true
}
$provenance | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $outputPath 'provenance.json') -Encoding utf8NoBOM

$payloads = Get-ChildItem -LiteralPath $outputPath -File |
    Where-Object { $_.Name -notin @('SHA256SUMS', 'BUNDLE.SHA256') } |
    Sort-Object Name
$sumLines = foreach ($file in $payloads) {
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $file.FullName).Hash.ToLowerInvariant()
    "$hash  $($file.Name)"
}
$sumsPath = Join-Path $outputPath 'SHA256SUMS'
[System.IO.File]::WriteAllText($sumsPath, (($sumLines -join "`n") + "`n"), [System.Text.UTF8Encoding]::new($false))
$bundleHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $sumsPath).Hash.ToLowerInvariant()
[System.IO.File]::WriteAllText((Join-Path $outputPath 'BUNDLE.SHA256'), "$bundleHash  SHA256SUMS`n", [System.Text.UTF8Encoding]::new($false))

$finalHead = Require-CleanSource -RepositoryRoot $repositoryRoot
if ($finalHead -ne $sourceCommit) {
    throw 'source HEAD changed during evidence capture'
}
Write-Output "captured finalized devnet program evidence for $Example"
Write-Output "bundle SHA-256: $bundleHash"
