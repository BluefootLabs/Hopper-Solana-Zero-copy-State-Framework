//! Devnet-ready stablecoin payment ledger using Hopper account validation,
//! checked SPL/Token-2022 transfers, and SPL Memo CPI.

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

pub const PAYMENT_MEMO_MAX: usize = 96;

#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 72, version = 1)]
pub struct StablecoinMerchant {
    pub merchant: Address,
    pub stable_mint: Address,
    pub total_collected: WireU64,
    pub payment_count: WireU64,
    pub last_reference: [u8; 32],
}

hopper::hopper_error! {
    base = 6800;
    ZeroPaymentAmount,
    EmptyPaymentMemo,
    StableMintMismatch,
    TokenProgramMismatch,
    PaymentDecimalsMismatch,
    InsufficientPaymentFunds,
}

#[derive(Accounts)]
pub struct InitMerchant<'info> {
    #[account(mut)]
    pub merchant: Signer<'info>,

    #[account(init, payer = merchant, space = StablecoinMerchant::INIT_SPACE)]
    pub ledger: InitAccount<'info, StablecoinMerchant>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Pay<'info> {
    pub payer: Signer<'info>,

    #[account(mut, has_one = merchant)]
    pub ledger: Account<'info, StablecoinMerchant>,

    pub merchant: UncheckedAccount<'info>,

    #[account(mut)]
    pub payer_token: UncheckedAccount<'info>,

    #[account(mut)]
    pub merchant_token: UncheckedAccount<'info>,

    pub stable_mint: UncheckedAccount<'info>,
}

#[program]
mod stablecoin_memo_pay_program {
    use super::*;

    #[instruction(0)]
    pub fn init_merchant(ctx: Ctx<InitMerchant>, stable_mint: Address) -> ProgramResult {
        ctx.init_ledger()?;
        ctx.accounts.init(stable_mint)
    }

    #[instruction(1)]
    pub fn pay(
        ctx: Ctx<Pay>,
        amount: u64,
        decimals: u8,
        reference: [u8; 32],
        memo: HopperString<PAYMENT_MEMO_MAX>,
    ) -> ProgramResult {
        ctx.accounts.pay(amount, decimals, reference, memo)
    }
}

impl<'info> InitMerchant<'info> {
    pub fn init(&self, stable_mint: Address) -> ProgramResult {
        let mut ledger = self.ledger.get_mut_after_init()?;
        ledger.set_inner(*self.merchant.key(), stable_mint, 0, 0, [0u8; 32])
    }
}

impl<'info> Pay<'info> {
    pub fn pay(
        &self,
        amount: u64,
        decimals: u8,
        reference: [u8; 32],
        memo: HopperString<PAYMENT_MEMO_MAX>,
    ) -> ProgramResult {
        hopper::hopper_require!(amount > 0, ZeroPaymentAmount);
        hopper::hopper_require!(!memo.is_empty(), EmptyPaymentMemo);

        self.verify_payment_accounts(amount, decimals)?;

        interface_transfer_checked(
            self.payer_token.as_account(),
            self.stable_mint.as_account(),
            self.merchant_token.as_account(),
            self.payer.as_account(),
            amount,
            decimals,
        )?;

        let memo_signers = [self.payer.as_account()];
        Memo {
            signers: &memo_signers,
            memo: memo.as_bytes(),
            program_id: None,
        }
        .invoke()?;

        let mut ledger = self.ledger.get_mut()?;
        ledger.total_collected.checked_add_assign(amount)?;
        ledger.payment_count.checked_add_assign(1)?;
        ledger.last_reference = reference;
        Ok(())
    }

    fn verify_payment_accounts(&self, amount: u64, decimals: u8) -> ProgramResult {
        let ledger = self.ledger.get()?;
        if ledger.stable_mint != *self.stable_mint.key() {
            return Err(StableMintMismatch.into());
        }
        drop(ledger);

        let source_kind = TokenProgramKind::for_account(self.payer_token.as_account())?;
        let destination_kind = TokenProgramKind::for_account(self.merchant_token.as_account())?;
        let mint_kind = TokenProgramKind::for_account(self.stable_mint.as_account())?;

        if source_kind != destination_kind || source_kind != mint_kind {
            return Err(TokenProgramMismatch.into());
        }

        {
            let source_data = self.payer_token.try_borrow()?;
            let source = InterfaceTokenAccount::from_data(&source_data, source_kind)?;
            source.assert_initialized()?;
            source.assert_mint(self.stable_mint.key())?;
            source.assert_owner(self.payer.key())?;
            if source.amount()? < amount {
                return Err(InsufficientPaymentFunds.into());
            }
        }

        {
            let destination_data = self.merchant_token.try_borrow()?;
            let destination =
                InterfaceTokenAccount::from_data(&destination_data, destination_kind)?;
            destination.assert_initialized()?;
            destination.assert_mint(self.stable_mint.key())?;
            destination.assert_owner(self.merchant.key())?;
        }

        {
            let mint_data = self.stable_mint.try_borrow()?;
            let mint = InterfaceMint::from_data(&mint_data, mint_kind)?;
            mint.assert_initialized()?;
            if mint.decimals()? != decimals {
                return Err(PaymentDecimalsMismatch.into());
            }
        }

        Ok(())
    }
}
