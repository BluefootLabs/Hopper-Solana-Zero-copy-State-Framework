# Hopper Vault

The minimal default Hopper example. This is the best starting point when you
want a small program that still uses the real Hopper language surface.

## What It Demonstrates

- one zero-copy layout via `hopper_layout!`
- Hopper-owned errors via `hopper_error!`
- Hopper entrypoint and dispatch
- account creation with `hopper_init!`
- phased instruction execution for deposit and withdraw

## Instruction Map

- `0` = `InitVault`
- `1` = `Deposit`
- `2` = `Withdraw`

## Verify

```bash
cargo check -p hopper-vault
hopper build --host -p hopper-vault
hopper build -p hopper-vault
cargo test -p hopper-vault -- --nocapture
```

## Manifest Path

Current CLI reference manifest: [../sample-manifest.json](../sample-manifest.json)

That sample manifest is vault-shaped and is the closest current checked-in
manager/client-generation artifact while this example remains code-first.

## CLI Walkthrough

```bash
hopper build --host -p hopper-vault
hopper test -p hopper-vault
hopper explain program @examples/sample-manifest.json
hopper manager summary @examples/sample-manifest.json
hopper client gen --ts @examples/sample-manifest.json
```

## Scenario CU And Safety Tests

The host-side tests in `src/tests.rs` load the compiled `hopper_vault` SBF
binary through Mollusk and cover:

- deposit CU
- withdraw CU
- unsigned withdraw rejection

Build first, then run:

```bash
hopper build -p hopper-vault
cargo test -p hopper-vault -- --nocapture
```

That output is also what `bench/compare-framework-vaults.ps1` parses when it
used to compare Hopper against Quasar's `vault` and `pinocchio-vault`
examples.

The fair cross-framework benchmark now uses `examples/hopper-parity-vault` plus
the shared runner in `bench/framework-vault-bench` so the comparison does not
inherit this example's extra init and zero-copy state semantics.
