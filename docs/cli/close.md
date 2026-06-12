# `hopper close`

Close an upgradeable program or a dangling deploy buffer and reclaim its rent.
**Irreversible:** a closed program id can never be reused.

## Usage

```
hopper close (--program-id <pubkey> | --buffer <pubkey> | --buffers) \
  [--cluster <name>] [--keypair <path>] [-y|--yes]
```

## Flags

| Flag | Meaning |
|---|---|
| `--program-id <pubkey>` | Close this program and reclaim its rent. |
| `--buffer <pubkey>` | Close a single deploy buffer. |
| `--buffers` | Close all dangling buffers owned by the authority. |
| `--cluster <name>` / `-u <url>` | Target cluster (default `devnet`). |
| `--keypair <path>` / `-k` | Authority keypair. |
| `--yes` / `-y` | Skip the confirmation prompt. |

## Confirmation

Unlike `deploy`/`upgrade`, `close` prompts for an interactive `yes` on **every**
cluster (not just mainnet), because the program id becomes permanently unusable.
Pass `--yes` to skip the prompt in automation.

## Examples

```bash
# Reclaim rent from a failed deploy's leftover buffers
hopper close --buffers --cluster devnet --keypair /abs/path/devnet-keypair.json

# Permanently close a program
hopper close --program-id <PUBKEY> --cluster devnet \
  --keypair /abs/path/devnet-keypair.json
```
