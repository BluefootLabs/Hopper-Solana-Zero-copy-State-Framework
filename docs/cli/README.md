# Hopper CLI command reference

Per-command pages for the `hopper` CLI. The full flat reference lives in
[`docs/CLI_REFERENCE.md`](../CLI_REFERENCE.md); the pages here go deeper on the
deploy-family and tx-decoding commands that drive a program from source to a
live cluster.

## Lifecycle

| Command | Page | What it does |
|---|---|---|
| `hopper init` | (see CLI_REFERENCE) | Scaffold a new Hopper program crate |
| `hopper build` | [build.md](build.md) | Build to SBF (default) or host (`--host`) |
| `hopper test` | (see CLI_REFERENCE) | Run `cargo test` for the package |
| `hopper deploy` | [deploy.md](deploy.md) | Build + deploy a fresh program to a cluster |
| `hopper upgrade` | [upgrade.md](upgrade.md) | Redeploy against an existing program id |
| `hopper close` | [close.md](close.md) | Close a program/buffer, reclaim rent |
| `hopper migrate` | [migrate.md](migrate.md) | LayoutMigration bytecode upgrade |
| `hopper dump` | (see CLI_REFERENCE) | Disassemble the built `.so` |

## Inspection & decoding

| Command | Page | What it does |
|---|---|---|
| `hopper explain <sig>` | [explain.md](explain.md) | Decode a confirmed tx against the manifest |
| `hopper explain account` | [explain.md](explain.md) | Decode an account header/layout |
| `hopper fetch` | (see CLI_REFERENCE) | Pull on-chain manifest/account state |
| `hopper plan` | [migrate.md](migrate.md) | Field-level layout migration plan |

## Shared cluster flags

Every deploy-family command (`deploy`, `upgrade`, `close`, `migrate`) takes the
same cluster selectors, parsed by `tools/hopper-cli/src/cmd/cluster.rs`:

| Flag | Meaning |
|---|---|
| `--cluster <name>` / `--url <url>` / `-u` | `devnet` (default), `testnet`, `mainnet-beta`, `localnet`, or a raw RPC URL |
| `--keypair <path>` / `-k` | Fee-payer / upgrade-authority keypair |
| `--commitment <level>` | `processed` / `confirmed` / `finalized` |
| `--yes` / `-y` | Skip the interactive confirmation prompt |

**Mainnet is opt-in only.** The default cluster is always devnet. A mainnet
target must be named explicitly with `--cluster mainnet-beta`; a raw RPC URL
containing `mainnet` also trips the guard. Destructive ops on mainnet
(`deploy`/`upgrade`/`close`) prompt for an interactive `yes` unless `--yes` is
passed. `close` prompts on *every* cluster because the program id becomes
permanently unusable.
