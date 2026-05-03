# Hopper Parity Vault

This example is the fair comparison target for Hopper versus Quasar's `vault`
example. The Anza Pinocchio comparison target lives in the sibling
`hopper-bench` product repo with its own lockfile and provenance; Hopper does
not publish Pinocchio numbers from a borrowed Quasar reference vault.

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
idiomatic Pinocchio and Quasar's minimal `vault` example.

`hopper-parity-vault` keeps only the shared vault semantics so the benchmark can
measure framework overhead instead of example-specific features.

## Verify

```bash
cargo check -p hopper-parity-vault
hopper build -p hopper-parity-vault
```

## Benchmark Path

The fair comparison runner lives in the sibling `hopper-bench` repo.

The runner averages 8 shared deterministic user seed cases across every
framework present so the comparison is not dominated by a single PDA bump
outcome.

It covers four matched instruction paths:

- authorize: signer + writable + PDA validation only
- counter-access: signer + writable + PDA validation plus a raw `[authority:32][counter:8]` state increment on the vault account
- deposit: system-program transfer CPI into the vault PDA
- withdraw: direct lamport mutation out of the vault PDA

Current release-facing averaged result:

- Hopper parity: authorize `432` CU, auth-fail `70` CU, counter `539` CU, deposit `1651` CU, withdraw `455` CU, binary `7.62` KiB
- Quasar: authorize `585` CU, auth-fail `66` CU, counter `607` CU, deposit `1768` CU, withdraw `605` CU, binary `8.36` KiB

Pinocchio results are intentionally omitted until the sibling benchmark repo
records a same-provenance Anza Pinocchio run: exact target source, dependency
versions, SBF toolchain, Mollusk version, seed count, and command line.

The Hopper-side gain here is not a benchmark-only trick. The parity target
uses Hopper Runtime's direct native PDA verification path, which improves
every existing vault path materially over the previous baseline. The
counter-access scenario also makes the next optimization target explicit:
Hopper's segment-safe mutation path now beats Quasar in the published table
while preserving byte-range borrow checks that Quasar's raw byte slicing does
not provide.