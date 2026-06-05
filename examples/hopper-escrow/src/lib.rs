//! # Hopper Escrow Example
//!
//! Macro-first token escrow sketch using Hopper's first-touch account API.
//!
//! Instructions:
//! - `0` = Make (create escrow offer)
//! - `1` = Take (accept escrow)
//! - `2` = Cancel (reclaim escrowed tokens)

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code, unused_variables)]

use hopper::prelude::*;
use hopper::systems::{Authority, Mint, Token, TypedAddress};

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

// --- State ----------------------------------------------------------

#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 2, version = 1)]
pub struct Escrow {
    pub maker: TypedAddress<Authority>,
    pub maker_ta: TypedAddress<Token>,
    pub mint_a: TypedAddress<Mint>,
    pub mint_b: TypedAddress<Mint>,
    pub amount_offered: WireU64,
    pub amount_wanted: WireU64,
    pub bump: u8,
}

// --- Errors ---------------------------------------------------------

hopper::hopper_error! {
    base = 6100;
    MintMismatch,
    AmountMismatch,
    EscrowUnauthorized,
    EscrowAlreadyFilled,
    ZeroEscrowAmount,
}

// --- Contexts -------------------------------------------------------

#[derive(Accounts)]
pub struct Make<'info> {
    #[account(mut)]
    pub maker: Signer<'info>,

    #[account(init, payer = maker, space = Escrow::INIT_SPACE)]
    pub escrow: InitAccount<'info, Escrow>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Take<'info> {
    pub taker: Signer<'info>,

    #[account(mut, has_one = maker)]
    pub escrow: Account<'info, Escrow>,

    pub maker: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct Cancel<'info> {
    pub maker: Signer<'info>,

    #[account(mut, has_one = maker)]
    pub escrow: Account<'info, Escrow>,
}

#[program]
mod escrow_program {
    use super::*;

    #[instruction(0)]
    pub fn make(
        ctx: Ctx<Make>,
        mint_a: Address,
        mint_b: Address,
        amount_offered: u64,
        amount_wanted: u64,
    ) -> ProgramResult {
        ctx.init_escrow()?;
        ctx.accounts
            .make(mint_a, mint_b, amount_offered, amount_wanted)
    }

    #[instruction(1)]
    pub fn take(ctx: Ctx<Take>) -> ProgramResult {
        ctx.accounts.take()
    }

    #[instruction(2)]
    pub fn cancel(ctx: Ctx<Cancel>) -> ProgramResult {
        ctx.accounts.cancel()
    }
}

impl<'info> Make<'info> {
    pub fn make(
        &self,
        mint_a: Address,
        mint_b: Address,
        amount_offered: u64,
        amount_wanted: u64,
    ) -> ProgramResult {
        hopper::hopper_require!(amount_offered > 0, ZeroEscrowAmount);
        hopper::hopper_require!(amount_wanted > 0, ZeroEscrowAmount);

        let mut escrow = self.escrow.get_mut_after_init()?;
        escrow.set_inner(
            TypedAddress::from_account(self.maker.as_account()),
            TypedAddress::zeroed(),
            TypedAddress::from_slice(mint_a.as_array()),
            TypedAddress::from_slice(mint_b.as_array()),
            amount_offered,
            amount_wanted,
            0,
        )
    }
}

impl<'info> Take<'info> {
    pub fn take(&self) -> ProgramResult {
        let escrow = self.escrow.get()?;
        if escrow.amount_offered.get() == 0 {
            return Err(EscrowAlreadyFilled.into());
        }
        drop(escrow);

        hopper::hopper_close!(self.escrow.as_account(), self.maker.as_account())
    }
}

impl<'info> Cancel<'info> {
    pub fn cancel(&self) -> ProgramResult {
        hopper::hopper_close!(self.escrow.as_account(), self.maker.as_account())
    }
}
