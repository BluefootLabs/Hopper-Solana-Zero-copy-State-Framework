//! # Hopper Smoke Test
//!
//! A single macro-first program that exercises a broad slice of the
//! framework end-to-end, intended to be built to SBF and run on devnet:
//!
//! - `#[account]` versioned zero-copy layout with a self-describing header
//! - `#[derive(Accounts)]` constraints: `init`, `payer`, `space`, `mut`,
//!   `signer`, `has_one`, `close`
//! - `#[program]` dispatch with typed handler args
//! - System-program CPI (`Transfer`) for deposits
//! - Direct lamport debit for program-owned withdrawals
//! - Clock sysvar read via the native syscall path
//! - A typed, zero-alloc event emitted through `sol_log_data`
//! - Checked arithmetic everywhere (no silent overflow)
//! - `safe_close` semantics via the `close` constraint
//!
//! Instructions:
//! - `0` = Initialize (creates the vault account, stamps the clock)
//! - `1` = Deposit     (signer transfers SOL in, count++)
//! - `2` = Withdraw    (authority pulls SOL out, checked)
//! - `3` = Close       (authority closes the vault, lamports refunded)

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code, unused_variables)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

// --- State ----------------------------------------------------------

/// The vault account. Versioned so a future layout change is a typed,
/// fingerprinted migration rather than a silent reinterpretation.
#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 7, version = 1)]
pub struct Vault {
    /// Owner allowed to withdraw and close.
    pub authority: Address,
    /// Running lamport balance tracked in state (mirrors the account's
    /// own lamports; kept in state to exercise checked arithmetic).
    pub balance: WireU64,
    /// Number of deposits seen, ever.
    pub deposit_count: WireU32,
    /// Unix timestamp the vault was created (from the Clock sysvar).
    pub created_at: WireI64,
    /// Last slot a deposit landed.
    pub last_deposit_slot: WireU64,
}

// --- Event ----------------------------------------------------------

/// Emitted on every deposit. Align-1 wire types only, so it is a valid
/// zero-copy `Pod` payload for `emit_event`.
#[hopper::pod]
#[derive(Clone, Copy)]
#[repr(C)]
pub struct DepositEvent {
    pub amount: WireU64,
    pub new_balance: WireU64,
    pub deposit_count: WireU32,
    pub slot: WireU64,
}

// --- Errors ---------------------------------------------------------

hopper::hopper_error! {
    base = 6000;
    Unauthorized,
    InsufficientBalance,
    ZeroAmount,
}

// --- Contexts -------------------------------------------------------

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(init, payer = payer, space = Vault::INIT_SPACE)]
    pub vault: InitAccount<'info, Vault>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(mut, has_one = authority)]
    pub vault: Account<'info, Vault>,

    /// Required because `deposit` CPIs into the System program.
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(mut, has_one = authority)]
    pub vault: Account<'info, Vault>,
}

#[derive(Accounts)]
pub struct CloseVault<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(mut, has_one = authority, close = authority)]
    pub vault: Account<'info, Vault>,
}

// --- Program --------------------------------------------------------

#[program]
mod smoke_program {
    use super::*;

    #[instruction(0)]
    pub fn initialize(ctx: Ctx<Initialize>) -> ProgramResult {
        ctx.init_vault()?;
        ctx.accounts.initialize()
    }

    #[instruction(1)]
    pub fn deposit(ctx: Ctx<Deposit>, amount: u64) -> ProgramResult {
        ctx.accounts.deposit(amount)
    }

    #[instruction(2)]
    pub fn withdraw(ctx: Ctx<Withdraw>, amount: u64) -> ProgramResult {
        ctx.accounts.withdraw(amount)
    }

    #[instruction(3)]
    pub fn close(ctx: Ctx<CloseVault>) -> ProgramResult {
        // The `close = authority` constraint zeroes the data, stamps the
        // revival sentinel, and refunds lamports to the authority. Nothing
        // else to do in the body.
        Ok(())
    }
}

impl<'info> Initialize<'info> {
    pub fn initialize(&self) -> ProgramResult {
        // Read the on-chain clock so the vault records its birth time.
        let now = hopper::substrate::sysvar::get_clock()?;

        let mut vault = self.vault.get_mut_after_init()?;
        vault.authority = *self.payer.key();
        vault.balance = WireU64::new(0);
        vault.deposit_count = WireU32::new(0);
        vault.created_at = WireI64::new(now.unix_timestamp);
        vault.last_deposit_slot = WireU64::new(now.slot);
        Ok(())
    }
}

impl<'info> Deposit<'info> {
    pub fn deposit(&self, amount: u64) -> ProgramResult {
        hopper::hopper_require!(amount > 0, ZeroAmount);

        // Move the SOL in via the System program.
        let authority = self.authority.as_account();
        let vault_account = self.vault.as_account();
        hopper::system::Transfer {
            from: authority,
            to: vault_account,
            lamports: amount,
        }
        .invoke()?;

        let now = hopper::substrate::sysvar::get_clock()?;

        let (new_balance, count, slot) = {
            let mut vault = self.vault.get_mut()?;
            vault.balance.checked_add_assign(amount)?;
            vault.deposit_count.checked_add_assign(1)?;
            vault.last_deposit_slot = WireU64::new(now.slot);
            (vault.balance.get(), vault.deposit_count.get(), now.slot)
        };

        // Emit a typed, zero-alloc event for off-chain indexers.
        let event = DepositEvent {
            amount: WireU64::new(amount),
            new_balance: WireU64::new(new_balance),
            deposit_count: WireU32::new(count),
            slot: WireU64::new(slot),
        };
        hopper::systems::emit_event_tagged(1, &event)?;
        Ok(())
    }
}

impl<'info> Withdraw<'info> {
    pub fn withdraw(&self, amount: u64) -> ProgramResult {
        hopper::hopper_require!(amount > 0, ZeroAmount);

        {
            let mut vault = self.vault.get_mut()?;
            if vault.balance.get() < amount {
                return Err(InsufficientBalance.into());
            }
            vault.balance.checked_sub_assign(amount)?;
        }

        // The vault is program-owned, so debit lamports directly after
        // Hopper has validated authority (`has_one`) and layout.
        let authority = self.authority.as_account();
        let vault_account = self.vault.as_account();
        vault_account.set_lamports(
            vault_account
                .lamports()
                .checked_sub(amount)
                .ok_or(ProgramError::InsufficientFunds)?,
        )?;
        authority.set_lamports(
            authority
                .lamports()
                .checked_add(amount)
                .ok_or(ProgramError::ArithmeticOverflow)?,
        )?;
        Ok(())
    }
}
