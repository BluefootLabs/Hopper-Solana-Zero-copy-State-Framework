# Bundled SPL programs for `hopper-svm`

This directory is the canonical location for SPL Token / Token-2022 / Associated Token Account `.so` ELFs that integration tests load through the Agave runtime path.

Hopper does **not** check the binaries into the repository. They are 200-300 KB each, version-tracked by Anza on a release cadence we don't control, and licensed under terms (Apache-2.0) that the Hopper repo would have to attribute on every clone. The harness reads them in via `include_bytes!` at the test binary's compile time, so the source-of-truth for which version is in use stays with the test author.

## Where to source the bytes

Three canonical sources, in decreasing order of stability:

1. **Anza's release artifacts** at `https://github.com/solana-labs/solana-program-library/releases`. Pick the matching SPL Token / Token-2022 / ATA release for the Solana toolchain version you build against. Each release attaches the prebuilt `.so` files.

2. **Local Solana SDK install** at `~/.config/solana/install/active_release/lib/`. The Solana CLI ships the canonical SPL programs there. Copy `spl_token-3.5.0.so`, `spl_token_2022-1.0.0.so`, and `spl_associated_token_account-1.1.1.so` (versions vary by toolchain).

3. **Build from source**. Clone `solana-labs/solana-program-library`, run `cargo build-sbf` in each program crate, copy the `target/deploy/*.so` artefact here.

## Expected filenames

The shipping integration test (`crates/hopper-svm/tests/agave_spl_token_cpi.rs`) reads:

```
crates/hopper-svm/programs/spl_token.so
crates/hopper-svm/programs/spl_token_2022.so
crates/hopper-svm/programs/spl_associated_token_account.so
```

A test that wants a specific version can use a different name and call the loader by name; the three filenames above are just the integration-test default.

## Loading at test time

```rust
use hopper_svm::HopperSvm;

let svm = HopperSvm::new()
    .with_agave_runtime()
    .with_real_spl_token(include_bytes!("../programs/spl_token.so"))?
    .with_real_spl_token_2022(include_bytes!("../programs/spl_token_2022.so"))?
    .with_real_spl_associated_token(include_bytes!("../programs/spl_associated_token_account.so"))?;
```

After these calls, every `process_instruction` whose program ID is `SPL_TOKEN_PROGRAM_ID` / `SPL_TOKEN_2022_PROGRAM_ID` / `ASSOCIATED_TOKEN_PROGRAM_ID` dispatches through Agave's real BPF loader running the canonical ELF. The harness reports `compute_units_consumed` as the ELF's actual CU cost, not a synthetic value.

## Verification

Once the bytes are in place, run:

```bash
cargo test -p hopper-svm --features agave-runtime --test agave_spl_token_cpi
```

The test mints, transfers, and burns SPL tokens through the real on-chain program, asserting balances against the mainnet-fidelity execution path.

## Why this matters

The headline difference between `hopper-svm`'s Phase 3 and other in-process Solana harnesses is execution fidelity. The Agave runtime running here is the same code the validator runs, against the same SPL ELFs the validator runs, with the same syscall surface and CPI dispatch. A `TransferChecked` CPI that succeeds in this harness will succeed on mainnet at the byte level.

A test author who pulls SPL bytes from a different version (e.g. SPL Token v3.4 vs v3.5) gets predictable, version-pinned coverage. Hopper does not paper over upstream version drift; the test fails loudly when the loaded ELF's wire shape doesn't match the test's expectations, which is the right behaviour for a mainnet-fidelity harness.
