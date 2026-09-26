//! Funded, all-or-nothing classic SPL Token escrow. See README for token policy.
#![cfg_attr(target_os = "solana", no_std)]
#![cfg_attr(not(target_os = "solana"), allow(dead_code))]
use hopper::prelude::*;
use hopper::token::{CloseAccount, InitializeAccount3, TransferChecked, TOKEN_PROGRAM_ID};
mod state;
mod validation;
pub use state::{Escrow, EscrowFields};
use validation::*;
pub const VAULT_SEED: &[u8] = b"escrow-vault";
#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}
hopper::hopper_error! {
    base = 6100;
    MintMismatch,
    AmountMismatch,
    EscrowUnauthorized,
    InvalidTokenAccount,
    ZeroEscrowAmount,
    UnsupportedMint,
    VaultMismatch,
    AliasedAccount,
}
#[derive(Accounts)]
pub struct Make<'info> {
    #[account(mut)]
    pub maker: Signer<'info>,
    #[account(init, signer, payer = maker, space = Escrow::INIT_SPACE)]
    pub escrow: InitAccount<'info, Escrow>,
    #[account(seeds = [VAULT_SEED, escrow.address().as_array()], bump)]
    pub vault_authority: UncheckedAccount<'info>,
    #[account(mut)]
    pub vault: Signer<'info>,
    pub mint_a: UncheckedAccount<'info>,
    pub mint_b: UncheckedAccount<'info>,
    #[account(mut)]
    pub maker_source: UncheckedAccount<'info>,
    pub maker_receive: UncheckedAccount<'info>,
    pub token_program: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}
#[derive(Accounts)]
pub struct Take<'info> {
    pub taker: Signer<'info>,
    #[account(mut, has_one = maker, has_one = vault, has_one = mint_a, has_one = mint_b, has_one = maker_receive)]
    pub escrow: Account<'info, Escrow>,
    #[account(mut)]
    pub maker: UncheckedAccount<'info>,
    #[account(seeds = [VAULT_SEED, escrow.address().as_array()], bump)]
    pub vault_authority: UncheckedAccount<'info>,
    #[account(mut)]
    pub vault: UncheckedAccount<'info>,
    pub mint_a: UncheckedAccount<'info>,
    pub mint_b: UncheckedAccount<'info>,
    #[account(mut)]
    pub taker_receive: UncheckedAccount<'info>,
    #[account(mut)]
    pub taker_source: UncheckedAccount<'info>,
    #[account(mut)]
    pub maker_receive: UncheckedAccount<'info>,
    #[account(mut)]
    pub maker_refund: UncheckedAccount<'info>,
    pub token_program: UncheckedAccount<'info>,
}
#[derive(Accounts)]
pub struct Cancel<'info> {
    #[account(mut)]
    pub maker: Signer<'info>,
    #[account(mut, has_one = maker, has_one = vault, has_one = mint_a)]
    pub escrow: Account<'info, Escrow>,
    #[account(seeds = [VAULT_SEED, escrow.address().as_array()], bump)]
    pub vault_authority: UncheckedAccount<'info>,
    #[account(mut)]
    pub vault: UncheckedAccount<'info>,
    pub mint_a: UncheckedAccount<'info>,
    #[account(mut)]
    pub maker_refund: UncheckedAccount<'info>,
    pub token_program: UncheckedAccount<'info>,
}
#[program]
mod escrow_program {
    use super::*;
    #[instruction(0)]
    pub fn make(ctx: Ctx<Make>, amount_offered: u64, amount_wanted: u64) -> ProgramResult {
        let a = &ctx.accounts;
        hopper::hopper_require!(amount_offered > 0 && amount_wanted > 0, ZeroEscrowAmount);
        verify_program(a.token_program.as_account())?;
        distinct(&[
            a.maker.key(),
            a.escrow.key(),
            a.vault_authority.key(),
            a.vault.key(),
            a.mint_a.key(),
            a.mint_b.key(),
            a.maker_source.key(),
            a.maker_receive.key(),
        ])?;
        let decimals = mint_decimals(a.mint_a.as_account())?;
        mint_decimals(a.mint_b.as_account())?;
        token_balance(
            a.maker_source.as_account(),
            a.mint_a.key(),
            a.maker.key(),
            false,
        )?;
        token_balance(
            a.maker_receive.as_account(),
            a.mint_b.key(),
            a.maker.key(),
            false,
        )?;
        ctx.init_escrow_with(EscrowFields {
            maker: *a.maker.key(),
            maker_receive: *a.maker_receive.key(),
            mint_a: *a.mint_a.key(),
            mint_b: *a.mint_b.key(),
            vault: *a.vault.key(),
            amount_offered,
            amount_wanted,
        })?;
        hopper::system::CreateAccount {
            from: a.maker.as_account(),
            to: a.vault.as_account(),
            lamports: hopper::hopper_runtime::rent::minimum_balance_live(165)?,
            space: 165,
            owner: &TOKEN_PROGRAM_ID,
        }
        .invoke()?;
        InitializeAccount3 {
            account: a.vault.as_account(),
            mint: a.mint_a.as_account(),
            owner: a.vault_authority.key(),
        }
        .invoke()?;
        TransferChecked {
            from: a.maker_source.as_account(),
            mint: a.mint_a.as_account(),
            to: a.vault.as_account(),
            authority: a.maker.as_account(),
            amount: amount_offered,
            decimals,
        }
        .invoke_strict()
    }
    #[instruction(1)]
    pub fn take(ctx: Ctx<Take>, amount_offered: u64, amount_wanted: u64) -> ProgramResult {
        let a = &ctx.accounts;
        verify_program(a.token_program.as_account())?;
        distinct(&[
            a.taker.key(),
            a.maker.key(),
            a.escrow.key(),
            a.vault_authority.key(),
            a.vault.key(),
            a.mint_a.key(),
            a.mint_b.key(),
            a.taker_receive.key(),
            a.taker_source.key(),
            a.maker_receive.key(),
            a.maker_refund.key(),
        ])?;
        let state = *a.escrow.get()?;
        hopper::hopper_require!(
            amount_offered == state.amount_offered.get()
                && amount_wanted == state.amount_wanted.get(),
            AmountMismatch
        );
        let decimals_a = mint_decimals(a.mint_a.as_account())?;
        let decimals_b = mint_decimals(a.mint_b.as_account())?;
        let balance = token_balance(
            a.vault.as_account(),
            a.mint_a.key(),
            a.vault_authority.key(),
            true,
        )?;
        hopper::hopper_require!(balance >= amount_offered, AmountMismatch);
        token_balance(
            a.taker_receive.as_account(),
            a.mint_a.key(),
            a.taker.key(),
            false,
        )?;
        token_balance(
            a.taker_source.as_account(),
            a.mint_b.key(),
            a.taker.key(),
            false,
        )?;
        token_balance(
            a.maker_receive.as_account(),
            a.mint_b.key(),
            a.maker.key(),
            false,
        )?;
        token_balance(
            a.maker_refund.as_account(),
            a.mint_a.key(),
            a.maker.key(),
            false,
        )?;
        // All data borrows end before CPI. Any later failure rolls back payment.
        TransferChecked {
            from: a.taker_source.as_account(),
            mint: a.mint_b.as_account(),
            to: a.maker_receive.as_account(),
            authority: a.taker.as_account(),
            amount: amount_wanted,
            decimals: decimals_b,
        }
        .invoke_strict()?;
        let bump = [ctx.bumps.vault_authority];
        let seeds = hopper::seeds!(VAULT_SEED, a.escrow.key().as_array(), &bump);
        let signers = [hopper::cpi::Signer::from(&seeds)];
        TransferChecked {
            from: a.vault.as_account(),
            mint: a.mint_a.as_account(),
            to: a.taker_receive.as_account(),
            authority: a.vault_authority.as_account(),
            amount: amount_offered,
            decimals: decimals_a,
        }
        .invoke_signed_strict(&signers)?;
        // Donations do not alter the quote or strand the vault.
        if balance > amount_offered {
            TransferChecked {
                from: a.vault.as_account(),
                mint: a.mint_a.as_account(),
                to: a.maker_refund.as_account(),
                authority: a.vault_authority.as_account(),
                amount: balance - amount_offered,
                decimals: decimals_a,
            }
            .invoke_signed_strict(&signers)?;
        }
        CloseAccount {
            account: a.vault.as_account(),
            destination: a.maker.as_account(),
            authority: a.vault_authority.as_account(),
        }
        .invoke_signed(&signers)?;
        hopper::hopper_close!(a.escrow.as_account(), a.maker.as_account())
    }
    #[instruction(2)]
    pub fn cancel(ctx: Ctx<Cancel>) -> ProgramResult {
        // No-argument handlers may inspect raw data; this ABI permits only
        // its discriminator, not an application-defined trailing payload.
        if ctx.instruction_data().len() != 1 {
            return Err(ProgramError::InvalidInstructionData);
        }
        let a = &ctx.accounts;
        verify_program(a.token_program.as_account())?;
        distinct(&[
            a.maker.key(),
            a.escrow.key(),
            a.vault_authority.key(),
            a.vault.key(),
            a.mint_a.key(),
            a.maker_refund.key(),
        ])?;
        let decimals = mint_decimals(a.mint_a.as_account())?;
        let balance = token_balance(
            a.vault.as_account(),
            a.mint_a.key(),
            a.vault_authority.key(),
            true,
        )?;
        token_balance(
            a.maker_refund.as_account(),
            a.mint_a.key(),
            a.maker.key(),
            false,
        )?;
        let bump = [ctx.bumps.vault_authority];
        let seeds = hopper::seeds!(VAULT_SEED, a.escrow.key().as_array(), &bump);
        let signers = [hopper::cpi::Signer::from(&seeds)];
        TransferChecked {
            from: a.vault.as_account(),
            mint: a.mint_a.as_account(),
            to: a.maker_refund.as_account(),
            authority: a.vault_authority.as_account(),
            amount: balance,
            decimals,
        }
        .invoke_signed_strict(&signers)?;
        CloseAccount {
            account: a.vault.as_account(),
            destination: a.maker.as_account(),
            authority: a.vault_authority.as_account(),
        }
        .invoke_signed(&signers)?;
        hopper::hopper_close!(a.escrow.as_account(), a.maker.as_account())
    }
}
hopper::program_manifest! { program = escrow_program, layouts = [Escrow], }
