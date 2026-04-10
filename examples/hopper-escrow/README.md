# Hopper Escrow

The SPL-facing Hopper example. It keeps the state model simple while showing
how Hopper code reads when token flows and authority checks enter the picture.

## What It Demonstrates

- zero-copy escrow state
- instruction parsing without leaving Hopper terminology
- Hopper account creation and typed state writes
- token-oriented program structure without changing framework identity

## Instruction Map

- `0` = `Make`
- `1` = `Take`
- `2` = `Cancel`

## Verify

```bash
cargo check -p hopper-escrow
hopper build --host -p hopper-escrow
hopper build -p hopper-escrow
```

## Manifest Path

This example does not ship a checked-in `ProgramManifest` JSON yet.

Canonical generation path:

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
