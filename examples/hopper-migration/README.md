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
- a tag-2 deposit that debits the external signer only through Hopper's safe
  System Program transfer CPI

## Instruction Map

- `0` = `InitV1`
- `1` = `MigrateV1ToV2`
- `2` = `DepositV2`
- `3` = `ReadEither`

## Devnet (versioned-state)

This is the brief's `versioned-state` example. The ids below are a historical
deployment record and do not attest the current tag-2 System CPI source:

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

After deploying the final source to a fresh program id, the on-chain `init_v1`
to `migrate_v1_to_v2` to `deposit_v2` lifecycle is covered by an opt-in,
fail-closed integration test. It verifies the public
devnet genesis, finalized signatures and state, the 56 B to 65 B append, and
exact agreement between the tag-2 recorded balance and the System CPI lamport
delta:

```bash
HOPPER_DEVNET=1 \
HOPPER_REQUIRE_DEVNET=1 \
HOPPER_MIGRATION_PROGRAM_ID=REPLACE_WITH_FRESH_PROGRAM_ID \
HOPPER_KEYPAIR=/abs/path/devnet-keypair.json \
HOPPER_DEVNET_RECEIPT=/abs/path/migration-receipt.json \
cargo test -p hopper-migration --test devnet -- --nocapture
```

The compiled-SBF regression proves tag 2 enters the canonical System Program,
updates state only after the CPI, and rolls back an insufficient-funds failure:

```bash
cargo build-sbf --manifest-path examples/hopper-migration/Cargo.toml -- --locked
HOPPER_REQUIRE_MIGRATION_SBF=1 \
  cargo test -p hopper-migration --test deposit_v2_sbf -- --nocapture
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
