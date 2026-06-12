# `hopper deploy`

Build the current program to SBF and deploy it as a **fresh** program to a
cluster via the BPF Loader Upgradeable. For redeploying against an existing
program id, use [`hopper upgrade`](upgrade.md).

## Usage

```
hopper deploy [-p <package>] [--no-build] \
  [--cluster <name>] [--keypair <path>] [--commitment <level>] [-y|--yes] \
  [<solana program deploy args>...]
```

## Flags

| Flag | Meaning |
|---|---|
| `-p <package>` | Workspace member to deploy. |
| `--no-build` | Skip the SBF build; deploy the existing `.so`. |
| `--cluster <name>` / `-u <url>` | Target cluster (default `devnet`). |
| `--keypair <path>` / `-k` | Fee-payer keypair. |
| `--commitment <level>` | Commitment for the deploy tx. |
| `--yes` / `-y` | Skip the mainnet confirmation prompt. |

See the [shared cluster flags](README.md#shared-cluster-flags) for the mainnet
guard. Unconsumed args are forwarded to `solana program deploy` (e.g.
`--program-id <keypair.json>` to fix the program id, `--max-len` for headroom).

## Behavior

1. Builds the SBF artifact (unless `--no-build`).
2. Resolves the `.so` for the selected package.
3. Confirms the operation (mainnet only, unless `--yes`).
4. Runs `solana program deploy <artifact> --use-rpc --url <cluster> ...`.

`--use-rpc` is added automatically unless you pass it yourself, so deploys work
against public RPC endpoints without a local validator.

## Example (devnet)

```bash
hopper deploy --cluster devnet \
  --keypair /abs/path/devnet-keypair.json \
  --program-id target/deploy/hopper_counter-keypair.json \
  -p hopper-counter
```

The counter example deployed this way is live on devnet at
`D8UGWDX5QRwEkKs2J9Sweabf4zd6hzdLqv7CB11SF91F` (4 688-byte `.so`).
