# Canonical PDA regression

One fixture compares runtime canonical search, a `canonical_pda!` constant,
generated context binding, explicit repeated validation, public per-field
validation, typed binding and typed seed helpers. Every mode must accept the
SDK-derived canonical address and bump. Every other bump, every valid
noncanonical off-curve address, and an unrelated address must be rejected.
The verifier compares complete account snapshots after every invocation.

Mode 3 deliberately calls validation before binding. It is a synthetic
repeated-work comparison, not a measurement of a historical Hopper binary.
The fixture has no application mutations and does not measure a complete
protocol or rank frameworks. Literal address checks do not replace ownership,
layout, signer or writable validation in application code.

```powershell
cargo build-sbf --manifest-path bench/canonical-pda/program/Cargo.toml --tools-version v1.54 --arch v0 --sbf-out-dir target/hopper/canonical-pda/v0 -- --locked
$env:HOPPER_CANONICAL_PDA_SBF = 'target/hopper/canonical-pda/v0/hopper_canonical_pda_fixture.so'
cargo test -p hopper-framework-verifier --test canonical_pda_sbf --locked -- --ignored --nocapture
```

The test is ignored by the generic host suite because it requires a compiled
SBF ELF. Build and invoke it explicitly for binary-backed validation.
