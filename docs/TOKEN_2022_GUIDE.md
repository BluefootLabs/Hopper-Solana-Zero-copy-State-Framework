# Writing Token-2022 programs in Hopper

Anchor stable has a genuine zero-copy `AccountLoader<T>` path for user-defined
accounts. Its Token-2022 extension constraints use the typed token-interface
path instead. Hopper provides allocation-free TLV readers, a declarative
subset of extension checks, and fail-closed policy helpers for the current
official extension discriminators 0 through 28. Quasar's pinned release has
base-layout token readers but no comparable TLV policy layer.

This guide is the reference for using them.

## Pin the token program first

Before you touch an extension, constrain the account's owner program. Otherwise a caller could pass a legacy SPL Token account and every extension scan would miss (because legacy accounts have no TLV region).

```rust
#[derive(Accounts)]
pub struct ConfigureMint<'info> {
    #[account(
        mut,
        mint::authority = authority,
        mint::token_program = ::hopper_runtime::token::TOKEN_2022_PROGRAM_ID,
    )]
    pub mint: UncheckedAccount<'info>,

    pub authority: Signer<'info>,
}
```

`token::token_program` and `mint::token_program` each emit a single `check_owned_by(program_id)` before any byte-level check runs. SPL Token is the default when the override is omitted.

## The extension constraint vocabulary

Every attribute below compiles to a TLV scan on the mint or token-account bytes. No Borsh, no heap, no deserialize pass.

### Mint-side

```rust
#[account(
    extensions::mint_close_authority::authority = close_authority,
    extensions::permanent_delegate::delegate = permanent_delegate,
    extensions::transfer_hook::authority = hook_authority,
    extensions::transfer_hook::program_id = hook_program,
    extensions::metadata_pointer::authority = metadata_authority,
    extensions::metadata_pointer::metadata_address = metadata_address,
    extensions::default_account_state::state = 2, // Frozen
    extensions::interest_bearing::rate_authority = rate_authority,
    extensions::transfer_fee_config::authority = fee_authority,
    extensions::transfer_fee_config::withdraw_withheld_authority = withdraw_authority,
    extensions::confidential_transfer::mint,
    extensions::scaled_ui_amount::config,
    extensions::non_transferable,
)]
pub mint: UncheckedAccount<'info>,
```

`default_account_state` takes the state byte directly: `0` Uninitialized, `1` Initialized, `2` Frozen.

`non_transferable` is a flag; no value needed.

### Token-account-side

```rust
#[account(
    extensions::immutable_owner,
    extensions::cpi_guard,
    extensions::confidential_transfer::account,
)]
pub ata: UncheckedAccount<'info>,
```

The token-account side now covers immutable owner, CPI guard, and confidential transfer account presence. `TransferHookAccount` (the per-account companion to the mint's `TransferHook`) is reachable through the raw TLV reader if you need it.

## Policy matrix checks

For low-level programs, use the no-alloc policy helper directly over a TLV region:

```rust
use hopper_runtime::token_2022_ext::{
    validate_extension_policy, ExtensionPolicy,
    EXT_CONFIDENTIAL_TRANSFER_MINT, EXT_SCALED_UI_AMOUNT_CONFIG,
    EXT_TRANSFER_HOOK,
};

validate_extension_policy(
    tlv,
    &ExtensionPolicy::new(
        &[EXT_CONFIDENTIAL_TRANSFER_MINT, EXT_SCALED_UI_AMOUNT_CONFIG],
        &[EXT_TRANSFER_HOOK],
    ),
)?;
```

This is useful for generated policy packs and devnet probes. The required and
forbidden policy first validates TLV structure, so truncated bytes cannot make
a forbidden extension look absent.

Custody and settlement code should normally use an explicit allowlist:

```rust
use hopper_runtime::token_2022_ext::validate_extension_allowlist;

// Empty means this program has accepted no extension semantics.
validate_extension_allowlist(tlv, &[])?;
```

The allowlist rejects unknown future discriminators, duplicate entries,
truncation, and every extension not explicitly named by the program.

## The raw TLV reader

For an extension outside the declarative constraint set, use the reader directly:

```rust
use hopper_runtime::token_2022_ext::{
    find_extension, mint_tlv_region, EXT_GROUP_POINTER,
};

let data = mint.as_account().try_borrow()?;
let tlv = mint_tlv_region(&data)
    .ok_or(ProgramError::InvalidAccountData)?;
let group = find_extension(tlv, EXT_GROUP_POINTER)
    .ok_or(ProgramError::InvalidAccountData)?;
// `group` is the raw extension payload. Layout for GroupPointer:
// [authority: 32][group_address: 32]
let authority: [u8; 32] = group[0..32].try_into().unwrap();
let group_address: [u8; 32] = group[32..64].try_into().unwrap();
```

The presence reader is intentionally best-effort and works on any extension
type. Do not interpret `None` as a complete safety decision. Validate TLV
structure or an explicit allowlist first. Current extension-code constants are
in `hopper_runtime::token_2022_ext` with `EXT_*` names.

## End-to-end: a capped-supply mint program

```rust
use hopper::prelude::*;

#[account]
#[repr(C)]
pub struct Config {
    pub admin: [u8; 32],
    pub max_supply: WireU64,
    pub bump: u8,
}

#[derive(Accounts)]
pub struct Configure<'info> {
    #[account(
        init,
        payer = admin,
        space = Config::INIT_SPACE,
        seeds = [b"config", mint.key().as_ref()],
        bump,
    )]
    pub config: InitAccount<'info, Config>,

    #[account(
        mut,
        mint::authority = admin,
        mint::token_program = ::hopper_runtime::token::TOKEN_2022_PROGRAM_ID,
        extensions::mint_close_authority::authority = admin,
        extensions::non_transferable,
    )]
    pub mint: UncheckedAccount<'info>,

    #[account(mut)]
    pub admin: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[program]
mod capped_mint {
    use super::*;

    #[instruction(0)]
    pub fn configure(ctx: Ctx<Configure>, max_supply: u64) -> ProgramResult {
        let mut config = ctx.accounts.config.get_mut_after_init()?;
        config.set_inner(*ctx.accounts.admin.key(), max_supply, ctx.bumps.config)
    }
}
```

The zero-copy path carries these extension checks without a whole-account
deserialize pass. CU claims belong to the pinned benchmark suite; this guide
does not infer a universal cost ranking from implementation style.

## What to reach for when

| Goal | Hopper path |
| --- | --- |
| Reject accounts that are not Token-2022 | `token::token_program = TOKEN_2022_PROGRAM_ID` |
| Enforce a specific transfer-hook program | `extensions::transfer_hook::program_id = X` |
| Bind a mint to a metadata-pointer account | `extensions::metadata_pointer::metadata_address = X` |
| Require a mint to be soulbound | `extensions::non_transferable` |
| Verify the ATA is immutable-owner | `extensions::immutable_owner` |
| Require CPI guard on a token account | `extensions::cpi_guard` |
| Require confidential transfer mint support | `extensions::confidential_transfer::mint` |
| Require confidential transfer account support | `extensions::confidential_transfer::account` |
| Require scaled UI amount config | `extensions::scaled_ui_amount::config` |
| Pin transfer-fee authorities | `extensions::transfer_fee_config::authority = X` |
| Inspect an extension outside the declarative subset | validate policy first, then use `find_extension(tlv, EXT_<NAME>)` |

## What still needs a separate CPI

Hopper's `hopper-token-2022` crate ships the common operation builders
(`Transfer`, `MintTo`, `Burn`, `CloseAccount`, `Approve`, `Revoke`,
`InitializeAccount`). **Creating extensions** (e.g. `InitializeTransferHook`,
`InitializeTransferFeeConfig`, `InitializeMetadataPointer`,
`InitializeNonTransferableMint`) is not yet wrapped, build those instructions
against the SPL Token-2022 program directly with a raw `InstructionView` /
`invoke`, then validate the result with Hopper's `extensions::*` constraints
and the `check_*`/`find_extension` TLV readers, which **are** shipped.

Wrapping the extension initializers is tracked future work; the validation
and screening surface is the part Hopper owns today.

## Resolving transfer-hook extra accounts

When a mint carries the `TransferHook` extension, a transfer must append the
extra accounts the hook program declares in its `ExtraAccountMetaList` PDA.
Hopper ships a `no_std`, zero-alloc resolver so you do not need
`spl-tlv-account-resolution`:

```rust
use hopper_token_2022::{extra_account_metas_pda, ExtraAccountMetaList, HookAccountBuf};

// PDA holding the list: ["extra-account-metas", mint] under the hook program.
let (metas_pda, _bump) = extra_account_metas_pda(mint.key(), hook_program.key());

let data = metas_account.try_borrow()?;
let list = ExtraAccountMetaList::unpack(&data)?;

let mut resolved = HookAccountBuf::<16>::new();
list.resolve_into(&mut resolved, instruction_data, hook_program.key(), &known_keys)?;
// `resolved.as_slice()` is the (address, is_signer, is_writable) set to append.
```

It resolves literal-pubkey (disc `0`), this-program PDA (disc `1`), and
external-program PDA (disc `≥128`) entries with `Literal`, `InstructionData`,
and `AccountKey` seeds. Seed kinds that need another account's bytes return
`HookError::UnsupportedSeed` so they are resolved explicitly rather than
silently mis-built.

## Gotchas

1. Extension constraints fire BEFORE the TLV scan confirms the account is Token-2022. Always pair an `extensions::*` check with a `token::token_program = TOKEN_2022_PROGRAM_ID` or `mint::token_program = TOKEN_2022_PROGRAM_ID` in the same field declaration, or the scan fails with `InvalidAccountData` when the account turns out to be legacy SPL.
2. `default_account_state` is validated as an integer byte, not as a named enum. Use `0`, `1`, or `2` directly.
3. Low-level presence readers accept an account-type byte of `0` during init sequencing. The strict mint safety gate requires the finalized `ACCOUNT_TYPE_MINT` (`0x01`) envelope.
4. Hopper defines the current official extension IDs through `PermissionedBurn` (`28`). A future unknown ID is inspectable as raw TLV data but is rejected by fail-closed allowlists until Hopper and the application explicitly review it.

## Worked example in the repo

`examples/hopper-token-2022-vault` is a complete vault program that mints a Token-2022-backed share token, enforces `non_transferable` on the share mint, and uses `extensions::mint_close_authority` to bind the close path to an admin key. It is the canonical reference for how the constraints compose.
