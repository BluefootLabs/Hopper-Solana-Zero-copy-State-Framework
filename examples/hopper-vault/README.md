# Hopper Vault

A compact macro-first Hopper vault. This is a good starting point when you want
the Anchor/Quasar-feeling API while keeping Hopper's checked zero-copy account
layout and runtime validation.

## What It Demonstrates

- one zero-copy account via `#[account]`
- typed contexts via `#[derive(Accounts)]`
- Hopper-owned errors via `hopper_error!`
- Hopper entrypoint and `#[program]` dispatch
- account creation through `InitAccount` and generated `ctx.init_vault()`
- business logic on `ctx.accounts.*`

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

The host-side tests in `src/tests.rs` execute the generated Hopper program
bridge through `hopper-svm` and cover:

- deposit CU
- withdraw CU
- unsigned withdraw rejection

Run them directly:

```bash
cargo test -p hopper-vault -- --nocapture
```

That output is useful for local smoke testing. `hopper-svm` executes Hopper
account-memory fixtures in-process, so these tests no longer require a compiled
SBF artifact.

The fair cross-framework benchmark now uses `examples/hopper-parity-vault` plus
the shared runner in the sibling `hopper-bench` repo so the comparison does not
inherit this example's extra init and zero-copy state semantics.
