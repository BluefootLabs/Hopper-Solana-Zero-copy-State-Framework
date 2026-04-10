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
