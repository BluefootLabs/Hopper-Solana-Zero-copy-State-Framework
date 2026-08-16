# hopper-token-2022

Hopper-owned Token-2022 builders plus extension screening helpers. The
extension-aware companion to [`hopper-token`](https://crates.io/crates/hopper-token).

[![Crates.io](https://img.shields.io/crates/v/hopper-token-2022.svg)](https://crates.io/crates/hopper-token-2022)
[![Docs.rs](https://img.shields.io/docsrs/hopper-token-2022)](https://docs.rs/crate/hopper-token-2022)

Part of the **[Hopper](https://hopperzero.dev)** framework.

## What this crate ships

- **Instruction builders** - `Transfer`, `MintTo`, `Burn`, `CloseAccount`,
  `Approve`, `Revoke`, `InitializeAccount` retargeted at the Token-2022
  program.
- **Extension screening** - fail-closed `check_safe_token_2022_mint`,
  `check_no_transfer_fee`, `check_no_permanent_delegate`,
  `check_no_confidential_transfer`, `check_no_transfer_hook`,
  `check_transferable`. The blanket safety gate validates the full TLV stream,
  permits only reviewed metadata and group extensions, and rejects unknown,
  duplicate, or malformed entries.
- **Extension readers** - `read_transfer_fee_config`, `read_transfer_hook`,
  `check_transfer_hook_program`. Zero-copy TLV scanners over the mint or
  token-account extension area.

## When to reach for this

Anything that accepts user-supplied Token-2022 mints. The extension family
adds attack surface that legacy SPL Token doesn't have. Hopper's screeners
let a DEX or lending market reject mints with transfer hooks, transfer fees,
permanent delegates, confidential transfers, new unreviewed semantics, or a
malformed extension envelope in one line.

```rust
use hopper::prelude::*;

let mint_data = mint_account.try_borrow()?;
hopper::hopper_token_2022::check_safe_token_2022_mint(&mint_data)?;
```

See [`examples/hopper-token-2022-transfer-hook`](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/examples/hopper-token-2022-transfer-hook/src/lib.rs)
for an end-to-end transfer-hook validation pattern.

Docs: <https://docs.rs/crate/hopper-token-2022>. The workspace's 0.3 API will
appear there only after the 0.3 publish train completes.

Support: `solanadevdao.sol` / `F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

License: Apache-2.0.
