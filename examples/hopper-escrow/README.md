# Hopper Escrow

The SPL-facing Hopper example. It keeps the state model simple while showing
how Hopper's macro-first API reads when token flows and authority checks enter
the picture.

## What It Demonstrates

- zero-copy escrow state via `#[account]`
- typed contexts via `#[derive(Accounts)]`
- `Ctx<T>` handlers with `ctx.accounts.*` business methods
- Hopper account creation and typed state writes
- `UncheckedAccount` for raw accounts that are intentionally not decoded

## Instruction Map

- `0` = `Make`
- `1` = `Take`
- `2` = `Cancel`

## Devnet

Deployed to devnet in this pass:

- Program id: `5Ficb6k1Lv8tV8pThmQLU9H4MAYGbArwGRH2vrTHoPuN`
- `.so` size: 18 736 bytes

```bash
hopper build -p hopper-escrow
hopper deploy --cluster devnet \
  --keypair /abs/path/devnet-keypair.json \
  --program-id target/deploy/hopper_escrow-keypair.json
```

Integration test against the deployed program (gated so the default
`cargo test` stays offline):

```bash
HOPPER_DEVNET=1 \
HOPPER_ESCROW_PROGRAM_ID=5Ficb6k1Lv8tV8pThmQLU9H4MAYGbArwGRH2vrTHoPuN \
HOPPER_KEYPAIR=/abs/path/devnet-keypair.json \
cargo test -p hopper-escrow --test devnet -- --nocapture
```

## Verify

```bash
cargo check -p hopper-escrow
hopper build --host -p hopper-escrow
hopper build -p hopper-escrow
```

## Manifest Path

This example ships a checked-in `hopper.manifest.json` describing the
`Escrow` layout and the make/take/cancel instructions. `hopper explain`
uses it to decode a real devnet `make` transaction even before the
program publishes its manifest on chain:

```bash
hopper explain <make-tx-signature> \
  --manifest examples/hopper-escrow/hopper.manifest.json
```

Canonical on-chain generation path:

1. publish the example program with an on-chain Hopper manifest
2. fetch it with `hopper fetch <program-id>`
3. drive `hopper manager` and `hopper client gen` from that fetched manifest

## CLI Walkthrough

```bash
hopper build --host -p hopper-escrow
hopper test -p hopper-escrow
hopper build -p hopper-escrow
hopper profile bench
```
