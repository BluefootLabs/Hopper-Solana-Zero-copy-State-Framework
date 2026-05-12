param(
    [switch]$Setup
)

$ErrorActionPreference = "Stop"

$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Error "cargo is required to run Hopper Kani proofs. Install Rust first."
}

$hasCargoKani = $false
try {
    cargo kani --version *> $null
    $hasCargoKani = $LASTEXITCODE -eq 0
} catch {
    $hasCargoKani = $false
}

if (-not $hasCargoKani) {
    $isNativeWindows = [System.Environment]::OSVersion.Platform -eq [System.PlatformID]::Win32NT
    $wsl = Get-Command wsl -ErrorAction SilentlyContinue
    if ($isNativeWindows -and $wsl) {
        $fullRepo = (Resolve-Path $repo).Path
        if ($fullRepo -match '^([A-Za-z]):\\(.*)$') {
            $drive = $Matches[1].ToLowerInvariant()
            $rest = $Matches[2] -replace '\\', '/'
            $wslRepo = "/mnt/$drive/$rest"
            Write-Host "cargo-kani is not installed in native Windows. Delegating to WSL."
            Write-Host "If this is the first WSL run, install with: wsl bash -lc 'cargo install --locked kani-verifier && cargo kani setup'"
            $setupArg = if ($Setup) { " --setup" } else { "" }
            wsl bash -lc "cd '$wslRepo' && sh ./scripts/kani-runtime-tail.sh$setupArg"
            exit $LASTEXITCODE
        }
    }

    Write-Host "cargo-kani is not installed on PATH."
    Write-Host "Install it on a Unix-like host or WSL with: cargo install --locked kani-verifier"
    Write-Host "Then run initial setup with: cargo kani setup"
    exit 1
}

if ($Setup) {
    cargo kani setup
}

cargo kani -p hopper-runtime