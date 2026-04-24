//! # Anchor Parity Vault (R9)
//!
//! Implements the shared 4-instruction parity contract defined in
//! [`bench/METHODOLOGY.md`](../../bench/METHODOLOGY.md) using Anchor's
//! canonical authoring style: `#[program]` module, `#[derive(Accounts)]`
//! context structs, and `AccountLoader` for the zero-copy state
//! account on the `counter_access` instruction.
//!
//! This is the "real Anchor" baseline the bench harness was reserving
//! a column for via the optional `--anchor-root` flag. With this crate
//! present the bench can load the binary from
//! `bench/anchor-vault/target/deploy/anchor_vault.so` without needing
//! an external Anchor checkout.
//!
//! ## Program ID
//!
//! The `declare_id!` string below must base58-decode to the same 32
//! bytes the bench harness uses for `ANCHOR_PROGRAM_ID`. If you
//! re-roll the ID, update both sides or Mollusk will report a
//! `ProgramMismatch` error.
//!
//! ## Behaviour parity with hopper-parity-vault
//!
//! | Discriminator (first byte of ix data) | Name             | Contract |
//! |---------------------------------------|------------------|----------|
//! | `0` (Anchor uses 8-byte discriminator) | `deposit`        | user → vault via system CPI |
//! | `1`                                    | `withdraw`       | vault → user via direct lamport mutation |
//! | `2`                                    | `authorize`      | signer + PDA validation |
//! | `3`                                    | `counter_access` | authority check + u64 counter increment |
//!
//! **Important detail.** Anchor's default instruction dispatch uses an
//! 8-byte SHA-256 discriminator computed from the handler name
//! (`global:deposit`, etc.), not a single byte. The bench harness
//! currently sends a single-byte discriminator (`[0]`, `[1]`, `[2]`,
//! `[3]`) to stay consistent with the Hopper, Pinocchio, and Quasar
//! vaults, which all use single-byte dispatch. To keep Anchor inside
//! the same contract, this program installs an 8-byte Anchor
//! discriminator of `[N, 0, 0, 0, 0, 0, 0, 0]` for each variant (so
//! the single-byte `[0]` becomes `[0, 0, 0, 0, 0, 0, 0, 0, ...args]`
//! on the wire — compatible with a tweaked harness entry). See the
//! `discriminator` attribute on each handler. If you prefer to keep
//! Anchor's native SHA-256 discriminators, remove the explicit
//! `discriminator = ...` attributes and update the harness's
//! `deposit_instruction` / `withdraw_instruction` /
//! `authorize_instruction` / `counter_access_instruction` builders to
//! emit the correct 8-byte prefixes.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::{program::invoke, system_instruction};

declare_id!("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS");

#[program]
pub mod anchor_vault {
    use super::*;

    /// Instruction 0: deposit `amount` lamports from `user` to `vault`.
    #[instruction(discriminator = &[0, 0, 0, 0, 0, 0, 0, 0])]
    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        // System-program transfer: user signs, vault receives.
        let ix = system_instruction::transfer(
            ctx.accounts.user.key,
            ctx.accounts.vault.key,
            amount,
        );
        invoke(
            &ix,
            &[
                ctx.accounts.user.to_account_info(),
                ctx.accounts.vault.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;
        Ok(())
    }

    /// Instruction 1: withdraw `amount` lamports from the program-owned
    /// vault to `user` via direct lamport mutation.
    #[instruction(discriminator = &[1, 0, 0, 0, 0, 0, 0, 0])]
    pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
        let vault_lamports = ctx.accounts.vault.lamports();
        if amount > vault_lamports {
            return err!(VaultError::InsufficientFunds);
        }

        // Direct lamport mutation. Anchor's `AccountInfo::try_borrow_mut_lamports`
        // is the idiomatic path when the program owns the source account
        // (no CPI needed because the program is the signer of the
        // implicit transfer).
        **ctx
            .accounts
            .vault
            .try_borrow_mut_lamports()? = vault_lamports - amount;
        **ctx
            .accounts
            .user
            .try_borrow_mut_lamports()? = ctx
            .accounts
            .user
            .lamports()
            .checked_add(amount)
            .ok_or(VaultError::ArithmeticOverflow)?;
        Ok(())
    }

    /// Instruction 2: signer + PDA gate, no balance mutation.
    #[instruction(discriminator = &[2, 0, 0, 0, 0, 0, 0, 0])]
    pub fn authorize(ctx: Context<Authorize>) -> Result<()> {
        // Everything already enforced by the `#[derive(Accounts)]`
        // constraints below (user is signer, vault is writable, PDA
        // seeds match).
        let _ = &ctx.accounts.vault;
        Ok(())
    }

    /// Instruction 3: increment the counter stored in the vault body.
    ///
    /// Uses `AccountLoader<CounterState>` — Anchor's canonical
    /// zero-copy reader — instead of the `Account<CounterState>`
    /// borsh path. This is the apples-to-apples comparison point
    /// against Hopper's `segment_mut::<WireU64>`.
    #[instruction(discriminator = &[3, 0, 0, 0, 0, 0, 0, 0])]
    pub fn counter_access(ctx: Context<CounterAccess>) -> Result<()> {
        let mut state = ctx.accounts.vault.load_mut()?;
        if state.authority != ctx.accounts.user.key.to_bytes() {
            return err!(VaultError::AuthorityMismatch);
        }
        state.counter = state
            .counter
            .checked_add(1)
            .ok_or(VaultError::ArithmeticOverflow)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Zero-copy account definition (Anchor's equivalent of Hopper's
// hopper_layout!). The `#[account(zero_copy)]` attribute derives
// bytemuck::{Pod, Zeroable} and installs Anchor's zero-copy
// discriminator + header handling.
// ---------------------------------------------------------------------------

/// 40-byte vault body: `[authority: [u8; 32]][counter: u64 LE]`.
/// Matches hopper-parity-vault's counter-access layout.
#[account(zero_copy)]
#[repr(C)]
pub struct CounterState {
    pub authority: [u8; 32],
    pub counter: u64,
}

// ---------------------------------------------------------------------------
// Instruction context structs.
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    /// CHECK: vault PDA; seeds asserted via constraint below.
    #[account(
        mut,
        seeds = [b"vault", user.key().as_ref()],
        bump,
    )]
    pub vault: AccountInfo<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    /// CHECK: vault PDA; seeds asserted via constraint below.
    #[account(
        mut,
        seeds = [b"vault", user.key().as_ref()],
        bump,
    )]
    pub vault: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct Authorize<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    /// CHECK: vault PDA; seeds asserted via constraint below.
    #[account(
        mut,
        seeds = [b"vault", user.key().as_ref()],
        bump,
    )]
    pub vault: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct CounterAccess<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(
        mut,
        seeds = [b"vault", user.key().as_ref()],
        bump,
    )]
    pub vault: AccountLoader<'info, CounterState>,
}

// ---------------------------------------------------------------------------
// Errors.
// ---------------------------------------------------------------------------

#[error_code]
pub enum VaultError {
    #[msg("Insufficient funds in vault")]
    InsufficientFunds,
    #[msg("Arithmetic overflow")]
    ArithmeticOverflow,
    #[msg("Stored authority does not match signer")]
    AuthorityMismatch,
}
