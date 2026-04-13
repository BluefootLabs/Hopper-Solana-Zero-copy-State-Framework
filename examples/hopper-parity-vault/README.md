# Hopper Parity Vault

This example is the fair comparison target for Hopper versus Quasar's `vault`
example and Quasar's `pinocchio-vault` example.

## What It Demonstrates

- PDA validation with `find_and_verify_pda`
- a system-program transfer CPI on deposit
- direct lamport mutation on withdraw from a program-owned PDA
- the minimal Hopper-owned instruction surface needed for an apples-to-apples
  framework comparison

## Instruction Map

- `0` = `Deposit`
- `1` = `Withdraw`
- `2` = `Authorize`

## Why This Exists

`examples/hopper-vault` is a Hopper feature demo with initialization, zero-copy
state, and phased execution. That is useful for showing Hopper's surface area,
but it is not the right benchmark target when the goal is a fair comparison to
Quasar's minimal `vault` and `pinocchio-vault` examples.

`hopper-parity-vault` keeps only the shared vault semantics so the benchmark can
measure framework overhead instead of example-specific features.

## Verify

```bash
cargo check -p hopper-parity-vault
hopper build -p hopper-parity-vault
```

## Benchmark Path

The fair comparison runner lives in `bench/framework-vault-bench` and is driven
through `bench/compare-framework-vaults.ps1`.

The runner averages 8 shared deterministic user seed cases across Hopper,
Quasar, and the Pinocchio-style target so the comparison is not dominated by a
single PDA bump outcome.

It now covers four matched instruction paths:

- authorize: signer + writable + PDA validation only
- counter-access: signer + writable + PDA validation plus a raw `[authority:32][counter:8]` state increment on the vault account
- deposit: system-program transfer CPI into the vault PDA
- withdraw: direct lamport mutation out of the vault PDA

Latest verified averaged result:

- Hopper parity: authorize `823` CU, auth-fail `122` CU, counter `993` CU, deposit `2050` CU, withdraw `851` CU, binary `8.30` KiB
- Quasar: authorize `585` CU, auth-fail `66` CU, counter `607` CU, deposit `1768` CU, withdraw `605` CU, binary `8.36` KiB
- Pinocchio-style: authorize `2543` CU, auth-fail `74` CU, counter `2575` CU, deposit `3763` CU, withdraw `2567` CU, binary `10.13` KiB

The latest Hopper-side gain here is not a benchmark-only trick. The parity
target now uses Hopper Runtime's direct native PDA verification path, which
improves every existing vault path materially over the previous baseline. The
new counter-access scenario also makes the next optimization target explicit:
Hopper's segment-safe mutation path is still meaningfully more expensive than
Quasar's raw byte slicing, even when both mutate the same 40-byte state region.