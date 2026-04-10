# Hopper Migration

The layout-evolution example. This is the clearest reference for why Hopper's
layout contracts and schema tooling are framework-level features rather than
just account helpers.

## What It Demonstrates

- append-safe versioned layouts
- `hopper_manifest!` layout manifests in code
- compile-time compatibility assertions
- runtime dual-version loading during rollout
- migration planning with `hopper-schema`

## Instruction Map

- `0` = `InitV1`
- `1` = `MigrateV1ToV2`
- `2` = `DepositV2`
- `3` = `ReadEither`

## Verify

```bash
cargo test -p hopper-migration
hopper build --host -p hopper-migration
hopper build -p hopper-migration
```

## Manifest Path

Canonical layout manifests are declared inline in [src/lib.rs](src/lib.rs):

- `VAULT_V1_MANIFEST`
- `VAULT_V2_MANIFEST`

Those manifest constants are the current source of truth for migration planning
and compatibility checks in this example.

## CLI Walkthrough

```bash
hopper build --host -p hopper-migration
hopper test -p hopper-migration
hopper profile bench
```

The migration example currently proves its schema path through in-code manifest
constants and tests rather than a checked-in `ProgramManifest` JSON.
