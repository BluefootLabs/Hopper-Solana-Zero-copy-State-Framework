# hopper-token-2022

Hopper-owned Token-2022 builders plus extension screening helpers. The
extension-aware companion to [`hopper-token`](https://crates.io/crates/hopper-token).

[![Crates.io](https://img.shields.io/crates/v/hopper-token-2022.svg)](https://crates.io/crates/hopper-token-2022)
[![Docs.rs](https://img.shields.io/docsrs/hopper-token-2022)](https://docs.rs/crate/hopper-token-2022)

Part of the **[Hopper](https://hopperzero.dev)** framework.

## What this crate ships

- **Instruction builders** - `Transfer`, `MintTo`, `Burn`, `CloseAccount`,
  `Approve`, `Revoke`, `InitializeAccount` retargeted at the Token-2022
  program, and the `checked` module with the shared checked set
  (`TransferChecked`, `MintToChecked`, `BurnChecked`, `ApproveChecked`,
  `SetAuthority`, `FreezeAccount`, `ThawAccount`, `InitializeMultisig2`,
  `InitializeImmutableOwner`, `GetAccountDataSize`, `AmountToUiAmount`,
  `UiAmountToAmount`, `WithdrawExcessLamports`, and the rest), sent with
  `invoke_on(TokenProgram::Token2022, ..)` or `invoke_for_owner(..)`.
- **Extension instructions** - `extension_instructions` carries every
  Token-2022-only instruction: `CreateNativeMint`,
  `InitializeNonTransferableMint`, `Reallocate`, and the transfer fee,
  default account state, memo transfer, interest bearing, CPI guard,
  permanent delegate, transfer hook, metadata / group / group member
  pointer, scaled UI amount, pausable, permissioned burn, and mint close
  authority families (initializers, updates, and toggles), each with
  direct, PDA-signed, and multisig entry points. Their bytes and metas are
  checked against the canonical `spl-token-2022-interface` constructors.
- **Metadata and group instructions** - `metadata_instructions` (in the
  tree after 0.4.0) carries the token-metadata interface
  (`InitializeTokenMetadata`, `UpdateMetadataField`, `RemoveMetadataKey`,
  `UpdateMetadataAuthority`, `EmitTokenMetadata`) and the token-group
  interface (`InitializeTokenGroup`, `UpdateTokenGroupMaxSize`,
  `UpdateTokenGroupAuthority`, `InitializeTokenGroupMember`). `invoke()`
  targets Token-2022; `invoke_on_program` targets any program that
  implements the interface. The payload is encoded on the stack, 512 bytes
  at most.
- **Confidential-transfer instructions** - `confidential_instructions` (in
  the tree after 0.4.0) carries the fifteen sub-instructions of instruction
  27. Ciphertexts, keys, and proofs are made off chain and carried as
  bytes; a `ProofLocation` names where each proof is, and the builder
  orders the sysvar, context-state, authority, and multisig accounts the
  way the processor reads them.
- **Extension screening** - fail-closed `check_safe_token_2022_mint`,
  `check_no_transfer_fee`, `check_no_permanent_delegate`,
  `check_no_confidential_transfer`, `check_no_transfer_hook`,
  `check_transferable`. The blanket safety gate validates the full TLV stream,
  permits only reviewed metadata and group extensions, and rejects unknown,
  duplicate, or malformed entries.
- **Extension readers** - `read_transfer_fee_config`, `read_transfer_hook`,
  `check_transfer_hook_program`. Zero-copy TLV scanners over the mint or
  token-account extension area.

## Mint creation in 0.3.1

`MintPlan` binds exact allocation to the thirteen fixed-size extension
initializers: transfer fee, mint close authority, non-transferable, permanent
delegate, transfer hook, metadata pointer, default account state, interest
bearing, scaled UI amount, pausable, group pointer, group member pointer,
and permissioned burn (the last seven are in the tree after 0.4.0). It initializes extensions before
`InitializeMint2`, uses live rent, and supports prefunded and PDA mints.
`check_space` rejects both smaller and larger allocations than the plan.

The plan rejects duplicate extensions and invalid configuration before creation.
It does not infer extension semantics, initialize variable-length metadata,
create token accounts, or mint supply. Propagate errors from the multi-CPI
sequence so the enclosing instruction rolls back earlier work.

These APIs ship in the verified 0.3.1 registry release. See the [Token-2022 guide](https://hopperzero.dev/docs/token-2022) for
complete examples and the distinction between readers, constraints, and creation.

## When to reach for this

Anything that accepts user-supplied Token-2022 mints. The extension family
adds attack surface that legacy SPL Token doesn't have. Hopper's screeners
let a DEX or lending market reject mints with transfer hooks, transfer fees,
permanent delegates, confidential transfers, new unreviewed semantics, or a
malformed extension envelope in one line.

```rust
use hopper::prelude::*;

mint_account.check_owned_by(&hopper::token_2022::TOKEN_2022_PROGRAM_ID)?;
let mint_data = mint_account.try_borrow()?;
hopper::hopper_token_2022::check_safe_token_2022_mint(&mint_data)?;
```

See [`examples/hopper-token-2022-transfer-hook`](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/blob/main/examples/hopper-token-2022-transfer-hook/src/lib.rs)
for an end-to-end transfer-hook validation pattern.

The raw byte screeners do not prove account ownership, expected mint identity,
or caller authority. Check those separately as the example checks ownership.

Versioned API docs: <https://docs.rs/crate/hopper-token-2022>.

Support: `solanadevdao.sol` / `F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

License: Apache-2.0.
