# Hopper Parity Vault

This example is the fair comparison target for Hopper versus the in-tree Anza
Pinocchio baseline ([`bench/pinocchio-vault`](../../bench/pinocchio-vault/src/lib.rs))
and Quasar's `vault` example. Pre-R2 it also compared against Quasar's
`examples/pinocchio-vault`; that indirection was removed (see
[AUDIT.md](../../AUDIT.md) R2) so the Pinocchio column is unambiguously Anza
Pinocchio.

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

The fair comparison runner lives in `bench/framework-vault-bench` and is driven
through `bench/compare-framework-vaults.ps1`.

The runner averages 8 shared deterministic user seed cases across every
framework present so the comparison is not dominated by a single PDA bump
outcome.

It covers four matched instruction paths:

- authorize: signer + writable + PDA validation only
- counter-access: signer + writable + PDA validation plus a raw `[authority:32][counter:8]` state increment on the vault account
- deposit: system-program transfer CPI into the vault PDA
- withdraw: direct lamport mutation out of the vault PDA

Latest verified averaged result (pre-R2; Pinocchio column was against Quasar's reference vault):

- Hopper parity: authorize `823` CU, auth-fail `122` CU, counter `993` CU, deposit `2050` CU, withdraw `851` CU, binary `8.30` KiB
- Quasar: authorize `585` CU, auth-fail `66` CU, counter `607` CU, deposit `1768` CU, withdraw `605` CU, binary `8.36` KiB
- Pinocchio-style (deprecated, Quasar reference): authorize `2543` CU, auth-fail `74` CU, counter `2575` CU, deposit `3763` CU, withdraw `2567` CU, binary `10.13` KiB

Post-R2 numbers against the in-tree Anza Pinocchio baseline will be
populated on the next bench run. Hopper's expected lead over idiomatic
Pinocchio is a few hundred CU on PDA-bearing instructions (attributable to
Hopper's verify-only sha256 PDA path vs Pinocchio's standard
`find_program_address`), not the ~2000 CU gap shown above.

The Hopper-side gain here is not a benchmark-only trick. The parity target
uses Hopper Runtime's direct native PDA verification path, which improves
every existing vault path materially over the previous baseline. The
counter-access scenario also makes the next optimization target explicit:
Hopper's segment-safe mutation path is still meaningfully more expensive
than Quasar's raw byte slicing, even when both mutate the same 40-byte
state region.