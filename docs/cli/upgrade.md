# `hopper upgrade`

Rebuild the current SBF program and upgrade an **existing** program id via the
BPF Loader Upgradeable. This replaces the deployed bytecode in place; the
program id, PDAs, and existing accounts are preserved.

## Usage

```
hopper upgrade --program-id <path|pubkey> [-p <package>] [--no-build] \
  [--cluster <name>] [--keypair <path>] [--commitment <level>] [-y|--yes]
```

## Flags

| Flag | Meaning |
|---|---|
| `--program-id <path\|pubkey>` | **Required.** The program id to upgrade (keypair path or pubkey). |
| `-p <package>` | Workspace member to build. |
| `--no-build` | Upgrade with the existing `.so`. |
| `--cluster <name>` / `-u <url>` | Target cluster (default `devnet`). |
| `--keypair <path>` / `-k` | Upgrade-authority keypair. |
| `--commitment <level>` | Commitment for the tx. |
| `--yes` / `-y` | Skip the mainnet confirmation prompt. |

See the [shared cluster flags](README.md#shared-cluster-flags).

## Behavior

1. Builds the SBF artifact (unless `--no-build`).
2. Confirms the operation (mainnet only, unless `--yes`).
3. Runs `solana program deploy <artifact> --program-id <id> --use-rpc --url <cluster> ...`.

The upgrade authority must match the keypair you pass; otherwise the loader
rejects the transaction.

## Example (devnet)

```bash
hopper upgrade --program-id EuDECNLNwPAptWC5NmenBBfjSuhZtmpPwpMQ7Z1P2GMt \
  --cluster devnet \
  --keypair /abs/path/devnet-keypair.json \
  -p hopper-migration
```
