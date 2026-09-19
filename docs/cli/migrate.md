# `hopper migrate`

Perform a `LayoutMigration` bytecode upgrade against a deployed program. A
migrate is an [`upgrade`](upgrade.md) with a louder banner: one CLI workflow
rebuilds the current (e.g. v2) program, uploads it to a loader-v3 buffer across
as many write transactions as required, and submits the final upgrade
transaction. The new bytecode, carrying the new layout handlers and migration
edges, then replaces the old one in place.

The *field-level* migration plan (which account fields change, and whether the
change is append-safe or requires a rewrite) is a separate, read-only analysis;
inspect it with `hopper plan` before running the migrate.

## Usage

```
hopper migrate --program-id <path|pubkey> [-p <package>] [--no-build] \
  [--cluster <name>|--url <url>] [--keypair <path>] \
  [--commitment <level>] [-y|--yes]
```

The flags match [`hopper upgrade`](upgrade.md) exactly.

## Two halves of a migration

| Half | Command | What it does |
|---|---|---|
| Plan (layout) | `hopper plan @old-layout.json @new-layout.json` | Reports field-level diffs and compatibility from two layout JSON objects. Read-only. |
| Apply (bytecode) | `hopper migrate` | Upgrades the on-chain program to the bytecode that carries the new layout + migration edges. |

The on-chain account evolution itself (e.g. V1 56 B → V2 65 B in place) is
driven by the program's migration instruction once the new bytecode is live.
The short `plan` command treats unprefixed arguments as inline JSON, so file
paths require the `@` prefix. It does not infer layouts from `-p <package>`.

## Example (devnet)

```bash
# 1. Inspect the planned layout change
hopper plan @path/to/layout-v1.json @path/to/layout-v2.json

# 2. Drive the bytecode upgrade
hopper migrate --program-id EuDECNLNwPAptWC5NmenBBfjSuhZtmpPwpMQ7Z1P2GMt \
  --cluster devnet \
  --keypair /abs/path/devnet-keypair.json \
  -p hopper-migration
```

A historical versioned-state build is deployed at
`EuDECNLNwPAptWC5NmenBBfjSuhZtmpPwpMQ7Z1P2GMt`. Use a fresh deployment of the
current source with the gated `HOPPER_DEVNET=1` integration test for release
evidence.
