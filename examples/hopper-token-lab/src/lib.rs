//! Token lab: one instruction per SPL Token / Token-2022 builder family
//! that Hopper ships, so that every builder's bytes and account order are
//! proven against the real programs on devnet, not only against a golden
//! byte string.
//!
//! Every instruction takes the executable token program as its last
//! account and derives the [`TokenProgram`] from it, so the same
//! instruction runs against SPL Token and Token-2022. See the README for
//! the lane the devnet runner drives.
#![cfg_attr(target_os = "solana", no_std)]
#![cfg_attr(not(target_os = "solana"), allow(dead_code))]

use hopper::prelude::*;
use hopper::token::{
    AmountToUiAmount, GetAccountDataSize, InitializeAccount3, InitializeImmutableOwner,
    InitializeMultisig2, MintConfig, MintPlan, MintToChecked, TokenBatch, TokenProgram,
    TransferChecked, UiAmountToAmount, UnwrapLamports, WithdrawExcessLamports, MAX_UI_AMOUNT_LEN,
};
use hopper::token_2022::extension_instructions::{Pause, Resume, UpdateScaledUiAmountMultiplier};
use hopper::token_2022::metadata_instructions::{
    EmitTokenMetadata, InitializeTokenGroup, InitializeTokenGroupMember, InitializeTokenMetadata,
    MetadataField, RemoveMetadataKey, UpdateMetadataAuthority, UpdateMetadataField,
};
use hopper::token_2022::MintExtension;

/// Token-2022 TLV type of the immutable-owner extension.
const EXT_IMMUTABLE_OWNER: u16 = 7;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

hopper::hopper_error! {
    base = 6500;
    RoundTripMismatch,
    UnknownExtensionMask,
    NotExecutable,
}

/// The token account layout every lane creates: base account plus the
/// immutable-owner extension when the program supports it.
pub const TOKEN_ACCOUNT_LEN: usize = 165;
/// An SPL Token multisig account.
pub const MULTISIG_LEN: usize = 355;

fn token_program(account: &AccountView<'_>) -> Result<TokenProgram, ProgramError> {
    account.check_executable()?;
    TokenProgram::from_program_account(account)
}

#[derive(Accounts)]
pub struct CreateMint<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut)]
    pub mint: Signer<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct ImmutableAccount<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut)]
    pub account: Signer<'info>,
    pub mint: UncheckedAccount<'info>,
    pub owner: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct MintTo<'info> {
    #[account(mut)]
    pub mint: UncheckedAccount<'info>,
    #[account(mut)]
    pub account: UncheckedAccount<'info>,
    pub authority: Signer<'info>,
    pub token_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct BatchRoundTrip<'info> {
    #[account(mut)]
    pub from: UncheckedAccount<'info>,
    pub mint: UncheckedAccount<'info>,
    #[account(mut)]
    pub to: UncheckedAccount<'info>,
    pub authority: Signer<'info>,
    pub token_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct UiAmountRoundTrip<'info> {
    pub mint: UncheckedAccount<'info>,
    pub token_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct WithdrawExcess<'info> {
    #[account(mut)]
    pub source: UncheckedAccount<'info>,
    #[account(mut)]
    pub destination: UncheckedAccount<'info>,
    pub authority: Signer<'info>,
    pub token_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct InitMultisig<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut)]
    pub multisig: Signer<'info>,
    pub member_a: UncheckedAccount<'info>,
    pub member_b: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct WrapAndUnwrap<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut)]
    pub account: Signer<'info>,
    pub native_mint: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct MintAuthority<'info> {
    #[account(mut)]
    pub mint: UncheckedAccount<'info>,
    pub authority: Signer<'info>,
    pub token_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct MetadataUpdate<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut)]
    pub mint: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct CreateGroupMember<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut)]
    pub member_mint: Signer<'info>,
    #[account(mut)]
    pub group: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: UncheckedAccount<'info>,
}

/// The bytes a `TokenMetadata` entry adds to a mint: the TLV header, the
/// update authority, the mint, three length-prefixed strings, and the
/// length of the (empty) list of additional fields.
const fn metadata_entry_len(name: usize, symbol: usize, uri: usize) -> usize {
    4 + 32 + 32 + (4 + name) + (4 + symbol) + (4 + uri) + 4
}

/// The bytes a `TokenGroup` entry adds: TLV header, update authority,
/// mint, size, max size.
const GROUP_ENTRY_LEN: usize = 4 + 32 + 32 + 8 + 8;
/// The bytes a `TokenGroupMember` entry adds: TLV header, mint, group,
/// member number.
const GROUP_MEMBER_ENTRY_LEN: usize = 4 + 32 + 32 + 8;

/// Token-2022 grows the mint to hold variable-length state and does not
/// fund it: top the mint up to the rent of its size plus `growth` first.
fn fund_growth(payer: &AccountView<'_>, mint: &AccountView<'_>, growth: usize) -> ProgramResult {
    let target = mint
        .data_len()
        .checked_add(growth)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    let needed = hopper::hopper_runtime::rent::minimum_balance_live(target)?;
    let missing = needed.saturating_sub(mint.lamports());
    if missing == 0 {
        return Ok(());
    }
    hopper::system::Transfer {
        from: payer,
        to: mint,
        lamports: missing,
    }
    .invoke()
}

/// Bits of the `create_extended_mint` mask, one per fixed-size extension.
pub const MASK_DEFAULT_ACCOUNT_STATE: u8 = 1 << 0;
pub const MASK_PAUSABLE: u8 = 1 << 1;
pub const MASK_SCALED_UI_AMOUNT: u8 = 1 << 2;
pub const MASK_INTEREST_BEARING: u8 = 1 << 3;
pub const MASK_GROUP_POINTER: u8 = 1 << 4;
pub const MASK_PERMISSIONED_BURN: u8 = 1 << 5;
pub const MASK_METADATA_POINTER: u8 = 1 << 6;
pub const MASK_GROUP_MEMBER_POINTER: u8 = 1 << 7;

/// The multiplier `create_extended_mint` installs and
/// `update_multiplier` replaces.
pub const INITIAL_MULTIPLIER: f64 = 2.0;
pub const INTEREST_RATE_BASIS_POINTS: i16 = 500;
pub const DECIMALS: u8 = 6;

#[program]
mod token_lab {
    use super::*;

    /// A plain mint with `DECIMALS` decimals on either token program;
    /// the payer is both authorities.
    #[instruction(0)]
    pub fn create_mint(ctx: Ctx<CreateMint>) -> ProgramResult {
        let a = &ctx.accounts;
        let program = token_program(a.token_program.as_account())?;
        let config = MintConfig {
            decimals: DECIMALS,
            mint_authority: a.payer.key(),
            freeze_authority: Some(a.payer.key()),
        };
        MintPlan::new(program, config, &[])?.create(a.payer.as_account(), a.mint.as_account(), &[])
    }

    /// A Token-2022 mint with the fixed-size extensions selected by
    /// `mask`, all authorities set to the payer.
    #[instruction(1)]
    pub fn create_extended_mint(ctx: Ctx<CreateMint>, mask: u8) -> ProgramResult {
        let a = &ctx.accounts;
        let program = token_program(a.token_program.as_account())?;
        let payer = a.payer.key();
        let mut extensions: [MintExtension<'_>; 8] = [MintExtension::NonTransferable; 8];
        let mut count = 0usize;
        let mut push = |extension| {
            extensions[count] = extension;
            count += 1;
        };
        if mask & MASK_DEFAULT_ACCOUNT_STATE != 0 {
            push(MintExtension::DefaultAccountState(1));
        }
        if mask & MASK_PAUSABLE != 0 {
            push(MintExtension::Pausable(payer));
        }
        if mask & MASK_SCALED_UI_AMOUNT != 0 {
            push(MintExtension::ScaledUiAmount {
                authority: Some(payer),
                multiplier: INITIAL_MULTIPLIER,
            });
        }
        if mask & MASK_INTEREST_BEARING != 0 {
            push(MintExtension::InterestBearing {
                rate_authority: Some(payer),
                rate: INTEREST_RATE_BASIS_POINTS,
            });
        }
        if mask & MASK_GROUP_POINTER != 0 {
            push(MintExtension::GroupPointer {
                authority: Some(payer),
                group_address: None,
            });
        }
        if mask & MASK_PERMISSIONED_BURN != 0 {
            push(MintExtension::PermissionedBurn(payer));
        }
        if mask & MASK_METADATA_POINTER != 0 {
            push(MintExtension::MetadataPointer {
                authority: Some(payer),
                metadata_address: None,
            });
        }
        if mask & MASK_GROUP_MEMBER_POINTER != 0 {
            push(MintExtension::GroupMemberPointer {
                authority: Some(payer),
                member_address: None,
            });
        }
        hopper::hopper_require!(mask != 0, UnknownExtensionMask);
        let config = MintConfig {
            decimals: DECIMALS,
            mint_authority: payer,
            freeze_authority: Some(payer),
        };
        MintPlan::new(program, config, &extensions[..count])?.create(
            a.payer.as_account(),
            a.mint.as_account(),
            &[],
        )
    }

    /// A token account for `mint` owned by `owner` whose owner can never
    /// change: the size comes from `GetAccountDataSize` (with the
    /// immutable-owner extension on Token-2022), then
    /// `InitializeImmutableOwner`, then `InitializeAccount3`. The size the
    /// program answered is the return data.
    #[instruction(2)]
    pub fn immutable_account(ctx: Ctx<ImmutableAccount>) -> ProgramResult {
        let a = &ctx.accounts;
        let program = token_program(a.token_program.as_account())?;
        let size = GetAccountDataSize {
            mint: a.mint.as_account(),
            extension_types: &[EXT_IMMUTABLE_OWNER],
        }
        .query(program)?;
        let space = usize::try_from(size).map_err(|_| ProgramError::InvalidAccountData)?;
        hopper::system::CreateAccount {
            from: a.payer.as_account(),
            to: a.account.as_account(),
            lamports: hopper::hopper_runtime::rent::minimum_balance_live(space)?,
            space: size,
            owner: program.address(),
        }
        .invoke()?;
        InitializeImmutableOwner {
            account: a.account.as_account(),
        }
        .invoke_on(program, &[], &[])?;
        InitializeAccount3 {
            account: a.account.as_account(),
            mint: a.mint.as_account(),
            owner: a.owner.key(),
        }
        .invoke_for_owner(&[], &[])?;
        hopper::return_data::set_return_data(&size.to_le_bytes());
        Ok(())
    }

    /// `MintToChecked` on whichever program owns the mint.
    #[instruction(3)]
    pub fn mint_to(ctx: Ctx<MintTo>, amount: u64, decimals: u8) -> ProgramResult {
        let a = &ctx.accounts;
        token_program(a.token_program.as_account())?;
        MintToChecked {
            mint: a.mint.as_account(),
            account: a.account.as_account(),
            mint_authority: a.authority.as_account(),
            amount,
            decimals,
        }
        .invoke_for_owner(&[], &[])
    }

    /// Two `TransferChecked`s, there and back, in one `Batch` CPI.
    #[instruction(4)]
    pub fn batch_round_trip(ctx: Ctx<BatchRoundTrip>, amount: u64, decimals: u8) -> ProgramResult {
        let a = &ctx.accounts;
        let program = token_program(a.token_program.as_account())?;
        let mint = a.mint.as_account();
        let authority = a.authority.as_account();
        let mut batch = TokenBatch::<64, 8>::new();
        batch.push(&TransferChecked {
            from: a.from.as_account(),
            mint,
            to: a.to.as_account(),
            authority,
            amount,
            decimals,
        })?;
        batch.push(&TransferChecked {
            from: a.to.as_account(),
            mint,
            to: a.from.as_account(),
            authority,
            amount,
            decimals,
        })?;
        batch.invoke_on(program, &[])
    }

    /// `AmountToUiAmount` then `UiAmountToAmount` on the string it
    /// returned; the raw amount must come back unchanged. The return data
    /// is the round-tripped amount followed by the UI string.
    #[instruction(5)]
    pub fn ui_amount_round_trip(ctx: Ctx<UiAmountRoundTrip>, amount: u64) -> ProgramResult {
        let a = &ctx.accounts;
        let program = token_program(a.token_program.as_account())?;
        let mut text = [0u8; MAX_UI_AMOUNT_LEN];
        let len = AmountToUiAmount {
            mint: a.mint.as_account(),
            amount,
        }
        .query(program, &mut text)?;
        let ui_amount =
            core::str::from_utf8(&text[..len]).map_err(|_| ProgramError::InvalidInstructionData)?;
        let back = UiAmountToAmount {
            mint: a.mint.as_account(),
            ui_amount,
        }
        .query(program)?;
        hopper::hopper_require!(back == amount, RoundTripMismatch);
        let mut out = [0u8; 8 + MAX_UI_AMOUNT_LEN];
        out[..8].copy_from_slice(&back.to_le_bytes());
        out[8..8 + len].copy_from_slice(&text[..len]);
        hopper::return_data::set_return_data(&out[..8 + len]);
        Ok(())
    }

    /// `WithdrawExcessLamports` from a token account to its owner.
    #[instruction(6)]
    pub fn withdraw_excess(ctx: Ctx<WithdrawExcess>) -> ProgramResult {
        let a = &ctx.accounts;
        let program = token_program(a.token_program.as_account())?;
        WithdrawExcessLamports {
            source: a.source.as_account(),
            destination: a.destination.as_account(),
            authority: a.authority.as_account(),
        }
        .invoke_on(program, &[], &[])
    }

    /// A 355-byte multisig of `member_a` and `member_b` needing `m`
    /// signatures, through `InitializeMultisig2`.
    #[instruction(7)]
    pub fn init_multisig(ctx: Ctx<InitMultisig>, m: u8) -> ProgramResult {
        let a = &ctx.accounts;
        let program = token_program(a.token_program.as_account())?;
        hopper::system::CreateAccount {
            from: a.payer.as_account(),
            to: a.multisig.as_account(),
            lamports: hopper::hopper_runtime::rent::minimum_balance_live(MULTISIG_LEN)?,
            space: MULTISIG_LEN as u64,
            owner: program.address(),
        }
        .invoke()?;
        InitializeMultisig2 {
            multisig: a.multisig.as_account(),
            signers: &[a.member_a.as_account(), a.member_b.as_account()],
            m,
        }
        .invoke_on(program, &[], &[])
    }

    /// Create a wrapped-SOL account holding `lamports` above rent, then
    /// `UnwrapLamports` `unwrap` of them back to the payer without closing
    /// the account.
    #[instruction(8)]
    pub fn wrap_and_unwrap(ctx: Ctx<WrapAndUnwrap>, lamports: u64, unwrap: u64) -> ProgramResult {
        let a = &ctx.accounts;
        let program = token_program(a.token_program.as_account())?;
        let rent = hopper::hopper_runtime::rent::minimum_balance_live(TOKEN_ACCOUNT_LEN)?;
        hopper::system::CreateAccount {
            from: a.payer.as_account(),
            to: a.account.as_account(),
            lamports: rent
                .checked_add(lamports)
                .ok_or(ProgramError::ArithmeticOverflow)?,
            space: TOKEN_ACCOUNT_LEN as u64,
            owner: program.address(),
        }
        .invoke()?;
        InitializeAccount3 {
            account: a.account.as_account(),
            mint: a.native_mint.as_account(),
            owner: a.payer.key(),
        }
        .invoke_on(program, &[], &[])?;
        UnwrapLamports {
            source: a.account.as_account(),
            destination: a.payer.as_account(),
            authority: a.payer.as_account(),
            amount: Some(unwrap),
        }
        .invoke_on(program, &[], &[])
    }

    /// `Pause` then `Resume` a pausable Token-2022 mint.
    #[instruction(9)]
    pub fn pause_resume(ctx: Ctx<MintAuthority>) -> ProgramResult {
        let a = &ctx.accounts;
        let program = token_program(a.token_program.as_account())?;
        hopper::hopper_require!(program == TokenProgram::Token2022, NotExecutable);
        Pause {
            mint: a.mint.as_account(),
            authority: a.authority.as_account(),
        }
        .invoke()?;
        Resume {
            mint: a.mint.as_account(),
            authority: a.authority.as_account(),
        }
        .invoke()
    }

    /// Schedule a new scaled-UI multiplier (the `f64` bits) effective now.
    #[instruction(10)]
    pub fn update_multiplier(ctx: Ctx<MintAuthority>, multiplier_bits: u64) -> ProgramResult {
        let a = &ctx.accounts;
        let program = token_program(a.token_program.as_account())?;
        hopper::hopper_require!(program == TokenProgram::Token2022, NotExecutable);
        UpdateScaledUiAmountMultiplier {
            mint: a.mint.as_account(),
            authority: a.authority.as_account(),
            multiplier: f64::from_bits(multiplier_bits),
            effective_timestamp: 0,
        }
        .invoke()
    }

    /// A Token-2022 mint that is its own metadata account: the metadata
    /// pointer names the mint, the mint is funded for the metadata it is
    /// about to hold, and `Initialize` writes name, symbol and URI.
    #[instruction(11)]
    pub fn create_metadata_mint(
        ctx: Ctx<CreateMint>,
        name: HopperString<32>,
        symbol: HopperString<10>,
        uri: HopperString<96>,
    ) -> ProgramResult {
        let a = &ctx.accounts;
        let program = token_program(a.token_program.as_account())?;
        hopper::hopper_require!(program == TokenProgram::Token2022, NotExecutable);
        let payer = a.payer.key();
        let mint = a.mint.as_account();
        let config = MintConfig {
            decimals: DECIMALS,
            mint_authority: payer,
            freeze_authority: Some(payer),
        };
        let extensions = [MintExtension::MetadataPointer {
            authority: Some(payer),
            metadata_address: Some(a.mint.key()),
        }];
        MintPlan::new(program, config, &extensions)?.create(a.payer.as_account(), mint, &[])?;
        let (name, symbol, uri) = (name.as_str()?, symbol.as_str()?, uri.as_str()?);
        fund_growth(
            a.payer.as_account(),
            mint,
            metadata_entry_len(name.len(), symbol.len(), uri.len()),
        )?;
        InitializeTokenMetadata {
            metadata: mint,
            update_authority: a.payer.as_account(),
            mint,
            mint_authority: a.payer.as_account(),
            name,
            symbol,
            uri,
        }
        .invoke()
    }

    /// Set an additional metadata key, then `Emit` the metadata: the
    /// transaction's return data is Token-2022's serialized metadata.
    #[instruction(12)]
    pub fn set_metadata_key(
        ctx: Ctx<MetadataUpdate>,
        key: HopperString<32>,
        value: HopperString<64>,
    ) -> ProgramResult {
        let a = &ctx.accounts;
        token_program(a.token_program.as_account())?;
        let mint = a.mint.as_account();
        let (key, value) = (key.as_str()?, value.as_str()?);
        // A new key adds both strings; an existing key only ever needs
        // less than that.
        fund_growth(a.payer.as_account(), mint, 4 + key.len() + 4 + value.len())?;
        UpdateMetadataField {
            metadata: mint,
            update_authority: a.payer.as_account(),
            field: MetadataField::Key(key),
            value,
        }
        .invoke()?;
        EmitTokenMetadata {
            metadata: mint,
            start: None,
            end: None,
        }
        .invoke()
    }

    /// Rename the token, remove an additional key, and give up the update
    /// authority; a second removal of the same key must be refused
    /// unless it is idempotent.
    #[instruction(13)]
    pub fn finalize_metadata(
        ctx: Ctx<MetadataUpdate>,
        name: HopperString<32>,
        key: HopperString<32>,
    ) -> ProgramResult {
        let a = &ctx.accounts;
        token_program(a.token_program.as_account())?;
        let mint = a.mint.as_account();
        let authority = a.payer.as_account();
        let (name, key) = (name.as_str()?, key.as_str()?);
        fund_growth(a.payer.as_account(), mint, name.len())?;
        UpdateMetadataField {
            metadata: mint,
            update_authority: authority,
            field: MetadataField::Name,
            value: name,
        }
        .invoke()?;
        RemoveMetadataKey {
            metadata: mint,
            update_authority: authority,
            idempotent: false,
            key,
        }
        .invoke()?;
        RemoveMetadataKey {
            metadata: mint,
            update_authority: authority,
            idempotent: true,
            key,
        }
        .invoke()?;
        UpdateMetadataAuthority {
            metadata: mint,
            current_authority: authority,
            new_authority: None,
        }
        .invoke()?;
        EmitTokenMetadata {
            metadata: mint,
            start: None,
            end: None,
        }
        .invoke()
    }

    /// A Token-2022 mint that is its own group of at most `max_size`
    /// members.
    #[instruction(14)]
    pub fn create_group(ctx: Ctx<CreateMint>, max_size: u64) -> ProgramResult {
        let a = &ctx.accounts;
        let program = token_program(a.token_program.as_account())?;
        hopper::hopper_require!(program == TokenProgram::Token2022, NotExecutable);
        let payer = a.payer.key();
        let mint = a.mint.as_account();
        let config = MintConfig {
            decimals: 0,
            mint_authority: payer,
            freeze_authority: None,
        };
        let extensions = [MintExtension::GroupPointer {
            authority: Some(payer),
            group_address: Some(a.mint.key()),
        }];
        MintPlan::new(program, config, &extensions)?.create(a.payer.as_account(), mint, &[])?;
        fund_growth(a.payer.as_account(), mint, GROUP_ENTRY_LEN)?;
        InitializeTokenGroup {
            group: mint,
            mint,
            mint_authority: a.payer.as_account(),
            update_authority: Some(payer),
            max_size,
        }
        .invoke()
    }

    /// A Token-2022 mint that is a member of `group`.
    #[instruction(15)]
    pub fn create_group_member(ctx: Ctx<CreateGroupMember>) -> ProgramResult {
        let a = &ctx.accounts;
        let program = token_program(a.token_program.as_account())?;
        hopper::hopper_require!(program == TokenProgram::Token2022, NotExecutable);
        let payer = a.payer.key();
        let mint = a.member_mint.as_account();
        let config = MintConfig {
            decimals: 0,
            mint_authority: payer,
            freeze_authority: None,
        };
        let extensions = [MintExtension::GroupMemberPointer {
            authority: Some(payer),
            member_address: Some(a.member_mint.key()),
        }];
        MintPlan::new(program, config, &extensions)?.create(a.payer.as_account(), mint, &[])?;
        fund_growth(a.payer.as_account(), mint, GROUP_MEMBER_ENTRY_LEN)?;
        InitializeTokenGroupMember {
            member: mint,
            member_mint: mint,
            member_mint_authority: a.payer.as_account(),
            group: a.group.as_account(),
            group_update_authority: a.payer.as_account(),
        }
        .invoke()
    }

    /// Deliberately construct a self-transfer inside a batch. Rejection
    /// must come from TokenBatch before a CPI, independently of Accounts.
    #[instruction(16)]
    pub fn batch_self_transfer(ctx: Ctx<MintTo>, amount: u64, decimals: u8) -> ProgramResult {
        let a = &ctx.accounts;
        let program = token_program(a.token_program.as_account())?;
        let mut batch = TokenBatch::<32, 4>::new();
        batch.push(&TransferChecked {
            from: a.account.as_account(),
            mint: a.mint.as_account(),
            to: a.account.as_account(),
            authority: a.authority.as_account(),
            amount,
            decimals,
        })?;
        batch.invoke_on(program, &[])
    }

    /// Exercise the hook resolver on caller-supplied bytes. This is a
    /// parser fixture, not a hook invocation or an account identity check.
    #[instruction(17)]
    pub fn resolve_hook_list(ctx: Ctx<UiAmountRoundTrip>, wire: [u8; 51]) -> ProgramResult {
        use hopper::token_2022::hook::{ExtraAccountMetaList, HookAccountBuf, HookError};
        let error = |e| {
            ProgramError::Custom(match e {
                HookError::InvalidDiscriminator => 6700,
                HookError::Truncated => 6701,
                HookError::BadCount => 6702,
                HookError::UnsupportedSeed => 6703,
                HookError::TooManySeeds => 6704,
                HookError::IndexOutOfRange => 6705,
                HookError::OutputFull => 6706,
                HookError::InvalidSeeds => 6707,
            })
        };
        let list = ExtraAccountMetaList::unpack(&wire).map_err(error)?;
        let mut out = HookAccountBuf::<1>::new();
        list.resolve_into(
            &mut out,
            &[0; 33],
            ctx.accounts.token_program.key(),
            &[*ctx.accounts.mint.key()],
        )
        .map_err(error)?;
        let first = out
            .as_slice()
            .first()
            .ok_or(ProgramError::InvalidArgument)?;
        hopper::return_data::set_return_data(first.address.as_array());
        Ok(())
    }
}
