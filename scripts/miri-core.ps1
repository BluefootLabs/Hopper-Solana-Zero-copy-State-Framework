# Miri lane over Hopper's aliasing core, under Tree Borrows.
# See miri-core.sh for the full rationale; this is its PowerShell twin.
#
# Scope: the modules where every `&`/`&mut` handed to user code is
# manufactured from raw loader-input pointers — the segment borrow
# ledger (+ the instruction-ambient touch log), the write-policy gate
# and its ambient lamport store, the native-boundary transmutes, and
# the account borrow registry. Deterministic unit tests only (proptest
# modules write regression files, which Miri's isolation refuses; the
# same generators run at full case counts in the normal lanes).

$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Error "cargo is required to run the Hopper Miri lane. Install Rust first."
}

$components = rustup component list --toolchain nightly 2>$null
if (-not ($components | Select-String '^miri.*\(installed\)')) {
    Write-Error "Miri is not installed on the nightly toolchain. Install it with: rustup component add miri rust-src --toolchain nightly"
}

$env:MIRIFLAGS = "-Zmiri-tree-borrows"
cargo +nightly miri test -p hopper-runtime --lib --features touch-map -- `
    segment_borrow::tests `
    segment_borrow::touch_map_tests `
    write_policy::tests `
    context::write_policy_tests `
    native_boundary `
    borrow_registry::tests
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
