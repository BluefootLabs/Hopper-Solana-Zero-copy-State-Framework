# `hopper migrate`

Perform a `LayoutMigration` bytecode upgrade against a deployed program. A
migrate is an [`upgrade`](upgrade.md) with a louder banner: it rebuilds the
current (e.g. v2) program and upgrades the existing program id in a single
transaction, so the new bytecode — carrying the new layout handlers and
migration edges — replaces the old one in place.

The *field-level* migration plan (which account fields change, and whether the
change is append-safe or requires a rewrite) is a separate, read-only analysis;
inspect it with `hopper plan` before running the migrate.

## Usage

```
hopper migrate --program-id <path|pubkey> [-p <package>] [--no-build] \
  [--cluster <name>] [--keypair <path>] [-y|--yes]
```

The flags match [`hopper upgrade`](upgrade.md) exactly.

## Two halves of a migration

| Half | Command | What it does |
|---|---|---|
| Plan (layout) | `hopper plan` | Reports field-level diffs and compatibility from the layout manifests. Read-only. |
| Apply (bytecode) | `hopper migrate` | Upgrades the on-chain program to the bytecode that carries the new layout + migration edges. |

The on-chain account evolution itself (e.g. V1 56 B → V2 65 B in place) is
driven by the program's migration instruction once the new bytecode is live.

## Example (devnet)

```bash
# 1. Inspect the planned layout change
hopper plan -p hopper-migration

# 2. Drive the bytecode upgrade
hopper migrate --program-id EuDECNLNwPAptWC5NmenBBfjSuhZtmpPwpMQ7Z1P2GMt \
  --cluster devnet \
  --keypair /abs/path/devnet-keypair.json \
  -p hopper-migration
```

The versioned-state example (`hopper-migration`) is live on devnet at
`EuDECNLNwPAptWC5NmenBBfjSuhZtmpPwpMQ7Z1P2GMt`; its V1→V2 in-place evolution is
covered by the gated `HOPPER_DEVNET=1` integration test.
