# `hopper deploy`

Build the current program to SBF and deploy it as a **fresh** program to a
cluster via the BPF Loader Upgradeable. For redeploying against an existing
program id, use [`hopper upgrade`](upgrade.md).

## Usage

```
hopper deploy [-p <package>] [--no-build] [--dry-run] \
  [--cluster <name>] [--keypair <path>] [--commitment <level>] [-y|--yes] \
  [<solana program deploy args>...]
```

## Flags

| Flag | Meaning |
|---|---|
| `-p <package>` | Workspace member to deploy. |
| `--no-build` | Skip the SBF build; deploy the existing `.so`. |
| `--dry-run` | Query live loader-v3 rent for the exact artifact and send no transaction. |
| `--cluster <name>` / `-u <url>` | Target cluster (default `devnet`). |
| `--keypair <path>` / `-k` | Fee-payer keypair. |
| `--commitment <level>` | Commitment forwarded to Solana CLI for upload/deploy operations and used for dry-run RPC reads (default `confirmed`). |
| `--yes` / `-y` | Skip the mainnet confirmation prompt. |

See the [shared cluster flags](README.md#shared-cluster-flags) for the mainnet
guard. Unconsumed args are forwarded to `solana program deploy` (e.g.
`--program-id <keypair.json>` to fix the program id, `--max-len` for headroom).

## Behavior

1. Builds the SBF artifact (unless `--no-build`).
2. Resolves the `.so` for the selected package.
3. With `--dry-run`, records the starting RPC slot and reads rent exemptions at
   the requested commitment for the Program, ProgramData, and Buffer
   allocations, prints the permanent and recycled balances separately, then
   exits without confirmation or a transaction.
4. Otherwise, confirms the operation (mainnet only, unless `--yes`).
5. Runs `solana program deploy <artifact> --use-rpc --url <cluster> ...`.

`--use-rpc` is added automatically unless you pass it yourself, so deploys work
against public RPC endpoints without a local validator.

## Cost quote

```bash
hopper deploy --dry-run --cluster mainnet-beta -p hopper-cicada
```

For a fresh loader-v3 deployment the permanent balance is
`R(36) + R(45 + max_len)`, where `R` is the live RPC rent query. The Buffer
account holds `37 + ELF_len` data bytes, but the stock Solana CLI funds it at
the ProgramData requirement; loader v3 drains and reuses that balance during a
successful deploy. The quote labels it as transient working capital and never
adds it to permanent rent. Transaction/priority fees are excluded because they
depend on the final messages and cluster conditions.

The quote is read-only even on Mainnet. It describes a fresh allocation;
upgrades of existing programs may fund only ProgramData growth.

## Example (devnet)

```bash
hopper deploy --cluster devnet \
  --keypair /abs/path/devnet-keypair.json \
  --program-id target/deploy/hopper_counter-keypair.json \
  -p hopper-counter
```

A historical counter build was deployed this way at
`D8UGWDX5QRwEkKs2J9Sweabf4zd6hzdLqv7CB11SF91F` (4,688-byte `.so`). That
deployment does not attest the current release source.
