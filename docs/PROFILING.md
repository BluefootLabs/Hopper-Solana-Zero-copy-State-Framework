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

## Crate type and LTO

Keep `[lib] crate-type = ["cdylib"]` for a program crate. With an rlib in the
list (`["cdylib", "lib"]`), cargo drops `-C lto` for the cdylib and the
release profile's `lto = "fat"` does nothing for the on-chain artifact. The
effect depends on the program: on one toolchain (cargo-build-sbf 4.1.0,
platform-tools v1.54) the crate type alone took hopper-vault from 22,288 to
15,264 bytes and hopper-sentinel from 71,168 to 63,688 bytes, while
hopper-parity-vault (9,040 to 9,032) and hopper-counter (5,576 to 5,600) did
not move. Unit tests under `#[cfg(test)]` in `src/lib.rs` need no rlib, and
neither does `hopper compile --emit manifest --package`, which reads the
manifest from a `cargo test` run. An integration test in `tests/` that
imports the crate does need one, which is why the example programs with such
tests keep `"lib"`.

Sizes are also toolchain-dependent. The same hopper-counter source that CI
builds to 3,736 bytes with the pinned cargo-build-sbf 2.3.13 builds to 5,576
bytes with cargo-build-sbf 4.1.0 (platform-tools v1.54), at HEAD and after
the 2026-09-19 changes alike. Compare sizes only within one pinned toolchain.

## sBPF v3

`cargo build-sbf --arch v3` emits sBPF v3 bytecode, which mainnet-beta has
executed and accepted for deployment since slot 428,976,000; v0 remains
accepted because SIMD-0500 has no feature account. On cargo-build-sbf 4.1.0
(platform-tools v1.54) the switch alone took hopper-counter from 5,600 to
4,832 bytes and hopper-vault from 13,288 to 12,400 bytes. Compute-unit
figures for v3 have not been measured in this repository, so `hopper build`
keeps the toolchain default (v0); pass `--arch v3` to `cargo build-sbf` to
opt in, and treat published CU numbers as v0 until a v3 matrix exists.

## Size regression gate

`hopper profile elf <program.so> --baseline <folded.txt> --fail-on-growth
<bytes> --fail-on-growth-pct <pct>` exits 2 when `.text` grew by more than
both thresholds (defaults 512 bytes and 10% when only one flag is given),
listing the symbols that grew most. The dual gate keeps a 12% jump on a tiny
helper and a 30-byte jump on a large program from failing CI while a real
regression still does. Exit 0 means the gate passed and 1 means the run
itself failed.

## Tiny profile and size budget

Use `#[program(profile = "tiny")]` on programs whose public contract includes a
small binary budget. The macro emits `HOPPER_PROGRAM_PROFILE` and enforces the
first size-shape rules at compile time:

- instruction discriminators must be one byte so dispatch stays in the dense
	`match data[0]` form;
- handler-level modifier instrumentation is rejected, including `#[pipeline]`,
	`#[receipt]`, `#[invariant]`, and `#[access_control]`.

Use `profile = "strict"` or `profile = "audit"` when a program needs those
instrumented paths. Tiny programs still use typed contexts and Hopper account
validation; the profile only keeps extra audit scaffolding out of the binary.

The repository enforces a 16 KiB SBF budget for [../examples/hopper-counter](../examples/hopper-counter)
in the Solana SBF workflow. Keep that budget tied to the built `.so` size, not a
source estimate:

```bash
cargo build-sbf -- -p hopper-counter
program=$(find target -type f -name hopper_counter.so -print -quit)
stat -c%s "${program}"
```

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

The current clean same-provenance five-way vault snapshot is recorded in
[../BENCHMARKS.md](../BENCHMARKS.md) and content-addressed by
[`audit/framework-matrix-2026-08-16.json`](../audit/framework-matrix-2026-08-16.json).
It is fixture evidence for its exact source and toolchain pins. Regenerate and
archive a new clean run before changing current-framework claims after any pin
changes.

## Release checks

```powershell
cargo fmt -- --check
cargo check -q -p hopper-cli --locked
cargo run -q -p hopper-cli -- solana-check --all
hopper publish-check --package hopper-parity-vault --full
```

Use `solana-check` before `build-sbf` in CI so crate-shape regressions fail with
plain text diagnostics instead of late SBF errors.
