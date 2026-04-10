# Hopper Showcase

The canonical Hopper program. This example is the strongest single reference
for how Hopper Lang is meant to feel when the framework is used as one system.

## What It Demonstrates

- typed layouts via `hopper_layout!`
- phased execution with `PhasedFrame`
- named capability packs and policy-aware validation
- receipts, invariants, and segment-aware state
- Hopper-native entrypoint and dispatch flow

## Instruction Map

- `0` = `InitPool`
- `1` = `Deposit`
- `2` = `Withdraw`
- `3` = `UpdateConfig`

## Verify

```bash
cargo check -p hopper-showcase
hopper build --host -p hopper-showcase
hopper build -p hopper-showcase
```

## Manifest Path

This example is currently code-first. It does not ship a checked-in
`ProgramManifest` JSON yet.

Canonical generation path:

1. publish the example program with an on-chain Hopper manifest
2. fetch it with `hopper fetch <program-id>`
3. use that manifest with `hopper manager` and `hopper client gen`

## CLI Walkthrough

```bash
hopper build --host -p hopper-showcase
hopper test -p hopper-showcase
hopper build -p hopper-showcase
hopper profile bench
```

Until showcase has a checked-in program manifest, use this example as the
reference for authored Hopper code and use the manager/client-generation flows
against a fetched manifest from a deployed Hopper program.
