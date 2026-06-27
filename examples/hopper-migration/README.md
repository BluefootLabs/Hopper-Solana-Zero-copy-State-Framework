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

## Devnet (versioned-state)

This is the brief's `versioned-state` example. Deployed to devnet in
this pass:

- Program id: `EuDECNLNwPAptWC5NmenBBfjSuhZtmpPwpMQ7Z1P2GMt`
- Latest program id: `7CuuiKRWqs6JPFbyfMZdAKedWULAAUBnzFRPee46bu2d`
- `.so` size: 29 680 bytes

```bash
hopper build -p hopper-migration
hopper deploy --cluster devnet \
  --keypair /abs/path/devnet-keypair.json \
  --program-id target/deploy/hopper_migration-keypair.json
```

`hopper migrate` drives a `LayoutMigration` bytecode upgrade against the
deployed program:

```bash
hopper migrate --cluster devnet \
  --keypair /abs/path/devnet-keypair.json \
  --program-id target/deploy/hopper_migration-keypair.json \
  -p hopper-migration
```

The on-chain `init_v1` → `migrate_v1_to_v2` account evolution is covered
by the gated integration test (V1 56 B → V2 65 B in place):

```bash
HOPPER_DEVNET=1 \
HOPPER_MIGRATION_PROGRAM_ID=7CuuiKRWqs6JPFbyfMZdAKedWULAAUBnzFRPee46bu2d \
HOPPER_KEYPAIR=/abs/path/devnet-keypair.json \
cargo test -p hopper-migration --test devnet -- --nocapture
```

Latest verified devnet run from this workspace:

```text
Vault: 2BCLiodDfcbZRNnQZtqAydoPYxhfdRnaNRmnWw74ruP6
Init Signature: 2xBgFYVesudma6vvUVQTgFKTbeH1Zy7pqc5ta4dGJ9QDF9JAp1KGuWNHhXekbmjodCks6SPRn89DFWVg7hDLUrw3
Migrate Signature: 5HujLW9Lio5xcqrrZzhzcpCSca5f6cz911pNGJv7Gu2nRJzHxJZZdA7NCfLkU2xqhu4BzKQYNWKWx8HCjYamMVHw
Verified: 56B V1 -> 65B V2
```

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
