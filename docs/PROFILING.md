# Profiling Hopper programs

Hopper keeps profiling tied to concrete artifacts. Use the CLI to inspect the
same SBF binary you deploy, then keep benchmark claims tied to exact commits,
lockfiles, and toolchain versions.

## Static SBF profile

```powershell
cargo build-sbf --manifest-path examples/hopper-vault/Cargo.toml
hopper profile elf target/deploy/hopper_vault.so
hopper profile elf target/deploy/hopper_vault.so --json > target/hopper-vault-profile.json
```

`hopper profile elf` reports section sizes, symbols, CU-ish static estimates,
and flamegraph export data. Treat it as a repeatable binary inspection tool, not
as a substitute for live compute-unit measurements.

## Same-provenance benchmark flow

```powershell
git rev-parse HEAD
cargo tree -p hopper-lang --locked > target/hopper-tree.txt
cargo build-sbf --manifest-path examples/hopper-parity-vault/Cargo.toml
hopper profile elf target/deploy/hopper_parity_vault.so --json > target/hopper-parity-vault-profile.json
```

Record the following next to any public benchmark claim:

- Hopper commit and `Cargo.lock` hash.
- Solana CLI and `cargo-build-sbf` versions.
- Exact manifest path and output `.so` path.
- `hopper profile elf` JSON.
- Live CU logs when the claim depends on runtime execution.

The current same-provenance vault benchmark snapshot remains in
[../BENCHMARKS.md](../BENCHMARKS.md). Regenerate it before changing launch or
benchmark claims.

## Release checks

```powershell
cargo fmt -- --check
cargo check -q -p hopper-cli --locked
cargo run -q -p hopper-cli -- solana-check --all
hopper publish-check --package hopper-parity-vault --full
```

Use `solana-check` before `build-sbf` in CI so crate-shape regressions fail with
plain text diagnostics instead of late SBF errors.